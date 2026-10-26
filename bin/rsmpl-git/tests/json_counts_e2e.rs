//! T56 e2e: JSON truncation counts are exact.
//!
//! Split from `cli_e2e.rs` to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

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

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

fn json(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(stdout)).expect("stdout is JSON")
}

#[test]
fn json_truncation_counts_are_exact() {
    // T56: JSON carries exact counts so the next budget is computable.
    // Inspect: `total` pre-cap entries + `omitted` hidden entries.
    // Report: `diff_lines` full lowering + `diff_shown` reported lines.
    let dir = init_repo();
    write(
        &dir.path().join(""),
        "src/lib.rs",
        "fn foo() {}\nfn a() {\n    foo(1);\n    obj.method(x);\n    assert_eq!(a, b);\n}\n",
    );

    // Single inspect, capped: 2 items + 3 calls = 5 total, cap 2 omits 3.
    let out = run(
        dir.path(),
        &[
            "--inspect",
            "src/lib.rs",
            "--calls",
            "--format",
            "json",
            "--max-items",
            "2",
        ],
    );
    assert!(out.status.success());
    let v = json(&out.stdout);
    assert_eq!(v["truncated"], serde_json::json!(true), "got: {v}");
    assert_eq!(v["total"], serde_json::json!(5), "got: {v}");
    assert_eq!(v["omitted"], serde_json::json!(3), "got: {v}");

    // Single inspect, uncapped: exact zeros, no guessing needed.
    let out = run(
        dir.path(),
        &["--inspect", "src/lib.rs", "--calls", "--format", "json"],
    );
    assert!(out.status.success());
    let v = json(&out.stdout);
    assert_eq!(v["truncated"], serde_json::json!(false), "got: {v}");
    assert_eq!(v["total"], serde_json::json!(5), "got: {v}");
    assert_eq!(v["omitted"], serde_json::json!(0), "got: {v}");

    // Dir inspect, capped: emitted + omitted == total.
    let out = run(
        dir.path(),
        &[
            "--inspect",
            "src",
            "--calls",
            "--format",
            "json",
            "--max-items",
            "2",
        ],
    );
    assert!(out.status.success());
    let v = json(&out.stdout);
    assert_eq!(v["truncated"], serde_json::json!(true), "got: {v}");
    let files = v["files"].as_array().unwrap();
    let emitted: i64 = files
        .iter()
        .map(|f| {
            f["items"].as_array().unwrap().len() as i64
                + f["calls"].as_array().unwrap().len() as i64
        })
        .sum();
    let total = v["total"].as_i64().unwrap();
    let omitted = v["omitted"].as_i64().unwrap();
    assert_eq!(total, 5, "got: {v}");
    assert_eq!(emitted + omitted, total, "got: {v}");

    // Report, capped: shown < full, truncated true.
    let patch =
        "diff --git src/lib.rs\n@@ -1,6 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run(
        dir.path(),
        &[
            "--patch",
            patch,
            "--format",
            "json",
            "--diff-max-lines",
            "1",
            "--check",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json(&out.stdout);
    assert_eq!(v["diff_truncated"], serde_json::json!(true), "got: {v}");
    assert_eq!(v["diff_shown"], serde_json::json!(1), "got: {v}");
    assert!(v["diff_lines"].as_i64().unwrap() > 1, "got: {v}");

    // Report, omitted: shown 0, full count still exact.
    let out = run(
        dir.path(),
        &[
            "--patch",
            patch,
            "--format",
            "json",
            "--diff-max-lines",
            "0",
            "--check",
        ],
    );
    assert!(out.status.success());
    let v = json(&out.stdout);
    assert_eq!(v["diff_truncated"], serde_json::json!(true), "got: {v}");
    assert_eq!(v["diff_shown"], serde_json::json!(0), "got: {v}");
    assert_eq!(v["diff"], serde_json::json!(""), "got: {v}");
    assert!(v["diff_lines"].as_i64().unwrap() > 0, "got: {v}");

    // Report, uncapped: shown == full, truncated false.
    let out = run(
        dir.path(),
        &["--patch", patch, "--format", "json", "--check"],
    );
    assert!(out.status.success());
    let v = json(&out.stdout);
    assert_eq!(v["diff_truncated"], serde_json::json!(false), "got: {v}");
    assert_eq!(v["diff_shown"], v["diff_lines"], "got: {v}");
}
