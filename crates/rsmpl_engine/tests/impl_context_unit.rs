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

#[test]
fn trailing_comma_where_matches_single_line_pattern() {
    // T224: rustfmt-canonical multi-line `where` (trailing comma) matches
    // the documented single-line pattern spelling; a pure rename keeps
    // the clause text, never collapses it to single-line.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,6 +1,6 @@ rule=x\n# lang=rust\n- fn f<T>(...) -> T where T: Clone {\n+ fn g<T>(...) -> T where T: Clone {\n      ...\n  }\n";
    let src = "fn f<T>(x: T) -> T\nwhere\n    T: Clone,\n{\n    x\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("fn g<T>(x: T) -> T"), "got: {out}");
    assert!(
        out.contains("    T: Clone,"),
        "where clause kept, got: {out}"
    );
}

#[test]
fn trailing_comma_params_match_single_line_pattern() {
    // T224 params half: multi-line params with trailing comma match the
    // single-line pattern; a pure rename keeps the list, never collapses it.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,6 +1,6 @@ rule=x\n# lang=rust\n- fn f(x: i32, y: i32) -> i32 {\n+ fn g(x: i32, y: i32) -> i32 {\n      ...\n  }\n";
    let src = "fn f(\n    x: i32,\n    y: i32,\n) -> i32 {\n    x + y\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("fn g("), "got: {out}");
    assert!(out.contains("    y: i32,"), "params kept, got: {out}");
}

#[test]
fn trailing_comma_where_matches_inside_impl() {
    // T224 via the impl-context slot planner: same tolerance for methods.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,8 +2,8 @@ rule=x\n# lang=rust\n  impl S {\n      ...\n-     fn f<T>(...) -> T where T: Clone {\n+     fn g<T>(...) -> T where T: Clone {\n          ...\n      }\n  }\n";
    let src = "struct S;\nimpl S {\n    fn f<T>(x: T) -> T\n    where\n        T: Clone,\n    {\n        x\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("fn g<T>(x: T) -> T"), "got: {out}");
    assert!(
        out.contains("        T: Clone,"),
        "where clause kept, got: {out}"
    );
}

#[test]
fn slot_metavars_rename_methods_inside_impl() {
    // T337: `V Q fn ... G (...) W` in method context failed fast with
    // `fn pattern must contain `fn` keyword` — `extract_fn_block` did not
    // recognise a leading slot-metavar line as the fn-block start.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# scope=file\n# allow=many\n# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n  impl Svc {\n      ...\n-     V Q fn old_name G (...) W {\n+     V Q fn new_name G (...) W {\n          ...\n      }\n  }\n";
    let src = "struct A;\nstruct B<T>(T);\nimpl A {\n    fn old_name(&self) -> i32 {\n        1\n    }\n}\nimpl B<i32> {\n    pub async fn old_name<T>(&self, x: T) -> T where T: Clone {\n        x.clone()\n    }\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("fn new_name(&self) -> i32"), "got: {out}");
    assert!(
        out.contains("pub async fn new_name<T>(&self, x: T) -> T where T: Clone"),
        "got: {out}"
    );
    assert!(!out.contains("old_name(&self)"), "got: {out}");
}

#[test]
fn where_predicate_difference_still_fails() {
    // T224 negative control: trailing-comma tolerance must not equate
    // genuinely different predicates.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n- fn f<T>(...) where T: Clone {\n+ fn g<T>(...) where T: Clone {\n      ...\n  }\n";
    let src = "fn f<T>(x: T) where T: Debug {\n    x\n}\n";
    let err = apply(patch, src).unwrap_err();
    assert!(err.contains("no match found"), "got: {err}");
}
