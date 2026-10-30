//! T365 + T367: hunk-header shape fail-fast pins (SPEC 25.2).
//! WI-9d-ordinary-at: inter-hunk patch-prose pins (SPEC 11).
//!
//! A hunk header with leading whitespace is rejected (T365: without the
//! gate it lands in metadata and lowers to an empty no-op with exit 0),
//! and ordinary hunk headers must agree with the body counts (T367:
//! without the gate a miscount surfaces downstream as a misleading
//! `GIT_HUNK_CONFLICT`). Both fail fast with `RSMPL_PARSE_CONFLICT`
//! (exit 2); text + JSON envelopes pinned, worktree untouched.

mod common;
use common::*;

fn check_rejected(patch: &str, file: &str, content: &str) -> (String, serde_json::Value) {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
        .expect("failure stdout must be a JSON envelope");
    assert_eq!(
        read(dir.path(), file),
        content,
        "worktree must be untouched"
    );
    (stderr, v)
}

#[test]
fn indented_hunk_header_rejected() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n @@ -1,1 +1,1 @@\n-fn a() {}\n+fn b() {}\n";
    let (stderr, v) = check_rejected(patch, "src/lib.rs", "fn a() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("leading whitespace"), "got: {stderr}");
    assert!(stderr.contains("column 0"), "got: {stderr}");
    assert!(stderr.contains("old_range: line 4"), "got: {stderr}");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["range"], serde_json::json!("line 4"));
    assert!(v["error"]["suggestion"].is_string(), "got: {v}");
}

#[test]
fn ordinary_hunk_count_mismatch_rejected() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,7 +1,7 @@\n fn a() {}\n-fn b() {}\n+fn b2() {}\n";
    let (stderr, v) = check_rejected(patch, "src/lib.rs", "fn a() {}\nfn b() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains(
            "ordinary hunk body has 2 old-side and 2 new-side lines but header claims -1,7 +1,7"
        ),
        "got: {stderr}"
    );
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
    assert!(v["error"]["range"].is_string(), "got: {v}");
    assert!(
        v["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("fix the header counts"),
        "got: {v}"
    );
    // WI-10 consideration: machine repair carries exact counts plus a
    // drop-in corrected header; swapping it in passes check first try.
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("fix_header"), "got: {stderr}");
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_header"));
    assert_eq!(repair["expected_old"], serde_json::json!(2));
    assert_eq!(repair["expected_new"], serde_json::json!(2));
    assert_eq!(repair["claimed_old"], serde_json::json!(7));
    assert_eq!(repair["claimed_new"], serde_json::json!(7));
    let expected_header = repair["expected_header"]
        .as_str()
        .expect("repair must carry expected_header");
    assert_eq!(expected_header, "@@ -1,2 +1,2 @@", "got: {v}");
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\nfn b() {}\n");
    let fixed = patch.replace("@@ -1,7 +1,7 @@", expected_header);
    let out = run_stdin(dir.path(), &fixed, &["--check"]);
    assert!(
        out.status.success(),
        "self-corrected header must pass check first try, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn well_formed_ordinary_hunk_applies() {
    // Control: truthful headers still apply for real (T361 doctrine).
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "mod old;\n\nfn main() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n mod old;\n+mod new;\n\n fn main() {}\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "mod old;\nmod new;\n\nfn main() {}\n"
    );
}

#[test]
fn at_prose_between_hunks_rejected() {
    // WI-9d-ordinary-at probe A: a line starting with `@@` between hunks
    // is a hunk-header attempt, never prose — loud, fail-safe.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n fn a() {}\n+fn c() {}\n@@ this is prose, not a header\n@@ -2,1 +2,2 @@\n fn b() {}\n+fn d() {}\n";
    let (stderr, v) = check_rejected(patch, "src/lib.rs", "fn a() {}\nfn b() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("invalid hunk header"), "got: {stderr}");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PARSE_CONFLICT")
    );
}

#[test]
fn bare_prose_between_hunks_skipped() {
    // WI-9d-ordinary-at probes B2/F: bare prose (no ` /+/-/\\` prefix, no
    // leading `@@`) between hunks, between sections, or trailing at EOF
    // is skipped — the lowering is byte-identical to the prose-free
    // patch, so prose carries no patch semantics and can never corrupt.
    let control = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n fn a() {}\n+fn c() {}\n@@ -2,1 +2,2 @@\n fn b() {}\n+fn d() {}\n";
    let with_prose = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n fn a() {}\n+fn c() {}\nSome prose explanation here.\n@@ -2,1 +2,2 @@\n fn b() {}\n+fn d() {}\nTrailing note at EOF.\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\nfn b() {}\n");
    let checked = run_stdin(dir.path(), control, &["--check"]);
    let prosed = run_stdin(dir.path(), with_prose, &["--check"]);
    assert!(
        checked.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(
        prosed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&prosed.stderr)
    );
    assert_eq!(
        checked.stdout, prosed.stdout,
        "lowering must be prose-invariant"
    );
    let d1 = init_repo();
    write(d1.path(), "src/lib.rs", "fn a() {}\nfn b() {}\n");
    let d2 = init_repo();
    write(d2.path(), "src/lib.rs", "fn a() {}\nfn b() {}\n");
    assert!(run_stdin(d1.path(), control, &[]).status.success());
    assert!(run_stdin(d2.path(), with_prose, &[]).status.success());
    assert_eq!(read(d1.path(), "src/lib.rs"), read(d2.path(), "src/lib.rs"));
    assert_eq!(
        read(d1.path(), "src/lib.rs"),
        "fn a() {}\nfn c() {}\nfn b() {}\nfn d() {}\n"
    );
}

#[test]
fn plus_prefixed_prose_is_body_line() {
    // WI-9d-ordinary-at probe D: a `+`-leading line is an addition under
    // Git semantics, never skippable prose — the count gate fires loud
    // with a machine repair. Guards a future prose-skip change against
    // silently eating `+` lines.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,2 @@\n fn a() {}\n+fn c() {}\n+NOTE: this line carries a plus prefix\n@@ -2,1 +2,1 @@\n fn b() {}\n";
    let (stderr, v) = check_rejected(patch, "src/lib.rs", "fn a() {}\nfn b() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("fix_header"), "got: {stderr}");
    assert_eq!(
        v["error"]["repair"]["action"],
        serde_json::json!("fix_header")
    );
    assert_eq!(v["error"]["repair"]["expected_old"], serde_json::json!(1));
    assert_eq!(v["error"]["repair"]["expected_new"], serde_json::json!(3));
}

#[test]
fn blank_insertion_needs_plus_prefix() {
    // WI-10 tooling note: a truly empty line inside an ordinary hunk body
    // is an empty context line under the T212 lookahead — author a blank
    // insertion with `+` on an otherwise empty line, never a bare empty.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn a() {}\nfn b() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,3 @@\n fn a() {}\n+\n fn b() {}\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), "fn a() {}\n\nfn b() {}\n");
    // Bare-empty twin: the empty line reads as context (T212), so the
    // header counts mismatch — loud, fail-safe, worktree untouched.
    let bare = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,3 @@\n fn a() {}\n\n fn b() {}\n";
    let (stderr, v) = check_rejected(bare, "src/lib.rs", "fn a() {}\nfn b() {}\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("fix_header"), "got: {stderr}");
    assert_eq!(
        v["error"]["repair"]["action"],
        serde_json::json!("fix_header")
    );
}

#[test]
fn bullet_removal_needs_double_dash_prefix() {
    // WI-10 tooling note, companion: removing a `- foo` bullet needs a
    // `-- ` body line (diff `-` + content `- foo`); ` - foo` is context.
    let dir = init_repo();
    write(dir.path(), "notes.md", "# Title\n- foo\n- bar\n");
    let patch = "diff --git a/notes.md b/notes.md\n--- a/notes.md\n+++ b/notes.md\n@@ -1,3 +1,2 @@\n # Title\n-- foo\n - bar\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "notes.md"), "# Title\n- bar\n");
    // Space-prefixed twin with the deletion header: ` - foo` reads as
    // context, so the header counts mismatch -- loud, fail-safe.
    let spaced = "diff --git a/notes.md b/notes.md\n--- a/notes.md\n+++ b/notes.md\n@@ -1,3 +1,2 @@\n # Title\n - foo\n - bar\n";
    let (stderr, v) = check_rejected(spaced, "notes.md", "# Title\n- foo\n- bar\n");
    assert!(stderr.contains("RSMPL_PARSE_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("fix_header"), "got: {stderr}");
    assert_eq!(
        v["error"]["repair"]["action"],
        serde_json::json!("fix_header")
    );
    // Same twin with corrected counts: green but a no-op (context keeps
    // `- foo`), proving the silent-wrong-intent risk the rule prevents.
    let ctx_patch = "diff --git a/notes.md b/notes.md\n--- a/notes.md\n+++ b/notes.md\n@@ -1,3 +1,3 @@\n # Title\n - foo\n - bar\n";
    let dir2 = init_repo();
    write(dir2.path(), "notes.md", "# Title\n- foo\n- bar\n");
    let out = run_stdin(dir2.path(), ctx_patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir2.path(), ctx_patch, &[]);
    assert!(out.status.success());
    assert_eq!(read(dir2.path(), "notes.md"), "# Title\n- foo\n- bar\n");
}
