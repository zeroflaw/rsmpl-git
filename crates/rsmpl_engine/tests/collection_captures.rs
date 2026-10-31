//! WI-6 collection-capture pins: `# bind=` + `# allow=many` on an item
//! cut binds every in-scope item in source order (SPEC 20.3); a `# use=`
//! pure-insertion paste splices all items with one blank between
//! (SPEC 20.2). Evolution pastes with a collection fail loud (one
//! header template cannot evolve many items).

use std::collections::HashMap;

use rsmpl_engine::ApplyReport;

fn apply_multi_report(
    patch_text: &str,
    initial: Vec<(&str, &str)>,
) -> Result<(HashMap<String, String>, ApplyReport), String> {
    let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
    let mut files: HashMap<String, String> = initial
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let report = rsmpl_engine::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
    Ok((files, report))
}

fn apply_multi(
    patch_text: &str,
    initial: Vec<(&str, &str)>,
) -> Result<HashMap<String, String>, String> {
    apply_multi_report(patch_text, initial).map(|(files, _)| files)
}

// Five helpers separated by single blanks; the cut range covers all five.
const SRC: &str = "fn keep() {}\nfn a() {}\n\nfn b() {}\n\nfn c() {}\n\nfn d() {}\n\nfn e() {}\n";
const CUT: &str = "@@ -2,9 @@ rule=cut_helpers\n# allow=many\n# bind=Helpers\n- I\n";
const PASTE: &str = "@@ -1,0 @@ rule=paste_helpers\n# use=Helpers\n+ Helpers\n";

fn move_patch() -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n{CUT}\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n{PASTE}"
    )
}

#[test]
fn five_item_move_is_one_cut_one_paste() {
    let (files, report) =
        apply_multi_report(&move_patch(), vec![("a.rs", SRC), ("b.rs", "use x;\n")]).unwrap();
    assert_eq!(files["a.rs"], "fn keep() {}\n", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"], "use x;\nfn a() {}\n\nfn b() {}\n\nfn c() {}\n\nfn d() {}\n\nfn e() {}\n",
        "got: {:?}",
        files["b.rs"]
    );
    assert_eq!(report.rules.len(), 2, "got: {:?}", report.rules);
    assert_eq!(report.rules[0].matches, 5, "got: {:?}", report.rules[0]);
    assert_eq!(
        report.rules[0].spans,
        vec![(2, 2), (4, 4), (6, 6), (8, 8), (10, 10)],
        "got: {:?}",
        report.rules[0].spans
    );
    assert_eq!(report.rules[1].matches, 1, "got: {:?}", report.rules[1]);
}

#[test]
fn paste_keeps_source_order_with_one_blank_join() {
    // Items land in cut (source) order with exactly one blank between —
    // the SPEC 20.2 paste join, independent of anchor context.
    let files = apply_multi(&move_patch(), vec![("a.rs", SRC), ("b.rs", "use x;\n")]).unwrap();
    let full = files["b.rs"].clone();
    let pasted = full["use x;\n".len()..].to_string();
    let order: Vec<&str> = ["fn a()", "fn b()", "fn c()", "fn d()", "fn e()"]
        .into_iter()
        .collect();
    let mut last = 0;
    for item in order {
        let pos = pasted[last..].find(item).expect("item present");
        last += pos + item.len();
    }
    assert!(
        !pasted.contains("\n\n\n"),
        "no double blanks, got: {pasted:?}"
    );
}

#[test]
fn optional_empty_collection_pastes_nothing() {
    // `# optional` + no match binds the empty collection; the paste
    // splices nothing and the tree is unchanged.
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 @@ rule=cut_none\n# allow=many\n# optional\n# bind=Helpers\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_helpers\n# use=Helpers\n+ Helpers\n";
    let (files, report) =
        apply_multi_report(patch, vec![("a.rs", "\n"), ("b.rs", "use x;\n")]).unwrap();
    assert_eq!(files["a.rs"], "\n", "got: {:?}", files["a.rs"]);
    assert_eq!(files["b.rs"], "use x;\n", "got: {:?}", files["b.rs"]);
    assert_eq!(report.rules[0].matches, 0, "got: {:?}", report.rules[0]);
    assert!(
        report.rules[0].spans.is_empty(),
        "got: {:?}",
        report.rules[0]
    );
}

#[test]
fn zero_match_without_optional_fails_loud() {
    // No `# optional`, no items in range: the collection cut fails like
    // any other zero-match hunk (SPEC 20.4).
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 @@ rule=cut_none\n# allow=many\n# bind=Helpers\n- I\n";
    let err = apply_multi(patch, vec![("a.rs", "\n")]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
}

#[test]
fn double_paste_splices_collection_twice() {
    // One collection cut, two pastes at different anchors: a copy, each
    // splice complete and blank-joined (scalar fan-out parity).
    let patch = format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n{CUT}\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_one\n# use=Helpers\n+ Helpers\n\n@@ -1,0 @@ rule=paste_two\n# use=Helpers\n+ Helpers\n"
    );
    let files = apply_multi(
        &patch,
        vec![
            ("a.rs", SRC),
            ("b.rs", "use x;\n"),
            ("c.rs", "fn c0() {}\n"),
        ],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "fn keep() {}\n", "got: {:?}", files["a.rs"]);
    // Same-anchor pastes land in document order with one blank between.
    assert_eq!(
        files["b.rs"].matches("fn a() {}").count(),
        2,
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn collection_evolution_paste_fails_loud() {
    // One header template cannot evolve many items (SPEC 20.3).
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,9 @@ rule=cut_helpers\n# allow=many\n# bind=Helpers\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_evo\n# use=Helpers\n+ pub fn evo(...) { ... }\n";
    let err = apply_multi(patch, vec![("a.rs", SRC), ("b.rs", "use x;\n")]).unwrap_err();
    assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
    assert!(err.contains("collection"), "got: {err}");
}

#[test]
fn collection_cut_vs_ordinary_overlap_fails_loud() {
    // An ordinary hunk rewriting a line inside the cut range overlaps the
    // collection deletes: the patch fails whole, never partially.
    let patch = format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n{CUT}\n@@ -4,1 +4,1 @@\n-fn b() {{}}\n+fn b2() {{}}\n"
    );
    let err = apply_multi(&patch, vec![("a.rs", SRC)]).unwrap_err();
    assert!(err.contains("overlapping edits"), "got: {err}");
}

#[test]
fn scalar_cut_still_binds_exactly_one() {
    // Regression pin: without `# allow=many` the cut still takes
    // `targets[0]` and the count gate rejects the second item.
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,3 @@ rule=cut_one\n# bind=One\n- I\n";
    let err = apply_multi(patch, vec![("a.rs", SRC)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("expected one match"), "got: {err}");
}
