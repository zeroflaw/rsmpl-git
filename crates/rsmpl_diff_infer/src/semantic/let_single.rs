//! WI-7 slice 2: sequence/pattern N=1 templated singles.
//!
//! Single-pair (or single-window) shapes that re-infer as holed
//! templates instead of the literal verbatim floor: a typed method
//! wrap-add (`let P0: T0 = E0;` → `let P0: T0 = E0.wrapping_add(1);`,
//! fixture `98`), an untyped method wrap-add with a preserved pattern
//! (`let Some(P0) = E0;` → `let Some(P0) = E0.unwrap_or(0);`,
//! fixtures `73`/`84`), a tuple-position swap (`let (P0, P1) = E0;` →
//! `let (P1, P0) = E0;`, fixture `72`), and one hetero two-line
//! window (pure type-add + typed method-wrap, fixture `107`) as a
//! single `single_block`-family intent.
//!
//! Soundness posture mirrors slice 1 (`repeat.rs`): `file_old` is
//! required (uniqueness needs the whole file — section-visible
//! fallback stays ordinary), exactly-one-hit engine-count totality
//! (`template_pair_holds` / `template_sequence_holds` at n = 1, so an
//! untouched twin vetoes), and the T361 byte-compare pre-reservation
//! (`single_reproduces` for K = 1, the local `window_reproduces`
//! twin for K = 2). Rule names are reserved only for emissions that
//! reproduce. Detector names reuse the multi-site families
//! (`let_template` for the K = 1 arms, `single_block` for the
//! window), and `guard_for` stays Strict for multis — the singles
//! ride the caller's manual precision guard like slices 1–23.
//!
//! Non-goals (stay literal/ordinary, pinned by tests): bare-call and
//! binary plus-shapes (method-call wraps only), typed tuple swaps,
//! struct-pattern swaps (no N ≥ 2 detector exists), infer-wildcard
//! (`_` inits), macro/opaque inits (`init_ok` gate), and windows
//! that are not exactly type-add + typed-wrap.

use super::antiunify::{is_plain_let_line, parse_typed_let_full, parse_untyped_let_full};
use super::antiunify_types::tok;
use super::shape::init_ok;
use super::unique_rule;

/// Preserved-pattern hoist for one single pair: a bare ident (no
/// sub-pattern) rides `P0`; a single-argument tuple struct over a
/// bare ident rides `Path(P0)` (fixtures pin the `Some(P0)` shape,
/// and the authored `Some(P)` sweep canonicalizes to exactly this).
/// Anything else (multi-arg, nested, `mut`/`ref` bindings, reference
/// or slice patterns) declines — the literal floor owns it. Holing
/// only the pattern's ident leaves keeps the constructor anchor, so
/// the exactly-one-hit proof still bites (a bare `P0` would also
/// match `let _ = v;` and veto on files like 73's).
fn hole_single_pat(pat: &syn::Pat) -> Option<String> {
    match pat {
        syn::Pat::Ident(pi) => {
            if pi.subpat.is_some() || pi.mutability.is_some() || pi.by_ref.is_some() {
                return None;
            }
            Some("P0".to_string())
        }
        syn::Pat::TupleStruct(ps) => {
            if ps.elems.len() != 1 {
                return None;
            }
            let syn::Pat::Ident(inner) = &ps.elems[0] else {
                return None;
            };
            if inner.subpat.is_some() || inner.mutability.is_some() || inner.by_ref.is_some() {
                return None;
            }
            Some(format!("{}(P0)", tok(&ps.path)))
        }
        _ => None,
    }
}

/// Plus-side `E0`-holed method-wrap suffix for one single pair: the
/// plus init must be a `MethodCall` whose receiver token-equals the
/// minus init (the wrapped value), rendered `.method(turbo)(args)`
/// with the first site's concrete method spelling. Bare calls,
/// binaries, and receiver-changing calls decline (other detectors'
/// territory or unprovable here).
fn method_wrap_suffix(minus_init: &syn::Expr, plus_init: &syn::Expr) -> Option<String> {
    let syn::Expr::MethodCall(call) = plus_init else {
        return None;
    };
    if tok(&*call.receiver) != tok(minus_init) {
        return None;
    }
    let method = call.method.to_string();
    if method.is_empty() {
        return None;
    }
    let turbo = tok(&call.turbofish);
    let args = call.args.iter().map(tok).collect::<Vec<_>>().join(", ");
    Some(format!(".{method}{turbo}({args})"))
}

/// Shared tail of the K = 1 arms: totality at n = 1, byte-compare
/// pre-reservation, then the allow-one emission. Returns `None` on
/// any veto; reserves the rule name only for emissions that
/// reproduce (slice-1 precedent).
#[allow(clippy::too_many_arguments)]
fn emit_let_single(
    path: &str,
    pre: &str,
    minus_line: &str,
    plus_line: &str,
    changed: &[usize],
    minus_tpl: &str,
    plus_tpl: &str,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minus_tpl == plus_tpl {
        return None;
    }
    if !crate::totality::template_pair_holds(path, Some(pre), minus_tpl, plus_tpl, 1) {
        return None;
    }
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minus_line, plus_line, minus_tpl, plus_tpl,
    ) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let header = crate::emission_header(path, &rule, changed);
    let mut hunk = header.replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// WI-7 slice 2, typed wrap-add single (fixture `98`): typed→typed
/// with per-side token-equal pattern and type plus a method-wrap
/// init delta. Holes are positional by construction (`P0`/`T0` for
/// the preserved sides, `E0` for the wrapped init).
pub(crate) fn try_typed_wrap_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let (m_pat, m_ty, m_init) = parse_typed_let_full(minuses[0])?;
    let (p_pat, p_ty, p_init) = parse_typed_let_full(pluses[0])?;
    if tok(&m_pat) != tok(&p_pat) || tok(&m_ty) != tok(&p_ty) {
        return None;
    }
    if !init_ok(&m_init) || !init_ok(&p_init) {
        return None;
    }
    let pat_tpl = hole_single_pat(&m_pat)?;
    let suffix = method_wrap_suffix(&m_init, &p_init)?;
    let minus_tpl = format!("let {pat_tpl}: T0 = E0;");
    let plus_tpl = format!("let {pat_tpl}: T0 = E0{suffix};");
    emit_let_single(
        path, pre, minuses[0], pluses[0], changed, &minus_tpl, &plus_tpl, used_rules,
    )
}

/// WI-7 slice 2, untyped wrap-add single (fixtures `73`/`84`): the
/// untyped twin of the arm above — any preserved pattern rides `P0`
/// (fixtures pin the `Some(P0)` shape), the wrapped init rides `E0`.
pub(crate) fn try_untyped_wrap_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let (m_pat, m_init) = parse_untyped_let_full(minuses[0])?;
    let (p_pat, p_init) = parse_untyped_let_full(pluses[0])?;
    if tok(&m_pat) != tok(&p_pat) {
        return None;
    }
    if !init_ok(&m_init) || !init_ok(&p_init) {
        return None;
    }
    let pat_tpl = hole_single_pat(&m_pat)?;
    let suffix = method_wrap_suffix(&m_init, &p_init)?;
    let minus_tpl = format!("let {pat_tpl} = E0;");
    let plus_tpl = format!("let {pat_tpl} = E0{suffix};");
    emit_let_single(
        path, pre, minuses[0], pluses[0], changed, &minus_tpl, &plus_tpl, used_rules,
    )
}

/// WI-7 slice 2, tuple-position swap single (fixture `72`):
/// untyped→untyped arity-2 tuple with the same two elements in
/// swapped order per side (real swap, no self-swap), initializer
/// token-equal per side (a per-site init delta is an untyped-wrap
/// template, never guessed here). Cross-shape hetero has no meaning
/// at N = 1, so both elements ride holes (`P0` minus-first, `P1`
/// minus-second, replayed swapped) and the preserved init rides
/// `E0` — the exactly-one-hit proof owns the generality.
pub(crate) fn try_pat_swap_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let (m_pat, m_init) = parse_untyped_let_full(minuses[0])?;
    let (p_pat, p_init) = parse_untyped_let_full(pluses[0])?;
    if tok(&m_init) != tok(&p_init) {
        return None;
    }
    if !init_ok(&m_init) {
        return None;
    }
    let syn::Pat::Tuple(mt) = m_pat else {
        return None;
    };
    let syn::Pat::Tuple(pt) = p_pat else {
        return None;
    };
    if mt.elems.len() != 2 || pt.elems.len() != 2 {
        return None;
    }
    let m0 = tok(&mt.elems[0]);
    let m1 = tok(&mt.elems[1]);
    let p0 = tok(&pt.elems[0]);
    let p1 = tok(&pt.elems[1]);
    if m0 == m1 || m0 != p1 || m1 != p0 {
        return None;
    }
    let minus_tpl = "let (P0, P1) = E0;".to_string();
    let plus_tpl = "let (P1, P0) = E0;".to_string();
    emit_let_single(
        path, pre, minuses[0], pluses[0], changed, &minus_tpl, &plus_tpl, used_rules,
    )
}

/// T361 byte-compare pre-reservation for the K = 2 window (fixture
/// `107`): the ranged allow-one two-line emission must real-apply to
/// the pre-image byte-identical to the native two-line swap. K-line
/// twin of `general_expr::single_reproduces` under the same contract
/// (line match at each changed line, exactly-one window match, byte
/// equality); the probe mirrors the real emission shape (tight
/// original range, directive-free allow-one).
fn window_reproduces(
    path: &str,
    pre: &str,
    changed: &[usize],
    minus_lines: &[&str],
    plus_lines: &[&str],
    minus_tpls: &[String],
    plus_tpls: &[String],
) -> bool {
    if changed.len() != 2 || minus_lines.len() != 2 || plus_lines.len() != 2 {
        return false;
    }
    let min = changed.iter().min().copied().unwrap_or(0);
    if min < 1 || changed[1] != changed[0] + 1 {
        return false;
    }
    let mut lines: Vec<&str> = pre.split('\n').collect();
    for (i, line_no) in changed.iter().enumerate() {
        if lines.get(line_no - 1).copied() != Some(minus_lines[i]) {
            return false;
        }
        lines[line_no - 1] = plus_lines[i];
    }
    let expected = lines.join("\n");
    let mut probe = format!("{}\n@@ -{min},2 @@ rule=probe\n", crate::git_header(path));
    for m in minus_tpls {
        probe.push_str(&format!("- {m}\n"));
    }
    for p in plus_tpls {
        probe.push_str(&format!("+ {p}\n"));
    }
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == 1
        && files.get(path).is_some_and(|got| *got == expected)
}

/// WI-7 slice 2, hetero pair window single (fixture `107`): exactly
/// two consecutive plain lets where position 0 is a pure type-add
/// (minus untyped, plus typed, pattern and init token-equal, concrete
/// type from the plus side) and position 1 is a typed method-wrap
/// (same gate as `try_typed_wrap_single`, positional `P1`/`T1`/`E1`
/// holes so window statements bind independently, SPEC 15.7).
/// A narrow arm — not a `single_block` gate widening (its one-sided
/// decline pin stands): any other delta combination declines and the
/// section falls through to the partition literals.
pub(crate) fn try_hetero_pair_window_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 2 || pluses.len() != 2 || changed.len() != 2 {
        return None;
    }
    let pre = file_old?;
    if changed[1] != changed[0] + 1 {
        return None;
    }
    if !minuses.iter().all(|l| is_plain_let_line(l)) || !pluses.iter().all(|l| is_plain_let_line(l))
    {
        return None;
    }
    // Position 0: pure type-add (no rename, no init delta).
    let (p0_pat_m, p0_init_m) = parse_untyped_let_full(minuses[0])?;
    let (p0_pat_p, p0_ty_p, p0_init_p) = parse_typed_let_full(pluses[0])?;
    if tok(&p0_pat_m) != tok(&p0_pat_p) || tok(&p0_init_m) != tok(&p0_init_p) {
        return None;
    }
    if !init_ok(&p0_init_m) {
        return None;
    }
    let ty0 = tok(&p0_ty_p);
    // Position 1: typed method-wrap with preserved pattern and type.
    let (p1_pat_m, p1_ty_m, p1_init_m) = parse_typed_let_full(minuses[1])?;
    let (p1_pat_p, p1_ty_p, p1_init_p) = parse_typed_let_full(pluses[1])?;
    if tok(&p1_pat_m) != tok(&p1_pat_p) || tok(&p1_ty_m) != tok(&p1_ty_p) {
        return None;
    }
    if !init_ok(&p1_init_m) || !init_ok(&p1_init_p) {
        return None;
    }
    let suffix1 = method_wrap_suffix(&p1_init_m, &p1_init_p)?;
    let minus_tpls = vec!["let P0 = E0;".to_string(), "let P1: T1 = E1;".to_string()];
    let plus_tpls = vec![
        format!("let P0: {ty0} = E0;"),
        format!("let P1: T1 = E1{suffix1};"),
    ];
    if !crate::totality::template_sequence_holds(path, Some(pre), &minus_tpls, &plus_tpls, 1) {
        return None;
    }
    if !window_reproduces(path, pre, changed, minuses, pluses, &minus_tpls, &plus_tpls) {
        return None;
    }
    let rule = unique_rule(used_rules, "single_block");
    // Allow-one tight range (single_block family posture): no
    // `# allow=many`, no `# sweep` — an unexpected twin fails loud
    // as too-many at apply time.
    let min = changed.iter().min().copied().unwrap_or(0);
    if min < 1 {
        return None;
    }
    let mut hunk = format!("{}\n@@ -{min},2 @@ rule={rule}\n", crate::git_header(path));
    for l in &minus_tpls {
        hunk.push_str(&format!("- {l}\n"));
    }
    for l in &plus_tpls {
        hunk.push_str(&format!("+ {l}\n"));
    }
    Some(hunk)
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

    const PRE_98: &str = "fn f() {\n    let a: i32 = 0;\n}\n";

    fn single_98_body() -> String {
        "@@ -2,1 +2,1 @@\n-    let a: i32 = 0;\n+    let a: i32 = 0.wrapping_add(1);\n".to_string()
    }

    #[test]
    fn typed_wrap_single_fires() {
        let raw = section("f.rs", &single_98_body());
        let pre = pre_map(&[("f.rs", PRE_98)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0: T0 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P0: T0 = E0.wrapping_add(1);\n"),
            "got: {out}"
        );
        assert!(!out.contains("# allow=many"), "allow-one, got: {out}");
        assert!(!out.contains("let a: i32"), "no concrete lines, got: {out}");
    }

    #[test]
    fn typed_wrap_single_twin_vetoes() {
        // Totality: a second untouched typed `let` matches the holed
        // minus — the single vetoes (falls back to the literal floor
        // downstream, pinned by the live probes).
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let a: i32 = 0;\n    let b: i32 = 5;\n}\n",
        )]);
        assert!(super::try_typed_wrap_single(
            &["    let a: i32 = 0;"],
            &["    let a: i32 = 0.wrapping_add(1);"],
            "f.rs",
            Some(&pre["f.rs"]),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
    }

    const PRE_73: &str = "fn a(o: Option<i32>) {\n    let Some(v) = o;\n    let _ = v;\n}\n";

    fn single_73_body() -> String {
        "@@ -2,1 +2,1 @@\n-    let Some(v) = o;\n+    let Some(v) = o.unwrap_or(0);\n".to_string()
    }

    #[test]
    fn untyped_wrap_single_fires() {
        let raw = section("f.rs", &single_73_body());
        let pre = pre_map(&[("f.rs", PRE_73)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let Some(P0) = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let Some(P0) = E0.unwrap_or(0);\n"),
            "got: {out}"
        );
        assert!(!out.contains("# allow=many"), "allow-one, got: {out}");
        assert!(!out.contains("Some(v)"), "no concrete lines, got: {out}");
    }

    #[test]
    fn untyped_wrap_single_twin_vetoes() {
        let pre = pre_map(&[(
            "f.rs",
            "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let _ = (v, w);\n}\n",
        )]);
        assert!(super::try_untyped_wrap_single(
            &["    let Some(v) = o;"],
            &["    let Some(v) = o.unwrap_or(0);"],
            "f.rs",
            Some(&pre["f.rs"]),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
    }

    const PRE_72: &str = "fn a(t: (i32, i32)) {\n    let (x, y) = t;\n    let _ = (x, y);\n}\n";

    fn single_72_body() -> String {
        "@@ -2,1 +2,1 @@\n-    let (x, y) = t;\n+    let (y, x) = t;\n".to_string()
    }

    #[test]
    fn pat_swap_single_fires() {
        let raw = section("f.rs", &single_72_body());
        let pre = pre_map(&[("f.rs", PRE_72)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let (P0, P1) = E0;\n"), "got: {out}");
        assert!(out.contains("+ let (P1, P0) = E0;\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one, got: {out}");
        assert!(!out.contains("let (x, y)"), "no concrete lines, got: {out}");
    }

    #[test]
    fn pat_swap_single_twin_vetoes() {
        let pre = pre_map(&[(
            "f.rs",
            "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (x, y) = t;\n    let (p, q) = u;\n    let _ = ((x, y), (p, q));\n}\n",
        )]);
        assert!(super::try_pat_swap_single(
            &["    let (x, y) = t;"],
            &["    let (y, x) = t;"],
            "f.rs",
            Some(&pre["f.rs"]),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
    }

    const PRE_107: &str =
        "fn first() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\n";

    fn pair_107_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta: u32 = wrap(alpha);\n+    let alpha: u32 = compute_first(input);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n"
            .to_string()
    }

    #[test]
    fn hetero_pair_window_fires() {
        let raw = section("f.rs", &pair_107_body());
        let pre = pre_map(&[("f.rs", PRE_107)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("- let P1: T1 = E1;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P1: T1 = E1.wrapping_add(1);\n"),
            "got: {out}"
        );
        // One window intent: a single rule, no partition.
        assert_eq!(out.matches("rule=").count(), 1, "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one, got: {out}");
        assert!(!out.contains("compute_first(input);"), "got: {out}");
    }

    #[test]
    fn hetero_pair_window_twin_vetoes() {
        // Totality: an untouched twin window vetoes the fusion (the
        // section falls back to the partition literals downstream).
        let pre = pre_map(&[(
            "f.rs",
            "fn first() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\nfn second() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\n",
        )]);
        assert!(super::try_hetero_pair_window_single(
            &[
                "    let alpha = compute_first(input);",
                "    let beta: u32 = wrap(alpha);"
            ],
            &[
                "    let alpha: u32 = compute_first(input);",
                "    let beta: u32 = wrap(alpha).wrapping_add(1);"
            ],
            "f.rs",
            Some(&pre["f.rs"]),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_none());
    }

    #[test]
    fn wrap_singles_decline_matrix() {
        // Seven gates, one pin: bare-call plus, binary plus,
        // receiver-changing method, pat/ty drift, `&`-led sides stay
        // outside the holed singles (literal floor or ordinary).
        let pre = PRE_98;
        // Bare-call plus-shape: not a method wrap.
        assert!(super::try_typed_wrap_single(
            &["    let a: i32 = f(0);"],
            &["    let a: i32 = g(0);"],
            "f.rs",
            Some(pre),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Binary plus-shape: not a method wrap.
        assert!(super::try_typed_wrap_single(
            &["    let a: i32 = 0;"],
            &["    let a: i32 = 0 + 1;"],
            "f.rs",
            Some(pre),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Receiver-changing method: the wrap must wrap the minus init.
        assert!(super::try_typed_wrap_single(
            &["    let a: i32 = 0;"],
            &["    let a: i32 = b.wrapping_add(1);"],
            "f.rs",
            Some(pre),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Pattern drift: a rename is not a wrap.
        assert!(super::try_typed_wrap_single(
            &["    let a: i32 = 0;"],
            &["    let b: i32 = 0.wrapping_add(1);"],
            "f.rs",
            Some(pre),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Type drift: a type change is not a wrap.
        assert!(super::try_typed_wrap_single(
            &["    let a: i32 = 0;"],
            &["    let a: u32 = 0.wrapping_add(1);"],
            "f.rs",
            Some(pre),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Untyped twin with init delta (not a preserved-pattern wrap).
        assert!(super::try_untyped_wrap_single(
            &["    let Some(v) = o;"],
            &["    let Some(v) = p.unwrap_or(0);"],
            "f.rs",
            Some(PRE_73),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Tuple swap with init delta: an untyped-wrap shape, never a swap.
        assert!(super::try_pat_swap_single(
            &["    let (x, y) = t;"],
            &["    let (y, x) = u;"],
            "f.rs",
            Some(PRE_72),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Nested pattern: no single-ident hoist exists.
        assert!(super::try_untyped_wrap_single(
            &["    let Some((a, b)) = o;"],
            &["    let Some((a, b)) = o.unwrap_or((0, 0));"],
            "f.rs",
            Some(PRE_73),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // Multi-arg tuple struct: no single-ident hoist exists.
        assert!(super::try_untyped_wrap_single(
            &["    let Pair(a, b) = o;"],
            &["    let Pair(a, b) = o.unwrap_or((0, 0));"],
            "f.rs",
            Some(PRE_73),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
        // `mut` binding: holing would drop the mutability anchor.
        assert!(super::try_typed_wrap_single(
            &["    let mut a: i32 = 0;"],
            &["    let mut a: i32 = 0.wrapping_add(1);"],
            "f.rs",
            Some(PRE_98),
            &[2],
            &mut Vec::new(),
        )
        .is_none());
    }

    #[test]
    fn window_declines_matrix() {
        // Five gates, one pin: non-consecutive lines, type-add with a
        // rename, wrap with a type drift, double type-add, double wrap
        // (an earlier detector's uniform class) — none fuses.
        let pre = PRE_107;
        let m = [
            "    let alpha = compute_first(input);",
            "    let beta: u32 = wrap(alpha);",
        ];
        let p = [
            "    let alpha: u32 = compute_first(input);",
            "    let beta: u32 = wrap(alpha).wrapping_add(1);",
        ];
        // Non-consecutive window: the engine only matches consecutive
        // statement windows.
        assert!(super::try_hetero_pair_window_single(
            &m,
            &p,
            "f.rs",
            Some(pre),
            &[2, 4],
            &mut Vec::new()
        )
        .is_none());
        // Position 0 with a rename: not a pure type-add.
        assert!(super::try_hetero_pair_window_single(
            &m,
            &[
                "    let aaa: u32 = compute_first(input);",
                "    let beta: u32 = wrap(alpha).wrapping_add(1);"
            ],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_none());
        // Position 1 with a type drift: not a pure wrap.
        assert!(super::try_hetero_pair_window_single(
            &m,
            &[
                "    let alpha: u32 = compute_first(input);",
                "    let beta: u64 = wrap(alpha).wrapping_add(1);"
            ],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_none());
        // Double type-add: two additions are not this hetero window.
        assert!(super::try_hetero_pair_window_single(
            &[
                "    let alpha = compute_first(input);",
                "    let beta = wrap(alpha);"
            ],
            &[
                "    let alpha: u32 = compute_first(input);",
                "    let beta: u32 = wrap(alpha);"
            ],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_none());
        // Double wrap: a uniform wrap pair is the typed-wrap class
        // (two singles after partition), never this window.
        assert!(super::try_hetero_pair_window_single(
            &[
                "    let alpha: u32 = compute_first(input);",
                "    let beta: u32 = wrap(alpha);"
            ],
            &[
                "    let alpha: u32 = compute_first(input).wrapping_add(1);",
                "    let beta: u32 = wrap(alpha).wrapping_add(1);"
            ],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut Vec::new(),
        )
        .is_none());
    }
}
