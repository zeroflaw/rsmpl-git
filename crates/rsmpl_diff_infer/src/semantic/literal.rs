//! Same-callee trailing-literal-delta call sweep (T448, split from
//! `super` for the 2000-line hygiene gate — peel.rs precedent).

use super::{call_parts, flat_ws, parse_call_expr, unique_rule};

/// N-site (N >= 2) same-callee same-arity trailing-literal-delta call
/// sweep (T448): every pair parses as a bare single-seg no-turbofish
/// call with one shared callee and uniform arity `k + 1`; each site's
/// leading `k` args token-equal minus-vs-plus (bound as `E0…`, hetero
/// across sites allowed); the trailing arg is token-equal concrete
/// text across ALL sites on each side with a real old/new delta
/// (`false` -> `"100644"`, never a plus-only metavar — T442-safe by
/// construction). Pre-image totality keys on the trailing old literal
/// (`call_literal_holds`, T420-style): same-arity sites with a
/// different trailing literal (7025a14's `true` site next to the 2x
/// `false` prize group) never match the pattern and must not veto.
pub(crate) fn try_call_literal_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    use quote::ToTokens;
    let tok = |e: &syn::Expr| e.to_token_stream().to_string();
    let mut cal: Option<String> = None;
    let mut prefix: Option<usize> = None;
    let mut old_lit: Option<String> = None;
    let mut new_lit: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o_cal, o_args) = call_parts(&parse_call_expr(m)?)?;
        let (n_cal, n_args) = call_parts(&parse_call_expr(p)?)?;
        if o_cal != n_cal || o_args.len() != n_args.len() || o_args.is_empty() {
            return None;
        }
        if cal.as_ref().is_some_and(|c| *c != o_cal) {
            return None;
        }
        if prefix.is_some_and(|k| k + 1 != o_args.len()) {
            return None;
        }
        if !o_args[..o_args.len() - 1]
            .iter()
            .zip(n_args[..n_args.len() - 1].iter())
            .all(|(a, b)| tok(a) == tok(b))
        {
            return None;
        }
        let (o_last, n_last) = (
            tok(&o_args[o_args.len() - 1]),
            tok(&n_args[n_args.len() - 1]),
        );
        if old_lit.as_ref().is_some_and(|l| *l != o_last) {
            return None;
        }
        if new_lit.as_ref().is_some_and(|l| *l != n_last) {
            return None;
        }
        cal = Some(o_cal);
        prefix = Some(o_args.len() - 1);
        old_lit = Some(o_last);
        new_lit = Some(n_last);
    }
    let (cal, k, old_lit, new_lit) = (cal?, prefix?, old_lit?, new_lit?);
    if old_lit == new_lit {
        return None;
    }
    // Hetero required (peel precedent): an all-identical class compresses
    // tighter as a verbatim hunk (no `# expr` directives, shorter rule
    // name — T422 closed this shape for verbatim), so leave it there.
    if minuses
        .iter()
        .zip(pluses.iter())
        .all(|(m, p)| flat_ws(m) == flat_ws(minuses[0]) && flat_ws(p) == flat_ws(pluses[0]))
    {
        return None;
    }
    if !crate::totality::call_literal_holds(file_old, &cal, k + 1, &old_lit, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{cal}_literal"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n",
        header = crate::git_header(path)
    );
    let mut vars = Vec::new();
    for i in 0..k {
        let v = format!("E{i}");
        hunk.push_str(&format!("# expr {v}\n"));
        vars.push(v);
    }
    let head = vars.join(", ");
    let (minus_call, plus_call) = if head.is_empty() {
        (format!("{cal}({old_lit})"), format!("{cal}({new_lit})"))
    } else {
        (
            format!("{cal}({head}, {old_lit})"),
            format!("{cal}({head}, {new_lit})"),
        )
    };
    hunk.push_str(&format!("- {minus_call}\n+ {plus_call}\n"));
    Some(hunk)
}

#[cfg(test)]
mod tests {
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
    fn call_literal_two_sites_fire() {
        // T448: hetero leading bound as E-vars, homo trailing literal delta.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(
            out.contains("rule=calls_make_widget_literal\n"),
            "got: {out}"
        );
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
        assert!(out.contains("- make_widget(E0, E1, false)\n"), "got: {out}");
        assert!(
            out.contains("+ make_widget(E0, E1, \"100644\")\n"),
            "got: {out}"
        );
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn call_literal_all_identical_stays_for_verbatim() {
        // T448 + T422: an all-identical class (here zero-prefix) compresses
        // tighter without metavars, so the literal sweep declines and the
        // verbatim sweep claims it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    make_widget(false);\n-    make_widget(false);\n+    make_widget(\"100644\");\n+    make_widget(\"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(false);\n    make_widget(false);\n}\n",
        )]);
        let out =
            crate::semantic::infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("rule=verbatim\n"), "got: {out}");
        assert!(!out.contains("calls_make_widget_literal"), "got: {out}");
    }

    #[test]
    fn call_literal_hetero_trailing_vetoes() {
        // T448: `false` vs `true` trailing is unexpressible (plus-side
        // metavars would bind nothing, T442).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, false);\n+    make_widget(beta, 2, true);\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_changed_prefix_vetoes() {
        // T448: a leading arg rewritten in transit is not a literal delta.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 9, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_extra_same_literal_vetoes() {
        // T448 tuple totality: an unchanged third same-trailing call in
        // the pre-image over-counts (3 != 2).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, false);\n}\n",
        )]);
        assert!(
            crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn call_literal_extra_different_trailing_fires() {
        // T448 point: a same-arity site with a DIFFERENT trailing literal
        // (`true`, the 7025a14 shape) never matches the `false` pattern
        // and must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, true);\n}\n",
        )]);
        let out = crate::semantic::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("must compress");
        validated_live(&out, &pre);
        assert!(
            out.contains("rule=calls_make_widget_literal\n"),
            "got: {out}"
        );
    }

    #[test]
    fn call_literal_single_site_vetoes() {
        // T448: lone literal deltas keep the ordinary hunk (no token win).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n+    make_widget(alpha, 1, \"100644\");\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_literal_same_literal_vetoes() {
        // T448: no old/new delta (identical lines) is not a sweep.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, false);\n+    make_widget(beta, 2, false);\n }\n",
        );
        assert!(crate::semantic::infer_semantic(std::slice::from_ref(&raw)).is_none());
    }
}
