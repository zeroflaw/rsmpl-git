//! D5 sibling of `use_graph_follow.rs` (`plan_tests.rs` precedent):
//! dispatcher-level pins for `follow_uses_workspace` — one happy-path
//! pin per `FollowTarget` arm plus the two error paths. Resolver
//! matrices keep their own pins; this module only proves the dispatch
//! (arm selection, `found == 0` no-op, labels, extras, tripwire).

use super::*;
use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};
use std::collections::HashMap;

fn hunk_of(patch: &str) -> (rsmpl_patch::RsmplHunk, rsmpl_pattern::CompiledPattern) {
    let parsed = rsmpl_patch::parse_patch(patch).expect("patch must parse");
    let rh = match &parsed.files[0].hunks[0] {
        rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
        _ => panic!("expected rsmpl hunk"),
    };
    let compiled = rsmpl_pattern::compile(&rh).expect("hunk must compile");
    (rh, compiled)
}

fn states_for(
    files: &[(&str, &str)],
) -> (HashMap<String, FileState>, HashMap<String, Vec<TextEdit>>) {
    let mut states = HashMap::new();
    let mut edits = HashMap::new();
    for (path, src) in files {
        states.insert(
            (*path).to_string(),
            FileState {
                original: (*src).to_string(),
                index: LineIndex::new(src),
                parsed: syn::parse_str::<syn::File>(src).ok(),
            },
        );
        edits.insert((*path).to_string(), Vec::new());
    }
    (states, edits)
}

/// Push the definition edit covering `old` inside `marker` in
/// `def_path`, so the dispatcher's `renamed` exclusion mirrors plan.rs.
fn push_def_edit(
    file_edits: &mut HashMap<String, Vec<TextEdit>>,
    files: &[(&str, &str)],
    def_path: &str,
    marker: &str,
    old: &str,
    new: &str,
) {
    let src = files
        .iter()
        .find(|(p, _)| *p == def_path)
        .expect("def file present")
        .1;
    let moff = src.find(marker).expect("marker present");
    let a = moff + marker.find(old).expect("old inside marker");
    file_edits
        .get_mut(def_path)
        .expect("edits entry")
        .push(TextEdit::new(a, a + old.len(), new.to_string()));
}

#[test]
fn found_zero_is_noop() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
    let (rh, compiled) = hunk_of(patch);
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
    ];
    let (states, mut file_edits) = states_for(&files);
    let before: HashMap<String, usize> = file_edits
        .iter()
        .map(|(k, v)| (k.clone(), v.len()))
        .collect();
    let (extras, details) = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &[],
        "src/a.rs",
        0,
        0,
    )
    .unwrap();
    assert!(extras.is_empty(), "got: {extras:?}");
    assert!(details.is_empty(), "got: {details:?}");
    for (k, v) in &file_edits {
        assert_eq!(v.len(), before[k], "edits touched for {k}");
    }
}

#[test]
fn scalar_dispatch_claims_cross_file_use() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
    let (rh, compiled) = hunk_of(patch);
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
    ];
    let (states, mut file_edits) = states_for(&files);
    push_def_edit(
        &mut file_edits,
        &files,
        "src/a.rs",
        "struct Old",
        "Old",
        "New",
    );
    let (extras, _) = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &[],
        "src/a.rs",
        0,
        1,
    )
    .unwrap();
    assert_eq!(extras.len(), 1, "got: {extras:?}");
    assert_eq!(extras[0].0, "src/b.rs");
    assert_ne!(extras[0].1, 0, "got: {extras:?}");
    assert!(
        extras[0].3.iter().all(|l| l == "follow"),
        "got: {:?}",
        extras[0].3
    );
    assert!(
        !file_edits["src/b.rs"].is_empty(),
        "use edits must land in file_edits"
    );
}

#[test]
fn member_dispatch_claims_typed_use() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,4 @@ rule=r\n# follow=uses\n  struct S {\n- old: u32\n+ new: u32\n";
    let (rh, compiled) = hunk_of(patch);
    let files = [
        ("src/a.rs", "struct S {\n    old: u32,\n}\n"),
        ("src/b.rs", "fn f(s: S) -> u32 {\n    s.old\n}\n"),
    ];
    let (states, mut file_edits) = states_for(&files);
    push_def_edit(
        &mut file_edits,
        &files,
        "src/a.rs",
        "old: u32",
        "old",
        "new",
    );
    let (extras, _) = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &[],
        "src/a.rs",
        0,
        1,
    )
    .unwrap();
    assert_eq!(extras.len(), 1, "got: {extras:?}");
    assert_eq!(extras[0].0, "src/b.rs");
    assert!(
        !file_edits["src/b.rs"].is_empty(),
        "member use must rewrite"
    );
}

#[test]
fn assoc_dispatch_labels_impl_definition_rename() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait Tr {\n- type Item;\n+ type Output;\n";
    let (rh, compiled) = hunk_of(patch);
    let src = "trait Tr {\n    type Item;\n}\nstruct S;\nimpl Tr for S {\n    type Item = u8;\n}\n";
    let files = [("src/a.rs", src)];
    let (states, mut file_edits) = states_for(&files);
    push_def_edit(
        &mut file_edits,
        &files,
        "src/a.rs",
        "type Item;",
        "Item",
        "Output",
    );
    let (extras, _) = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &[],
        "src/a.rs",
        0,
        1,
    )
    .unwrap();
    assert!(extras.is_empty(), "same-file only, got: {extras:?}");
    let labels: Vec<Option<&str>> = file_edits["src/a.rs"].iter().map(|e| e.label).collect();
    assert!(
        labels.contains(&Some("rename")),
        "impl def must label `rename`, got: {labels:?}"
    );
    assert!(
        labels.contains(&Some("follow")) || labels.len() >= 2,
        "use/def pair expected, got: {labels:?}"
    );
}

#[test]
fn method_dispatch_accepts_trait_header() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n";
    let (rh, compiled) = hunk_of(patch);
    let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";
    let files = [("src/a.rs", src)];
    let (states, mut file_edits) = states_for(&files);
    push_def_edit(&mut file_edits, &files, "src/a.rs", "fn old", "old", "new");
    let (extras, _) = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &[],
        "src/a.rs",
        0,
        1,
    )
    .unwrap();
    assert!(extras.is_empty(), "same-file only, got: {extras:?}");
    let labels: Vec<Option<&str>> = file_edits["src/a.rs"].iter().map(|e| e.label).collect();
    assert!(
        labels.contains(&Some("rename")),
        "impl def must label `rename`, got: {labels:?}"
    );
}

#[test]
fn call_shape_follow_is_c17() {
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -2,1 +2,1 @@ rule=x\n# follow=uses\n# expr E\n- foo(E)\n+ bar(E)\n";
    let (rh, compiled) = hunk_of(patch);
    let files = [("src/a.rs", "fn f() {\n    foo(1);\n}\n")];
    let (states, mut file_edits) = states_for(&files);
    let err = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &[],
        "src/a.rs",
        0,
        1,
    )
    .unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
    assert!(
        err.render().contains("# follow=uses is only supported"),
        "got: {}",
        err.render()
    );
}

#[test]
fn tripwire_vetoes_without_fallback_and_skips_with_it() {
    let plain = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
    let fallback = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let files = [
        ("src/a.rs", "pub struct Old;\n"),
        ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
    ];
    let unparseable = vec![("src/c.rs".to_string(), "fn f( { Old \n".to_string())];
    // Without fallback: loud veto.
    let (rh, compiled) = hunk_of(plain);
    let (states, mut file_edits) = states_for(&files);
    push_def_edit(
        &mut file_edits,
        &files,
        "src/a.rs",
        "struct Old",
        "Old",
        "New",
    );
    let err = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &unparseable,
        "src/a.rs",
        0,
        1,
    )
    .unwrap_err();
    assert_eq!(err.code, rsmpl_patch::ErrorCode::SymbolConflict);
    // With fallback: skip + details, no veto.
    let (rh, compiled) = hunk_of(fallback);
    let (states, mut file_edits) = states_for(&files);
    push_def_edit(
        &mut file_edits,
        &files,
        "src/a.rs",
        "struct Old",
        "Old",
        "New",
    );
    let (_, details) = follow_uses_workspace(
        &rh,
        &compiled,
        &states,
        &mut file_edits,
        &unparseable,
        "src/a.rs",
        0,
        1,
    )
    .unwrap();
    assert!(
        details.iter().any(|d| d.file == "src/c.rs"),
        "got: {details:?}"
    );
}
