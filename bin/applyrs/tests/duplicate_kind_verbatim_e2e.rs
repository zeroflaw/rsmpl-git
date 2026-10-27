//! T320: engine-verbatim duplicate-declaration kind-mismatch fence (feeds T246).
//!
//! Re-declaring one metavariable name under two different kinds fails fast
//! with `RSMPL_PATTERN_CONFLICT` (rule + file, null range/suggestion,
//! SPEC 9.4). Same-name/same-kind repeats stay allowed (idempotent); the
//! check runs in declaration order across directive families, so the
//! `Directive::Symbol` arm fails the same way in both orders (pre-T320
//! `# expr E` then `# symbol E=...` silently overwrote — order-dependent
//! 2.6 violation). Pins: text + JSON + both-orders untouched worktree, a
//! same-kind-repeat apply control, and a SPEC rot-gate.

mod common;
use common::*;

const DUP: &str = "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n# type E\n- foo(E)\n+ bar(E)\n";
const REASON: &str = "metavariable `E` declared twice with different kinds (expr vs type)";

const SYM_THEN_EXPR: &str =
    "diff --git a.rs\n@@ -1,1 @@ rule=r\n# symbol E=crate::x\n# expr E\n- foo(E)\n+ bar(E)\n";
const SYM_THEN_EXPR_REASON: &str =
    "metavariable `E` declared twice with different kinds (symbol vs expr)";

const EXPR_THEN_SYM: &str =
    "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n# symbol E=crate::x\n- foo(E)\n+ bar(E)\n";
const EXPR_THEN_SYM_REASON: &str =
    "metavariable `E` declared twice with different kinds (expr vs symbol)";

const SAME_KIND: &str =
    "diff --git a.rs\n@@ -1,1 @@ rule=r\n# expr E\n# expr E\n# allow=many\n- foo(E)\n+ bar(E)\n";

#[test]
fn duplicate_kind_verbatim_text_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), DUP, &[]);
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
fn duplicate_kind_verbatim_json_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), DUP, &["--format", "json"]);
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
fn duplicate_kind_symbol_then_expr_order_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), SYM_THEN_EXPR, &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(SYM_THEN_EXPR_REASON), "got: {stderr}");
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn main() { foo(1); }\n",
        "worktree must be untouched"
    );
}

#[test]
fn duplicate_kind_expr_then_symbol_order_untouched() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), EXPR_THEN_SYM, &[]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains(EXPR_THEN_SYM_REASON), "got: {stderr}");
    assert_eq!(
        read(dir.path(), "a.rs"),
        "fn main() { foo(1); }\n",
        "worktree must be untouched"
    );
}

#[test]
fn duplicate_kind_same_kind_repeat_still_applies() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() { foo(1); }\n");
    let out = run_stdin(dir.path(), SAME_KIND, &["--check"]);
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
fn spec_carries_duplicate_kind_fence() {
    let root = workspace_root();
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    for needle in [
        "declared twice with different kinds (expr vs type)",
        "Re-declaring one name under one kind is allowed and idempotent",
        "duplicate_kind_verbatim_e2e.rs",
    ] {
        assert!(spec.contains(needle), "SPEC.md must carry `{needle}`");
    }
}
