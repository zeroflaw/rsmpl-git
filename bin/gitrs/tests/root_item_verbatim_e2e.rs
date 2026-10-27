//! T319: engine-verbatim `# root=item` mismatch fence (feeds T246).
//!
//! `# root=item` forces item capture: a minus side that is not a single
//! declared metavariable fails fast with `RSMPL_PATTERN_CONFLICT`
//! (rule + file, null range/suggestion — same envelope as the C3/C4/C5
//! stmt/expr/type siblings, SPEC 13.4/18.6 C16). Before T319 the directive
//! was silently ignored and heuristic routing applied the hunk (probed: a
//! call shape applied as a call rewrite). Pins: text + JSON +
//! `--check`-mode untouched worktree, a valid-capture control, and a SPEC
//! rot-gate.

mod common;
use common::*;

const MISMATCH: &str =
    "diff --git a.rs\n@@ -1,1 @@ rule=r\n# root=item\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
const REASON: &str = "`# root=item` requires a single-metavar item-capture pattern (write `- I` for a declared `# item I` metavariable)";

const CAPTURE: &str = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# root=item\n# item I\n# bind=H\n- I\n";

#[test]
fn root_item_mismatch_verbatim_text_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), MISMATCH, &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: a.rs"), "got: {stderr}");
    assert!(stderr.contains(REASON), "got: {stderr}");
    assert!(!stderr.contains("range:"), "got: {stderr}");
    assert!(!stderr.contains("suggestion:"), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn main() { foo(1); }\n",
        "worktree must be untouched"
    );
}

#[test]
fn root_item_mismatch_verbatim_json_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), MISMATCH, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("a.rs"));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert_eq!(v["error"]["reason"], serde_json::json!(REASON));
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn main() { foo(1); }\n",
        "worktree must be untouched"
    );
}

#[test]
fn root_item_mismatch_check_mode_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), MISMATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(REASON), "got: {stderr}");
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn main() { foo(1); }\n",
        "worktree must be untouched"
    );
}

#[test]
fn root_item_still_captures_declared_metavar() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn helper() {}\nfn main() {}\n");
    let out = run_stdin(dir.path(), CAPTURE, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("check:"), "got: {stderr}");
}

#[test]
fn spec_carries_root_item_fence() {
    let root = workspace_root();
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    for needle in [
        "`# root=item` requires a single-metavar item-capture pattern",
        "four `# root=` mismatches",
        "The `# root=item` path is strict the same way",
    ] {
        assert!(spec.contains(needle), "SPEC.md must carry `{needle}`");
    }
}
