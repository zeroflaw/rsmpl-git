// T175/T190 impl-context unit tests (moved byte-identical from
// `matcher_fn.rs` inline tests per the T170/T171 recipe: the T221
// `replace_self_receiver` helper needed headroom under the 2000-LOC cap).
use std::collections::HashMap;

fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
    let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
    let mut files = HashMap::new();
    files.insert("src/lib.rs".to_string(), file_text.to_string());
    let mut patch = patch;
    for f in &mut patch.files {
        f.target_path = "src/lib.rs".to_string();
    }
    rsmpl_engine::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
    Ok(files["src/lib.rs"].clone())
}

#[test]
fn evolves_pub_assoc_fn_inside_impl() {
    // T175: the impl-context presence gate builds its synthetic
    // ItemFn from the `Signature` only, which carries no visibility.
    // The synth now carries the method's real vis, so an explicit
    // `pub` pattern matches a `pub` method (bare patterns are
    // unaffected: inherited vis renders empty).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,5 +3,5 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     pub fn make(x: i32) -> i32 {\n+     pub fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let src = "struct S;\n\nimpl S {\n    pub fn make(x: i32) -> i32 {\n        x\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("pub fn make(x: i64) -> i64"), "got: {out}");
}

#[test]
fn evolves_pub_self_method_inside_impl() {
    // T175 `&self` variant: the receiver is preserved, never rebound.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,5 +3,5 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     pub fn get(&self, x: i32) -> i32 {\n+     pub fn get(&self, x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let src =
        "struct S;\n\nimpl S {\n    pub fn get(&self, x: i32) -> i32 {\n        x\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(
        out.contains("pub fn get(&self, x: i64) -> i64"),
        "got: {out}"
    );
}

#[test]
fn bare_vis_pattern_still_rejects_pub_method() {
    // T175 negative control: strictness parity holds -- a vis-less
    // pattern still requires inherited vis, even in impl context.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,5 +3,5 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     fn make(...) {\n+     fn put(...) {\n          ...\n      }\n  }\n";
    let src = "struct S;\n\nimpl S {\n    pub fn make(x: i32) -> i32 {\n        x\n    }\n}\n";
    let err = apply(patch, src).unwrap_err();
    assert!(err.contains("no match found"), "got: {err}");
}

#[test]
fn evolves_const_fn_inside_impl() {
    // T190: the impl-context synth prepended `fn ` to any sig not
    // starting with it, so `const fn ...` became `fn const fn ...`
    // (unparsable, bare `cannot analyze method signature`).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,5 +3,5 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     const fn make(x: i32) -> i32 {\n+     const fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let src =
        "struct S;\n\nimpl S {\n    const fn make(x: i32) -> i32 {\n        x + 1\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("const fn make(x: i64) -> i64"), "got: {out}");
}

#[test]
fn evolves_async_fn_inside_impl() {
    // T190: same qualifier-prefix class as `const fn`.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,5 +3,5 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     async fn make(x: i32) -> i32 {\n+     async fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let src =
        "struct S;\n\nimpl S {\n    async fn make(x: i32) -> i32 {\n        x + 1\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("async fn make(x: i64) -> i64"), "got: {out}");
}

#[test]
fn evolves_unsafe_fn_inside_impl() {
    // T190: same class; `unsafe` doubles as an `extract_fn_block`
    // start prefix, so this pins both halves.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,5 +3,5 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     unsafe fn make(x: i32) -> i32 {\n+     unsafe fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let src =
        "struct S;\n\nimpl S {\n    unsafe fn make(x: i32) -> i32 {\n        x + 1\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("unsafe fn make(x: i64) -> i64"), "got: {out}");
}

// T221: word-aware `self`-receiver synth (moved-block companion tests).
// The blind `str::replace("self", …)` broke lifetime receivers, idents
// containing `self`, and typed receivers with the bare
// `cannot analyze method signature` needle.

#[test]
fn evolves_lifetime_receiver_inside_impl() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,4 +2,4 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     fn get<'a>(&'a self, x: &'a i32) -> &'a i32 {\n+     fn fetch<'a>(&'a self, x: &'a i32) -> &'a i32 {\n          ...\n      }\n  }\n";
    let src = "struct S;\nimpl S {\n    fn get<'a>(&'a self, x: &'a i32) -> &'a i32 {\n        x\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(
        out.contains("fn fetch<'a>(&'a self, x: &'a i32) -> &'a i32"),
        "got: {out}"
    );
}

#[test]
fn evolves_mut_lifetime_receiver_inside_impl() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,4 +2,4 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     fn get<'a>(&'a mut self, x: &'a i32) -> &'a i32 {\n+     fn fetch<'a>(&'a mut self, x: &'a i32) -> &'a i32 {\n          ...\n      }\n  }\n";
    let src = "struct S;\nimpl S {\n    fn get<'a>(&'a mut self, x: &'a i32) -> &'a i32 {\n        x\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(
        out.contains("fn fetch<'a>(&'a mut self, x: &'a i32) -> &'a i32"),
        "got: {out}"
    );
}

#[test]
fn evolves_method_with_self_named_param_inside_impl() {
    // `myself` merely contains `self`: the synth must not touch it.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,4 +2,4 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     fn handle(myself: i32, other: i32) -> i32 {\n+     fn handle2(myself: i32, other: i32) -> i32 {\n          ...\n      }\n  }\n";
    let src = "struct S;\nimpl S {\n    fn handle(myself: i32, other: i32) -> i32 {\n        myself + other\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(
        out.contains("fn handle2(myself: i32, other: i32) -> i32"),
        "got: {out}"
    );
}

#[test]
fn evolves_typed_self_receiver_inside_impl() {
    // `self: Box<Self>` keeps its type in the synth: the placeholder
    // carries the written type instead of gaining a bogus `: u8`.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,4 +2,4 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     fn boxed(self: Box<Self>) -> i32 {\n+     fn unboxed(self: Box<Self>) -> i32 {\n          ...\n      }\n  }\n";
    let src = "struct S;\nimpl S {\n    fn boxed(self: Box<Self>) -> i32 {\n        1\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(
        out.contains("fn unboxed(self: Box<Self>) -> i32"),
        "got: {out}"
    );
}
