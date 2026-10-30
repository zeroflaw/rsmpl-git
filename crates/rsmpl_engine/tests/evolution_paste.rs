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

// WI-8 slice 1: struct evolution pastes.

const STRUCT_SRC: &str = "struct Old {\n    x: i32,\n    y: i64,\n}\n";
const B_STRUCT: &str = "fn other() {}\n";

fn cut_and_paste_struct(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

fn cut_and_paste_struct_single(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

#[test]
fn struct_name_and_vis_evolve_cross_file() {
    let files = apply_multi(
        &cut_and_paste_struct("pub struct New { ... }"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"], "fn other() {}\npub struct New {\n    x: i32,\n    y: i64,\n}\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn struct_tuple_shape_round_trips_verbatim_suffix() {
    // The `{ ... }` tail is a shape marker only: tuple/unit generics
    // ride verbatim from the captured item.
    let src = "pub(crate) struct Old<T>(T);\n";
    let files = apply_multi(
        &cut_and_paste_struct_single("pub struct New { ... }"),
        vec![("a.rs", src), ("b.rs", B_STRUCT)],
    )
    .unwrap();
    assert!(
        files["b.rs"].contains("pub struct New<T>(T);"),
        "tuple suffix must ride verbatim, got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn struct_malformed_template_is_loud() {
    let err = apply_multi(
        &cut_and_paste_struct("pub struct New"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .expect_err("malformed struct template must fail loud");
    assert!(err.contains("evolution paste template"), "got: {err}");
}

#[test]
fn fn_template_on_struct_capture_is_loud() {
    // Kind mismatch: an `fn` template cannot splice a `struct` capture.
    let err = apply_multi(
        &cut_and_paste_struct("pub fn New(...) { ... }"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `fn`"), "got: {err}");
}

#[test]
fn struct_template_on_fn_capture_is_loud() {
    // Converse kind mismatch: a `struct` template cannot splice an
    // `fn` capture.
    let err = apply_multi(
        &cut_and_paste("pub struct New { ... }"),
        vec![("a.rs", SRC), ("b.rs", B_SRC)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `struct`"), "got: {err}");
}

// WI-8 slice 2: enum evolution pastes.

const ENUM_SRC: &str = "enum Old {\n    A,\n    B(i32),\n}\n";
const B_ENUM: &str = "fn other() {}\n";

fn cut_and_paste_enum(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,4 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

#[test]
fn enum_name_and_vis_evolve_cross_file() {
    let files = apply_multi(
        &cut_and_paste_enum("pub enum New { ... }"),
        vec![("a.rs", ENUM_SRC), ("b.rs", B_ENUM)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"], "fn other() {}\npub enum New {\n    A,\n    B(i32),\n}\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn enum_malformed_template_is_loud() {
    let err = apply_multi(
        &cut_and_paste_enum("pub enum New"),
        vec![("a.rs", ENUM_SRC), ("b.rs", B_ENUM)],
    )
    .expect_err("malformed enum template must fail loud");
    assert!(err.contains("evolution paste template"), "got: {err}");
}

#[test]
fn enum_template_on_struct_capture_is_loud() {
    // Kind mismatch: an `enum` template cannot splice a `struct` capture.
    let err = apply_multi(
        &cut_and_paste_struct("pub enum New { ... }"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `enum`"), "got: {err}");
}

#[test]
fn struct_template_on_enum_capture_is_loud() {
    // Converse kind mismatch: a `struct` template cannot splice an
    // `enum` capture.
    let err = apply_multi(
        &cut_and_paste_enum("pub struct New { ... }"),
        vec![("a.rs", ENUM_SRC), ("b.rs", B_ENUM)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `struct`"), "got: {err}");
}

// WI-8 slice 3: union evolution pastes.

const UNION_SRC: &str = "union Old {\n    x: i32,\n}\n";
const B_UNION: &str = "fn other() {}\n";

fn cut_and_paste_union(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

#[test]
fn union_name_and_vis_evolve_cross_file() {
    let files = apply_multi(
        &cut_and_paste_union("pub union New { ... }"),
        vec![("a.rs", UNION_SRC), ("b.rs", B_UNION)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"], "fn other() {}\npub union New {\n    x: i32,\n}\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn union_malformed_template_is_loud() {
    let err = apply_multi(
        &cut_and_paste_union("pub union New"),
        vec![("a.rs", UNION_SRC), ("b.rs", B_UNION)],
    )
    .expect_err("malformed union template must fail loud");
    assert!(err.contains("evolution paste template"), "got: {err}");
}

#[test]
fn union_template_on_struct_capture_is_loud() {
    // Kind mismatch: a `union` template cannot splice a `struct` capture.
    let err = apply_multi(
        &cut_and_paste_struct("pub union New { ... }"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `union`"), "got: {err}");
}

#[test]
fn struct_template_on_union_capture_is_loud() {
    // Converse kind mismatch: a `struct` template cannot splice a
    // `union` capture.
    let err = apply_multi(
        &cut_and_paste_union("pub struct New { ... }"),
        vec![("a.rs", UNION_SRC), ("b.rs", B_UNION)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `struct`"), "got: {err}");
}

// WI-8 slice 4: type-alias evolution pastes.

const TYPE_SRC: &str = "type Old = u32;\n";
const B_TYPE: &str = "fn other() {}\n";

fn cut_and_paste_type(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

#[test]
fn type_name_and_vis_evolve_cross_file() {
    let files = apply_multi(
        &cut_and_paste_type("pub type New = ...;"),
        vec![("a.rs", TYPE_SRC), ("b.rs", B_TYPE)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"], "fn other() {}\npub type New = u32;\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn type_generic_suffix_rides_verbatim() {
    let src = "type Old<T> = Vec<T>;\n";
    let files = apply_multi(
        &cut_and_paste_type("pub type New = ...;"),
        vec![("a.rs", src), ("b.rs", B_TYPE)],
    )
    .unwrap();
    assert!(
        files["b.rs"].contains("pub type New<T> = Vec<T>;"),
        "generic suffix must ride verbatim, got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn type_malformed_template_is_loud() {
    let err = apply_multi(
        &cut_and_paste_type("pub type New"),
        vec![("a.rs", TYPE_SRC), ("b.rs", B_TYPE)],
    )
    .expect_err("malformed type template must fail loud");
    assert!(err.contains("evolution paste template"), "got: {err}");
}

#[test]
fn type_template_on_struct_capture_is_loud() {
    // Kind mismatch: a `type` template cannot splice a `struct` capture.
    let err = apply_multi(
        &cut_and_paste_struct("pub type New = ...;"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `type`"), "got: {err}");
}

#[test]
fn struct_template_on_type_capture_is_loud() {
    // Converse kind mismatch: a `struct` template cannot splice a
    // `type` capture.
    let err = apply_multi(
        &cut_and_paste_type("pub struct New { ... }"),
        vec![("a.rs", TYPE_SRC), ("b.rs", B_TYPE)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `struct`"), "got: {err}");
}

// WI-8 slice 5: const evolution pastes.

const CONST_SRC: &str = "const Old: u32 = 1;\n";
const B_CONST: &str = "fn other() {}\n";

fn cut_and_paste_const(template: &str) -> String {
    format!(
        "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 @@ rule=cut_old\n# bind=M1\n- I\n\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 @@ rule=paste_new\n# use=M1\n+ {template}\n"
    )
}

#[test]
fn const_name_and_vis_evolve_cross_file() {
    let files = apply_multi(
        &cut_and_paste_const("pub const New = ...;"),
        vec![("a.rs", CONST_SRC), ("b.rs", B_CONST)],
    )
    .unwrap();
    assert_eq!(files["a.rs"], "", "got: {:?}", files["a.rs"]);
    assert_eq!(
        files["b.rs"], "fn other() {}\npub const New: u32 = 1;\n",
        "got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn const_generic_suffix_rides_verbatim() {
    let src = "const Old: [u8; 3] = [1, 2, 3];\n";
    let files = apply_multi(
        &cut_and_paste_const("pub const New = ...;"),
        vec![("a.rs", src), ("b.rs", B_CONST)],
    )
    .unwrap();
    assert!(
        files["b.rs"].contains("pub const New: [u8; 3] = [1, 2, 3];"),
        "type+expr suffix must ride verbatim, got: {:?}",
        files["b.rs"]
    );
}

#[test]
fn const_malformed_template_is_loud() {
    let err = apply_multi(
        &cut_and_paste_const("pub const New"),
        vec![("a.rs", CONST_SRC), ("b.rs", B_CONST)],
    )
    .expect_err("malformed const template must fail loud");
    assert!(err.contains("evolution paste template"), "got: {err}");
}

#[test]
fn const_template_on_struct_capture_is_loud() {
    // Kind mismatch: a `const` template cannot splice a `struct` capture.
    let err = apply_multi(
        &cut_and_paste_struct("pub const New = ...;"),
        vec![("a.rs", STRUCT_SRC), ("b.rs", B_STRUCT)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `const`"), "got: {err}");
}

#[test]
fn struct_template_on_const_capture_is_loud() {
    // Converse kind mismatch: a `struct` template cannot splice a
    // `const` capture.
    let err = apply_multi(
        &cut_and_paste_const("pub struct New { ... }"),
        vec![("a.rs", CONST_SRC), ("b.rs", B_CONST)],
    )
    .expect_err("kind mismatch must fail loud");
    assert!(err.contains("captured `struct`"), "got: {err}");
}
