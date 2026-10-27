//! T151/T152/T153/T154/T155/T156: JSON apply-path report pins (T151 trio) +
//! overlap wording (T152) + attrs/doc-mirror apply pins (T153-T156).
//!
//! Split from `cli_e2e.rs` to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

use std::fs;
mod common;
use common::*;

#[test]
fn json_apply_mutates_worktree() {
    // T151: `--format json` without `--check` applies like text mode —
    // `--format` chooses the report language, never whether the patch
    // lands. Before the fix this exited 0 with a full report while
    // leaving the worktree untouched.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E)\n";
    let before = read(dir.path(), "src/lib.rs");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["check"], serde_json::json!(false), "got: {v}");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1));
    let after = read(dir.path(), "src/lib.rs");
    assert_ne!(before, after, "JSON apply must touch the worktree");
    assert!(after.contains("bar(1)"), "got: {after}");
    assert!(!after.contains("foo(1)"), "got: {after}");
}

#[test]
fn json_check_still_applies_nothing() {
    // T151 guard: the mirror fix must not over-apply — JSON `--check`
    // verifies only.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn a() {\n    foo(1);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["check"], serde_json::json!(true), "got: {v}");
    let text = read(dir.path(), "src/lib.rs");
    assert!(
        text.contains("foo(1)"),
        "check must not mutate, got: {text}"
    );
}

#[test]
fn json_empty_lowering_without_check_reports_clean() {
    // T151 parity: an empty lowering (optional zero-match) never touches
    // git in any mode — JSON non-check reports clean with empty
    // `files`/`diff` and `"check": false`.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let noop = "diff --git src/lib.rs\n@@ -1,1 @@ rule=noop\n# lang=rust\n# optional\n- fn missing_xyz(...) {\n+ fn renamed_xyz(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), noop, &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["check"], serde_json::json!(false), "got: {v}");
    assert_eq!(v["files"].as_array().unwrap().len(), 0, "got: {v}");
    assert_eq!(v["diff"], serde_json::json!(""), "got: {v}");
    let text = read(dir.path(), "src/lib.rs");
    assert_eq!(text, "fn foo() {}\n", "got: {text}");
}

#[test]
fn nested_semantic_overlap_suggests_separate_runs() {
    // T152: nested semantic-semantic edits (method call inside a `let`
    // statement) fail safe with a kind-neutral diagnostic — no
    // ordinary-hunk guess, separate-runs repair pointer.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f(v: S) {\n    let a = v.old_method(1);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=m2c\n# expr Receiver\n# expr Arg\n- Receiver.old_method(Arg)\n+ new_function(Receiver, Arg)\n\ndiff --git src/lib.rs\n@@ -2,1 @@ rule=letrw\n# stmt S\n# expr E\n- let a = E;\n+ let c = E;\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "overlap must fail");
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("GIT_HUNK_CONFLICT"), "got: {err}");
    assert!(
        err.contains("overlapping edits cannot be merged"),
        "got: {err}"
    );
    assert!(!err.contains("ordinary hunk"), "got: {err}");
    assert!(err.contains("separate runs"), "got: {err}");
}

#[test]
fn mirrored_attrs_preserve_target_indent() {
    // T153: mirrored `#[...]` lines compare whitespace-insensitively and the
    // Stmt replacement reuses the original envelope slice, so the target's
    // `let` indent survives (no one-space diff-prefix artifact leak).
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f() {\n    #[allow(x)]\n    let x = 1;\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# stmt S\n# expr E\n- #[allow(x)]\n- let x = E;\n+ #[allow(x)]\n+ let y = E;\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert_eq!(
        text, "fn f() {\n    #[allow(x)]\n    let y = 1;\n}\n",
        "got: {text:?}"
    );
}

#[test]
fn strip_with_mirrored_minus_drops_attrs() {
    // T154: `# attrs=strip` skips the mirror gate — the envelope is
    // dropped at rewrite instead of failing with "or use `# attrs=strip`".
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f() {\n    #[allow(x)]\n    let x = 1;\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# attrs=strip\n# stmt S\n# expr E\n- #[allow(x)]\n- let x = E;\n+ let y = E;\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert_eq!(text, "fn f() {\n    let y = 1;\n}\n", "got: {text:?}");
}

#[test]
fn explicit_doc_comment_mirror_applies_end_to_end() {
    // T155: `///` doc lines mirror like `#[...]` attrs end to end —
    // `--check` previews the rename, `--output` saves the full diff.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "/// Doc comment.\nfn foo() {}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=r\n# attrs=explicit\n- /// Doc comment.\n- fn foo(...) {\n+ /// Doc comment.\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let saved_path = dir.path().join("full.patch");
    let out = run_stdin(
        dir.path(),
        patch,
        &["--output", saved_path.to_str().unwrap()],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let saved = fs::read_to_string(&saved_path).unwrap();
    assert!(saved.contains("+fn bar() {}"), "got: {saved}");
    let text = read(dir.path(), "src/lib.rs");
    assert_eq!(text, "/// Doc comment.\nfn bar() {}\n", "got: {text:?}");
}

#[test]
fn item_slash_doc_and_block_doc_mirrors_apply_end_to_end() {
    // T156: item `///` parity + single-line `/** */` normalization end to
    // end; multi-line block docs fail safe with the needle (JSON pin).
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "/// Doc.\nstruct Old {\n    x: i32,\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# attrs=explicit\n- /// Doc.\n- struct Old\n+ /// Doc.\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = read(dir.path(), "src/lib.rs");
    assert!(text.contains("struct New {"), "got: {text:?}");
    assert!(text.contains("/// Doc."), "got: {text:?}");

    write(
        dir.path(),
        "src/block.rs",
        "/** Block doc. */\nfn foo() {}\n",
    );
    let block = "diff --git src/block.rs\n@@ -1,2 @@ rule=r\n# attrs=explicit\n- /** Block doc. */\n- fn foo(...) {\n+ /** Block doc. */\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), block, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Multi-line block docs fail safe with the targeted needle.
    write(dir.path(), "src/multi.rs", "fn foo() {}\n");
    let multi = "diff --git src/multi.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- /** Multi\n- line. */\n- fn foo(...) {\n+ /** Multi\n+ line. */\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), multi, &["--format", "json"]);
    assert!(!out.status.success(), "must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("block comments"), "got: {stdout}");
}
