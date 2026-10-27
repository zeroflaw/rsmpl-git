//! T270: declaration forms + `# macro=expanded` SPEC wording pins.
//!
//! - `# expr Receiver, Arg` is equivalent to separate lines (SPEC 9.4);
//!   the lowered diff is byte-identical to the separate-lines control,
//!   and a trailing comma is harmless.
//! - Lowercase metavariable names are accepted (SPEC 15.2): name
//!   validation is case-insensitive, no rejection path exists.
//! - `# macro=expanded` fails fast with a verbatim
//!   `RSMPL_PATTERN_CONFLICT` reason (SPEC 21.5).

mod common;
use common::*;

const CALLS: &str = "fn foo(x: i32) -> i32 { x }\nfn main() {\n    foo(1);\n    foo(2);\n}\n";

#[test]
fn comma_separated_declarations_match_separate_lines() {
    let dir = init_repo();
    write(dir.path(), "a.rs", CALLS);
    let comma =
        "diff --git a.rs\n@@ -3,2 @@ rule=comma\n# allow=many\n# expr Receiver, Arg\n- Receiver(Arg)\n+ bar(Receiver, Arg)\n";
    let separate =
        "diff --git a.rs\n@@ -3,2 @@ rule=separate\n# allow=many\n# expr Receiver\n# expr Arg\n- Receiver(Arg)\n+ bar(Receiver, Arg)\n";
    let out_comma = run_stdin(dir.path(), comma, &["--check"]);
    assert!(
        out_comma.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_comma.stderr)
    );
    let out_separate = run_stdin(dir.path(), separate, &["--check"]);
    assert!(
        out_separate.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out_separate.stderr)
    );
    assert_eq!(
        out_comma.stdout, out_separate.stdout,
        "comma form must lower identically"
    );
    let stdout = String::from_utf8_lossy(&out_comma.stdout).to_string();
    assert!(stdout.contains("-    foo(1);"), "got: {stdout}");
    assert!(stdout.contains("+    bar(foo, 1);"), "got: {stdout}");
    assert!(stdout.contains("+    bar(foo, 2);"), "got: {stdout}");
    let out = run_stdin(
        dir.path(),
        comma,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(2), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[3, 3], [4, 4]]),
        "got: {v}"
    );
}

#[test]
fn lowercase_metavariable_names_accepted() {
    let dir = init_repo();
    write(dir.path(), "a.rs", CALLS);
    let patch =
        "diff --git a.rs\n@@ -3,2 @@ rule=lower\n# allow=many\n# expr e\n- foo(e)\n+ bar(e)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("+    bar(1);"), "got: {stdout}");
    assert!(stdout.contains("+    bar(2);"), "got: {stdout}");
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "a.rs");
    assert!(got.contains("bar(1);"), "got: {got}");
    assert!(!got.contains("foo(1);"), "got: {got}");
}

#[test]
fn macro_expanded_rejected_verbatim() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() { m!(x); }\n");
    let before = read(dir.path(), "a.rs");
    let patch =
        "diff --git a.rs\n@@ -1,1 @@ rule=exp\n# lang=rust\n# macro=expanded\n- m!(...)\n+ n!(...)\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "expanded mode must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("`# macro=expanded` requires build integration (macro expansion span mapping), unavailable in v0.1"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "a.rs"), before, "untouched");
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(!out.status.success(), "json must fail");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT"),
        "got: {v}"
    );
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("unavailable in v0.1"),
        "got: {v}"
    );
    assert!(v["error"]["range"].is_null(), "got: {v}");
    assert!(v["error"]["suggestion"].is_null(), "got: {v}");
}
