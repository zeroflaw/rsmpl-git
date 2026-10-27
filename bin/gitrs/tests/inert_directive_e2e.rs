//! T169: inert directives fail fast with `RSMPL_PATTERN_CONFLICT`
//! instead of applying cleanly while constraining nothing.
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

mod common;
use common::*;

#[test]
fn lone_fallback_fails_fast_text_and_json() {
    // T169a: `# fallback=syntax` with zero `# symbol` lines used to apply
    // cleanly (the engine symbol loop iterates zero times). Now it fails
    // fast; worktree untouched; JSON envelope carries the needle.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {}\nfn main() { foo(); }\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let patch = "diff --git src/lib.rs\n@@ -2,1 @@ rule=t\n# allow=many\n# fallback=syntax\n- foo()\n+ bar()\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("without `# symbol`"), "got: {stderr}");
    assert!(
        stderr.contains(
            "without `# symbol` does nothing (call matching is already syntactic in v0.1)"
        ),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("or add `# symbol Name=path` (SPEC 19.3)"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(!out.status.success(), "json must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
    assert!(stdout.contains("without `# symbol`"), "got: {stdout}");
    assert!(stdout.contains("(SPEC 19.3)"), "got: {stdout}");
    assert!(stdout.contains("\"suggestion\":null"), "got: {stdout}");
}

#[test]
fn attrs_on_call_shape_fails_fast() {
    // T169b: `# attrs=strip` on a call rewrite used to apply cleanly
    // while neither matcher_expr nor matcher_macro reads the policy.
    // Now it fails fast with the SPEC 14.6 pointer; worktree untouched.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {}\nfn main() { foo(); }\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let patch =
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=t\n# allow=many\n# attrs=strip\n- foo()\n+ bar()\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("ignored on call/macro shapes"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("ignored on call/macro shapes in v0.1 (attribute envelopes are only checked on fn/item/general/use shapes)"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("remove the line (SPEC 14.6)"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(!out.status.success(), "json must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
    assert!(stdout.contains("(SPEC 14.6)"), "got: {stdout}");
    assert!(stdout.contains("\"suggestion\":null"), "got: {stdout}");
}
