//! Macro-rename sweep (WI-3). Split from `super` per the D5 hygiene
//! gate. One shape, infer-only (the engine already matches it — probed
//! green): N >= 2 single macro invocations `old!(...)` -> `new!(...)`
//! with per-site token-equal bodies (bodies vary across sites — the
//! bench-17 hetero shape). Delimiters are unrestricted (the engine
//! `...` matches `!()`, `![]`, and `!{}` alike, probed). Macro
//! *interiors* stay ordinary (WI-9): a per-site body rewrite is not a
//! rename.

use super::{ident_len, unique_rule};

/// Parse a diff line as a macro invocation: strip one terminator, take
/// the whole line as `Expr::Macro`, else unwrap a `let`-init macro
/// (T449 precedent — `let x = old!(a);` matches `- old!(...)`,
/// probed). `let-else` never qualifies.
fn parse_macro_expr(line: &str) -> Option<syn::ExprMacro> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    if let Ok(syn::Expr::Macro(mac)) = syn::parse_str::<syn::Expr>(text) {
        return Some(mac);
    }
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            match *init.expr {
                syn::Expr::Macro(mac) => Some(mac),
                _ => None,
            }
        }
        _ => None,
    }
}

/// N-site (N >= 2) macro-rename sweep: every pair parses as
/// `old!(body)` -> `new!(body)` (bare) or `prefix::old!(body)` ->
/// `prefix::new!(body)` (qualified, one uniform class per hunk) with one
/// shared old/new macro path, per-site token-equal bodies, and a real
/// last-segment name delta. Emits one `# allow=many`
/// `# macro=token_tree` hunk with the verbatim `...` body (token-tree
/// matching is verbatim-only by design, T133): every `old!` invocation
/// rewrites regardless of body or delimiter.
pub(crate) fn try_macro_rename_sweep(
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
    let mut old_path: Option<String> = None;
    let mut new_path: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n) = parse_macro_pair(m, p)?;
        if old_path.as_ref().is_some_and(|x| *x != o) {
            return None;
        }
        if new_path.as_ref().is_some_and(|x| *x != n) {
            return None;
        }
        old_path = Some(o);
        new_path = Some(n);
    }
    let (old_path, new_path) = (old_path?, new_path?);
    if old_path == new_path {
        return None;
    }
    // Last-segment delta shared across the class; the prefix (possibly
    // empty for bare) must match segment-wise or the class is hetero.
    let (o_pre, o_l) = split_macro_path(&old_path)?;
    let (n_pre, n_l) = split_macro_path(&new_path)?;
    if o_pre != n_pre || o_l == n_l {
        return None;
    }
    let (old_last, _) = (o_l, n_l);
    if o_pre.is_empty() {
        if !crate::totality::macro_holds(file_old, &old_last, minuses, changed) {
            return None;
        }
    } else if !crate::totality::qualified_macro_holds(file_old, &old_path, minuses, changed) {
        return None;
    }
    let flat = |p: &str| p.replace("::", "_");
    let rule = unique_rule(
        used_rules,
        &format!("macros_{}_to_{}", flat(&old_path), flat(&new_path)),
    );
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str("# macro=token_tree\n");
    hunk.push_str(&format!("- {old_path}!(...)\n+ {new_path}!(...)\n"));
    Some(hunk)
}

/// Split a rendered macro path into (prefix segments, last segment).
fn split_macro_path(path: &str) -> Option<(Vec<String>, String)> {
    let mut segs: Vec<String> = path.split("::").map(str::to_string).collect();
    if segs.is_empty() {
        return None;
    }
    let last = segs.pop()?;
    Some((segs, last))
}

/// Parse a `-`/`+` line pair as a macro rename; return (old path, new
/// path) rendered with `::` joins when both sides are macro invocations
/// with a shared prefix, a last-segment delta, generic-free segments,
/// no leading `::`, and per-site token-equal bodies.
fn parse_macro_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(old_line)?;
    let n_mac = parse_macro_expr(new_line)?;
    let old = macro_path(&o_mac)?;
    let new = macro_path(&n_mac)?;
    if o_mac.mac.tokens.to_string() != n_mac.mac.tokens.to_string() {
        return None;
    }
    Some((old, new))
}

/// The rendered path of a macro invocation (`old!` or `prefix::old!`);
/// `None` for leading `::` (no engine spelling, probed — declined like
/// qualified calls), segment-count mismatch is checked by the caller via
/// exact rendered-path equality, and generic args.
fn macro_path(mac: &syn::ExprMacro) -> Option<String> {
    if mac.mac.path.leading_colon.is_some() {
        return None;
    }
    let mut segs = Vec::new();
    for seg in mac.mac.path.segments.iter() {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return None;
        }
        let name = seg.ident.to_string();
        if ident_len(&name)? != name.len() {
            return None;
        }
        segs.push(name);
    }
    if segs.is_empty() {
        return None;
    }
    // Bare (len 1) and qualified (len >= 2) both qualify; the sweep
    // requires a uniform class (exact rendered-path equality above), so
    // mixed bare/qualified classes stay ordinary by construction.
    Some(segs.join("::"))
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;
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
    fn macro_rename_sweep_compresses() {
        // Hetero bodies (bench-17 shape): verbatim could never group
        // these (count 1 each) — the rename sweep claims them in one hunk.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument, extra_flag);\n-    old_render_widget!(third_widget_argument + offset);\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget!(second_widget_argument, extra_flag);\n+    new_render_widget!(third_widget_argument + offset);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget!(second_widget_argument, extra_flag);\n    old_render_widget!(third_widget_argument + offset);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("# macro=token_tree\n"), "got: {out}");
        assert!(out.contains("- old_render_widget!(...)\n"), "got: {out}");
        assert!(out.contains("+ new_render_widget!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_mixed_delimiters_fire() {
        // The engine `...` matches `!()`, `![]`, `!{}` alike (probed) —
        // a delimiter-mixed rename still compresses.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget![second_widget_argument];\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget![second_widget_argument];\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget![second_widget_argument];\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_render_widget!(...)\n"), "got: {out}");
        assert!(out.contains("+ new_render_widget!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_let_wrapped_fires() {
        // `let x = old!(a);` matches `- old!(...)` (probed) — the sweep
        // claims let-wrapped invocation renames too.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = old_render_widget!(first_widget_argument);\n-    let second_value = old_render_widget!(second_widget_argument);\n+    let first_value = new_render_widget!(first_widget_argument);\n+    let second_value = new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = old_render_widget!(first_widget_argument);\n    let second_value = old_render_widget!(second_widget_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_render_widget!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_body_rewrite_stays_passthrough() {
        // A per-site body rewrite is an interior edit (WI-9), not a rename.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    old_render_widget!(changed_first_argument);\n+    old_render_widget!(changed_second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_divergent_new_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    other_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_single_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n+    new_render_widget!(first_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_extra_site_vetoes() {
        // A third untouched `old!` in the pre-image would also match the
        // `...` pattern — totality vetoes.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget!(second_widget_argument);\n    old_render_widget!(untouched_third_argument);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_qualified_twin_does_not_veto() {
        // Exact-path matching (probed): `crate::old!` never matches the
        // bare pattern — the twin must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_render_widget!(first_widget_argument);\n    old_render_widget!(second_widget_argument);\n    crate::old_render_widget!(qualified_twin_argument);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn macro_qualified_path_fires() {
        // WI-3 residue: qualified macro renames compress like bare ones
        // (probed: the engine matches the concrete qualified path).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_render_widget!(first_widget_argument);\n    crate::old_render_widget!(second_widget_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- crate::old_render_widget!(...)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ crate::new_render_widget!(...)\n"),
            "got: {out}"
        );
        assert!(out.contains("# macro=token_tree\n"), "got: {out}");
    }

    #[test]
    fn macro_qualified_delimiter_mix_fires() {
        // The engine `...` matches all delimiters for qualified paths too.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget![second_widget_argument];\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget![second_widget_argument];\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(
            out.contains("- crate::old_render_widget!(...)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn macro_qualified_let_wrapped_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = crate::old_render_widget!(first_widget_argument);\n-    let second_value = crate::old_render_widget!(second_widget_argument);\n+    let first_value = crate::new_render_widget!(first_widget_argument);\n+    let second_value = crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(
            out.contains("- crate::old_render_widget!(...)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn macro_qualified_prefix_divergent_stays_passthrough() {
        // Different prefixes share no path class.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    other::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    other::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_qualified_last_same_stays_passthrough() {
        // Same last segment is not a rename.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    other::old_render_widget!(first_widget_argument);\n+    other::old_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_bare_twin_does_not_veto_qualified() {
        // Exact-path parity both directions: a bare twin never matches
        // the qualified pattern (probed) so it must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_render_widget!(first_widget_argument);\n    crate::old_render_widget!(second_widget_argument);\n    old_render_widget!(bare_twin_argument);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn macro_qualified_extra_site_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_render_widget!(first_widget_argument);\n    crate::old_render_widget!(second_widget_argument);\n    crate::old_render_widget!(untouched_third_argument);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn macro_leading_colon_stays_passthrough() {
        // Leading `::` has no engine spelling (probed) — declined.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    ::crate::old_render_widget!(first_widget_argument);\n-    ::crate::old_render_widget!(second_widget_argument);\n+    ::crate::new_render_widget!(first_widget_argument);\n+    ::crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_mixed_bare_qualified_stays_passthrough() {
        // Bare + qualified share no path class.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn macro_uppercase_fires_plain() {
        // WI-7-upper: macro names never infer (no `# macro` hole for
        // the name — probed clean), so uppercase renames stay plain:
        // no backticks, real-apply byte-exact.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    Old!(first_argument);\n-    Old!(second_argument);\n+    New!(first_argument);\n+    New!(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    Old!(first_argument);\n    Old!(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains('`'), "no backticks expected, got: {out}");
        assert!(out.contains("- Old!(...)\n"), "got: {out}");
        assert!(out.contains("+ New!(...)\n"), "got: {out}");
    }

    #[test]
    fn macro_qualified_uppercase_fires_plain() {
        // WI-7-upper: qualified twin of the above — the whole path
        // stays verbatim (probed clean, names never infer).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::Old!(first_argument);\n-    crate::Old!(second_argument);\n+    crate::New!(first_argument);\n+    crate::New!(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::Old!(first_argument);\n    crate::Old!(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains('`'), "no backticks expected, got: {out}");
        assert!(out.contains("- crate::Old!(...)\n"), "got: {out}");
        assert!(out.contains("+ crate::New!(...)\n"), "got: {out}");
    }
}
