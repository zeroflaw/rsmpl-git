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
fn minimal_file_section_header_applies() {
    // T60: `diff --git` alone suffices (no `---`/`+++`/`index`), `rule=`
    // may be omitted, new side = copy of old (ignored for RSmPL hunks).
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,3 +1,3 @@\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
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
fn lang_directive_optional_applies() {
    // T69: hunks that already carry other directives omit `# lang=rust`.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=rename_calls\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run(dir.path(), &["--patch", patch, "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("-    foo(1);"), "got: {stdout}");
    assert!(stdout.contains("+    bar(1);"), "got: {stdout}");
}

#[test]
fn single_path_and_range_elision_applies() {
    // T68+T64: `diff --git <path>` + `@@ -old,len @@` is the minimal
    // recipe (single path ≡ a/X b/X; elided new side defaults to old).
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run(dir.path(), &["--patch", patch, "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("+fn bar(x: i32)"), "got: {stdout}");
}

#[test]
fn diet_extreme_call_rewrite_applies() {
    // T73: the four header-diet relaxations compose freely — single-path
    // header (T68) + elided new side (T64) + lang-less directive block
    // (T69) + rule-less header (T60). This is the cheapest possible
    // call-rewrite hunk an agent can emit.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run(dir.path(), &["--patch", patch, "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("-    foo(1);"), "got: {stdout}");
    assert!(stdout.contains("+    bar(1);"), "got: {stdout}");
}

#[test]
fn single_range_ordinary_hunk_rejected() {
    // T64: new-side elision is RSmPL-only; ordinary hunks need both ranges.
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "foo\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,1 @@\n-foo\n+bar\n";
    let out = run(dir.path(), &["--patch", patch, "--check"]);
    assert!(!out.status.success(), "expected failure");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("both") || stderr.contains("RSmPL"),
        "got: {stderr}"
    );
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
fn json_report_arrays_follow_document_order() {
    // T36/T45: `rules` and `files` follow patch document order
    // (section order, hunk order within a section) — never sorted.
    // Sections are non-alphabetical (z before a) and the cut hunk
    // sorts second in the document although the engine evaluates
    // cuts first: old code reported [r_cut, r_rename, r_paste].
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/z.rs",
        "fn keep() {}\nfn helper() {}\n",
    );
    write(&dir.path().join(""), "src/a.rs", "fn a() {}\n");
    let patch = "diff --git a/src/z.rs b/src/z.rs\n@@ -1,1 +1,1 @@ rule=r_rename\n# lang=rust\n- fn keep(...) {\n+ fn keep2(...) {\n      ...\n  }\n@@ -2,1 +2,1 @@ rule=r_cut\n# lang=rust\n# item I\n# bind=H\n- I\ndiff --git a/src/a.rs b/src/a.rs\n@@ -1,0 +1,0 @@ rule=r_paste\n# lang=rust\n# use=H\n+ H\n";
    let out = Command::new(binary())
        .arg("--patch")
        .arg(patch)
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
    let rules: Vec<&str> = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["rule"].as_str().unwrap())
        .collect();
    assert_eq!(rules, vec!["r_rename", "r_cut", "r_paste"]);
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(files, vec!["src/z.rs", "src/a.rs"]);
}

#[test]
fn empty_patch_fails_text() {
    // T59: zero file sections is an authoring typo, not a no-op.
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let out = run(dir.path(), &["--patch", ""]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
}

#[test]
fn empty_patch_fails_json_envelope() {
    // T59: same failure in machine mode carries the JSON envelope.
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let out = Command::new(binary())
        .arg("--patch")
        .arg("")
        .arg("--repo")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
}

#[test]
fn unknown_directive_fails_fast_text_and_json() {
    // T113: silent no-op directives fail fast (SPEC 2.6) — text keeps the
    // stderr diagnostic, JSON carries the envelope on stdout (exit 2 both).
    let dir = init_repo();
    write(&dir.path().join(""), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# foo=bar\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let out = run(dir.path(), &["--patch", patch]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("unknown directive"), "got: {stderr}");
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
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
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
            .contains("read `src/lib.rs`"),
        "suggestion must name the file read, got: {err}"
    );
    // File untouched by the failed run.
    assert!(read(dir.path(), "src/lib.rs").contains("fn a()"));
}

#[test]
fn json_envelope_zero_match_unique_elsewhere_hints_scope_file() {
    // T62: tight wrong range (lines 1-1) but the only `foo` is on line 3 —
    // the JSON envelope must offer the zero-discovery `# scope=file` retry.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {}\nfn b() {}\nfn foo() {}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
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
    assert!(
        err["suggestion"].as_str().unwrap().contains("# scope=file"),
        "suggestion must offer the scope=file retry, got: {err}"
    );
    assert!(
        err["suggestion"].as_str().unwrap().contains("@@ -1,1 @@"),
        "suggestion must name the dummy range, got: {err}"
    );
}

#[test]
fn json_envelope_on_range_failure_names_file_read() {
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
            .contains("read `src/lib.rs`"),
        "suggestion must name the file read, got: {err}"
    );
}

#[test]
fn json_match_failure_suggestion_carries_scope_file_hint() {
    // T62: the whole-file retry hint composes with the file-read pointer.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {}\nfn b() {}\nfn foo() {}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
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
    let suggestion = err["suggestion"].as_str().unwrap();
    assert!(
        suggestion.contains("read `src/lib.rs`"),
        "suggestion must name the file read, got: {err}"
    );
    assert!(
        suggestion.contains("# scope=file"),
        "scope=file retry must compose with the repo prefix, got: {err}"
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
    // T54: `files_changed` removed (duplicated `files[].path`); the single
    // path source is `files[].path`.
    assert!(v.get("files_changed").is_none(), "got: {v}");
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
    // T54: `files_changed` removed (duplicated `files[].path`); the single
    // path source is `files[].path`.
    assert!(v.get("files_changed").is_none(), "got: {v}");
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs"], "got: {v}");
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
fn output_creates_missing_parent_dirs() {
    // T70: `--output a/b/full.patch` needs no prior `mkdir -p`; missing
    // parents are created, the saved diff is full, and the repo is applied.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let nested = dir.path().join("a").join("b").join("full.patch");
    assert!(!dir.path().join("a").exists());
    let out = run(
        dir.path(),
        &["--patch", patch, "--output", nested.to_str().unwrap()],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let saved = fs::read_to_string(&nested).unwrap();
    assert!(saved.contains("diff --git"), "got: {saved}");
    assert!(saved.contains("+fn bar(x: i32)"), "got: {saved}");
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("fn bar(x: i32)"), "got: {text}");
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
fn context_help_documents_default_value() {
    // The default (3) is rendered by clap's `[default: 3]` now that the
    // absent-vs-explicit distinction is gone with the inspect surface.
    let out = Command::new(binary()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        help.contains("--context") && help.contains("[default: 3]"),
        "got: {help}"
    );
}

#[test]
fn text_apply_summary_matches_json_files() {
    // T65 (+T78): text `applied …` and JSON `files[].path` share one
    // diff-based source (old-vs-new contents differ). A rename touches
    // one file in both modes; an `# optional` zero-match hunk touches
    // none in both modes (`(no changes)` vs `files: []`).
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    // JSON --check on a clone: pin `files[].path` without applying.
    let dir_json = init_repo();
    write(
        &dir_json.path().join(""),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let json_out = run(
        dir_json.path(),
        &["--patch", patch, "--format", "json", "--check"],
    );
    assert!(
        json_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&json_out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout)).unwrap();
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["src/lib.rs"], "got: {v}");
    // Text apply on the real repo: same single path on stderr.
    let out = run(dir.path(), &["--patch", patch]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("applied 1 file(s): src/lib.rs"),
        "got: {stderr}"
    );
    // Zero-change case: both modes agree on empty.
    let noop = "diff --git src/lib.rs\n@@ -1,1 @@ rule=noop\n# lang=rust\n# optional\n- fn missing_xyz(...) {\n+ fn renamed_xyz(...) {\n      ...\n  }\n";
    let dir_noop = init_repo();
    write(&dir_noop.path().join(""), "src/lib.rs", "fn foo() {}\n");
    let json_noop = run(
        dir_noop.path(),
        &["--patch", noop, "--format", "json", "--check"],
    );
    assert!(
        json_noop.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&json_noop.stderr)
    );
    let v2: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&json_noop.stdout)).unwrap();
    assert_eq!(v2["files"].as_array().unwrap().len(), 0, "got: {v2}");
    let dir_noop2 = init_repo();
    write(&dir_noop2.path().join(""), "src/lib.rs", "fn foo() {}\n");
    let text_noop = run(dir_noop2.path(), &["--patch", noop, "--check"]);
    assert!(
        text_noop.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&text_noop.stderr)
    );
    let stdout_noop = String::from_utf8_lossy(&text_noop.stdout).to_string();
    assert!(stdout_noop.contains("(no changes)"), "got: {stdout_noop}");
}

#[test]
fn scope_file_dummy_range_applies() {
    // T61: `@@ -1,1 @@` + `# scope=file` renames a fn on lines 5-7
    // with no discovery roundtrip; the same dummy without `scope=file`
    // fails with `RSMPL_MATCH_CONFLICT`.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run(dir.path(), &["--patch", patch, "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bare = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run(dir.path(), &["--patch", bare, "--check"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    // Apply for real with scope=file.
    let out = run(dir.path(), &["--patch", patch]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("fn bar()"), "got: {text}");
}
