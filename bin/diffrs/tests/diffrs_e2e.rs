//! T340 e2e: `diffrs` compresses native git state into a valid
//! RSmPL-Git patch (v0: ordinary-hunk passthrough).
//!
//! T341: semantic inference (fn/item rename, N-site call sweep) with
//! round-trip pins; T343: hunk-title strip; T377: renames on
//! regressions.
//!
//! Validity = the output parses and `applyrs --check` accepts it
//! against the pre-change tree. Requires a workspace build so the
//! sibling `applyrs` binary sits next to `diffrs`
//! (`cargo test --workspace`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn sibling_applyrs() -> PathBuf {
    let me = binary();
    let file = if cfg!(windows) {
        "applyrs.exe".to_string()
    } else {
        "applyrs".to_string()
    };
    me.parent().map(|p| p.join(&file)).unwrap()
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

fn check_with_applyrs(dir: &Path, patch: &str) -> std::process::Output {
    Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(patch)
        .arg("--check")
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

/// WI-1b: ranged-sweep pin — tight original range + `# allow=many`,
/// never `# scope=file`.
fn assert_ranged(patch: &str, header: &str) {
    assert!(patch.contains(header), "got: {patch}");
    assert!(patch.contains("# allow=many\n"), "got: {patch}");
    assert!(!patch.contains("# scope=file"), "got: {patch}");
}

#[test]
fn unstaged_edit_round_trips_through_applyrs() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git src/lib.rs"));
    assert!(!patch.contains("diff --git a/"));
    assert!(patch.contains("+fn bar() {}"));

    // Back to the pre-change tree: the captured patch must check clean.
    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn untracked_new_file_round_trips_through_applyrs() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/new.rs", "fn brand_new() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git src/new.rs"));
    assert!(!patch.contains("diff --git a/"));
    assert!(patch.contains("new file mode"));
    assert!(patch.contains("+fn brand_new() {}"));

    // Remove the new file: the tree is now the pre-change state and the
    // captured patch (a creation) must check clean.
    std::fs::remove_file(dir.path().join("src/new.rs")).unwrap();
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn index_lines_are_stripped_from_passthrough() {
    // T374: `index <sha>…` is optional metadata (parsed, never
    // lowered), so diffrs drops it from ordinary passthrough. The
    // index-less patch still checks clean via applyrs and via bare
    // `git apply --check --unidiff-zero` (agents may pipe it to git
    // directly, not just applyrs).
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.lines().any(|l| l.starts_with("index ")),
        "got: {patch}"
    );
    assert!(patch.contains("--- a/src/lib.rs"), "got: {patch}");
    assert!(patch.contains("diff --git src/lib.rs\n"), "got: {patch}");
    assert!(!patch.contains("diff --git a/"), "got: {patch}");

    // Back to the pre-change tree: the captured patch must check clean.
    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    // Bare git accepts the index-less ordinary patch too.
    let mut child = std::process::Command::new("git")
        .args(["apply", "--check", "--unidiff-zero"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    let git_out = child.wait_with_output().unwrap();
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
}

#[test]
fn empty_repo_reports_no_changes() {
    let dir = init_repo();
    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("(no changes)"), "got: {stderr}");
}

#[test]
fn staged_flag_isolates_index() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\n");
    write(dir.path(), "b.rs", "fn b() {}\n");
    commit_all(dir.path());
    write(dir.path(), "a.rs", "fn a() {}\n// staged\n");
    write(dir.path(), "b.rs", "fn b() {}\n// unstaged\n");
    git(dir.path(), &["add", "a.rs"]);

    let staged = run(dir.path(), &["--staged"]);
    assert_eq!(staged.status.code(), Some(0));
    let staged_patch = String::from_utf8_lossy(&staged.stdout).to_string();
    assert!(staged_patch.contains("a.rs"), "got: {staged_patch}");
    assert!(!staged_patch.contains("b.rs"), "got: {staged_patch}");

    let unstaged = run(dir.path(), &[]);
    assert_eq!(unstaged.status.code(), Some(0));
    let unstaged_patch = String::from_utf8_lossy(&unstaged.stdout).to_string();
    assert!(unstaged_patch.contains("b.rs"), "got: {unstaged_patch}");
    assert!(!unstaged_patch.contains("a.rs"), "got: {unstaged_patch}");
}

// T392: untracked files are pending-worktree scope only — `git diff
// --cached` never shows them, so neither must `diffrs --staged` (they
// leaked into staged conversions, misrepresenting the index).
#[test]
fn staged_scope_excludes_untracked_files() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\n");
    commit_all(dir.path());
    write(dir.path(), "u.txt", "untracked body\n");

    // Nothing staged: index is clean, so the conversion is empty even
    // though an untracked file is present.
    let staged = run(dir.path(), &["--staged"]);
    assert_eq!(staged.status.code(), Some(0));
    assert!(staged.stdout.is_empty(), "got: {:?}", staged.stdout);
    let stderr = String::from_utf8_lossy(&staged.stderr).to_string();
    assert!(stderr.contains("(no changes)"), "got: {stderr}");
    let staged_json = run(dir.path(), &["--staged", "--format", "json"]);
    assert_eq!(staged_json.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&staged_json.stdout)).unwrap();
    assert_eq!(v["files"], serde_json::json!([]), "got: {v}");

    // With a staged tracked edit, `--staged` carries it but still not
    // the untracked file; pending (no flags) still embeds both.
    write(dir.path(), "a.rs", "fn a() {}\n// staged\n");
    git(dir.path(), &["add", "a.rs"]);
    let staged = run(dir.path(), &["--staged"]);
    assert_eq!(staged.status.code(), Some(0));
    let staged_patch = String::from_utf8_lossy(&staged.stdout).to_string();
    assert!(staged_patch.contains("a.rs"), "got: {staged_patch}");
    assert!(!staged_patch.contains("u.txt"), "got: {staged_patch}");
    let pending = run(dir.path(), &[]);
    assert_eq!(pending.status.code(), Some(0));
    let pending_patch = String::from_utf8_lossy(&pending.stdout).to_string();
    assert!(pending_patch.contains("u.txt"), "got: {pending_patch}");
}

// T392: same gate for history scope — `git diff [rev]` never shows
// untracked files, so neither must `diffrs [rev]` (worktree state
// polluted history conversions).
#[test]
fn rev_scope_excludes_untracked_files() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\n");
    commit_all(dir.path());
    write(dir.path(), "u.txt", "untracked body\n");

    // Zero tracked delta vs HEAD: the conversion is empty even though
    // an untracked file is present.
    let out = run(dir.path(), &["HEAD"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty(), "got: {:?}", out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("(no changes)"), "got: {stderr}");

    // With a tracked worktree edit, the rev conversion carries it but
    // still not the untracked file.
    write(dir.path(), "a.rs", "fn a() {}\n// worktree\n");
    let out = run(dir.path(), &["HEAD"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+// worktree"), "got: {patch}");
    assert!(!patch.contains("u.txt"), "got: {patch}");
}

// T246s23: same gate for the `--staged [rev]` combo — `git diff
// --cached <rev>` never shows untracked files, so neither must
// `diffrs --staged <rev>` (the T392 gate covers it; this pins the
// previously unprobed CLI form).
#[test]
fn staged_rev_combo_excludes_untracked_files() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\n");
    commit_all(dir.path());
    write(dir.path(), "a.rs", "fn a() {}\n// staged\n");
    git(dir.path(), &["add", "a.rs"]);
    write(dir.path(), "u.txt", "untracked body\n");

    let out = run(dir.path(), &["--staged", "HEAD"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("a.rs"), "got: {patch}");
    assert!(!patch.contains("u.txt"), "got: {patch}");
    let json = run(dir.path(), &["--staged", "HEAD", "--format", "json"]);
    assert_eq!(json.status.code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&json.stdout)).unwrap();
    assert_eq!(
        v["files"],
        serde_json::json!([{"path": "a.rs"}]),
        "got: {v}"
    );
}

#[test]
fn rev_argument_covers_history() {
    let dir = init_repo();
    write(dir.path(), "f.rs", "fn v1() {}\n");
    commit_all(dir.path());
    write(dir.path(), "f.rs", "fn v2() {}\n");
    commit_all(dir.path());

    // Worktree is clean; `HEAD~1` still surfaces the last commit.
    let out = run(dir.path(), &["HEAD~1"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+fn v2() {}"), "got: {patch}");
}

#[test]
fn json_envelope_shape() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", "fn foo() {}\nfn bar() {}\n");

    let out = run(dir.path(), &["--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["version"], serde_json::json!(1));
    assert_eq!(
        v["files"],
        serde_json::json!([{"path": "src/lib.rs"}]),
        "got: {v}"
    );
    assert!(
        v["patch"].as_str().unwrap_or("").contains("diff --git"),
        "got: {v}"
    );
}

#[test]
fn bad_repo_fails_with_envelope() {
    let out = run(Path::new("/nonexistent-diffrs-repo"), &["--format", "json"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).unwrap();
    assert_eq!(v["error"]["code"], serde_json::json!("RSMPL_IO_CONFLICT"));
    assert!(v["error"]["rule"].is_null());
}

// T341: a whole-file fn rename (identical body, one ident delta)
// compresses to a single semantic hunk that round-trips. The long
// signature is load-bearing: the `fn …(...)` pattern lines must beat
// the ordinary lines (T375 token-win guard) — short headers stay
// ordinary.
#[test]
fn fn_rename_compresses_to_semantic_hunk() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n    1\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        "fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n    1\n}\n",
    );

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_ranged(&patch, "@@ -1,3 @@ rule=r1\n");
    assert!(patch.contains("- fn foo\n"), "got: {patch}");
    assert!(!patch.contains("- fn foo(...)"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T341: an N-site call rewrite (same callee/arity delta) compresses
// to one `# allow=many` hunk with `matches == N`.
#[test]
fn call_sweep_compresses_to_one_hunk() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        "fn m() {\n    bar(1);\n    bar(2);\n    bar(3);\n}\n",
    );

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_ranged(&patch, "@@ -2,3 @@ rule=r1\n");
    assert!(!patch.contains("# expr"), "got: {patch}");
    assert!(patch.contains("- foo(E0)"), "got: {patch}");
    assert_eq!(
        patch.matches("@@ -2,3 @@ rule=r1\n").count(),
        1,
        "one hunk for three sites, got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(3), "got: {v}");
}

// T341: sweeps in two files share one patch (rule names stay unique,
// `# expr` reuse across hunks parses) and round-trip together.
#[test]
fn call_sweep_two_files_round_trip() {
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "fn m() {\n    old_function(argument1);\n    old_function(argument2);\n    old_function(argument3);\n}\n",
    );
    write(
        dir.path(),
        "b.rs",
        "fn n() {\n    old_function(argument1);\n    old_function(argument2);\n    old_function(argument3);\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "a.rs",
        "fn m() {\n    new_function(argument1);\n    new_function(argument2);\n    new_function(argument3);\n}\n",
    );
    write(
        dir.path(),
        "b.rs",
        "fn n() {\n    new_function(argument1);\n    new_function(argument2);\n    new_function(argument3);\n}\n",
    );

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("rule=r1\n"), "got: {patch}");
    assert!(patch.contains("rule=r2\n"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T343: git's `@@ … @@ <funcname>` section titles are stripped, not
// rejected — the conversion must exit 0 and round-trip.
#[test]
fn hunk_section_titles_do_not_break_conversion() {
    let dir = init_repo();
    let body: String = (0..20).map(|i| format!("    let v{i} = {i};\n")).collect();
    write(dir.path(), "f.rs", &format!("fn alpha() {{\n{body}}}\n"));
    commit_all(dir.path());
    // Sanity: this shape really carries a title pre-fix.
    write(
        dir.path(),
        "f.rs",
        &format!("fn alpha() {{\n{body}    let tail = 1;\n}}\n"),
    );
    let native = Command::new("git")
        .args(["diff", "--no-color", "--no-ext-diff"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let native = String::from_utf8_lossy(&native.stdout).to_string();
    assert!(
        native
            .lines()
            .any(|l| l.starts_with("@@") && l.contains("fn alpha")),
        "expected a titled hunk, got: {native}"
    );

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch
            .lines()
            .any(|l| l.starts_with("@@") && l.contains("fn alpha")),
        "titles stripped, got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T377: file moves surface as `rename from/to` sections (rename
// detection on), never as delete+create — conversion exits 0 and
// round-trips (replaces the T343 delete+create pin).
#[test]
fn file_move_converts_as_rename_section() {
    let dir = init_repo();
    write(dir.path(), "old.rs", "fn moved() {}\n");
    commit_all(dir.path());
    git(dir.path(), &["mv", "old.rs", "new.rs"]);

    // The move is staged by `git mv`; collect the index diff.
    let out = run(dir.path(), &["--staged"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("rename from old.rs"), "got: {patch}");
    assert!(patch.contains("rename to new.rs"), "got: {patch}");
    assert!(!patch.contains("deleted file mode"), "got: {patch}");
    assert!(!patch.contains("new file mode"), "got: {patch}");

    // Back to the pre-move tree: the rename section must check clean.
    git(dir.path(), &["mv", "new.rs", "old.rs"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T344: heterogeneous free-fn rename compresses to one slot sweep
// (`V Q fn … G (…) W`) with `matches == 4`.
#[test]
fn free_slot_sweep_compresses_to_one_hunk() {
    let dir = init_repo();
    let before = "fn foo(x: i32) -> i32 {\n    x\n}\n\nasync fn foo(x: i32) -> i32 {\n    x\n}\n\npub fn foo(x: i32) -> i32 {\n    x\n}\n\npub(crate) async fn foo<T>(x: T) -> T where T: Clone {\n    x\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        &before.replace("fn foo", "fn bar"),
    );

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_ranged(&patch, "@@ -1,15 @@ rule=r1\n");
    assert!(!patch.contains("# visibility"), "got: {patch}");
    assert!(patch.contains("- V Q fn foo G (...) W {"), "got: {patch}");
    assert!(patch.contains("+ V Q fn bar G (...) W {"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(4), "got: {v}");
}

// T344: heterogeneous method rename compresses to one impl-context
// slot sweep; the free-fn distractor stays untouched.
#[test]
fn method_slot_sweep_compresses_to_one_hunk() {
    let dir = init_repo();
    let before = "struct A;\nstruct B<T>(T);\nstruct C;\nstruct D;\n\nimpl A {\n    fn old_name(&self) -> i32 {\n        1\n    }\n}\n\nimpl B<i32> {\n    pub fn old_name(&self) -> i32 {\n        2\n    }\n}\n\nimpl C {\n    pub async fn old_name<T>(&self, x: T) -> T where T: Clone {\n        x.clone()\n    }\n}\n\nimpl crate::T for D {\n    fn old_name(&self) -> i32 {\n        4\n    }\n}\n\nfn old_name() -> i32 {\n    0\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after: String = before
        .lines()
        .map(|l| {
            if l.contains("fn old_name") && !l.contains("fn old_name()") {
                l.replace("fn old_name", "fn new_name")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("impl Svc {"), "got: {patch}");
    assert!(
        patch.contains("-     V Q fn old_name G (...) W {"),
        "got: {patch}"
    );
    assert!(
        patch.contains("+     V Q fn new_name G (...) W {"),
        "got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(4), "got: {v}");
}

// T347: an N-site method-to-method rename fan-out compresses to one
// `Receiver.old(E…)` → `Receiver.new(E…)` hunk with `matches == N`.
#[test]
fn method_to_method_sweep_compresses_to_one_hunk() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn m() {\n    a.foo(1);\n    b.foo(2);\n    c.foo(3);\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        "fn m() {\n    a.bar(1);\n    b.bar(2);\n    c.bar(3);\n}\n",
    );

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_ranged(&patch, "@@ -2,3 @@ rule=r1\n");
    assert!(!patch.contains("# expr"), "got: {patch}");
    assert!(patch.contains("- Receiver.foo(E0)"), "got: {patch}");
    assert!(patch.contains("+ Receiver.bar(E0)"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(3), "got: {v}");
}

// T345: an N-site method-to-function fan-out compresses to one
// `Receiver.old_method(E…)` hunk with `matches == N`.
#[test]
fn method_to_fn_sweep_compresses_to_one_hunk() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn m() {\n    a.old_method(1);\n    b.old_method(2);\n    c.old_method(3);\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/lib.rs",
        "fn m() {\n    new_function(a, 1);\n    new_function(b, 2);\n    new_function(c, 3);\n}\n",
    );

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_ranged(&patch, "@@ -2,3 @@ rule=r1\n");
    assert!(!patch.contains("# expr"), "got: {patch}");
    assert!(patch.contains("- Receiver.old_method(E0)"), "got: {patch}");
    assert!(
        patch.contains("+ new_function(Receiver, E0)"),
        "got: {patch}"
    );
    assert_eq!(
        patch.matches("@@ -2,3 @@ rule=r1\n").count(),
        1,
        "got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(3), "got: {v}");
}

// T349: a mixed file (call sweep in one hunk + body edit in a distant
// hunk) compresses partially: one semantic hunk for the sweep beside
// one ordinary hunk, and the mix round-trips with `matches == 2`.
// The const padding forces two native hunks (one change run would
// merge into a single mixed hunk, which stays ordinary).
#[test]
fn mixed_file_compresses_sweep_hunk_and_round_trips() {
    let dir = init_repo();
    let before = "fn m() {\n    foo(argument1);\n    foo(argument2);\n    foo(argument3);\n}\nconst A: i32 = 1;\nconst B: i32 = 2;\nconst C: i32 = 3;\nconst D: i32 = 4;\nconst E: i32 = 5;\nconst F: i32 = 6;\nconst G: i32 = 7;\nconst H: i32 = 8;\nfn n() {\n    1\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = before
        .replace(
            "    foo(argument1);\n    foo(argument2);\n    foo(argument3);",
            "    bar(argument1);\n    bar(argument2);\n    bar(argument3);",
        )
        .replace("fn n() {\n    1\n}", "fn n() {\n    2\n}");
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_ranged(&patch, "@@ -2,3 @@ rule=r1\n");
    assert!(patch.contains("- foo(E0)"), "got: {patch}");
    assert!(patch.contains("+ bar(E0)"), "got: {patch}");
    assert!(patch.contains("-    1\n"), "got: {patch}");
    assert!(patch.contains("+    2\n"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(3), "got: {v}");
}

// T351: ordinary edits are collected zero-context (`--unified=0`):
// a mid-file one-line edit emits no ` ` context body lines and
// still round-trips through `applyrs --check`.
#[test]
fn ordinary_edit_emits_zero_context_and_round_trips() {
    let dir = init_repo();
    let before: String = (0..10).map(|i| format!("line {i}\n")).collect();
    write(dir.path(), "f.txt", &before);
    commit_all(dir.path());
    let after = before.replace("line 5\n", "line FIVE\n");
    write(dir.path(), "f.txt", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("-line 5\n"), "got: {patch}");
    assert!(patch.contains("+line FIVE\n"), "got: {patch}");
    let context: Vec<&str> = patch.lines().filter(|l| l.starts_with(' ')).collect();
    assert!(context.is_empty(), "zero-context, got: {context:?}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T352: untracked `.history/` audit files are skipped, never embedded.
// A scratch repo with no `.gitignore` entry (like an old-commit
// worktree) still converts small, warns once, and keeps real untracked
// files.
#[test]
fn history_dir_untracked_is_skipped_with_warning() {
    let dir = init_repo();
    write(dir.path(), "base.txt", "base\n");
    commit_all(dir.path());
    write(dir.path(), ".history/old.rsmpl", "audit noise\n");
    write(dir.path(), "new.txt", "new file\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(patch.contains("diff --git new.txt"), "got: {patch}");
    assert!(!patch.contains(".history"), "history embedded: {patch}");
    assert!(stderr.contains("warning: .history/"), "got: {stderr}");

    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T354: an untracked symlink converts as a 120000 new-file section
// carrying the link-target string (never followed), and round-trips
// through `applyrs --check`. Unix-only: `std::os::unix::fs::symlink`.
#[cfg(unix)]
#[test]
fn untracked_symlink_converts_as_120000_and_round_trips() {
    let dir = init_repo();
    write(dir.path(), "real.txt", "real\n");
    commit_all(dir.path());
    std::os::unix::fs::symlink("real.txt", dir.path().join("link.txt")).unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git link.txt"), "got: {patch}");
    assert!(patch.contains("new file mode 120000"), "got: {patch}");
    assert!(patch.contains("+real.txt"), "got: {patch}");

    std::fs::remove_file(dir.path().join("link.txt")).unwrap();
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T354: a dangling untracked symlink (target missing) converts instead
// of failing with `SourceError` (exit 2) — `read_link` never follows.
// A link-to-dir shares the same `read_link` path (EISDIR avoided).
#[cfg(unix)]
#[test]
fn dangling_untracked_symlink_converts() {
    let dir = init_repo();
    write(dir.path(), "base.txt", "base\n");
    commit_all(dir.path());
    std::os::unix::fs::symlink("missing-target", dir.path().join("dangle.txt")).unwrap();
    std::os::unix::fs::symlink(".", dir.path().join("dirlink")).unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git dangle.txt"), "got: {patch}");
    assert!(patch.contains("+missing-target"), "got: {patch}");
    assert!(patch.contains("diff --git dirlink"), "got: {patch}");
}

// T363: an N-site use-path prefix rename (`use <parent>::Old::<tail>`
// → `use <parent>::New::<tail>` with heterogeneous tails) compresses
// to one `# allow=many` hunk; `matches == N` guards sweep over-match
// (T361) and a real apply byte-compares against the expected tree.
#[test]
fn use_prefix_sweep_compresses_to_one_hunk() {
    let dir = init_repo();
    let base = "use crate::old::Thing;\nuse crate::old::{A, B};\nuse crate::old::*;\nuse crate::old::Widget as W;\npub use crate::old::Reexport;\n";
    let rewritten = base.replace("crate::old", "crate::new");
    write(dir.path(), "src/lib.rs", base);
    commit_all(dir.path());
    write(dir.path(), "src/lib.rs", &rewritten);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("rule=r1\n"), "got: {patch}");
    assert_ranged(&patch, "@@ -1,5 @@ rule=r1\n");
    assert!(patch.contains("- use crate::old\n"), "got: {patch}");
    assert!(patch.contains("+ use crate::new\n"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--format")
        .arg("json")
        .arg("--diff-max-lines")
        .arg("0")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&checked.stdout)).unwrap();
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(5), "got: {v}");

    // Real-apply byte-compare (T361): `--check` is blind to sweep
    // over-match, so apply for real and compare bytes.
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let text = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert_eq!(text, rewritten, "applied tree differs");
}

#[test]
fn empty_untracked_converts_to_headers_only_and_checks_clean() {
    // T364: an empty untracked file converts to exactly what real git
    // shows (headers only, no hunk) and round-trips through
    // `applyrs --check` against the pre-change tree.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    std::fs::write(dir.path().join("empty.txt"), "").unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    // T393: headers-only (no `---`/`+++`) keeps two-path for bare-git.
    assert!(
        patch.contains("diff --git a/empty.txt b/empty.txt\n"),
        "got: {patch}"
    );
    assert!(patch.contains("new file mode 100644"), "got: {patch}");
    assert!(
        !patch.contains("@@"),
        "empty file must emit no hunk, got: {patch}"
    );

    // Stash the untracked file away; the captured patch must check
    // clean against the pre-change tree.
    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[cfg(unix)]
#[test]
fn exec_bit_untracked_round_trips_through_real_apply() {
    // T364: an untracked +x script converts as `new file mode 100755`
    // (the old hardcoded 100644 silently dropped +x); a real apply
    // materializes it +x. Real-apply, not just `--check` (T361).
    use std::os::unix::fs::PermissionsExt;
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    commit_all(dir.path());
    write(dir.path(), "run.sh", "#!/bin/sh\necho hi\n");
    std::fs::set_permissions(
        dir.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("new file mode 100755"), "got: {patch}");

    // Stash the untracked file away, apply for real, compare mode
    // and bytes against the pre-change-plus-patch expectation.
    git(dir.path(), &["stash", "-q", "-u"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let mode = std::fs::metadata(dir.path().join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "exec bit must survive the round-trip");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("run.sh")).unwrap(),
        "#!/bin/sh\necho hi\n"
    );
}

// T378: a tracked binary-only change has nothing expressible in
// RSmPL-Git — it reports empty + a warning (exit 0), not an
// `empty patch` exit-2 error.
#[test]
fn tracked_binary_only_change_warns_and_reports_no_changes() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "hello\n");
    std::fs::write(dir.path().join("img.bin"), [0u8, 1, 2, 3]).unwrap();
    commit_all(dir.path());
    std::fs::write(dir.path().join("img.bin"), [0u8, 1, 2, 4]).unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("img.bin"), "got: {stderr}");
    assert!(stderr.contains("(no changes)"), "got: {stderr}");
}

// T378: an untracked binary is stripped with the same warning (never
// silently dropped — T364 class); alongside text the text converts.
#[test]
fn untracked_binary_is_stripped_with_warning() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "hello\n");
    commit_all(dir.path());
    std::fs::write(dir.path().join("new.bin"), [0u8, 9, 9, 9]).unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("new.bin"), "got: {stderr}");

    write(dir.path(), "a.txt", "hello world\n");
    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git a.txt"), "got: {patch}");
    assert!(!patch.contains("new.bin"), "binary embedded: {patch}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("new.bin"), "got: {stderr}");

    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T379: real git never quotes spaces in `diff --git` headers — a
// space-named file converts and round-trips through `applyrs --check`.
#[test]
fn space_path_converts_and_round_trips() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "my file.txt", "a\n");
    commit_all(dir.path());
    write(dir.path(), "my file.txt", "b\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git my file.txt"), "got: {patch}");
    assert!(!patch.contains("diff --git a/"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[test]
fn space_path_rs_rename_uses_single_header_and_round_trips() {
    // T381: a space-path `.rs` rename-shaped edit compresses (the T380
    // shape) with a single-path header and round-trips through applyrs
    // — the single form resolves to the whole path, not the last word.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(
        dir.path(),
        "my file.rs",
        "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n    1\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "my file.rs",
        "fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n    1\n}\n",
    );

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git my file.rs\n"), "got: {patch}");
    assert!(!patch.contains("diff --git a/"), "got: {patch}");
    assert!(patch.contains("- fn foo\n"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

#[cfg(unix)]
#[test]
fn mode_only_pending_change_converts_and_checks_clean() {
    // T364: a chmod-only worktree change converts as passthrough
    // (zero semantic hunks) and checks clean against the pre-change
    // tree. The flip itself is pinned by real-apply in applyrs
    // mode_empty_e2e.
    use std::os::unix::fs::PermissionsExt;
    let dir = init_repo();
    write(dir.path(), "run.sh", "#!/bin/sh\n");
    std::fs::set_permissions(
        dir.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    commit_all(dir.path());
    std::fs::set_permissions(
        dir.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("old mode 100755"), "got: {patch}");
    assert!(patch.contains("new mode 100644"), "got: {patch}");
    assert!(
        !patch.contains("rule="),
        "mode-only must stay ordinary, got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T246 session 13: a tracked binary modification alongside a text
// edit — the binary section strips with a warning (T378 third cell)
// while the text section converts and checks clean.
#[test]
fn tracked_binary_modify_alongside_text_strips_with_warning() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn a() {}\n");
    std::fs::write(dir.path().join("img.bin"), [0u8, 1, 2, 3]).unwrap();
    commit_all(dir.path());
    std::fs::write(dir.path().join("img.bin"), [0u8, 1, 2, 4]).unwrap();
    write(dir.path(), "a.rs", "fn a() {}\nfn b() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git a.rs"), "got: {patch}");
    assert!(!patch.contains("diff --git a/"), "got: {patch}");
    assert!(!patch.contains("img.bin"), "binary embedded: {patch}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("img.bin"), "got: {stderr}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
}

// T246 session 13: a live file→symlink typechange converts as a
// delete+create pair (T357) and a real apply materializes the link.
#[cfg(unix)]
#[test]
fn live_file_to_symlink_typechange_round_trips() {
    let dir = init_repo();
    write(dir.path(), "real.txt", "real\n");
    write(dir.path(), "target.txt", "content\n");
    commit_all(dir.path());
    std::fs::remove_file(dir.path().join("real.txt")).unwrap();
    std::os::unix::fs::symlink("target.txt", dir.path().join("real.txt")).unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("deleted file mode"), "got: {patch}");
    assert!(patch.contains("new file mode 120000"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read_link(dir.path().join("real.txt")).unwrap(),
        PathBuf::from("target.txt")
    );
}

// T246 session 13: a live symlink→file typechange converts as a
// delete+create pair and a real apply materializes the regular file.
#[cfg(unix)]
#[test]
fn live_symlink_to_file_typechange_round_trips() {
    let dir = init_repo();
    write(dir.path(), "target.txt", "content\n");
    std::os::unix::fs::symlink("target.txt", dir.path().join("link.txt")).unwrap();
    commit_all(dir.path());
    std::fs::remove_file(dir.path().join("link.txt")).unwrap();
    write(dir.path(), "link.txt", "now a file\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("deleted file mode 120000"), "got: {patch}");
    assert!(patch.contains("new file mode"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(!std::fs::symlink_metadata(dir.path().join("link.txt"))
        .unwrap()
        .is_symlink());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("link.txt")).unwrap(),
        "now a file\n"
    );
}

// T246 session 13: a live symlink retarget converts as a plain
// modify and a real apply repoints the link.
#[cfg(unix)]
#[test]
fn live_symlink_retarget_round_trips() {
    let dir = init_repo();
    write(dir.path(), "a.txt", "a\n");
    write(dir.path(), "b.txt", "b\n");
    std::os::unix::fs::symlink("a.txt", dir.path().join("link.txt")).unwrap();
    commit_all(dir.path());
    std::fs::remove_file(dir.path().join("link.txt")).unwrap();
    std::os::unix::fs::symlink("b.txt", dir.path().join("link.txt")).unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git link.txt"), "got: {patch}");
    assert!(!patch.contains("file mode"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read_link(dir.path().join("link.txt")).unwrap(),
        PathBuf::from("b.txt")
    );
}

fn check_with_bare_git_unidiff_zero(dir: &Path, patch: &str) -> std::process::Output {
    // The project's bare-git compat bar (T351/T374): plain `git apply`
    // rejects zero-context hunks that native `git diff -U0` itself
    // emits (e.g. pure-insertion `@@ -1,0 +2 @@`), so compat is
    // measured with `--unidiff-zero`, which the reference CLI passes
    // internally.
    use std::io::Write;
    let mut child = Command::new("git")
        .args(["apply", "--check", "--unidiff-zero"])
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

// T246 session 15: space-path modify (git emits TAB-suffixed `---`/`+++`
// markers) alongside a content delete and a plain edit — all ordinary
// passthrough, single-path headers, `deleted file mode` kept (the only
// delete signal besides `/dev/null`; T383 rejects stripping it), and
// the output checks clean via applyrs, real-applies byte-exact, and
// passes bare `git apply --check --unidiff-zero`.
#[test]
fn space_path_delete_and_edit_round_trip() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "sp ace.txt", "q\n");
    write(dir.path(), "gone.txt", "bye\n");
    write(dir.path(), "plain.txt", "v1\n");
    commit_all(dir.path());
    write(dir.path(), "sp ace.txt", "q2\n");
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    write(dir.path(), "plain.txt", "v2\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git sp ace.txt\n"), "got: {patch}");
    assert!(!patch.contains("diff --git a/"), "got: {patch}");
    assert!(patch.contains("deleted file mode 100644"), "got: {patch}");
    assert!(
        !patch.contains("rule="),
        "mixed ordinary shapes must stay ordinary, got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("sp ace.txt")).unwrap(),
        "q2\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("plain.txt")).unwrap(),
        "v2\n"
    );
    assert!(
        !dir.path().join("gone.txt").exists(),
        "delete must remove the file"
    );
}

// T246 session 15: a mode flip bundled with a content change converts
// as ordinary passthrough (mode lines load-bearing for both
// consumers) and real-applies content + mode splice byte-exact.
#[cfg(unix)]
#[test]
fn mode_flip_with_content_round_trips() {
    use std::os::unix::fs::PermissionsExt;
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "run.sh", "exe\n");
    std::fs::set_permissions(
        dir.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    commit_all(dir.path());
    std::fs::set_permissions(
        dir.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    write(dir.path(), "run.sh", "exe\nmore\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("old mode 100755"), "got: {patch}");
    assert!(patch.contains("new mode 100644"), "got: {patch}");
    assert!(patch.contains("+more"), "got: {patch}");
    assert!(
        !patch.contains("rule="),
        "mode flip must stay ordinary, got: {patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("run.sh")).unwrap(),
        "exe\nmore\n"
    );
    let mode = std::fs::metadata(dir.path().join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0, "mode splice must clear exec, got {mode:o}");
}

// T246 session 16: a staged rename (`git mv`) converts via `--staged`
// to a headers-only rename section (rename-aware T377 path) and
// real-applies byte-exact onto the HEAD tree (stronger than the
// `--check`-only `file_move_converts_as_rename_section` pin).
#[test]
fn staged_rename_real_apply_round_trip() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "old.rs", "fn moved() {}\n");
    commit_all(dir.path());
    git(dir.path(), &["mv", "old.rs", "new.rs"]);

    let out = run(dir.path(), &["--staged"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("rename from old.rs"), "got: {patch}");
    assert!(patch.contains("rename to new.rs"), "got: {patch}");
    assert!(!patch.contains("rule="), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("new.rs")).unwrap(),
        "fn moved() {}\n"
    );
    assert!(
        !dir.path().join("old.rs").exists(),
        "rename must remove the old path"
    );
}

// T246 session 16: a staged delete (`git rm`) converts via `--staged`
// to a headers-only delete section (`deleted file mode` is the sole
// delete signal; T383 rejects stripping it) and real-apply removes
// the file from the HEAD tree.
#[test]
fn staged_delete_round_trips_through_applyrs() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "old.rs", "fn gone() {}\n");
    commit_all(dir.path());
    git(dir.path(), &["rm", "-q", "old.rs"]);

    let out = run(dir.path(), &["--staged"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git old.rs\n"), "got: {patch}");
    assert!(patch.contains("deleted file mode 100644"), "got: {patch}");
    assert!(patch.contains("+++ /dev/null"), "got: {patch}");
    assert!(!patch.contains("rule="), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert!(
        !dir.path().join("old.rs").exists(),
        "delete must remove the file"
    );
}

// T246 session 16: intent-to-add (`git add -N`) surfaces the worktree
// content as a create section under the default collection and
// real-applies onto an empty tree (stash cannot stage `-N` entries,
// so the pre-image is a fresh dir, not the stashed worktree).
#[test]
fn intent_to_add_round_trips_through_applyrs() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "base.rs", "fn base() {}\n");
    commit_all(dir.path());
    write(dir.path(), "new.rs", "fn added() {}\n");
    git(dir.path(), &["add", "-N", "new.rs"]);

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git new.rs\n"), "got: {patch}");
    assert!(patch.contains("new file mode 100644"), "got: {patch}");
    assert!(patch.contains("+fn added() {}"), "got: {patch}");
    assert!(!patch.contains("rule="), "got: {patch}");

    let pre = tempfile::tempdir().unwrap();
    git(pre.path(), &["init", "-q"]);
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--repo")
        .arg(pre.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(pre.path(), &patch);
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(pre.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(pre.path().join("new.rs")).unwrap(),
        "fn added() {}\n"
    );
}

// T246 session 17 (T385): CRLF line endings round-trip byte-exact.
// A tracked CRLF modify used to fail `applyrs --check` with
// `GIT_HUNK_CONFLICT` (the parser normalized `\r` away) and an
// untracked CRLF create silently converted to LF (the source built
// `+` lines via `str::lines`). Both now preserve `\r`.
#[test]
fn crlf_modify_and_create_round_trip_byte_exact() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "win.txt", "line1\r\nline2\r\nline3\r\n");
    write(dir.path(), "unix.txt", "a\n");
    commit_all(dir.path());
    write(dir.path(), "win.txt", "line1\r\nCHANGED\r\nline3\r\n");
    write(dir.path(), "newcrlf.txt", "newline-untracked\r\nsecond\r\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("-line2\r"), "got: {patch:?}");
    assert!(patch.contains("+CHANGED\r"), "got: {patch:?}");
    assert!(patch.contains("+newline-untracked\r"), "got: {patch:?}");
    assert!(!patch.contains("rule="), "got: {patch}");

    // Back to the pre-change tree: `git stash` reverts the tracked
    // CRLF modify but leaves untracked files, so remove the create
    // (the `untracked_new_file` pin's recipe) before checking.
    std::fs::remove_file(dir.path().join("newcrlf.txt")).unwrap();
    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "git apply --check failed: {}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    assert_eq!(
        std::fs::read(dir.path().join("win.txt")).unwrap(),
        b"line1\r\nCHANGED\r\nline3\r\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("newcrlf.txt")).unwrap(),
        b"newline-untracked\r\nsecond\r\n"
    );
}

// T246 session 17 (T386): a staged submodule add strips to a warning
// (mode-160000 sections can never round-trip — `applyrs` cannot read
// the directory on the pre-image side, and real git needs submodule
// machinery no patch carries), while a staged ordinary edit in the
// same index state still converts.
#[test]
fn staged_submodule_add_strips_with_warning() {
    let origin = init_repo();
    write(origin.path(), "f.txt", "data\n");
    commit_all(origin.path());
    let dir = init_repo();
    write(dir.path(), "main.rs", "fn main() {}\n");
    commit_all(dir.path());
    write(dir.path(), "main.rs", "fn main() {}\nfn other() {}\n");
    git(dir.path(), &["add", "main.rs"]);
    git(
        dir.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &origin.path().to_string_lossy(),
            "sub",
        ],
    );

    let out = run(dir.path(), &["--staged"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git main.rs"), "got: {patch}");
    assert!(!patch.contains("diff --git sub"), "got: {patch}");
    assert!(!patch.contains("Subproject commit"), "got: {patch}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("submodule (gitlink) stripped"),
        "got: {stderr}"
    );
}

// T246 session 17 (T387): an untracked nested repo (`?? inner/`) or
// empty dir must not fail the whole conversion (exit 2) — real `git
// diff` hides such entries, so they skip with a warning while real
// untracked files still embed.
#[test]
fn untracked_nested_repo_skips_with_warning() {
    let dir = init_repo();
    write(dir.path(), "base.rs", "fn base() {}\n");
    commit_all(dir.path());
    let inner = dir.path().join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    git(&inner, &["init", "-q"]);
    write(&inner, "g.txt", "nested\n");
    write(dir.path(), "new.rs", "fn added() {}\n");

    let out = run(dir.path(), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git new.rs"), "got: {patch}");
    assert!(!patch.contains("inner"), "got: {patch}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("untracked directory skipped"),
        "got: {stderr}"
    );
}
