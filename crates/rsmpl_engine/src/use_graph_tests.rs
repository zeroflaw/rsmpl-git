//! D5 sibling of `use_graph.rs` (`plan_tests.rs` precedent):
//! the inline `mod tests` body moved verbatim here (dedented one level).
//! Prod code stays in `use_graph.rs`; new core-resolver pins land here.

use super::*;

fn resolve_for_test(
    src: &str,
    old: &str,
    renamed_site: &str,
    fallback: bool,
) -> Result<FollowOutcome, FollowVeto> {
    let parsed: syn::File = syn::parse_str(src).expect("test src must parse");
    let index = LineIndex::new(src);
    // The renamed definition: byte range of `renamed_site`'s `old`.
    let off = src.find(renamed_site).expect("renamed site must exist");
    let a = off + renamed_site.find(old).expect("old inside site");
    resolve(
        &parsed,
        src,
        &index,
        old,
        &[(a, a + old.len())],
        fallback,
        false,
    )
}

const STRUCT_SRC: &str = "use m::Old;\nstruct Old {\n    field: i32,\n}\nimpl Old {\n    fn make() -> Old {\n        Old { field: 0 }\n    }\n}\nfn f(x: Old) -> Old {\n    let y: Old = Old::make();\n    let z = Old;\n    z\n}\n";

#[test]
fn struct_follow_collects_type_literal_use_and_value_sites() {
    let out = resolve_for_test(STRUCT_SRC, "Old", "struct Old", false).unwrap();
    assert!(out.skipped.is_empty());
    // use-tree + impl self-ty (filtered: it is the renamed def range…
    // no — the renamed range is the struct header, so the self-ty IS a
    // use here) + return type + literal + param + return + ascription +
    // leading `Old::make` + value = 9 use spans.
    assert_eq!(out.spans.len(), 9, "spans: {:?}", out.spans);
}

#[test]
fn struct_follow_skips_nothing_when_clean() {
    let out = resolve_for_test(STRUCT_SRC, "Old", "struct Old", true).unwrap();
    assert!(out.skipped.is_empty());
    assert_eq!(out.spans.len(), 9);
}

#[test]
fn middle_segment_module_paths_rewrite() {
    // `crate::Old::make` names `Old` mid-path; every-segment rule
    // rewrites it.
    let src = "mod m;\nstruct Old;\nfn f() {\n    let x = crate::Old::make();\n    x\n}\n";
    let out = resolve_for_test(src, "Old", "struct Old", false).unwrap();
    assert!(out.skipped.is_empty());
    assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
}

#[test]
fn enum_follow_rewrites_leading_variant_qualifier() {
    // Renaming `E` must reach `E::V`: the item names the first segment.
    let src = "enum E {\n    V,\n}\nfn f(x: E) -> E {\n    let y = E::V;\n    y\n}\n";
    let out = resolve_for_test(src, "E", "enum E", false).unwrap();
    assert!(out.skipped.is_empty());
    assert_eq!(out.spans.len(), 3, "spans: {:?}", out.spans);
}

#[test]
fn nested_macro_groups_are_opaque() {
    let src = "struct Old;\nfn f() {\n    m!(vec![Old]);\n    let x = 1;\n    x\n}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("opaque"), "got: {}", err.reason);
}

#[test]
fn fn_follow_rewrites_calls_but_not_methods_or_fields() {
    let src = "fn old(x: i32) -> i32 {\n    x + 1\n}\nfn g(s: S) -> i32 {\n    let a = old(1);\n    let b = m::old(2);\n    let c = s.old();\n    let d = s.old;\n    a + b\n}\n";
    let out = resolve_for_test(src, "old", "fn old", false).unwrap();
    assert!(out.skipped.is_empty());
    // `old(1)` + `m::old(2)` only: method call + field stay.
    assert_eq!(out.spans.len(), 2, "spans: {:?}", out.spans);
}

#[test]
fn shadowed_definition_vetoes_without_fallback() {
    let src = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n}\n";
    let err = resolve_for_test(src, "Old", "pub struct Old", false).unwrap_err();
    assert!(
        err.reason.contains("shadowed definition"),
        "got: {}",
        err.reason
    );
}

#[test]
fn shadowed_definition_skips_all_uses_with_fallback() {
    let src = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n    pub fn f(x: Old) -> Old {\n        x\n    }\n}\n";
    let out = resolve_for_test(src, "Old", "pub struct Old", true).unwrap();
    assert!(out.spans.is_empty(), "all uses ambiguous: {:?}", out.spans);
    assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
    assert!(out.skipped[0].note.contains("use sites left untouched"));
}

#[test]
fn macro_mention_vetoes_without_fallback_and_skips_with_it() {
    let src = "struct Old;\nfn f() {\n    m!(Old);\n    let x: Old = make();\n    x\n}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("opaque"), "got: {}", err.reason);
    let out = resolve_for_test(src, "Old", "struct Old", true).unwrap();
    // Clean sites still rewrite; only the macro site is skipped.
    assert_eq!(out.spans.len(), 1, "spans: {:?}", out.spans);
    assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
}

#[test]
fn qself_mention_is_opaque() {
    let src = "struct Old;\nfn f<T: Tr>() where T: Tr {\n    let x = <T as Tr>::Old;\n    x\n}";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("qself"), "got: {}", err.reason);
}

#[test]
fn multiline_macro_reports_ident_line() {
    // `Old` sits on line 4; the macro starts on line 2.
    let src = "struct Old;\nfn f() {\n    m!(\n        Old\n    );\n}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("line 4"), "got: {}", err.reason);
}

#[test]
fn multiline_attribute_reports_ident_line() {
    // `Old` sits on line 3; the attribute starts on line 2.
    let src = "struct Old;\n#[a::\nOld]\nfn f() {}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("line 3"), "got: {}", err.reason);
}

#[test]
fn multiline_qself_reports_ident_line() {
    // `Old` sits on line 3; the `<...>` starts on line 2.
    let src = "struct Old;\nfn f(x: <T as Tr>::a::\n    Old) {}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("line 3"), "got: {}", err.reason);
}

#[test]
fn variant_with_same_name_shadows() {
    let src = "struct Old;\nenum E {\n    Old,\n}\nfn f(x: E) -> E {\n    x\n}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
}

#[test]
fn local_binding_named_old_shadows_fn_rename() {
    let src =
        "fn old() -> i32 {\n    1\n}\nfn g() -> i32 {\n    let old = 2;\n    old + old()\n}\n";
    let err = resolve_for_test(src, "old", "fn old", false).unwrap_err();
    assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
}

fn workspace_for_test(
    files: &[(&str, &str)],
    old: &str,
    renamed_file: &str,
    renamed_site: &str,
    fallback: bool,
) -> Result<WorkspaceOutcome, FollowVeto> {
    let parsed: Vec<syn::File> = files
        .iter()
        .map(|(_, src)| syn::parse_str(src).expect("test src must parse"))
        .collect();
    let originals: Vec<String> = files.iter().map(|(_, s)| (*s).to_string()).collect();
    let indexes: Vec<LineIndex> = originals.iter().map(|s| LineIndex::new(s)).collect();
    let refs: Vec<(String, &syn::File, &str, &LineIndex)> = files
        .iter()
        .enumerate()
        .map(|(i, (path, _))| {
            (
                (*path).to_string(),
                &parsed[i],
                originals[i].as_str(),
                &indexes[i],
            )
        })
        .collect();
    let def_src = files
        .iter()
        .find(|(p, _)| *p == renamed_file)
        .expect("renamed file present")
        .1;
    let off = def_src.find(renamed_site).expect("renamed site must exist");
    let a = off + renamed_site.find(old).expect("old inside site");
    resolve_workspace(
        &refs,
        old,
        renamed_file,
        &[(a, a + old.len())],
        fallback,
        false,
    )
}

#[test]
fn workspace_rewrites_uses_across_files() {
    let out = workspace_for_test(
        &[
            ("a.rs", "pub struct Old;\n"),
            (
                "b.rs",
                "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
            ),
        ],
        "Old",
        "a.rs",
        "struct Old",
        false,
    )
    .unwrap();
    assert!(out.skipped.is_empty());
    let total: usize = out.per_file.iter().map(|(_, s)| s.len()).sum();
    assert_eq!(total, 4, "per_file: {:?}", out.per_file);
    let b = out
        .per_file
        .iter()
        .find(|(p, _)| p == "b.rs")
        .expect("b.rs entry");
    assert_eq!(b.1.len(), 4);
    assert!(out.per_file.iter().all(|(p, _)| p != "a.rs"));
}

#[test]
fn workspace_second_definition_vetoes_and_skips() {
    let files = [
        ("a.rs", "pub struct Old;\n"),
        ("b.rs", "pub struct Old;\nfn f(x: Old) -> Old {\n    x\n}\n"),
    ];
    let err = workspace_for_test(&files, "Old", "a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("shadowed"), "got: {}", err.reason);
    assert!(err.reason.contains("b.rs"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "a.rs", "struct Old", true).unwrap();
    assert!(out.per_file.is_empty(), "got: {:?}", out.per_file);
    assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
    assert!(
        out.skipped[0].note.contains("b.rs"),
        "got: {:?}",
        out.skipped
    );
}

#[test]
fn workspace_opaque_site_in_other_file_vetoes_then_skips() {
    let files = [
        ("a.rs", "pub struct Old;\n"),
        (
            "b.rs",
            "fn f() {\n    m!(Old);\n}\nfn g(x: Old) -> Old {\n    x\n}\n",
        ),
    ];
    let err = workspace_for_test(&files, "Old", "a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("opaque"), "got: {}", err.reason);
    assert!(err.reason.contains("b.rs"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "a.rs", "struct Old", true).unwrap();
    let b = out
        .per_file
        .iter()
        .find(|(p, _)| p == "b.rs")
        .expect("b.rs entry");
    assert_eq!(b.1.len(), 2, "per_file: {:?}", out.per_file);
    assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
    assert!(
        out.skipped[0].note.contains("b.rs"),
        "got: {:?}",
        out.skipped
    );
}

#[test]
fn glob_with_old_use_vetoes_without_fallback() {
    let src = "use other::*;\nstruct Old;\nfn f(x: Old) -> Old {\n    x\n}\n";
    let err = resolve_for_test(src, "Old", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("glob"), "got: {}", err.reason);
}

#[test]
fn glob_with_old_use_skips_only_that_file_with_fallback() {
    let files = [
        ("a.rs", "pub struct Old;\n"),
        ("b.rs", "use other::*;\nfn f(x: Old) -> Old {\n    x\n}\n"),
        ("c.rs", "fn g(x: Old) -> Old {\n    x\n}\n"),
    ];
    let err = workspace_for_test(&files, "Old", "a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("glob"), "got: {}", err.reason);
    assert!(err.reason.contains("b.rs"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "a.rs", "struct Old", true).unwrap();
    assert!(
        out.per_file.iter().all(|(p, _)| p != "b.rs"),
        "globbed file must not rewrite: {:?}",
        out.per_file
    );
    let c = out
        .per_file
        .iter()
        .find(|(p, _)| p == "c.rs")
        .expect("clean file still rewrites");
    assert_eq!(c.1.len(), 2, "per_file: {:?}", out.per_file);
    assert!(
        out.skipped.iter().any(|s| s.note.contains("b.rs")),
        "got: {:?}",
        out.skipped
    );
}

#[test]
fn glob_without_old_use_is_inert() {
    let src = "use other::*;\nstruct Old;\nfn f(x: i32) -> i32 {\n    x\n}\n";
    let out = resolve_for_test(src, "Old", "struct Old", false).unwrap();
    assert!(out.spans.is_empty(), "spans: {:?}", out.spans);
    assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
}

#[test]
fn def_module_path_covers_layouts() {
    assert_eq!(def_module_path("a.rs"), vec!["a".to_string()]);
    assert_eq!(def_module_path("src/a.rs"), vec!["a".to_string()]);
    assert_eq!(
        def_module_path("src/x/y.rs"),
        vec!["x".to_string(), "y".to_string()]
    );
    assert!(def_module_path("src/lib.rs").is_empty());
    assert_eq!(def_module_path("src/b/mod.rs"), vec!["b".to_string()]);
}

#[test]
fn workspace_one_hop_reexport_fires() {
    // def in a.rs, `pub use crate::a::Old` in b.rs, through-uses in c.rs:
    // the re-export line plus every through-use rewrites in one hunk.
    let out = workspace_for_test(
        &[
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "pub use crate::a::Old;\n"),
            (
                "src/c.rs",
                "use crate::b::Old;\nfn f(x: Old) -> Old {\n    let y = b::Old;\n    y\n}\n",
            ),
        ],
        "Old",
        "src/a.rs",
        "struct Old",
        false,
    )
    .unwrap();
    assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
    let b = out
        .per_file
        .iter()
        .find(|(p, _)| p == "src/b.rs")
        .expect("re-export line rewrites");
    assert_eq!(b.1.len(), 1, "per_file: {:?}", out.per_file);
    let c = out
        .per_file
        .iter()
        .find(|(p, _)| p == "src/c.rs")
        .expect("through-uses rewrite");
    assert_eq!(c.1.len(), 4, "per_file: {:?}", out.per_file);
    assert!(out.per_file.iter().all(|(p, _)| p != "src/a.rs"));
}

#[test]
fn workspace_fanout_reexports_to_one_target_fire() {
    // Two files re-exporting the same definition is one hop each to a
    // single target: every edge resolves, so everything rewrites.
    let out = workspace_for_test(
        &[
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "pub use crate::a::Old;\n"),
            ("src/c.rs", "pub use crate::a::Old;\n"),
            ("src/d.rs", "fn f(x: b::Old) -> c::Old {\n    x\n}\n"),
        ],
        "Old",
        "src/a.rs",
        "struct Old",
        false,
    )
    .unwrap();
    assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
    let total: usize = out.per_file.iter().map(|(_, s)| s.len()).sum();
    assert_eq!(total, 4, "per_file: {:?}", out.per_file);
}

#[test]
fn workspace_off_target_reexport_vetoes_and_skips() {
    // `pub use other::Old` names a foreign `Old`: rewriting it as ours
    // would be wrong, so the hunk vetoes loud.
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        (
            "src/b.rs",
            "pub use other::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
        ),
    ];
    let err = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("does not name"), "got: {}", err.reason);
    assert!(err.reason.contains("src/b.rs"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", true).unwrap();
    assert!(out.per_file.is_empty(), "got: {:?}", out.per_file);
    assert_eq!(out.skipped.len(), 1, "got: {:?}", out.skipped);
    assert!(
        out.skipped[0].note.contains("src/b.rs"),
        "got: {:?}",
        out.skipped
    );
}

#[test]
fn workspace_second_hop_reexport_vetoes_and_skips() {
    // c.rs re-exports through b.rs instead of the definition: a second
    // hop is opaque and vetoes, even though it transitively resolves.
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        ("src/b.rs", "pub use crate::a::Old;\n"),
        ("src/c.rs", "pub use crate::b::Old;\n"),
    ];
    let err = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("second"), "got: {}", err.reason);
    assert!(err.reason.contains("src/c.rs"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", true).unwrap();
    assert!(out.per_file.is_empty(), "got: {:?}", out.per_file);
    assert!(
        out.skipped.iter().any(|s| s.note.contains("src/c.rs")),
        "got: {:?}",
        out.skipped
    );
}

#[test]
fn workspace_def_pointing_glob_with_through_use_fires() {
    // `pub use crate::a::*` forwards our own `Old`: benign, fires.
    let out = workspace_for_test(
        &[
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "pub use crate::a::*;\n"),
            ("src/c.rs", "fn f(x: b::Old) -> Old {\n    x\n}\n"),
        ],
        "Old",
        "src/a.rs",
        "struct Old",
        false,
    )
    .unwrap();
    assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
    let total: usize = out.per_file.iter().map(|(_, s)| s.len()).sum();
    assert_eq!(total, 2, "per_file: {:?}", out.per_file);
}

#[test]
fn workspace_foreign_glob_with_through_use_vetoes() {
    // `pub use other::*` in b.rs may forward a different `Old`, and
    // c.rs spells `Old` through `b`: opaque, vetoes loud.
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        ("src/b.rs", "pub use other::*;\n"),
        ("src/c.rs", "fn f(x: b::Old) -> Old {\n    x\n}\n"),
    ];
    let err = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("glob re-export"), "got: {}", err.reason);
    assert!(err.reason.contains("src/b.rs"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", true).unwrap();
    assert!(out.per_file.is_empty(), "got: {:?}", out.per_file);
    assert!(
        out.skipped.iter().any(|s| s.note.contains("src/b.rs")),
        "got: {:?}",
        out.skipped
    );
}

#[test]
fn workspace_foreign_glob_without_through_use_fires() {
    // No file spells `Old` through the globbing module: inert.
    let out = workspace_for_test(
        &[
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "pub use other::*;\n"),
            ("src/c.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
        ],
        "Old",
        "src/a.rs",
        "struct Old",
        false,
    )
    .unwrap();
    assert!(out.skipped.is_empty(), "got: {:?}", out.skipped);
    let total: usize = out.per_file.iter().map(|(_, s)| s.len()).sum();
    assert_eq!(total, 2, "per_file: {:?}", out.per_file);
}

#[test]
fn workspace_root_foreign_glob_with_crate_use_vetoes() {
    // lib.rs forwards a foreign glob and b.rs spells `crate::Old`:
    // through-root-namespace use, opaque.
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        ("src/lib.rs", "pub use other::*;\n"),
        (
            "src/b.rs",
            "use crate::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
        ),
    ];
    let err = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", false).unwrap_err();
    assert!(err.reason.contains("glob re-export"), "got: {}", err.reason);
    let out = workspace_for_test(&files, "Old", "src/a.rs", "struct Old", true).unwrap();
    assert!(out.per_file.is_empty(), "got: {:?}", out.per_file);
}
