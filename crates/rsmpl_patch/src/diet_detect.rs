//! WI-1d: `rule=`-only hunk detection (SPEC 7.1).
//!
//! A hunk is RSmPL iff the header carries `rule=` or at least one
//! unprefixed `#` directive line follows the header. Directive-free
//! renames use the terse form plus `rule=`.

#[cfg(test)]
mod tests {
    use crate::model::Hunk;
    use crate::parser::parse_patch;

    #[test]
    fn rule_only_call_rewrite_is_rsmpl() {
        // WI-1d: no `#` lines, but `rule=` present => RSmPL (E inferred downstream).
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => {
                assert!(rh.directives.is_empty());
                assert_eq!(rh.rule_name(), "r");
            }
            Hunk::Ordinary(_) => panic!("expected rsmpl for rule-only hunk"),
        }
    }

    #[test]
    fn rule_only_terse_fn_is_rsmpl() {
        // WI-1d: directive-free terse rename.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=rename_foo\n- fn foo\n+ fn bar\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn rule_only_item_rename_is_rsmpl() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- struct Old\n+ struct New\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn rule_only_elided_new_side_is_rsmpl() {
        // WI-1d: new-side elision stays RSmPL-only; rule-only hunks may elide.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,1 @@ rule=r\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn directive_only_without_rule_is_rsmpl() {
        // `rule=` optional for single hunks: directive block alone suffices.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn no_rule_no_directive_stays_ordinary() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        assert!(!p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn rule_with_text_body_parses_as_rsmpl() {
        // `rule=` claims semantic intent: an ordinary-shaped text edit
        // with `rule=` parses as RSmPL (pattern compile rejects it later).
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n  mod old;\n+mod new;\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn ordinary_elided_hunk_suggests_rule_or_directive() {
        // Ordinary hunks still require both ranges; hint names both markers.
        use crate::parser::ParseError;
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@\n-fn foo() {}\n-fn bar() {}\n";
        let err = parse_patch(text).unwrap_err();
        match err {
            ParseError::Structured(s) => {
                let sug = s.suggestion.unwrap_or_default();
                assert!(sug.contains("rule="), "got suggestion: {sug}");
            }
            ParseError::Message(m) => panic!("unexpected message error: {m}"),
        }
    }
}
