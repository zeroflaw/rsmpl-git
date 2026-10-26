//! End-to-end tests driving the `rsmpl-git` binary in temp worktrees.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"))
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(st.success());
    fs::create_dir_all(dir.path().join("src")).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, text: &str) {
    fs::write(dir.join(rel), text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn inline_fn_rename_applies() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run(dir.path(), &["--patch", patch]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("fn bar(x: i32)"), "got: {text}");
}

#[test]
fn check_prints_diff_and_does_not_modify() {
    let dir = init_repo();
    let before = "fn a() {\n    foo(1);\n    foo(2);\n}\n";
    write(&dir.path().join(""), "src/lib.rs", before);
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, patch).unwrap();

    let out = run(
        dir.path(),
        &["--file", patch_file.to_str().unwrap(), "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("diff --git a/src/lib.rs b/src/lib.rs"),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("# lang=rust"),
        "lowered diff must not contain directives"
    );
    assert!(
        stdout.contains("-    foo(1);") || stdout.contains("-    foo(2);"),
        "got: {stdout}"
    );
    // --check must not modify the file.
    assert_eq!(read(dir.path(), "src/lib.rs"), before);
}

#[test]
fn no_match_exits_two_with_structured_error() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run(dir.path(), &["--patch", patch]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
}

#[test]
fn stdin_patch_applies() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let mut child = Command::new(binary())
        .args(["--patch", "-", "--repo"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("fn bar(x: i32)"));
}

#[test]
fn json_report_carries_rules_and_diff() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, patch).unwrap();
    let out = Command::new(binary())
        .arg("--file")
        .arg(&patch_file)
        .arg("--repo")
        .arg(dir.path())
        .args(["--format", "json", "--check"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["rule"], serde_json::json!("r"));
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1));
    assert!(v["diff"].as_str().unwrap().contains("diff --git"));
    assert!(!v["diff"].as_str().unwrap().contains("# lang=rust"));
    assert_eq!(v["check"], serde_json::json!("ok"));
    // --check must not modify the file.
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn json_envelope_on_match_failure() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = Command::new(binary())
        .arg("--patch")
        .arg(patch)
        .arg("--repo")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    let err = &v["error"];
    assert_eq!(err["code"], serde_json::json!("RSMPL_MATCH_CONFLICT"));
    assert_eq!(err["rule"], serde_json::json!("r"));
    assert_eq!(err["file"], serde_json::json!("src/lib.rs"));
    assert!(
        err["reason"].as_str().unwrap().contains("no match"),
        "got: {err}"
    );
    assert!(err["suggestion"].is_string(), "got: {err}");
    assert!(
        err["suggestion"]
            .as_str()
            .unwrap()
            .contains("--inspect src/lib.rs"),
        "suggestion must carry the discovery command, got: {err}"
    );
    // File untouched by the failed run.
    assert!(read(dir.path(), "src/lib.rs").contains("fn a()"));
}

#[test]
fn json_envelope_on_range_failure_carries_inspect_command() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -90,8 +90,8 @@ rule=r\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let out = Command::new(binary())
        .arg("--patch")
        .arg(patch)
        .arg("--repo")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    let err = &v["error"];
    assert_eq!(err["code"], serde_json::json!("RSMPL_RANGE_CONFLICT"));
    assert_eq!(err["file"], serde_json::json!("src/lib.rs"));
    assert!(
        err["suggestion"]
            .as_str()
            .unwrap()
            .contains("--inspect src/lib.rs"),
        "suggestion must carry the discovery command, got: {err}"
    );
}

#[test]
fn json_envelope_on_missing_patch_file() {
    let dir = init_repo();
    let out = Command::new(binary())
        .arg("--file")
        .arg("does-not-exist.rsmpl")
        .arg("--repo")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert!(v["error"]["file"].is_string(), "got: {v}");
}

#[test]
fn json_envelope_on_inspect_parse_failure() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn broken( {}\n");
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
}

#[test]
fn inspect_calls_lists_sites_in_text_and_json() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo() {}\nfn a() {\n    foo(1);\n    obj.method(x);\n    assert_eq!(a, b);\n}\n",
    );
    // Text mode: items first (unchanged), then calls.
    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--calls"])
        .output()
        .unwrap();
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout).to_string();
    assert!(stdout.contains("fn foo 1-1"), "got: {stdout}");
    assert!(stdout.contains("call foo 3-3"), "got: {stdout}");
    assert!(stdout.contains("method method 4-4"), "got: {stdout}");
    assert!(stdout.contains("macro assert_eq 5-5"), "got: {stdout}");

    // JSON mode: items + calls.
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--calls", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let calls = v["calls"].as_array().expect("calls array present");
    assert!(calls
        .iter()
        .any(|c| c["kind"] == "call" && c["name"] == "foo"));
    assert!(calls
        .iter()
        .any(|c| c["kind"] == "method" && c["name"] == "method"));
    assert!(calls
        .iter()
        .any(|c| c["kind"] == "macro" && c["name"] == "assert_eq"));
}

#[test]
fn inspect_without_calls_omits_calls_key() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert!(v.get("calls").is_none(), "got: {v}");
}

#[test]
fn calls_without_inspect_is_usage_error() {
    let dir = init_repo();
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--calls"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("--calls requires --inspect"),
        "got: {stderr}"
    );
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    // JSON mode: same failure as a structured envelope on stdout.
    let json_out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--calls", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(json_out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("--calls requires --inspect"),
        "got: {v}"
    );
    assert!(v["error"]["suggestion"].is_string(), "got: {v}");
}

#[test]
fn inspect_dir_lists_every_rs_file_in_text_and_json() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    write(&dir.path().join(""), "src/a.rs", "struct S;\n");
    fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    write(
        &dir.path().join(""),
        "src/nested/b.rs",
        "fn b() {\n    foo(1);\n}\n",
    );
    write(&dir.path().join(""), "src/notes.txt", "not rust\n");
    fs::create_dir_all(dir.path().join("target")).unwrap();
    write(&dir.path().join(""), "target/t.rs", "fn t() {}\n");

    // Text mode: grep-style `file: kind name s-e`, files sorted.
    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src", "--calls"])
        .output()
        .unwrap();
    assert!(
        text.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&text.stderr)
    );
    let stdout = String::from_utf8_lossy(&text.stdout).to_string();
    assert!(stdout.contains("src/a.rs: struct S 1-1"), "got: {stdout}");
    assert!(stdout.contains("src/lib.rs: fn foo 1-1"), "got: {stdout}");
    assert!(
        stdout.contains("src/nested/b.rs: fn b 1-3"),
        "got: {stdout}"
    );
    assert!(
        stdout.contains("src/nested/b.rs: call foo 2-2"),
        "got: {stdout}"
    );
    assert!(!stdout.contains("notes.txt"), "got: {stdout}");
    assert!(!stdout.contains("target/t.rs"), "got: {stdout}");
    let lib_pos = stdout.find("src/lib.rs").unwrap();
    let nested_pos = stdout.find("src/nested/b.rs").unwrap();
    assert!(lib_pos < nested_pos, "files sorted, got: {stdout}");

    // JSON mode: `files` array with per-file items (+ calls).
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src", "--calls", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["path"], serde_json::json!("src"));
    let files = v["files"].as_array().expect("files array present");
    assert_eq!(files.len(), 3, "got: {v}");
    let lib = files.iter().find(|f| f["path"] == "src/lib.rs").unwrap();
    assert!(lib["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "foo"));
    assert!(lib.get("calls").is_some(), "got: {lib}");
    let nested = files
        .iter()
        .find(|f| f["path"] == "src/nested/b.rs")
        .unwrap();
    assert!(nested["calls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "foo"));

    // Without --calls the per-file `calls` key is omitted (compat rule).
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    for f in v["files"].as_array().unwrap() {
        assert!(f.get("calls").is_none(), "got: {f}");
    }
}

#[test]
fn inspect_dir_skips_broken_file_with_warning() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/good.rs", "fn good() {}\n");
    write(&dir.path().join(""), "src/broken.rs", "fn broken( {}\n");
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("src/good.rs: fn good 1-1"), "got: {stdout}");
    assert!(!stdout.contains("broken.rs: fn"), "got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("skipping src/broken.rs"), "got: {stderr}");

    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let files = v["files"].as_array().unwrap();
    assert!(files.iter().any(|f| f["path"] == "src/good.rs"));
    assert!(
        !files.iter().any(|f| f["path"] == "src/broken.rs"),
        "got: {v}"
    );
}

#[test]
fn inspect_missing_path_is_io_conflict() {
    let dir = init_repo();
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "does-not-exist/"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
}

#[test]
fn inspect_lists_item_spans() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo() {}\nstruct S;\nimpl S {\n    fn m(&self) {}\n}\n",
    );
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let items = v["items"].as_array().unwrap();
    let foo = items.iter().find(|i| i["name"] == "foo").unwrap();
    assert_eq!(foo["kind"], serde_json::json!("fn"));
    assert_eq!(foo["start"], serde_json::json!(1));
    let method = items.iter().find(|i| i["name"] == "S::m").unwrap();
    assert_eq!(method["kind"], serde_json::json!("method"));

    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs"])
        .output()
        .unwrap();
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout).to_string();
    assert!(stdout.contains("fn foo 1-1"), "got: {stdout}");
}

#[test]
fn inspect_max_items_caps_dir_text_and_json() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    write(&dir.path().join(""), "src/a.rs", "struct S;\n");
    fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    write(
        &dir.path().join(""),
        "src/nested/b.rs",
        "fn b() {\n    foo(1);\n}\n",
    );

    // Text mode: 3 items + 1 call = 4 entries; cap 2 keeps the first two.
    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src", "--calls", "--max-items", "2"])
        .output()
        .unwrap();
    assert!(
        text.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&text.stderr)
    );
    let stdout = String::from_utf8_lossy(&text.stdout).to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "got: {stdout}");
    assert!(stdout.contains("src/a.rs: struct S 1-1"), "got: {stdout}");
    assert!(stdout.contains("src/lib.rs: fn foo 1-1"), "got: {stdout}");
    let stderr = String::from_utf8_lossy(&text.stderr).to_string();
    assert!(stderr.contains("truncated:"), "got: {stderr}");
    assert!(stderr.contains("2 more entries omitted"), "got: {stderr}");

    // JSON mode: same cap, `truncated` true, exactly 2 entries total.
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args([
            "--inspect",
            "src",
            "--calls",
            "--format",
            "json",
            "--max-items",
            "2",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["truncated"], serde_json::json!(true), "got: {v}");
    let files = v["files"].as_array().unwrap();
    let total: usize = files
        .iter()
        .map(|f| f["items"].as_array().unwrap().len() + f["calls"].as_array().unwrap().len())
        .sum();
    assert_eq!(total, 2, "got: {v}");

    // Uncapped JSON reports `truncated: false` (compat: extra key only).
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src", "--calls", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["truncated"], serde_json::json!(false), "got: {v}");
}

#[test]
fn inspect_max_items_single_file_truncates_calls_first() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo() {}\nfn a() {\n    foo(1);\n    obj.method(x);\n    assert_eq!(a, b);\n}\n",
    );

    // 2 items + 3 calls = 5 entries; cap 2 keeps items only.
    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--calls", "--max-items", "2"])
        .output()
        .unwrap();
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout).to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "got: {stdout}");
    assert!(stdout.contains("fn foo 1-1"), "got: {stdout}");
    assert!(!stdout.contains("call foo"), "got: {stdout}");
    assert!(
        String::from_utf8_lossy(&text.stderr).contains("truncated:"),
        "got: {}",
        String::from_utf8_lossy(&text.stderr)
    );

    // Cap 3 keeps items + first call.
    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--calls", "--max-items", "3"])
        .output()
        .unwrap();
    assert!(text.status.success());
    let stdout = String::from_utf8_lossy(&text.stdout).to_string();
    assert!(stdout.contains("call foo 3-3"), "got: {stdout}");
    assert!(!stdout.contains("method method"), "got: {stdout}");

    // JSON single-file carries `truncated` in both capped and uncapped runs.
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args([
            "--inspect",
            "src/lib.rs",
            "--format",
            "json",
            "--max-items",
            "1",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["items"].as_array().unwrap().len(), 1, "got: {v}");
    assert_eq!(v["truncated"], serde_json::json!(true), "got: {v}");

    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--format", "json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["truncated"], serde_json::json!(false), "got: {v}");
}

#[test]
fn max_items_without_inspect_is_usage_error() {
    let dir = init_repo();
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--max-items", "5"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("--max-items requires --inspect"),
        "got: {stderr}"
    );
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    // JSON mode: same failure as a structured envelope on stdout.
    let json_out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--max-items", "5", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(json_out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("--max-items requires --inspect"),
        "got: {v}"
    );
    assert!(v["error"]["suggestion"].is_string(), "got: {v}");
}

#[test]
fn inspect_max_items_zero_yields_truncated() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");

    let text = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args(["--inspect", "src/lib.rs", "--max-items", "0"])
        .output()
        .unwrap();
    assert!(text.status.success());
    assert!(
        String::from_utf8_lossy(&text.stdout).is_empty(),
        "got: {}",
        String::from_utf8_lossy(&text.stdout)
    );
    assert!(
        String::from_utf8_lossy(&text.stderr).contains("truncated:"),
        "got: {}",
        String::from_utf8_lossy(&text.stderr)
    );

    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args([
            "--inspect",
            "src/lib.rs",
            "--format",
            "json",
            "--max-items",
            "0",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert!(v["items"].as_array().unwrap().is_empty(), "got: {v}");
    assert_eq!(v["truncated"], serde_json::json!(true), "got: {v}");

    // Empty file with cap 0 is not truncated (nothing to omit).
    write(&dir.path().join(""), "src/empty.rs", "");
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path())
        .args([
            "--inspect",
            "src/empty.rs",
            "--format",
            "json",
            "--max-items",
            "0",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["truncated"], serde_json::json!(false), "got: {v}");
}

// --- T33 output budget (D.2.1): --diff-max-lines + files[] ---

fn two_call_patch() -> &'static str {
    "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n"
}

#[test]
fn json_diff_max_lines_zero_omits_diff_but_keeps_stats_and_check() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--diff-max-lines",
            "0",
            "--check",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["diff"], serde_json::json!(""), "got: {v}");
    assert_eq!(v["diff_truncated"], serde_json::json!(true), "got: {v}");
    assert_eq!(v["check"], serde_json::json!("ok"), "got: {v}");
    assert_eq!(
        v["files"],
        serde_json::json!([{"path": "src/lib.rs", "added": 2, "removed": 2}]),
        "got: {v}"
    );
    assert_eq!(
        v["files_changed"],
        serde_json::json!(["src/lib.rs"]),
        "got: {v}"
    );
    // --check must not modify the file even though the diff was omitted.
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn json_diff_max_lines_zero_no_changes_is_not_truncated() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# optional\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run(
        dir.path(),
        &[
            "--patch",
            patch,
            "--format",
            "json",
            "--diff-max-lines",
            "0",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["diff"], serde_json::json!(""), "got: {v}");
    assert_eq!(v["diff_truncated"], serde_json::json!(false), "got: {v}");
    assert_eq!(v["files"], serde_json::json!([]), "got: {v}");
}

#[test]
fn json_diff_max_lines_truncates_but_stats_stay_exact() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();

    let full = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(full.status.success());
    let full_v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&full.stdout)).expect("stdout is JSON");
    let full_lines: Vec<&str> = full_v["diff"].as_str().unwrap().lines().collect();
    assert!(full_lines.len() > 2, "got: {full_v}");

    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--check",
            "--diff-max-lines",
            "2",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let shown: Vec<&str> = v["diff"].as_str().unwrap().lines().collect();
    assert_eq!(shown.len(), 2, "got: {v}");
    assert_eq!(v["diff_truncated"], serde_json::json!(true), "got: {v}");
    // Stats are computed from the full contents, not the truncated string.
    assert_eq!(v["files"], full_v["files"], "got: {v} vs {full_v}");
    assert_eq!(v["check"], serde_json::json!("ok"), "got: {v}");
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn json_default_report_carries_files_and_untruncated() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["diff_truncated"], serde_json::json!(false), "got: {v}");
    assert_eq!(
        v["files"],
        serde_json::json!([{"path": "src/lib.rs", "added": 2, "removed": 2}]),
        "got: {v}"
    );
    // `files_changed` stays the path list for compat, in the same order.
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        v["files_changed"],
        serde_json::Value::Array(paths.iter().map(|p| serde_json::json!(p)).collect()),
        "got: {v}"
    );
}

#[test]
fn text_check_diff_max_lines_truncates_with_marker() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--check",
            "--diff-max-lines",
            "2",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(stdout.lines().count(), 2, "got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("diff truncated:"), "got: {stderr}");
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn text_check_diff_max_lines_zero_omits_with_stats_marker() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--check",
            "--diff-max-lines",
            "0",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("diff omitted"), "got: {stderr}");
    assert!(stderr.contains("src/lib.rs: +2 -2"), "got: {stderr}");
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn output_file_stays_full_when_report_truncated() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let full_patch = dir.path().join("full.patch");
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--diff-max-lines",
            "1",
            "--output",
            full_patch.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["diff"].as_str().unwrap().lines().count(), 1);
    assert_eq!(v["diff_truncated"], serde_json::json!(true));
    // The saved file holds the complete, appliable diff.
    let saved = fs::read_to_string(&full_patch).unwrap();
    assert!(saved.contains("-    foo(1);"), "got: {saved}");
    assert!(saved.contains("-    foo(2);"), "got: {saved}");
    let check = Command::new("git")
        .args(["apply", "--check", full_patch.to_str().unwrap()])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "saved patch must apply: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    // JSON mode is report-only (no apply): the repo file is untouched, and
    // the saved full patch is what the agent reviews/applies separately.
    assert!(read(dir.path(), "src/lib.rs").contains("foo(2);"));
}

#[test]
fn budget_flags_rejected_with_inspect() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    for args in [
        vec!["--inspect", "src/lib.rs", "--diff-max-lines", "5"],
        vec!["--inspect", "src/lib.rs", "--context", "0"],
    ] {
        let out = run(dir.path(), &args);
        assert_eq!(out.status.code(), Some(2), "args: {args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            stderr.contains("--inspect does not take --diff-max-lines/--context"),
            "got: {stderr}"
        );
        assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
        // JSON mode: same failure as a structured envelope on stdout.
        let mut json_args = args.clone();
        json_args.extend(["--format", "json"]);
        let json_out = run(dir.path(), &json_args);
        assert_eq!(json_out.status.code(), Some(2), "args: {json_args:?}");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout))
            .expect("failure stdout must be a JSON envelope");
        assert_eq!(
            v["error"]["code"],
            serde_json::json!("RSMPL_IO_CONFLICT"),
            "args: {json_args:?}, got: {v}"
        );
        assert!(
            v["error"]["reason"]
                .as_str()
                .unwrap()
                .contains("--inspect does not take --diff-max-lines/--context"),
            "got: {v}"
        );
        assert!(v["error"]["suggestion"].is_string(), "got: {v}");
    }
}

#[test]
fn inspect_rejects_patch_and_check_flags_in_both_modes() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    for (args, needle) in [
        (
            vec!["--inspect", "src/lib.rs", "--patch", "x"],
            "--inspect does not take --patch/--file",
        ),
        (
            vec!["--inspect", "src/lib.rs", "--check"],
            "--inspect does not take --check/--output",
        ),
    ] {
        let out = run(dir.path(), &args);
        assert_eq!(out.status.code(), Some(2), "args: {args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(stderr.contains(needle), "got: {stderr}");
        assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
        let mut json_args = args.clone();
        json_args.extend(["--format", "json"]);
        let json_out = run(dir.path(), &json_args);
        assert_eq!(json_out.status.code(), Some(2), "args: {json_args:?}");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout))
            .expect("failure stdout must be a JSON envelope");
        assert_eq!(
            v["error"]["code"],
            serde_json::json!("RSMPL_IO_CONFLICT"),
            "args: {json_args:?}, got: {v}"
        );
        assert!(
            v["error"]["reason"].as_str().unwrap().contains(needle),
            "got: {v}"
        );
        assert!(v["error"]["suggestion"].is_string(), "got: {v}");
    }
}

// --- T37 lowering context (D.2.2): --context N ---

#[test]
fn text_context_zero_omits_context_but_stays_check_clean() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--check",
            "--context",
            "0",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("@@"), "got: {stdout}");
    assert!(stdout.contains("-    foo(1);"), "got: {stdout}");
    assert!(stdout.contains("+    bar(1);"), "got: {stdout}");
    // Unchanged surroundings are not shown at context 0 ...
    assert!(!stdout.contains("fn a() {"), "got: {stdout}");
    // ... but the lowering still verifies cleanly and check mode is read-only.
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("check:"),
        "got: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
}

#[test]
fn context_zero_applies_for_real() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let out = run(
        dir.path(),
        &["--file", patch_file.to_str().unwrap(), "--context", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("bar(1);"), "got: {text}");
    assert!(text.contains("bar(2);"), "got: {text}");
}

#[test]
fn context_zero_output_file_is_git_apply_clean() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let saved_patch = dir.path().join("ctx0.patch");
    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--context",
            "0",
            "--output",
            saved_patch.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // JSON mode is report-only: the repo is untouched, so the saved
    // zero-context patch must verify against the pristine tree.
    assert!(read(dir.path(), "src/lib.rs").contains("foo(1);"));
    let saved = fs::read_to_string(&saved_patch).unwrap();
    assert!(saved.contains("-    foo(1);"), "got: {saved}");
    assert!(!saved.contains(" fn a() {"), "got: {saved}");
    // Bare `git apply` rejects zero-context hunks by default, so outside
    // callers need `--unidiff-zero` (the CLI passes it internally).
    let check = Command::new("git")
        .args([
            "apply",
            "--check",
            "--unidiff-zero",
            saved_patch.to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "zero-context patch must apply: {}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn json_context_zero_shrinks_diff_stats_stay_exact() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let full = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--check",
        ],
    );
    assert!(full.status.success());
    let full_v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&full.stdout)).expect("stdout is JSON");

    let out = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--format",
            "json",
            "--check",
            "--context",
            "0",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let full_lines = full_v["diff"].as_str().unwrap().lines().count();
    let shown_lines = v["diff"].as_str().unwrap().lines().count();
    assert!(shown_lines > 0, "got: {v}");
    assert!(
        shown_lines < full_lines,
        "context 0 should shrink the diff: {shown_lines} vs {full_lines}"
    );
    // Stats come from the full contents, not the hunk rendering.
    assert_eq!(v["files"], full_v["files"], "got: {v} vs {full_v}");
    assert_eq!(v["diff_truncated"], serde_json::json!(false), "got: {v}");
    assert_eq!(v["check"], serde_json::json!("ok"), "got: {v}");
}

#[test]
fn explicit_context_three_matches_default() {
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch_file = dir.path().join("change.rsmpl");
    fs::write(&patch_file, two_call_patch()).unwrap();
    let base = run(
        dir.path(),
        &["--file", patch_file.to_str().unwrap(), "--check"],
    );
    let explicit = run(
        dir.path(),
        &[
            "--file",
            patch_file.to_str().unwrap(),
            "--check",
            "--context",
            "3",
        ],
    );
    assert!(base.status.success() && explicit.status.success());
    assert_eq!(
        String::from_utf8_lossy(&base.stdout),
        String::from_utf8_lossy(&explicit.stdout)
    );
}

#[test]
fn context_rejected_with_inspect() {
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    let out = run(dir.path(), &["--inspect", "src/lib.rs", "--context", "0"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("--inspect does not take --diff-max-lines/--context"),
        "got: {stderr}"
    );
}

// --- T53: `--context` stays `Option` so absent-vs-explicit is detectable ---

#[test]
fn context_help_documents_default_manually() {
    // The default (3) is rendered by hand in the help text, not by clap's
    // `[default: 3]` (which needs `default_value_t` and would erase the
    // absent-vs-explicit distinction `--inspect` relies on). Pin the marker
    // so dropping it breaks loudly instead of silently hiding the default.
    let out = Command::new(binary()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        help.contains("--context") && help.contains("(default 3"),
        "got: {help}"
    );
}

#[test]
fn explicit_default_context_three_rejected_with_inspect() {
    // Even the default value, when passed explicitly, is rejected with
    // `--inspect` — proof the `Option` (absent-vs-explicit) shape is
    // load-bearing and `default_value_t = 3` would be a behavior change.
    // Both modes pinned per T47 (text block / JSON envelope).
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn foo() {}\n");
    let out = run(dir.path(), &["--inspect", "src/lib.rs", "--context", "3"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("--inspect does not take --diff-max-lines/--context"),
        "got: {stderr}"
    );
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    let json_out = run(
        dir.path(),
        &[
            "--inspect",
            "src/lib.rs",
            "--context",
            "3",
            "--format",
            "json",
        ],
    );
    assert_eq!(json_out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("--inspect does not take --diff-max-lines/--context"),
        "got: {v}"
    );
    assert!(v["error"]["suggestion"].is_string(), "got: {v}");
}
