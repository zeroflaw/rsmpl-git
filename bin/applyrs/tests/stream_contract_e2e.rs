//! T87 e2e: output-stream, empty-lowering, and `--output`-order contract.
//!
//! Pins SPEC D.2.3: which bytes land on which stream in each of the 4
//! modes, what an empty lowering does, and when `--output` is written.
//! Split from `cli_e2e.rs` to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

use std::fs;
use std::process::Command;

mod common;
use common::*;

fn rename_patch() -> &'static str {
    "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n"
}

fn noop_patch() -> &'static str {
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=noop\n# lang=rust\n# optional\n- fn missing_xyz(...) {\n+ fn renamed_xyz(...) {\n      ...\n  }\n"
}

#[test]
fn text_apply_is_stdout_quiet() {
    // D.2.3: plain text apply prints nothing to stdout; the `applied …`
    // line goes to stderr and exit 0 is the signal.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let out = run_stdin(dir.path(), rename_patch(), &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "apply stdout must be empty, got: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("applied 1 file(s): src/lib.rs"),
        "got: {stderr}"
    );
}

#[test]
fn text_check_diff_on_stdout_check_on_stderr() {
    // D.2.3: text `--check` carries the reported diff on stdout and the
    // human `check:` signal on stderr.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let out = run_stdin(dir.path(), rename_patch(), &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("@@"), "got: {stdout}");
    assert!(stdout.contains("-fn foo("), "got: {stdout}");
    assert!(stdout.contains("+fn bar("), "got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("check: lowered patch applies cleanly"),
        "got: {stderr}"
    );
}

#[test]
fn json_check_has_no_stderr_check_line() {
    // D.2.3 (T85): JSON `--check` is stdout-only. The `"checked"`
    // payload is the machine signal; no redundant stderr `check:` line.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let out = run_stdin(dir.path(), rename_patch(), &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": true, "build": "skipped"}),
        "got: {v}"
    );
    assert_eq!(v["version"], serde_json::json!(2), "got: {v}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !stderr.contains("check:"),
        "JSON mode must not print a stderr check: line, got: {stderr}"
    );
}

#[test]
fn failing_check_still_writes_output() {
    // D.2.3 (T77): `--output` is written before `--check` runs, so a
    // failing `--check` still leaves the full appliable diff on disk.
    // An empty `PATH` makes the `git apply --check` child fail to spawn
    // deterministically, after engine planning + lowering + `--output`
    // have already succeeded.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let saved = dir.path().join("lowered.patch");
    let out = {
        use std::io::Write;
        use std::process::Stdio;
        let mut child = Command::new(binary())
            .args(["--check", "--output"])
            .arg(&saved)
            .arg("--repo")
            .arg(dir.path())
            .env("PATH", "/nonexistent-gitrs-bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(rename_patch().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("GIT_HUNK_CONFLICT"),
        "git failure must surface as GIT_HUNK_CONFLICT, got: {stderr}"
    );
    let written = fs::read_to_string(&saved).unwrap();
    assert!(
        written.contains("-fn foo(") && written.contains("+fn bar("),
        "output must hold the full diff, got: {written}"
    );
}

#[test]
fn empty_lowering_writes_no_output_file() {
    // D.2.3 (T86): an empty lowering writes no `--output` file —
    // no changes means no file, not a 0-byte artifact.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let saved = dir.path().join("empty.patch");
    let out = run_stdin(
        dir.path(),
        noop_patch(),
        &[
            "--format",
            "json",
            "--check",
            "--output",
            saved.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !saved.exists(),
        "empty lowering must not create an output file"
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["diff"], serde_json::json!(""), "got: {v}");
    assert_eq!(v["diff_truncated"], serde_json::json!(false), "got: {v}");
}

#[test]
fn empty_lowering_removes_preexisting_output_file() {
    // D.2.3 (T258 mirror invariant): `--output` reflects the current
    // lowering, so an empty lowering removes a stale file left by a
    // previous run instead of leaving a phantom diff behind.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let saved = dir.path().join("stale.patch");
    fs::write(&saved, "stale diff from a previous run\n").unwrap();
    let out = run_stdin(
        dir.path(),
        noop_patch(),
        &["--check", "--output", saved.to_str().unwrap()],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !saved.exists(),
        "empty lowering must remove a pre-existing output file"
    );
}

#[test]
fn empty_text_check_prints_no_changes() {
    // D.2.3 (T76): text `--check` on an empty lowering prints
    // `(no changes)` to stdout and exits clean without touching git.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let out = run_stdin(dir.path(), noop_patch(), &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("(no changes)"), "got: {stdout}");
}

#[test]
fn json_failure_is_stdout_only() {
    // D.2.3 (T89): JSON failures carry the envelope on stdout only —
    // no redundant stderr `error:` line (symmetry with the silenced
    // success `check:` line). `warning:`-only stderr stays allowed.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    let out = run_stdin(dir.path(), "", &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT"),
        "got: {v}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        !stderr.contains("error:"),
        "JSON failure must not print a stderr error: line, got: {stderr}"
    );
}

#[test]
fn text_failure_keeps_stderr_code() {
    // D.2.3 (T89 contrast): text-mode failures keep the human stderr
    // diagnostic with the structured code.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    let out = run_stdin(dir.path(), "", &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
}

#[test]
fn unknown_header_tag_fails_fast_text_and_json() {
    // T107 (SPEC 2.6/8.3): dead header tags fail fast. Text keeps the
    // stderr diagnostic; JSON carries the stdout-only envelope (exit 2).
    // T274 (SPEC 8.3/25.3, feeds T246): verbatim reason + `old_range: line 2`
    // envelope (header line number, null rule/file/suggestion).
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x id=y\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("unknown header tag"), "got: {stderr}");
    assert!(stderr.contains("old_range: line 2"), "got: {stderr}");
    assert!(
        stderr.contains("reason: unknown header tag `id=y` (only `rule=` is supported in v0.1; remove it, see SPEC 8.3)"),
        "got: {stderr}"
    );
    assert!(!stderr.contains("rule:"), "got: {stderr}");
    assert!(!stderr.contains("file:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("unknown header tag `id=y` (only `rule=` is supported in v0.1; remove it, see SPEC 8.3)")
    );
    assert_eq!(v["error"]["range"], serde_json::json!("line 2"));
    assert!(v["error"]["rule"].is_null(), "got: {v}");
    assert!(v["error"]["file"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn unknown_root_and_kind_fail_fast_text_and_json() {
    // T110+T111 (SPEC 2.6/13.4/15.1): unimplemented `# root=` values and
    // never-binding metavar kinds fail fast. Text keeps the stderr
    // diagnostic; JSON carries the stdout-only envelope (exit 2).
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    for (patch, needle) in [
        (
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# root=bogus\n- foo\n+ bar\n",
            "unknown `# root=` value",
        ),
        (
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# lifetime L\n- foo(L)\n+ bar(L)\n",
            "is not supported in v0.1",
        ),
    ] {
        let out = run_stdin(dir.path(), patch, &[]);
        assert_eq!(out.status.code(), Some(2), "patch: {patch}");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
        assert!(stderr.contains(needle), "got: {stderr}");
        let out = run_stdin(dir.path(), patch, &["--format", "json"]);
        assert_eq!(out.status.code(), Some(2), "patch: {patch}");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let v: serde_json::Value =
            serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
        assert_eq!(
            v["error"]["code"],
            serde_json::json!("RSMPL_PATTERN_CONFLICT")
        );
    }
    // T274 (SPEC 13.4/25.3, feeds T246): the `# root=` arm verbatim —
    // `rule:` + `file:`, null range/suggestion, em dash before the pointer.
    let root_patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# root=bogus\n- foo\n+ bar\n";
    let out = run_stdin(dir.path(), root_patch, &[]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("rule: x"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(
        stderr.contains("reason: unknown `# root=` value `bogus` (only `item/expr/stmt/type` are supported in v0.1; remove it or use a supported value — see SPEC 13.4)"),
        "got: {stderr}"
    );
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    let out = run_stdin(dir.path(), root_patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("unknown `# root=` value `bogus` (only `item/expr/stmt/type` are supported in v0.1; remove it or use a supported value — see SPEC 13.4)")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("x"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}

#[test]
fn unwritable_output_fails_io_conflict_without_applying() {
    // T268 (SPEC 25.8/D.2): an `--output` path that cannot be created
    // fails with `RSMPL_IO_CONFLICT` before anything is checked or
    // applied. The parent here is a regular file, so auto-create fails
    // deterministically (even as root — no permission tricks).
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    write(dir.path(), "blocker", "not a dir\n");
    let out_arg = dir.path().join("blocker").join("out.patch");
    let out_arg = out_arg.to_str().unwrap();
    // Apply mode (no `--check`): must abort before touching the worktree.
    let out = run_stdin(dir.path(), rename_patch(), &["--output", out_arg]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_IO_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("cannot write"), "got: {stderr}");
    assert!(stderr.contains(out_arg), "got: {stderr}");
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
        "failed --output must not apply"
    );
    // JSON mode: the same failure as a stdout-only envelope.
    let json = run_stdin(
        dir.path(),
        rename_patch(),
        &["--format", "json", "--output", out_arg],
    );
    assert_eq!(json.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&json.stdout).to_string();
    let v: serde_json::Value =
        serde_json::from_str(&stdout).expect("failure stdout must be a JSON envelope");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_IO_CONFLICT"),
        "got: {v}"
    );
    assert_eq!(v["error"]["file"], serde_json::json!(out_arg), "got: {v}");
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("cannot write"),
        "got: {v}"
    );
    for key in ["rule", "range", "suggestion"] {
        assert!(
            v["error"][key].is_null(),
            "IO envelope {key} must be null, got: {v}"
        );
    }
}
