//! T253 + WI-1c: `--patch '<diff>'` inline source + `.history/` auto-archive.
//!
//! The agent check loop needs no `Write` round-trip: the patch travels as an
//! argv string. Real applies archive it to `<repo>/.history/{id}.rsmpl`
//! (last 128 kept) and report the path on stderr plus the JSON `history`
//! key; `--check` skips the archive unless `--history` is passed.

mod common;
use common::*;

const RENAME: &str = "diff --git src/lib.rs\n@@ -1,3 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";

fn history_entries(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let h = dir.join(".history");
    if !h.exists() {
        return Vec::new();
    }
    let mut v: Vec<std::path::PathBuf> = std::fs::read_dir(&h)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rsmpl"))
        .collect();
    v.sort();
    v
}

#[test]
fn patch_inline_check_skips_history_unless_flagged() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    // WI-1c: `--check` leaves the worktree untouched by default.
    let out = run(dir.path(), &["--patch", RENAME, "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("history: "), "got: {stderr}");
    assert!(
        history_entries(dir.path()).is_empty(),
        "no archive on --check"
    );
    // Opt-in: `--check --history` archives exactly once.
    let out = run(dir.path(), &["--patch", RENAME, "--check", "--history"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("history: "), "got: {stderr}");
    // History file holds the exact raw patch text.
    let entries = history_entries(dir.path());
    assert_eq!(entries.len(), 1, "got: {entries:?}");
    assert_eq!(std::fs::read_to_string(&entries[0]).unwrap(), RENAME);
    let reported = stderr
        .lines()
        .find(|l| l.starts_with("history: "))
        .unwrap()
        .trim_start_matches("history: ")
        .to_string();
    assert!(
        reported.ends_with(entries[0].file_name().unwrap().to_str().unwrap()),
        "got: {reported}"
    );
}

#[test]
fn patch_inline_apply_for_real_and_json_reports_history() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    // WI-1c: `--check` JSON carries `"history": null` and no stderr line.
    let out = run(
        dir.path(),
        &["--patch", RENAME, "--format", "json", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert!(v["history"].is_null(), "got: {}", v["history"]);
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("history: "),
        "no history line on --check"
    );
    assert!(history_entries(dir.path()).is_empty());
    // Opt-in `--check --history` JSON still reports the path.
    let out = run(
        dir.path(),
        &[
            "--patch",
            RENAME,
            "--format",
            "json",
            "--check",
            "--history",
        ],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    let hist = v["history"].as_str().expect("JSON carries history key");
    assert!(hist.ends_with(".rsmpl"), "got: {hist}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("history: "),
        "stderr keeps history line in JSON mode"
    );
    // Real apply via --patch (no --check) always archives.
    let out = run(dir.path(), &["--patch", RENAME]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("fn bar(x: i32)"));
}

#[test]
fn patch_inline_conflicts_with_file() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\n");
    let out = run(dir.path(), &["--patch", RENAME, "--file", "x", "--check"]);
    assert_eq!(out.status.code(), Some(2));
    // JSON mode envelopes the clap conflict as RSMPL_IO_CONFLICT.
    let out = run(
        dir.path(),
        &[
            "--patch", RENAME, "--file", "x", "--format", "json", "--check",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
}

#[test]
fn history_prunes_to_128() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let hist_dir = dir.path().join(".history");
    std::fs::create_dir_all(&hist_dir).unwrap();
    for i in 0..130 {
        std::fs::write(
            hist_dir.join(format!("{i:010}-000000000-000001-0000.rsmpl")),
            "seed",
        )
        .unwrap();
    }
    let out = run_stdin(dir.path(), RENAME, &["--check", "--history"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let entries = history_entries(dir.path());
    assert_eq!(entries.len(), 128, "keeps last 128, got {}", entries.len());
    // Newest (real timestamp id) sorts last and survives.
    assert!(
        entries
            .iter()
            .any(|p| std::fs::read_to_string(p).unwrap() == RENAME),
        "newest patch survives pruning"
    );
}
