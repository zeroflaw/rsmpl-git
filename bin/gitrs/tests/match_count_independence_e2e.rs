//! T324: `# optional` suppresses only zero-match, never too-many.
//!
//! SPEC 17.5: the two flags are independent — `# optional` turns a
//! zero-match into success (`matches: 0`, `spans: []`, empty diff),
//! while a too-many match still fails with the verbatim
//! `expected one match, found 2` fence even when `# optional` is set.
//! Combined `# optional` + `# allow=many` means zero-or-more.

mod common;
use common::*;

const ZERO_SOURCE: &str = "fn f() {}\n";
const ZERO_OPTIONAL_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# optional\n# expr E\n- foo(E)\n+ bar(E)\n";

const TWO_SOURCE: &str = "fn f() { foo(1); foo(2); }\n";
const TWO_OPTIONAL_PATCH: &str =
    "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# optional\n# expr E\n- foo(E)\n+ bar(E)\n";

const TWO_OPTIONAL_MANY_PATCH: &str = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# optional\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";

#[test]
fn optional_zero_succeeds_with_empty_report() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", ZERO_SOURCE);
    let out = run_stdin(
        dir.path(),
        ZERO_OPTIONAL_PATCH,
        &["--format", "json", "--check"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(0), "got: {v}");
    assert_eq!(v["rules"][0]["spans"], serde_json::json!([]), "got: {v}");
    assert_eq!(v["diff"], serde_json::json!(""), "got: {v}");
    assert_eq!(read(dir.path(), "src/lib.rs"), ZERO_SOURCE);
}

#[test]
fn optional_does_not_suppress_too_many_text() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", TWO_SOURCE);
    let out = run_stdin(dir.path(), TWO_OPTIONAL_PATCH, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_MATCH_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("expected one match, found 2"),
        "got: {stderr}"
    );
    assert!(stderr.contains("add `# allow=many`"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), TWO_SOURCE);
}

#[test]
fn optional_does_not_suppress_too_many_json() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", TWO_SOURCE);
    let out = run_stdin(dir.path(), TWO_OPTIONAL_PATCH, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_MATCH_CONFLICT")
    );
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("1..2"));
    assert!(
        v["error"]["reason"]
            .as_str()
            .unwrap()
            .contains("expected one match, found 2"),
        "got: {v}"
    );
    assert!(
        v["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("allow=many"),
        "got: {v}"
    );
    assert!(
        v["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("# allow=many"),
        "got: {v}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), TWO_SOURCE);
}

#[test]
fn optional_plus_allow_many_covers_zero_or_more() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", TWO_SOURCE);
    let out = run_stdin(
        dir.path(),
        TWO_OPTIONAL_MANY_PATCH,
        &["--format", "json", "--check"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(2), "got: {v}");
    assert_eq!(
        v["rules"][0]["spans"].as_array().unwrap().len(),
        2,
        "got: {v}"
    );
}
