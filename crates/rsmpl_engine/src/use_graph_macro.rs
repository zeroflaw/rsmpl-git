//! WI-7b: `macro_rules!` path helpers (D5 sibling).
//!
//! Split from `use_graph`/`matcher_item` per the 1600-line hygiene gate:
//! the exact-`macro_rules` path predicate shared by the definition
//! collector (`use_graph::Collector::visit_item_macro`), the orphan
//! mirror, and the engine item matcher (`matcher_item::consider`).
//! Non-`macro_rules` macros (attribute-style macros in item position)
//! never match: only the single-segment unqualified `macro_rules`
//! path with no generic arguments counts.

/// True when `path` is exactly the `macro_rules` macro path (single
/// unqualified segment, no generic arguments, no leading `::`).
pub(crate) fn is_macro_rules_path(path: &syn::Path) -> bool {
    path.leading_colon.is_none()
        && path.segments.len() == 1
        && path.segments[0].ident == "macro_rules"
        && matches!(path.segments[0].arguments, syn::PathArguments::None)
}

#[cfg(test)]
mod tests {
    use super::is_macro_rules_path;
    use crate::lines::LineIndex;

    fn resolve_macro_for_test(
        src: &str,
        old: &str,
        renamed_site: &str,
        fallback: bool,
    ) -> Result<crate::use_graph::FollowOutcome, crate::use_graph::FollowVeto> {
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let index = LineIndex::new(src);
        let off = src.find(renamed_site).expect("renamed site must exist");
        let a = off + renamed_site.find(old).expect("old inside site");
        crate::use_graph::resolve(
            &parsed,
            src,
            &index,
            old,
            &[(a, a + old.len())],
            fallback,
            true,
        )
    }

    #[test]
    fn macro_rules_follow_rewrites_invocations() {
        // WI-7b: `macro_rules! Old` + macro-path collection rewrites
        // `Old!(…)` invocations; other paths are untouched.
        let src = "macro_rules! Old {\n    ($x:expr) => { $x };\n}\nfn f() {\n    let a = Old!(1);\n    let b = Old!(2);\n}\n";
        let out = resolve_macro_for_test(src, "Old", "macro_rules! Old", false).unwrap();
        assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
        assert_eq!(out.spans.len(), 2, "spans: {:?}", out.spans);
    }

    #[test]
    fn macro_paths_ignored_without_flag() {
        // Non-macro renames never claim `Old!(…)` (different namespaces).
        let src =
            "struct Old;\nfn f() {\n    let a = Old!(1);\n    let x: Old = make();\n    x\n}\n";
        let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
        let index = LineIndex::new(src);
        let off = src.find("struct Old").expect("renamed site must exist");
        let a = off + "struct Old".find("Old").expect("old inside site");
        let out =
            crate::use_graph::resolve(&parsed, src, &index, "Old", &[(a, a + 3)], false, false)
                .unwrap();
        assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
    }

    #[test]
    fn path_predicate_accepts_only_bare_macro_rules() {
        let bare: syn::Path = syn::parse_str("macro_rules").unwrap();
        assert!(is_macro_rules_path(&bare));
        let qualified: syn::Path = syn::parse_str("crate::macro_rules").unwrap();
        assert!(!is_macro_rules_path(&qualified));
        let leading: syn::Path = syn::parse_str("::macro_rules").unwrap();
        assert!(!is_macro_rules_path(&leading));
        let other: syn::Path = syn::parse_str("my_attr").unwrap();
        assert!(!is_macro_rules_path(&other));
    }
}
