//! Literal-frame raising (WI-7): lone literal deltas keep their frame.
//!
//! A balanced multi-site class (N ≥ 2) whose every changed line differs
//! from its partner in exactly one literal token — string, char, number,
//! or bool, same kind on both sides — with a uniform `(old, new)` pair
//! across sites and hetero surrounding frames. Emits one `# allow=many`
//! general hunk with the bare literal pair (`- "old"` / `+ "new"`),
//! which the engine matches as literal subterms wherever they sit
//! (`return`, call args, `let` inits — probed `verified`).
//!
//! The verbatim sweep deliberately vetoes bare literals ("would match
//! every mention") and no tighter detector owns a lone literal delta,
//! so this detector registers last in the pair chain (after
//! `single_block`): call/let/sequence/window shapes keep their classes
//! first. All-identical line classes decline (T422, verbatim owns
//! them). Totality is the real-pair engine probe
//! (`template_pair_holds`): the emitted pair must match exactly N
//! literal subterms in the pre-image — an untouched twin vetoes, macro
//! interiors and string-in-string never count (the engine never enters
//! them), and a delta sitting inside a macro body counts 0 and vetoes.
//!
//! New module (D5: `literal.rs` owns trailing-arg call literals and is
//! not grown; expression generalisation lives in `general_expr.rs`).

use super::unique_rule;
use quote::ToTokens;
use syn::visit::Visit;

/// Supported literal kinds: the lone-literal delta must keep its kind
/// across the pair (an int→float change is not one template).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LitKind {
    Str,
    Char,
    Int,
    Float,
    Bool,
}

/// Classify one `syn::Lit`. Byte strings, C strings, raw strings, and
/// verbatim literals veto (`None`): raw-string interiors are opaque by
/// design (WI-7 non-goal) and byte/C spellings never occur in the daily
/// edit class this detector owns.
fn lit_kind(lit: &syn::Lit) -> Option<LitKind> {
    match lit {
        syn::Lit::Str(s) => {
            // `to_token_stream` preserves the source spelling (`r".."`,
            // `r#".."#` keep their prefix); raw interiors stay opaque.
            let tok = s.to_token_stream().to_string();
            if tok.starts_with('r') || tok.starts_with('R') {
                return None;
            }
            Some(LitKind::Str)
        }
        syn::Lit::Char(_) => Some(LitKind::Char),
        syn::Lit::Int(_) => Some(LitKind::Int),
        syn::Lit::Float(_) => Some(LitKind::Float),
        syn::Lit::Bool(_) => Some(LitKind::Bool),
        _ => None,
    }
}

/// One collected literal: its kind plus token spelling (source-faithful
/// for strings — quotes and escapes ride the token, so the emission
/// replays the site's own spelling).
struct LitCell {
    kind: LitKind,
    tok: String,
}

/// Collect `syn::Lit` nodes in visit order. Macro bodies contribute
/// nothing (opaque `TokenStream`, never visited as expressions), so a
/// delta *inside* a macro invocation yields equal/empty lists and the
/// pair vetoes below; a macro *elsewhere* on the line is simply not
/// collected and does not veto.
struct Lits {
    out: Vec<LitCell>,
}

impl<'ast> Visit<'ast> for Lits {
    fn visit_lit(&mut self, node: &'ast syn::Lit) {
        // Unsupported spellings veto the whole line, not just the cell:
        // a skipped cell would shift positions and forge an alignment
        // the plus side cannot reproduce.
        if let Some(kind) = lit_kind(node) {
            self.out.push(LitCell {
                kind,
                tok: node.to_token_stream().to_string(),
            });
        } else {
            self.out.push(LitCell {
                kind: LitKind::Str,
                tok: String::new(),
            });
        }
        syn::visit::visit_lit(self, node);
    }
}

/// Parse one changed line and collect its literals. `syn::Stmt` covers
/// expression statements (`f("old");`), `let`s, `return`s, and items
/// (`const A: &str = "old";` via `Stmt::Item`); header/attr/control
/// lines fail to parse and veto (WI-8/WI-10 territory, never guessed
/// here). An empty token marks an unsupported spelling (see
/// `visit_lit`); any line holding one vetoes.
fn parse_line_lits(line: &str) -> Option<Vec<(LitKind, String)>> {
    let text = line.trim();
    if text.is_empty() {
        return None;
    }
    let stmt: syn::Stmt = syn::parse_str(text).ok()?;
    let mut lits = Lits { out: Vec::new() };
    lits.visit_stmt(&stmt);
    let mut cells = Vec::with_capacity(lits.out.len());
    for c in lits.out {
        if c.tok.is_empty() {
            return None;
        }
        cells.push((c.kind, c.tok));
    }
    Some(cells)
}

/// The uniform literal delta of one pair: the shared `(kind, old, new)`
/// plus the per-pair differing index (frames are hetero, so positions
/// may vary across sites). `None` when this pair is not a lone-literal
/// delta (length mismatch, zero deltas, two deltas, kind change).
fn pair_delta(
    minus: &[(LitKind, String)],
    plus: &[(LitKind, String)],
) -> Option<(LitKind, String, String)> {
    if minus.len() != plus.len() || minus.is_empty() {
        return None;
    }
    let mut delta: Option<(LitKind, String, String)> = None;
    for ((mk, mt), (pk, pt)) in minus.iter().zip(plus.iter()) {
        if mt == pt {
            continue;
        }
        if mk != pk || delta.is_some() {
            return None;
        }
        delta = Some((*mk, mt.clone(), pt.clone()));
    }
    delta
}

/// N-site (N ≥ 2) lone-literal sweep: every pair is the same
/// `(kind, old, new)` literal delta inside a hetero frame. Emits one
/// `# allow=many` hunk (tight original range or `# sweep` per the D.5
/// 64-line rule, via `emission_header`). Detector name `literal_frame`.
pub(crate) fn try_literal_frame_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    // All-identical classes stay for the tighter verbatim sweep (T422).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut uniform: Option<(LitKind, String, String)> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let m_lits = parse_line_lits(m)?;
        let p_lits = parse_line_lits(p)?;
        let (kind, old, new) = pair_delta(&m_lits, &p_lits)?;
        if old == new {
            return None;
        }
        // Exactly-once gate: the old literal occurs exactly once among
        // this minus line's literals, so one template application
        // reproduces the plus line exactly (`f("old","old")` →
        // `f("old","new")` would rewrite both and stays ordinary).
        if m_lits.iter().filter(|(_, t)| *t == old).count() != 1 {
            return None;
        }
        match &uniform {
            Some((k, o, n)) => {
                if *k != kind || *o != old || *n != new {
                    return None;
                }
            }
            None => uniform = Some((kind, old, new)),
        }
    }
    let (_, old_tok, new_tok) = uniform?;
    if !crate::totality::template_pair_holds(path, file_old, &old_tok, &new_tok, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "literal_frame");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {old_tok}\n+ {new_tok}\n"));
    Some(hunk)
}

#[cfg(test)]
mod tests {
    use super::parse_line_lits;
    use super::LitKind;

    fn delta(m: &str, p: &str) -> Option<(LitKind, String, String)> {
        super::pair_delta(&parse_line_lits(m)?, &parse_line_lits(p)?)
    }

    #[test]
    fn string_delta_fires() {
        let out = delta("    return \"old\";", "    return \"new\";");
        assert_eq!(
            out,
            Some((LitKind::Str, "\"old\"".to_string(), "\"new\"".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn char_delta_fires() {
        let out = delta("    let c = 'a';", "    let c = 'b';");
        assert_eq!(
            out,
            Some((LitKind::Char, "'a'".to_string(), "'b'".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn int_delta_fires() {
        let out = delta("    let n = 10;", "    let n = 20;");
        assert_eq!(
            out,
            Some((LitKind::Int, "10".to_string(), "20".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn float_delta_fires() {
        let out = delta("    let n = 1.5;", "    let n = 2.5;");
        assert_eq!(
            out,
            Some((LitKind::Float, "1.5".to_string(), "2.5".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn bool_delta_fires() {
        let out = delta("    let b = false;", "    let b = true;");
        assert_eq!(
            out,
            Some((LitKind::Bool, "false".to_string(), "true".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn negative_number_delta_fires() {
        // `-1` is a unary over the literal `1`; the inner literal is
        // the collected delta.
        let out = delta("    let n = -1;", "    let n = -2;");
        assert_eq!(
            out,
            Some((LitKind::Int, "1".to_string(), "2".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn kind_change_vetoes() {
        assert_eq!(delta("    let n = 1;", "    let n = 1.0;"), None);
        assert_eq!(delta("    let b = false;", "    let b = \"false\";"), None);
    }

    #[test]
    fn double_differ_vetoes() {
        let out = delta("    log(\"a\", \"b\");", "    log(\"c\", \"d\");");
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn no_delta_vetoes() {
        let out = delta("    let n = 1;", "    let n = 1;");
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn literal_count_mismatch_vetoes() {
        let out = delta("    f(\"a\");", "    f(\"b\", \"c\");");
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn header_line_vetoes() {
        assert_eq!(delta("fn old_name() {", "fn new_name() {"), None);
        assert_eq!(delta("    } else {", "    } else if x {"), None);
    }

    #[test]
    fn attribute_line_vetoes() {
        assert_eq!(delta("    #[inline]", "    #[track_caller]"), None);
    }

    #[test]
    fn macro_body_delta_vetoes() {
        // The delta sits inside a macro invocation: opaque TokenStream
        // contributes no literals, so no delta is found.
        let out = delta("    m!(\"old\");", "    m!(\"new\");");
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn raw_string_vetoes() {
        let out = delta("    let s = r\"old\";", "    let s = r\"new\";");
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn empty_line_vetoes() {
        assert_eq!(delta("", ""), None);
    }
}

#[cfg(test)]
mod chain_tests {
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

    #[test]
    fn hetero_string_frames_fire() {
        // WI-7 canonical: same ("old" -> "new") delta, hetero frames.
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    return \"old\";\n+    return \"new\";\n }\n fn b() {\n-    log(\"old\", x);\n+    log(\"new\", x);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a() {\n    return \"old\";\n}\nfn b() {\n    log(\"old\", x);\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("- \"old\"\n"), "got: {out}");
        assert!(out.contains("+ \"new\"\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn numeric_and_bool_frames_fire() {
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    let n = 10;\n+    let n = 20;\n }\n fn b() {\n-    let ok = false;\n+    let ok = true;\n }\n",
        );
        // Two uniform pairs, different kinds: no single template — the
        // section stays ordinary (one delta per class, never guessed).
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn numeric_pair_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    let n = 10;\n+    let n = 20;\n }\n fn b() {\n-    let m = 10 + 1;\n+    let m = 20 + 1;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a() {\n    let n = 10;\n}\nfn b() {\n    let m = 10 + 1;\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("- 10\n"), "got: {out}");
        assert!(out.contains("+ 20\n"), "got: {out}");
    }

    #[test]
    fn untouched_twin_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    return \"old\";\n+    return \"new\";\n }\n fn b() {\n-    log(\"old\", x);\n+    log(\"new\", x);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a() {\n    return \"old\";\n}\nfn b() {\n    log(\"old\", x);\n}\nfn c() {\n    log(\"old\", y);\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none(),
            "extra same-literal site must veto"
        );
    }

    #[test]
    fn string_in_string_never_counts() {
        // `"say old"` is a different literal value, not a twin of
        // `"old"` — the sweep still fires (structural, never textual).
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    return \"old\";\n+    return \"new\";\n }\n fn b() {\n-    log(\"old\", x);\n+    log(\"new\", x);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a() {\n    return \"old\";\n}\nfn b() {\n    log(\"old\", x);\n}\nfn c() {\n    log(\"say old\", y);\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("- \"old\"\n"), "got: {out}");
    }

    #[test]
    fn dense_zero_vetoes() {
        // Tiny literals occur everywhere: the count-must-equal-N gate
        // keeps the detector silent unless the diff owns every `0`.
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    let n = 0;\n+    let n = 1;\n }\n fn b() {\n-    let m = f(0);\n+    let m = f(1);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a() {\n    let n = 0;\n}\nfn b() {\n    let m = f(0);\n}\nfn c() {\n    let z = 0;\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none(),
            "dense-0 twin must veto"
        );
    }

    #[test]
    fn macro_elsewhere_on_line_still_fires() {
        // A macro invocation elsewhere on the line is not a veto: the
        // delta literal sits outside it and the engine probe counts
        // exactly the two real sites.
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn a() {\n-    let x = [m!(\"x\"), \"old\"];\n+    let x = [m!(\"x\"), \"new\"];\n }\n fn b() {\n-    let y = [m!(\"y\"), \"old\"];\n+    let y = [m!(\"y\"), \"new\"];\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn a() {\n    let x = [m!(\"x\"), \"old\"];\n}\nfn b() {\n    let y = [m!(\"y\"), \"old\"];\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("- \"old\"\n"), "got: {out}");
        assert!(out.contains("+ \"new\"\n"), "got: {out}");
    }

    #[test]
    fn same_callee_literal_stays_with_call_literal() {
        // Tighter detector keeps its class: same-callee trailing-literal
        // deltas emit the call frame, never the bare literal.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    check(c1, \"a\");\n-    check(c2, \"a\");\n+    check(c1, \"b\");\n+    check(c2, \"b\");\n }\n",
        );
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        assert!(out.contains("check(E0,"), "got: {out}");
        assert!(
            !out.contains("- \"a\"\n"),
            "bare literal must not fire, got: {out}"
        );
    }

    #[test]
    fn old_twice_on_line_vetoes() {
        // One template application could not reproduce the plus line.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    f(\"old\", \"old\");\n-    g(\"old\", \"old\");\n+    f(\"old\", \"new\");\n+    g(\"old\", \"new\");\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn all_identical_stays_for_verbatim() {
        // T422: line-identical classes compress tighter without holes.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    return \"old\";\n-    return \"old\";\n+    return \"new\";\n+    return \"new\";\n }\n",
        );
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        assert!(
            !out.contains("- \"old\"\n"),
            "verbatim must win, got: {out}"
        );
        assert!(out.contains("rule=r1\n"), "got: {out}");
    }

    #[test]
    fn single_site_stays_ordinary() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    return \"old\";\n+    return \"new\";\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }
}
