//! WI-8a evolution-paste pins: `# use=N` plus a single-line `fn`
//! header template splices the captured body under the evolved header.
//! Bodies, params, and return types ride byte-verbatim; only the
//! header line is rewritten.

use std::collections::HashMap;

fn apply_multi(
    patch_text: &str,
    initial: Vec<(&str, &str)>,
) -> Result<HashMap<String, String>, String> {
    let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
    let mut files: HashMap<String, String> = initial
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    rsmpl_engine::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
    Ok(files)
}

const SRC: &str =
    "fn keep() {}\n\nfn old_name(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\n";
const B_SRC: &str = "fn other() {}\n";

fn cut_and_paste(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -3,4 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

#[test]
fn name_and_vis_evolve_cross_file() {
    let files = apply_multi(
        &cut_and_paste("pub fn new_name(...) { ... }"),
        vec![("a.rs", SRC), ("b.rs", B_SRC)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "fn keep() {}\n", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"],
        "fn other() {}\npub fn new_name(x: i32) -> i32 {\n    let y = x + 1;\n    y * 2\n}\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn qualifier_evolution_rides_the_prefix() {
    let files = apply_multi(
        &cut_and_paste("pub async fn new_name(...) { ... }"),
        vec![("a.rs", SRC), ("b.rs", B_SRC)],
    )
    .unwrap();
    assert!(
        files["b.rs"].contains("pub async fn new_name(x: i32) -> i32 {"),
        "got: {:?}",
        files["b.rs"]
    );
    assert!(
        files["b.rs"].contains("    let y = x + 1;\n    y * 2\n"),
        "body must ride byte-verbatim, got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn multiline_signature_splice_keeps_trailing_header_lines() {
    let src = "fn old_name(\n    x: i32,\n    y: i32,\n) -> i32 {\n    x + y\n}\n";
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,6 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ pub fn new_name(...) { ... }\n";
    let files = apply_multi(patch, vec![("a.rs", src), ("b.rs", B_SRC)]).unwrap();
    assert_eq!(
        files["b.rs"],
        "fn other() {}\npub fn new_name(\n    x: i32,\n    y: i32,\n) -> i32 {\n    x + y\n}\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn attrs_and_doc_lines_ride_verbatim() {
    let src = "#[inline]\n/// calls fn old_name\nfn old_name(x: i32) -> i32 {\n    x\n}\n";
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,5 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ pub fn new_name(...) { ... }\n";
    let files = apply_multi(patch, vec![("a.rs", src), ("b.rs", B_SRC)]).unwrap();
    assert_eq!(
        files["b.rs"],
        "fn other() {}\n#[inline]\n/// calls fn old_name\npub fn new_name(x: i32) -> i32 {\n    x\n}\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn restricted_visibility_template_applies() {
    // WI-history-70d72c0: a vis-only `fn` → `pub(crate) fn` move raises
    // `+ pub(crate) fn ...( ... ) { ... }`; the template gate must
    // accept restricted visibilities (parens are part of `pub(...)`).
    let files = apply_multi(
        &cut_and_paste("pub(crate) fn new_name(...) { ... }"),
        vec![("a.rs", SRC), ("b.rs", B_SRC)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "fn keep() {}\n", "got: {:?}", files["a.rs"]);
    assert!(
        files["b.rs"].contains("pub(crate) fn new_name(x: i32) -> i32 {"),
        "got: {:?}",
        files["b.rs"]
    );
    assert!(
        files["b.rs"].contains("    let y = x + 1;\n    y * 2\n"),
        "body must ride byte-verbatim, got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn verbatim_paste_still_works() {
    // The bare-name shape is untouched by the evolution branch.
    let files = apply_multi(&cut_and_paste("M1"), vec![("a.rs", SRC), ("b.rs", B_SRC)]).unwrap();
    assert!(
        files["b.rs"].contains("fn old_name(x: i32) -> i32 {"),
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn malformed_template_is_loud() {
    // A single-line `fn`-looking plus that is not a valid template
    // fails at compile time instead of silently inserting text.
    let err = apply_multi(
        &cut_and_paste("pub fn new_name(x: i32)"),
        vec![("a.rs", SRC), ("b.rs", B_SRC)],
    )
    .expect_err("malformed template must fail loud");
    assert!(err.contains("evolution paste template"), "got: {err}");
}

#[test]
fn multiline_plus_with_use_is_loud() {
    // Multi-line plus with `# use=` has no evolution meaning (the
    // template is single-line by construction) and fails loud instead
    // of silently ignoring the capture.
    let patch = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -3,4 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ fn literal() {}\n+ // note\n";
    let err = apply_multi(patch, vec![("a.rs", SRC), ("b.rs", B_SRC)])
        .expect_err("multi-line plus with `# use=` must fail loud");
    assert!(err.contains("# use="), "got: {err}");
}
