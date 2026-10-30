//! WI-3a: match-count machine repairs (SPEC D.2): zero-match `widen_scope` /
//! `read_file` and too-many `add_directive` in both text and JSON modes,
//! plus the mechanical first-retry probe (apply `repair` verbatim, retry
//! green without reading prose).

mod common;
use common::*;

fn check_json(patch: &str, file: &str, content: &str) -> serde_json::Value {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

#[test]
fn widen_scope_repair_pins_and_retries_green() {
    let content = "fn m() {\n    let x = 1;\n    foo(1);\n}\n";
    // Ranged hunk cites lines 2..3 (one line), the single `foo(1)` site is
    // at line 3 — zero-match in range, exactly one match file-wide.
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=rename_calls\n# expr E\n- foo(E)\n+ bar(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({"action":"widen_scope","directive":"# scope=file","range":"@@ -1,1 @@"})
    );
    // Mechanical first-retry: apply repair verbatim (directive + range),
    // retry without reading prose — must go green.
    let retry =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_calls\n# scope=file\n# expr E\n- foo(E)\n+ bar(E)\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), retry, &["--check"]);
    assert!(
        out.status.success(),
        "mechanical retry must pass, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Text mode carries the same repair line.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("widen_scope"), "got: {stderr}");
}

#[test]
fn read_file_repair_pins_json_and_text() {
    let content = "fn foo() {}\nfn bar() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=rename_fn\n# lang=rust\n- fn baz(...) {\n+ fn qux(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["repair"]["action"],
        serde_json::json!("read_file")
    );
    assert_eq!(
        v["error"]["repair"]["file"],
        serde_json::json!("src/lib.rs")
    );
    assert_eq!(v["error"]["repair"]["range"], serde_json::json!("1..3"));
    assert!(
        v["error"]["repair"]["candidate"].is_string(),
        "T67 excerpt must ride the repair, got: {}",
        v["error"]["repair"]
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("read_file"), "got: {stderr}");
}

// WI-3b: `fix_pattern` / capture-symbol repairs (SPEC D.2): C-table
// `fix_pattern` in JSON + text, symbol `add_directive` with a mechanical
// first-retry probe, and capture `fix_name` with the available list.

#[test]
fn fix_pattern_repair_pins_c3_json_and_text() {
    let content = "fn a() {}\nfn b() {}\n";
    // C3: `# root=stmt` with a non-`let` shape.
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# lang=rust\n# root=stmt\n- foo\n+ bar\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({"action":"fix_pattern","code":"C3","write_instead":"write a `let` statement pattern"})
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("fix_pattern"), "got: {stderr}");
}

#[test]
fn symbol_repair_pins_and_retries_green() {
    let content = "fn a() {\n    crate::old_api(1);\n}\n";
    // C13: `# symbol` without `# fallback=syntax`.
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r\n# lang=rust\n# symbol Old=crate::old_api\n# expr E\n- Old(E)\n+ crate::new_api(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_SYMBOL_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({"action":"add_directive","directive":"# fallback=syntax"})
    );
    // Mechanical first-retry: apply repair verbatim, retry green.
    let retry = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r\n# lang=rust\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), retry, &["--check"]);
    assert!(
        out.status.success(),
        "mechanical retry must pass, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("add_directive"), "got: {stderr}");
}

#[test]
fn capture_fix_name_repair_pins_json_and_text() {
    let content = "fn a() {}\nfn b() {}\n";
    // Unbound `# use=` paste with no `# bind=` hunk anywhere.
    let patch = "diff --git src/lib.rs\n@@ -2,0 @@ rule=paste\n# use=Helper\n+ Helper\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_CAPTURE_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({"action":"fix_name","name":"Helper","available":[]})
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("fix_name"), "got: {stderr}");
}

#[test]
fn too_many_repair_pins_json_and_text() {
    let content = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
    let patch =
        "diff --git src/lib.rs\n@@ -1,4 @@ rule=rename_calls\n# expr E\n- foo(E)\n+ bar(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({"action":"add_directive","directive":"# allow=many"})
    );
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("add_directive"), "got: {stderr}");
}
