//! WI-5: scoped `let`-binder rename raising.
//!
//! A `let` pair whose init is token-equal and whose binder ident changes
//! (`let count = E;` → `let n = E;`, typed twin with a token-equal
//! ascription) raises as the single holed pair the authored language
//! already completes in-block (WI-2b): the engine rewrites the
//! definition (`rename`) plus every unshadowed in-scope value use
//! (`follow`), so one hunk covers the def line and all its use lines.
//! The raiser never invents a use edit the native diff did not show.
//!
//! Soundness posture mirrors the let singles (`let_single.rs`):
//! `file_old` is required (no pre-image means no proof — shape `Miss`,
//! never section-visible emission); exactly-one-hit engine-count
//! totality (`template_pair_counts` + `engine_verdict` at `want = 1`, so
//! an untouched twin vetoes with its counts); follow-coverage honesty
//! (every engine-computed follow line must be a claimed use line, else
//! `Veto("unproven-use")`; every claimed use line must be an
//! engine-computed follow line, else `Veto("capture-risk")`); and the
//! T361 byte-compare pre-reservation (the candidate hunk applied to a
//! pre-image clone must reproduce exactly the native plus lines at the
//! claimed sites with every other line byte-identical).
//!
//! Non-goals (stay ordinary, pinned by tests): destructuring binder
//! renames (single- or multi-leaf — no binder-identity proof for the
//! raiser here), anti-unified (non-token-equal) inits, use lines the
//! diff did not edit, non-contiguous or out-of-block uses, shorthand
//! struct-literal uses (`S { old }` expands to `S { old: new }`, which
//! is not a whole-ident swap), dead binders with no uses in the diff
//! (N = 1 singles territory), and macro/opaque inits (`init_ok` gate).

use super::antiunify_types::tok;
use super::pat_template::site_spelling;
use super::prov::Counted;
use super::shape::init_ok;
use super::unique_rule;

/// Whole-ident occurrence helpers (ident-boundary gated, never
/// substring): the binder proof is keyed by binding, never by text
/// scan — these only gate the *diff-text* shape (the engine remains
/// the occurrence oracle via [`rsmpl_engine::let_binder_follow_edits`]).
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn contains_ident_bounded(hay: &str, needle: &str) -> bool {
    if needle.is_empty() || !hay.contains(needle) {
        return false;
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    let mut i = 0;
    while i + nb.len() <= hb.len() {
        if &hb[i..i + nb.len()] == nb {
            let prev_ok = i == 0 || !is_ident_byte(hb[i - 1]);
            let next_ok = i + nb.len() == hb.len() || !is_ident_byte(hb[i + nb.len()]);
            if prev_ok && next_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Replace whole-ident occurrences of `needle` with `replacement`,
/// returning the rewritten text plus whether at least one occurrence
/// was replaced.
fn replace_ident_bounded(hay: &str, needle: &str, replacement: &str) -> (String, bool) {
    if needle.is_empty() || !hay.contains(needle) {
        return (hay.to_string(), false);
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    let mut out = String::with_capacity(hay.len());
    let mut hit = false;
    let mut i = 0;
    while i < hb.len() {
        if i + nb.len() <= hb.len() && &hb[i..i + nb.len()] == nb {
            let prev_ok = i == 0 || !is_ident_byte(hb[i - 1]);
            let next_ok = i + nb.len() == hb.len() || !is_ident_byte(hb[i + nb.len()]);
            if prev_ok && next_ok {
                out.push_str(replacement);
                hit = true;
                i += nb.len();
                continue;
            }
        }
        let ch = hay[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, hit)
}

/// Parsed binder-def side: single plain ident binder plus optional
/// ascription, both sides structurally comparable.
struct DefSide {
    binder: String,
    is_mut: bool,
    ty: Option<syn::Type>,
    init: syn::Expr,
}

fn parse_def_side(line: &str) -> Option<DefSide> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    if !local.attrs.is_empty() {
        return None;
    }
    let init = local.init?;
    // `let-else` (`diverge`) is a different statement shape.
    if init.diverge.is_some() {
        return None;
    }
    // Plain ident or ascribed ident — destructuring never enters
    // binder identity here (single-leaf twins stay ordinary).
    let (pat, ty) = match local.pat {
        syn::Pat::Ident(ref pi) => {
            if pi.subpat.is_some() || pi.by_ref.is_some() {
                return None;
            }
            let name = pi.ident.to_string();
            if name.starts_with("__RSMPL_") {
                return None;
            }
            (pi, None)
        }
        syn::Pat::Type(ref pt) => {
            let syn::Pat::Ident(ref pi) = *pt.pat else {
                return None;
            };
            if pi.subpat.is_some() || pi.by_ref.is_some() {
                return None;
            }
            let name = pi.ident.to_string();
            if name.starts_with("__RSMPL_") {
                return None;
            }
            (pi, Some((*pt.ty).clone()))
        }
        _ => return None,
    };
    if !init_ok(&init.expr) {
        return None;
    }
    Some(DefSide {
        binder: pat.ident.to_string(),
        is_mut: pat.mutability.is_some(),
        ty,
        init: (*init.expr).clone(),
    })
}

/// Pre-image-free binder-def gate for the subhunk fusion scan:
/// `Some((old, new))` iff the pair is a plain binder rename with a
/// token-equal init (typed: token-equal ascription too).
pub(crate) fn is_binder_def_pair(minus: &str, plus: &str) -> Option<(String, String)> {
    let m = parse_def_side(minus)?;
    let p = parse_def_side(plus)?;
    if m.binder == p.binder || m.is_mut != p.is_mut {
        return None;
    }
    if tok(&m.init) != tok(&p.init) {
        return None;
    }
    match (&m.ty, &p.ty) {
        (None, None) => {}
        (Some(mt), Some(pt)) => {
            if tok(mt) != tok(pt) {
                return None;
            }
        }
        _ => return None,
    }
    Some((m.binder, p.binder))
}

/// Use-pair gate: `plus` must equal `minus` under whole-ident
/// `old → new` replacement with at least one replacement (the line
/// carries a genuine use of the renamed binder, nothing else), and the
/// line must not bind `old` itself (a shadowing `let old` is a second
/// rename, never a use).
pub(crate) fn is_binder_use_pair(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    if minus == plus || old.is_empty() || new.is_empty() || old == new {
        return false;
    }
    let (rewritten, hit) = replace_ident_bounded(minus, old, new);
    if !hit || rewritten != plus {
        return false;
    }
    // A `let` line whose own pattern mentions `old` binds it (shadow),
    // it does not use the outer binder.
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if contains_ident_bounded(&tok(&local.pat), old) {
            return false;
        }
    }
    true
}

/// WI-5 binder window: exactly `[binder-def + 1..N use pairs]`.
/// Returns the single holed pair emission (full section text, diff
/// header included) with the engine's in-block follow covering the use
/// lines. Every decline arm is counted (`Totality`/`Opaque`) or named
/// (`Veto("capture-risk" | "unproven-use" | "ambiguous-binder")`);
/// anything else is shape `Miss`.
pub(crate) fn try_let_binder_window(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 || changed.len() != minuses.len() {
        return Counted::Miss;
    }
    let Some(pre) = file_old else {
        return Counted::Miss;
    };
    // Contiguous old-side sites (callers hand within-hunk runs or
    // whole sections; a gapped claim is never one intent).
    for w in changed.windows(2) {
        if w[1] != w[0] + 1 {
            return Counted::Miss;
        }
    }
    let (old, new) = match is_binder_def_pair(minuses[0], pluses[0]) {
        Some(pair) => pair,
        None => return Counted::Miss,
    };
    for (m, p) in minuses.iter().zip(pluses.iter()).skip(1) {
        if !is_binder_use_pair(m, p, &old, &new) {
            return Counted::Miss;
        }
    }
    // Emission spelling: concrete binders, byte-exact ascription slice,
    // holed init — the authored `rename_let` diet form.
    let m_def = parse_def_side(minuses[0]).expect("def pair re-parses (gated above)");
    let mut_binder = |b: &str| {
        if m_def.is_mut {
            format!("mut {b}")
        } else {
            b.to_string()
        }
    };
    let ty_suffix = match &m_def.ty {
        None => String::new(),
        Some(ty) => format!(": {}", site_spelling(minuses[0], ty, &tok(ty))),
    };
    let minus_tpl = format!("let {} = E;", mut_binder(&old));
    let plus_tpl = format!("let {} = E;", mut_binder(&new));
    let (minus_tpl, plus_tpl) = if ty_suffix.is_empty() {
        (minus_tpl, plus_tpl)
    } else {
        (
            format!("let {}{} = E;", mut_binder(&old), ty_suffix),
            format!("let {}{} = E;", mut_binder(&new), ty_suffix),
        )
    };
    // Exactly-one-hit engine-count totality (an untouched twin def
    // vetoes with its counts, never a silent second rewrite).
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), &minus_tpl, &plus_tpl),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => {
            return Counted::Opaque;
        }
        crate::totality_counted::FamilyVerdict::Unproven => {
            return Counted::Miss;
        }
    }
    // Follow-coverage honesty: the engine is the occurrence oracle.
    // Every follow line must be a claimed use line (never invent an
    // edit the diff did not show); every claimed use line must be a
    // follow line (a shadowed/macro/nested-fn/out-of-block use the
    // proof cannot own vetoes as capture-risk).
    let Ok(parsed) = syn::parse_str::<syn::File>(pre) else {
        return Counted::Miss;
    };
    let index = rsmpl_engine::LineIndex::new(pre);
    let def_line = changed[0] as u32;
    let (Some(bs), be) = (
        index.line_start(def_line),
        index.line_start(def_line + 1).unwrap_or(pre.len()),
    ) else {
        return Counted::Miss;
    };
    let follows = rsmpl_engine::let_binder_follow_edits(&parsed, pre, &index, bs, be, &old, &new);
    let mut follow_lines: Vec<usize> = follows
        .iter()
        .map(|e| index.line_of(e.start) as usize)
        .collect();
    follow_lines.sort_unstable();
    follow_lines.dedup();
    let use_lines: Vec<usize> = changed[1..].to_vec();
    if follow_lines.iter().any(|l| !use_lines.contains(l)) {
        return Counted::Veto("unproven-use");
    }
    if use_lines.iter().any(|l| !follow_lines.contains(l)) {
        return Counted::Veto("capture-risk");
    }
    // T361 pre-reservation: the candidate hunk applied to a pre-image
    // clone must reproduce exactly the native plus lines at the claimed
    // sites with every other line byte-identical.
    let rule = unique_rule(used_rules, "let_binder");
    let header = crate::emission_header(path, &rule, &changed[..1]);
    let mut hunk = header.replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    let Ok(patch) = rsmpl_patch::parse_patch(&hunk) else {
        return Counted::Miss;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &patch) else {
        return Counted::Veto("ambiguous-binder");
    };
    if report.rules.iter().map(|r| r.matches).sum::<usize>() != 1 {
        return Counted::Veto("ambiguous-binder");
    }
    let Some(got) = files.get(path) else {
        return Counted::Miss;
    };
    let mut expected: Vec<&str> = pre.split('\n').collect();
    let all_pluses: Vec<&&str> = pluses.iter().collect();
    for (i, line_no) in changed.iter().enumerate() {
        if line_no < &1 || *line_no > expected.len() {
            return Counted::Miss;
        }
        expected[line_no - 1] = all_pluses[i];
    }
    if *got != expected.join("\n") {
        return Counted::Veto("capture-risk");
    }
    Counted::Fire(hunk)
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    const PRE: &str = "fn scale(values: Vec<u32>) -> u32 {\n    let count = values.len() as u32;\n    let bumped = count + 1;\n    bumped\n}\n";

    fn binder_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let count = values.len() as u32;\n-    let bumped = count + 1;\n+    let n = values.len() as u32;\n+    let bumped = n + 1;\n"
            .to_string()
    }

    #[test]
    fn binder_window_fires_verified_byte_identical() {
        let raw = section("f.rs", &binder_body());
        let pre = pre_map(&[("f.rs", PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("binder window must fire");
        validated_live(&out, &pre);
        assert!(out.contains("- let count = E;\n"), "got: {out}");
        assert!(out.contains("+ let n = E;\n"), "got: {out}");
        assert!(!out.contains("let bumped"), "use rides follow, got: {out}");
        assert_eq!(out.matches("rule=").count(), 1, "one intent, got: {out}");
    }

    #[test]
    fn binder_window_typed_fires() {
        let pre_text =
            "fn f() -> i32 {\n    let count: u32 = 1;\n    let bumped: u32 = count + 1;\n    bumped as i32\n}\n";
        let body = "@@ -2,2 +2,2 @@\n-    let count: u32 = 1;\n-    let bumped: u32 = count + 1;\n+    let n: u32 = 1;\n+    let bumped: u32 = n + 1;\n"
            .to_string();
        let raw = section("f.rs", &body);
        let pre = pre_map(&[("f.rs", pre_text)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("typed binder window must fire");
        validated_live(&out, &pre);
        assert!(out.contains("- let count: u32 = E;\n"), "got: {out}");
        assert!(out.contains("+ let n: u32 = E;\n"), "got: {out}");
    }

    #[test]
    fn binder_window_twin_def_totality() {
        // Totality: a second untouched `let count` matches the holed
        // minus — the window vetoes with its counts.
        let pre_text = "fn f() {\n    let count = 1;\n    let bumped = count + 1;\n}\nfn g() {\n    let count = 2;\n    let _ = count;\n}\n";
        assert!(matches!(
            super::try_let_binder_window(
                &["    let count = 1;", "    let bumped = count + 1;"],
                &["    let n = 1;", "    let bumped = n + 1;"],
                "f.rs",
                Some(pre_text),
                &[2, 3],
                &mut Vec::new(),
            ),
            crate::semantic::prov::Counted::Totality { found: 2, want: 1 }
        ));
    }

    #[test]
    fn binder_window_shadow_vetoes_capture_risk() {
        // A closure-param shadow on the use line: the diff rewrites a
        // use the engine proof cannot own (param + body name the inner
        // binder), so the window vetoes instead of mis-resolving.
        let pre_text =
            "fn f() {\n    let count = 1;\n    let g = |count| count + 1;\n    let _ = g;\n}\n";
        assert!(matches!(
            super::try_let_binder_window(
                &["    let count = 1;", "    let g = |count| count + 1;"],
                &["    let n = 1;", "    let g = |n| n + 1;"],
                "f.rs",
                Some(pre_text),
                &[2, 3],
                &mut Vec::new(),
            ),
            crate::semantic::prov::Counted::Veto("capture-risk")
        ));
    }

    #[test]
    fn binder_window_single_pair_misses() {
        // N = 1 is singles territory, never this window (a dead binder
        // with no uses in the diff stays ordinary this slice).
        let pre_text =
            "fn f() {\n    let count = 1;\n    let bumped = count + 1;\n    let _ = count;\n}\n";
        assert!(super::try_let_binder_window(
            &["    let count = 1;"],
            &["    let n = 1;"],
            "f.rs",
            Some(pre_text),
            &[2],
            &mut Vec::new(),
        )
        .is_miss());
    }

    #[test]
    fn binder_window_unedited_use_vetoes_unproven_use() {
        // The diff covers the def and one use but leaves a second live
        // use untouched: the follow would invent an edit the diff did
        // not show, so the window vetoes.
        let pre_text =
            "fn f() {\n    let count = 1;\n    let bumped = count + 1;\n    let c2 = count * 2;\n}\n";
        assert!(matches!(
            super::try_let_binder_window(
                &["    let count = 1;", "    let bumped = count + 1;"],
                &["    let n = 1;", "    let bumped = n + 1;"],
                "f.rs",
                Some(pre_text),
                &[2, 3],
                &mut Vec::new(),
            ),
            crate::semantic::prov::Counted::Veto("unproven-use")
        ));
    }

    #[test]
    fn binder_window_unchanged_third_pair_misses() {
        // A pair with no delta is not a use: the window shape declines
        // instead of claiming lines it cannot explain.
        let pre_text =
            "fn f() {\n    let count = 1;\n    let bumped = count + 1;\n    let c2 = count * 2;\n}\n";
        assert!(super::try_let_binder_window(
            &[
                "    let count = 1;",
                "    let bumped = count + 1;",
                "    let c2 = count * 2;"
            ],
            &[
                "    let n = 1;",
                "    let bumped = n + 1;",
                "    let c2 = count * 2;"
            ],
            "f.rs",
            Some(pre_text),
            &[2, 3, 4],
            &mut Vec::new(),
        )
        .is_miss());
    }

    #[test]
    fn binder_window_gapped_sites_miss() {
        // Non-contiguous sites are never one intent (the shadow-def
        // case below would otherwise strand a middle line).
        let pre_text = "fn f() {\n    let count = 1;\n    {\n        let count = 9;\n        let inner = count + 1;\n    }\n}\n";
        assert!(super::try_let_binder_window(
            &["    let count = 1;", "        let inner = count + 1;"],
            &["    let n = 1;", "        let inner = n + 1;"],
            "f.rs",
            Some(pre_text),
            &[2, 5],
            &mut Vec::new(),
        )
        .is_miss());
    }

    #[test]
    fn binder_window_declines_matrix() {
        let pre = PRE;
        // Init change: not a rename (another detector's shape).
        assert!(super::try_let_binder_window(
            &["    let count = 1;", "    let bumped = count + 1;"],
            &["    let n = 2;", "    let bumped = n + 1;"],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_miss());
        // Same binder: not a rename.
        assert!(super::try_let_binder_window(
            &["    let count = 1;", "    let bumped = count + 1;"],
            &["    let count = 1;", "    let bumped = count + 2;"],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_miss());
        // Destructuring binder: out of scope this slice.
        assert!(super::try_let_binder_window(
            &["    let (a, count) = t;", "    let bumped = count + 1;"],
            &["    let (a, n) = t;", "    let bumped = n + 1;"],
            "f.rs",
            Some(
                "fn f(t: (i32, i32)) {\n    let (a, count) = t;\n    let bumped = count + 1;\n}\n"
            ),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_miss());
        // Macro init: opaque interior, never holed.
        assert!(super::try_let_binder_window(
            &["    let count = vec![1];", "    let bumped = count.len();"],
            &["    let n = vec![1];", "    let bumped = n.len();"],
            "f.rs",
            Some("fn f() {\n    let count = vec![1];\n    let bumped = count.len();\n}\n"),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_miss());
        // No pre-image: no proof, no emission.
        assert!(super::try_let_binder_window(
            &[
                "    let count = values.len() as u32;",
                "    let bumped = count + 1;"
            ],
            &[
                "    let n = values.len() as u32;",
                "    let bumped = n + 1;"
            ],
            "f.rs",
            None,
            &[2, 3],
            &mut Vec::new(),
        )
        .is_miss());
        // Single pair: N = 1 singles territory, never this window.
        assert!(super::try_let_binder_window(
            &["    let count = values.len() as u32;"],
            &["    let n = values.len() as u32;"],
            "f.rs",
            Some(pre),
            &[2],
            &mut Vec::new(),
        )
        .is_miss());
    }

    #[test]
    fn binder_def_gate_matrix() {
        // The pre-image-free gate admits only plain renames.
        assert_eq!(
            super::is_binder_def_pair(
                "    let count = values.len() as u32;",
                "    let n = values.len() as u32;"
            ),
            Some(("count".to_string(), "n".to_string()))
        );
        assert_eq!(
            super::is_binder_def_pair("    let count: u32 = 1;", "    let n: u32 = 1;"),
            Some(("count".to_string(), "n".to_string()))
        );
        // Same binder, init change, type change, mut change: declined.
        assert!(super::is_binder_def_pair("    let count = 1;", "    let count = 1;").is_none());
        assert!(super::is_binder_def_pair("    let count = 1;", "    let n = 2;").is_none());
        assert!(
            super::is_binder_def_pair("    let count: u32 = 1;", "    let n: u64 = 1;").is_none()
        );
        assert!(super::is_binder_def_pair("    let count = 1;", "    let mut n = 1;").is_none());
        // Destructuring: declined this slice.
        assert!(
            super::is_binder_def_pair("    let (a, count) = t;", "    let (a, n) = t;").is_none()
        );
        // Use pairs: clean swaps admit, anything else declines.
        assert!(super::is_binder_use_pair(
            "    let bumped = count + 1;",
            "    let bumped = n + 1;",
            "count",
            "n"
        ));
        assert!(!super::is_binder_use_pair(
            "    let bumped = count + 1;",
            "    let bumped = count + 2;",
            "count",
            "n"
        ));
        assert!(!super::is_binder_use_pair(
            "    let counter = 1;",
            "    let counter = 1;",
            "count",
            "n"
        ));
        // Substring trap: `counter` must not read as a use of `count`.
        assert!(!super::is_binder_use_pair(
            "    let counter = count + 1;",
            "    let nter = count + 1;",
            "count",
            "n"
        ));
    }

    #[test]
    fn subhunk_fuses_fixture12_shape() {
        // The fused-header 3-pair hunk: the header pair raises as its
        // type single, binder+use fuse into one `let_binder` intent,
        // zero ordinary remainder, byte-identical post tree.
        let pre = PRE;
        let post = "fn scale(values: Box<[u32]>) -> u32 {\n    let n = values.len() as u32;\n    let bumped = n + 1;\n    bumped\n}\n";
        let hunk = "@@ -1,3 +1,3 @@\n-fn scale(values: Vec<u32>) -> u32 {\n-    let count = values.len() as u32;\n-    let bumped = count + 1;\n+fn scale(values: Box<[u32]>) -> u32 {\n+    let n = values.len() as u32;\n+    let bumped = n + 1;\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = super::super::subhunk::try_subhunk_partition(
            hunk,
            "f.rs",
            Some(pre),
            &mut used,
            &mut prov,
            &std::collections::HashSet::new(),
        )
        .expect("fixture-12 shape must partition");
        assert!(out.contains("- let count = E;\n"), "got: {out}");
        assert!(out.contains("+ let n = E;\n"), "got: {out}");
        assert!(
            !out.contains("-    let count"),
            "binder rides follow, got: {out}"
        );
        assert!(
            !out.contains("-    let bumped"),
            "use rides follow, got: {out}"
        );
        // Two semantic hunks (type single + binder), no ordinary rest.
        assert_eq!(out.matches("rule=").count(), 2, "got: {out}");
        let entry = prov
            .iter()
            .find(|p| p.detector == "let_binder")
            .expect("let_binder provenance, got: {prov:?}");
        // `sites` is emitted-range coverage (the def line); the use
        // line rides the engine follow (no ordinary remainder above
        // proves it is claimed).
        assert_eq!(entry.sites, vec![2], "got: {entry:?}");
        // Byte-compare: the fragment (under its file header, as the
        // per-hunk caller emits it) applied to pre rebuilds post.
        let section = format!("{}\n{out}", crate::git_header("f.rs"));
        let parsed = rsmpl_patch::parse_patch(&section).expect("fragment must parse");
        let mut files = pre_map(&[("f.rs", pre)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("fragment must apply");
        assert_eq!(files["f.rs"], post, "post tree differs");
    }

    #[test]
    fn fixture_body_stays_whole_until_subhunk() {
        // The 3-pair fused-header shape declines whole (header pair is
        // not binder territory); the subhunk fusion owns the split.
        let pre = PRE;
        assert!(super::try_let_binder_window(
            &[
                "fn scale(values: Vec<u32>) -> u32 {",
                "    let count = values.len() as u32;",
                "    let bumped = count + 1;"
            ],
            &[
                "fn scale(values: Box<[u32]>) -> u32 {",
                "    let n = values.len() as u32;",
                "    let bumped = n + 1;"
            ],
            "f.rs",
            Some(pre),
            &[1, 2, 3],
            &mut Vec::new(),
        )
        .is_miss());
    }
}
