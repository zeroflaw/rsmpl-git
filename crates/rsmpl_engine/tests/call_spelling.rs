// WI-7d call-path spelling tests (moved byte-identical from
// `matcher_expr.rs` inline tests per the T170/T171 recipe: WI-7d
// helpers needed headroom under the 2000-LOC cap).
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
fn leading_colon_parity_both_directions() {
    // WI-7d: `::a::old(E)` matches only `::a::old` sites; a bare
    // `a::old(E)` pattern never touches `::a::old` sites.
    let lead = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- ::a::old(E)\n+ ::a::new(E)\n";
    let src = "fn m() {\n    ::a::old(1);\n    a::old(2);\n}\n";
    let out = apply(lead, src).unwrap();
    assert!(out.contains("::a::new(1);"), "got: {out}");
    assert!(out.contains("a::old(2);"), "bare twin must survive: {out}");
    let bare = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- a::old(E)\n+ a::new(E)\n";
    let out = apply(bare, src).unwrap();
    assert!(out.contains("a::new(2);"), "got: {out}");
    assert!(
        out.contains("::a::old(1);"),
        "`::` site must survive: {out}"
    );
}

#[test]
fn leading_colon_turbofish_parity() {
    // WI-7d: turbofish patterns compare leading too.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- ::a::old::<u8>(E)\n+ ::a::new::<u8>(E)\n";
    let src = "fn m() {\n    ::a::old::<u8>(1);\n    a::old::<u8>(2);\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("::a::new::<u8>(1);"), "got: {out}");
    assert!(
        out.contains("a::old::<u8>(2);"),
        "bare twin must survive: {out}"
    );
}

#[test]
fn qself_token_equal_only() {
    // WI-7d: `<T as Tr>::old(E)` matches only identical-qualifier
    // sites — bare and divergent-qualifier sites survive.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- <Target as Trait>::old(E)\n+ <Target as Trait>::new(E)\n";
    let src =
        "fn m() {\n    <Target as Trait>::old(1);\n    <Other as Trait>::old(2);\n    old(3);\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("<Target as Trait>::new(1);"), "got: {out}");
    assert!(
        out.contains("<Other as Trait>::old(2);"),
        "divergent qualifier must survive: {out}"
    );
    assert!(out.contains("old(3);"), "bare site must survive: {out}");
}

#[test]
fn bare_pattern_ignores_qself_site() {
    // WI-7d converse: a bare `old(E)` pattern never matches a
    // `<T as Tr>::old` site (previously lenient).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- old(E)\n+ new(E)\n";
    let src = "fn m() {\n    <Target as Trait>::old(1);\n    old(2);\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("new(2);"), "got: {out}");
    assert!(
        out.contains("<Target as Trait>::old(1);"),
        "qself site must survive: {out}"
    );
}

#[test]
fn general_shape_qself_parity() {
    // WI-7d: the general matcher compares `qself` too — a
    // `<T as Tr>::old(E) + 1` pattern (Raw shape, call trailer fails)
    // never matches a bare `old(1) + 1` site and vice versa.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- <Target as Trait>::old(E) + 1\n+ <Target as Trait>::new(E) + 1\n";
    let src = "fn m() {\n    <Target as Trait>::old(1) + 1;\n    old(2) + 1;\n}\n";
    let out = apply(patch, src).unwrap();
    assert!(out.contains("<Target as Trait>::new(1) + 1;"), "got: {out}");
    assert!(out.contains("old(2) + 1;"), "bare site must survive: {out}");
    let bare = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- old(E) + 1\n+ new(E) + 1\n";
    let out = apply(bare, src).unwrap();
    assert!(out.contains("new(2) + 1;"), "got: {out}");
    assert!(
        out.contains("<Target as Trait>::old(1) + 1;"),
        "qself site must survive: {out}"
    );
}
