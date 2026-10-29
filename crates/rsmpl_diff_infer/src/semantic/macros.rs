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
/// `old!(body)` -> `new!(body)` with one shared old/new single-segment
/// macro name, per-site token-equal bodies, and a real name delta.
/// Emits one `# allow=many` `# macro=token_tree` hunk with the verbatim
/// `...` body (token-tree matching is verbatim-only by design, T133):
/// every `old!` invocation rewrites regardless of body or delimiter.
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
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n) = parse_macro_pair(m, p)?;
        if old_name.as_ref().is_some_and(|x| *x != o) {
            return None;
        }
        if new_name.as_ref().is_some_and(|x| *x != n) {
            return None;
        }
        old_name = Some(o);
        new_name = Some(n);
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    if !crate::totality::macro_holds(file_old, &old_name, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("macros_{old_name}_to_{new_name}"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str("# macro=token_tree\n");
    hunk.push_str(&format!("- {old_name}!(...)\n+ {new_name}!(...)\n"));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a macro rename; return (old name, new
/// name) when both sides are single-segment macro invocations with a
/// shared delta and per-site token-equal bodies.
fn parse_macro_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(old_line)?;
    let n_mac = parse_macro_expr(new_line)?;
    let old = macro_name(&o_mac)?;
    let new = macro_name(&n_mac)?;
    if o_mac.mac.tokens.to_string() != n_mac.mac.tokens.to_string() {
        return None;
    }
    Some((old, new))
}

/// The single-segment ident of a macro invocation path; `None` for
/// qualified paths (`crate::old!` stays ordinary in v1), leading `::`,
/// and generic args.
fn macro_name(mac: &syn::ExprMacro) -> Option<String> {
    if mac.mac.path.leading_colon.is_some() || mac.mac.path.segments.len() != 1 {
        return None;
    }
    let seg = &mac.mac.path.segments[0];
    if !matches!(seg.arguments, syn::PathArguments::None) {
        return None;
    }
    let name = seg.ident.to_string();
    if ident_len(&name)? != name.len() {
        return None;
    }
    Some(name)
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
    fn macro_qualified_path_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_render_widget!(first_widget_argument);\n-    crate::old_render_widget!(second_widget_argument);\n+    crate::new_render_widget!(first_widget_argument);\n+    crate::new_render_widget!(second_widget_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }
}
