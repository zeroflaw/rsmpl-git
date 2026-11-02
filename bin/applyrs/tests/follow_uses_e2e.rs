//! WI-6a: `# follow=uses` same-file rename completion (SPEC 14.8).
//!
//! One hunk rewrites the definition plus its same-file use sites (type
//! paths, struct literals, values, calls, `use` trees) while method idents
//! and field accesses stay untouched. Shadowing and macro-opacity veto
//! loud without `# fallback=syntax`, skip + report with it; any other
//! shape carrying `# follow=uses` fails at the SPEC 18.6 gate.

mod common;
use common::*;

const STRUCT_BEFORE: &str = "use m::Old;\nstruct Old {\n    field: i32,\n}\nfn f(x: Old) -> Old {\n    let y: Old = Old { field: 1 };\n    let z = Old;\n    z\n}\n";

const STRUCT_PATCH: &str =
    "diff --git src/lib.rs\n@@ -2,3 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

const STRUCT_WANT: &str = "use m::New;\nstruct New {\n    field: i32,\n}\nfn f(x: New) -> New {\n    let y: New = New { field: 1 };\n    let z = New;\n    z\n}\n";

#[test]
fn struct_follow_rewrites_definition_and_all_use_sites() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", STRUCT_BEFORE);
    let out = run_stdin(dir.path(), STRUCT_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), STRUCT_WANT);
}

#[test]
fn struct_rename_without_follow_stays_definition_only() {
    // No behavior change when the directive is absent.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", STRUCT_BEFORE);
    let patch = "diff --git src/lib.rs\n@@ -2,3 @@ rule=rename\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("struct New"), "got:\n{got}");
    assert!(got.contains("x: Old"), "uses untouched, got:\n{got}");
}

const FN_BEFORE: &str = "fn old(x: i32) -> i32 {\n    x + 1\n}\nstruct S;\nfn g(s: S) -> i32 {\n    let a = old(1);\n    let b = m::old(2);\n    let c = s.old();\n    a + b + c\n}\n";

const FN_PATCH: &str = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_fn\n# follow=uses\n- fn old(...) {\n+ fn new(...) {\n      ...\n  }\n";

#[test]
fn fn_follow_rewrites_calls_but_not_methods() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", FN_BEFORE);
    let out = run_stdin(dir.path(), FN_PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("fn new(x: i32)"), "def renamed, got:\n{got}");
    assert!(got.contains("let a = new(1);"), "got:\n{got}");
    assert!(got.contains("m::new(2)"), "got:\n{got}");
    assert!(
        got.contains("s.old()"),
        "method ident untouched, got:\n{got}"
    );
    assert!(!got.contains("old(1)"), "got:\n{got}");
}

const SHADOW_BEFORE: &str = "mod a {\n    pub struct Old;\n}\nmod b {\n    pub struct Old;\n}\n";

const SHADOW_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

#[test]
fn shadowed_definition_vetoes_without_fallback() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", SHADOW_BEFORE);
    let out = run_stdin(dir.path(), SHADOW_PATCH, &["--check"]);
    assert!(!out.status.success(), "veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("shadowed"), "got: {stderr}");
}

#[test]
fn shadowed_definition_skips_with_fallback_and_reports() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", SHADOW_BEFORE);
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    // WI-20 v4: the `follow_skipped` prose string is removed; the
    // rule carries structured `follow_skipped_details` triples alone
    // (hunk-level, no span), not a `bindings` entry.
    assert!(
        v["rules"][0].get("follow_skipped").is_none(),
        "v4 removed the prose string, got: {v}"
    );
    let details = v["rules"][0]["follow_skipped_details"]
        .as_array()
        .expect("follow_skipped_details present");
    assert!(
        details.iter().any(|d| d["reason"] == "shadowed"),
        "fallback skip reported, got: {v}"
    );
}

#[test]
fn follow_on_call_shape_is_rejected() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {\n    foo(1);\n}\n");
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# follow=uses\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "gate rejection expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("# follow=uses"), "got: {stderr}");
}

#[test]
fn cross_file_uses_rewrite_without_targeting_their_file() {
    // WI-6b1: the patch sections only `src/a.rs`; `src/b.rs` is found by
    // the CLI workspace load and rewritten under the same rule.
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
    );
    let patch =
        "diff --git src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/a.rs"), "pub struct New;\n");
    assert_eq!(
        read(dir.path(), "src/b.rs"),
        "use crate::New;\nfn f(x: New) -> New {\n    New\n}\n"
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"src/a.rs"), "got: {v}");
    assert!(files.contains(&"src/b.rs"), "got: {v}");
    let rules: Vec<(&str, &str)> = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| (r["rule"].as_str().unwrap(), r["file"].as_str().unwrap()))
        .collect();
    assert!(rules.contains(&("rename", "src/a.rs")), "got: {v}");
    assert!(rules.contains(&("rename", "src/b.rs")), "got: {v}");
}

#[test]
fn macro_mention_vetoes_without_fallback() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct Old;\nfn f() {\n    m!(Old);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "opacity veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("opaque"), "got: {stderr}");
}

#[test]
fn fn_attribute_value_vetoes_then_skips_with_fallback() {
    // Attribute values are opaque to item renames (binder parity):
    // `#[doc = old]` vetoes without fallback, rewrites def + call
    // and reports `macro-hidden` with fallback while the value stays
    // byte-verbatim.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn old_fn() {}\nfn f() {\n    old_fn();\n}\n#[doc = old_fn]\nfn g() {}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=fn_attr_val\n# follow=uses\n- fn old_fn\n+ fn new_fn\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(!out.status.success(), "opacity veto expected");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["error"]["code"], "RSMPL_SYMBOL_CONFLICT", "got: {v}");

    let patch_fb = "diff --git src/lib.rs\n@@ -1,1 @@ rule=fn_attr_val\n# follow=uses\n# fallback=syntax\n- fn old_fn\n+ fn new_fn\n";
    let out = run_stdin(
        dir.path(),
        patch_fb,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["checked"]["semantics"], "partial", "got: {v}");
    let details = v["rules"][0]["follow_skipped_details"]
        .as_array()
        .expect("skips present");
    assert!(
        details
            .iter()
            .any(|d| d["line"] == 5 && d["reason"] == "macro-hidden"),
        "attr line named, got: {v}"
    );

    // Real apply: def + call rewrite, attribute value verbatim.
    let out = run_stdin(dir.path(), patch_fb, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("fn new_fn() {}"), "got:\n{got}");
    assert!(got.contains("new_fn();"), "got:\n{got}");
    assert!(got.contains("#[doc = old_fn]"), "value kept, got:\n{got}");
}

#[test]
fn trait_method_attribute_value_vetoes_then_skips_with_fallback() {
    // Attribute values are opaque to method renames (fn/item parity):
    // bare (`#[doc = old]`), qualified (`#[doc = S::old]`), and list
    // (`#[foo(old)]`) idents veto without fallback, rewrite def + impl
    // def + anchored call and report `macro-hidden` with fallback while
    // every value stays byte-verbatim. Pre-fix, the qualified value
    // rewrote to `#[doc = S::new]` with `verified` and no orphans.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "trait Tr { fn old(&self); }\nstruct S;\nimpl Tr for S { fn old(&self) {} }\nfn f(s: S) { s.old(); }\n#[doc = old]\n#[doc = S::old]\n#[foo(old)]\nfn g() {}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=method_attr_val\n# follow=uses\n  trait Tr {\n-     fn old(&self);\n+     fn new(&self);\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(!out.status.success(), "opacity veto expected");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["error"]["code"], "RSMPL_SYMBOL_CONFLICT", "got: {v}");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("an attribute"),
        "attr ctx named, got: {v}"
    );

    let patch_fb = "diff --git src/lib.rs\n@@ -1,2 @@ rule=method_attr_val\n# follow=uses\n# fallback=syntax\n  trait Tr {\n-     fn old(&self);\n+     fn new(&self);\n";
    let out = run_stdin(
        dir.path(),
        patch_fb,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["checked"]["semantics"], "partial", "got: {v}");
    let details = v["rules"][0]["follow_skipped_details"]
        .as_array()
        .expect("skips present");
    for line in [5, 6, 7] {
        assert!(
            details
                .iter()
                .any(|d| d["line"] == line && d["reason"] == "macro-hidden"),
            "attr line {line} named, got: {v}"
        );
    }

    // Real apply: trait decl + impl def + anchored call rewrite, every
    // attribute value verbatim.
    let out = run_stdin(dir.path(), patch_fb, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("trait Tr { fn new(&self); }"), "got:\n{got}");
    assert!(
        got.contains("impl Tr for S { fn new(&self) {} }"),
        "got:\n{got}"
    );
    assert!(got.contains("s.new();"), "got:\n{got}");
    assert!(got.contains("#[doc = old]"), "bare value kept, got:\n{got}");
    assert!(
        got.contains("#[doc = S::old]"),
        "qualified kept, got:\n{got}"
    );
    assert!(got.contains("#[foo(old)]"), "list kept, got:\n{got}");
}

#[test]
fn trait_method_attribute_string_literal_stays_silent() {
    // String literals are not idents: `#[doc = "old"]` never fires the
    // opacity gate — `verified`, no skips, untouched.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "trait Tr { fn old(&self); }\nstruct S;\nimpl Tr for S { fn old(&self) {} }\nfn f(s: S) { s.old(); }\n#[doc = \"old\"]\nfn g() {}\n",
    );
    let patch_fb = "diff --git src/lib.rs\n@@ -1,2 @@ rule=method_attr_lit\n# follow=uses\n  trait Tr {\n-     fn old(&self);\n+     fn new(&self);\n";
    let out = run_stdin(
        dir.path(),
        patch_fb,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["checked"]["semantics"], "verified", "got: {v}");

    let out = run_stdin(dir.path(), patch_fb, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "src/lib.rs");
    assert!(got.contains("s.new();"), "got:\n{got}");
    assert!(
        got.contains("#[doc = \"old\"]"),
        "literal kept, got:\n{got}"
    );
}
