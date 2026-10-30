//! WI-7c: orphan evidence for definition-only renames.

mod common;
use common::*;

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

const RENAME: &str = "diff --git src/a.rs\n@@ -1,1 @@ rule=rename\n- struct Old\n+ struct New\n";

#[test]
fn plain_rename_reports_cross_file_orphans_and_leaves_them() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        RENAME,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Definition rewrites; uses stay stale (report-only).
    assert_eq!(read(dir.path(), "src/a.rs"), "pub struct New;\n");
    assert!(read(dir.path(), "src/b.rs").contains("Old"), "uses stay");
    let v = json_of(&out);
    let orphans = v["rules"][0]["orphans"]
        .as_array()
        .expect("orphans present");
    assert_eq!(orphans.len(), 3, "got: {v}");
    assert_eq!(orphans[0]["file"], "src/b.rs");
    assert_eq!(orphans[0]["line"], 1);
    assert!(orphans[0]["text"].as_str().unwrap().contains("Old"));
    // Workspace provenance covers the orphan scan.
    assert_eq!(v["follow_files"], 2, "got: {v}");
    assert_eq!(v["follow_source"], "git", "got: {v}");
    assert!(v["rules"][0].get("orphans_truncated").is_none());
}

#[test]
fn opaque_macro_mention_is_an_orphan() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "fn f() {\n    m!(Old);\n    let x: Old = make();\n    x\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        RENAME,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(out.status.success());
    let v = json_of(&out);
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    // Clean use (line 3) + opaque macro line (line 2).
    assert_eq!(orphans.len(), 2, "got: {v}");
    assert!(orphans.iter().any(|o| o["line"] == 2), "got: {v}");
}

#[test]
fn orphans_truncate_at_eight_per_rule() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    let mut b = String::new();
    for i in 0..10 {
        b.push_str(&format!("fn f{i}(x: Old) -> Old {{\n    x\n}}\n"));
    }
    write(dir.path(), "src/b.rs", &b);
    let out = run_stdin(
        dir.path(),
        RENAME,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(out.status.success());
    let v = json_of(&out);
    assert_eq!(v["rules"][0]["orphans"].as_array().unwrap().len(), 8);
    assert!(v["rules"][0].get("orphans_truncated").is_some(), "got: {v}");
}

#[test]
fn long_orphan_text_carries_ellipsis_marker() {
    // WI-1a: an orphan `text` cut at 80 chars carries a trailing `…` and
    // the rule counts the cut in `orphans_truncated_chars`.
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    let pad = "x".repeat(100);
    write(
        dir.path(),
        "src/b.rs",
        &format!("fn f(x: Old) -> Old {{ // {pad}\n    x\n}}\n"),
    );
    let out = run_stdin(
        dir.path(),
        RENAME,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(out.status.success());
    let v = json_of(&out);
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    assert_eq!(orphans.len(), 1, "got: {v}");
    let text = orphans[0]["text"].as_str().unwrap();
    assert_eq!(text.chars().count(), 81, "got: {v}");
    assert!(text.ends_with('…'), "got: {v}");
    assert_eq!(
        v["rules"][0]["orphans_truncated_chars"],
        serde_json::json!(1),
        "got: {v}"
    );
    assert!(v["rules"][0].get("orphans_truncated").is_none(), "got: {v}");
    // Text mode marks the same cut with `…`.
    let out = run_stdin(dir.path(), RENAME, &["--check"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains('…'), "got: {stderr}");
}

#[test]
fn text_mode_warns_about_orphans() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(dir.path(), "src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n");
    let out = run_stdin(dir.path(), RENAME, &["--check"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("orphans:"), "got: {stderr}");
    assert!(stderr.contains("src/b.rs:1"), "got: {stderr}");
}

#[test]
fn verify_build_failure_names_orphans() {
    let dir = init_repo();
    write(
        dir.path(),
        "Cargo.toml",
        "[package]\nname = \"wicrate\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "use crate::a::Old;\npub fn f(x: Old) -> Old {\n    x\n}\n",
    );
    write(dir.path(), "src/lib.rs", "mod a;\nmod b;\n");
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=rename_old\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--check", "--verify", "build"]);
    assert!(!out.status.success(), "orphaned caller must not build");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("--verify build"), "got: {stderr}");
    assert!(
        stderr.contains("src/b.rs:1"),
        "orphan context, got: {stderr}"
    );
    assert!(stderr.contains("worktree untouched"), "got: {stderr}");
}
