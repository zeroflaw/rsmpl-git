//! T246 live-path regression pins: revision-form, layered-index, and
//! path-shape cells that native git accepts and `diffrs` must convert
//! + round-trip byte-exact.
//!
//! Split from `diffrs_e2e.rs` (session 20) to hold that file's 2000-line
//! budget (fixtures gate); helpers are duplicated, not shared.
//! Requires a workspace build so the sibling `applyrs` binary sits next
//! to `diffrs` (`cargo test --workspace`).

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

// T246 session 19: one file dirty in both index layers. Pending must show
// only the unstaged delta and "--staged" only the staged delta; stashing
// back to HEAD then applying staged-then-pending rebuilds the worktree
// byte-exact (the layer stack replays in order).
#[test]
fn same_file_staged_and_unstaged_layers_round_trip() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "f.rs", "fn base() {}\n");
    commit_all(dir.path());
    write(dir.path(), "f.rs", "fn base() {}\n// staged\n");
    git(dir.path(), &["add", "f.rs"]);
    write(dir.path(), "f.rs", "fn base() {}\n// staged\n// unstaged\n");

    let staged_out = run(dir.path(), &["--staged"]);
    assert_eq!(staged_out.status.code(), Some(0));
    let staged_patch = String::from_utf8_lossy(&staged_out.stdout).to_string();
    assert!(staged_patch.contains("+// staged"), "got: {staged_patch}");
    assert!(!staged_patch.contains("unstaged"), "got: {staged_patch}");

    let pending_out = run(dir.path(), &[]);
    assert_eq!(pending_out.status.code(), Some(0));
    let pending_patch = String::from_utf8_lossy(&pending_out.stdout).to_string();
    assert!(
        pending_patch.contains("+// unstaged"),
        "got: {pending_patch}"
    );
    assert!(
        !pending_patch.contains("+// staged"),
        "got: {pending_patch}"
    );

    git(dir.path(), &["stash", "-q"]);
    for patch in [&staged_patch, &pending_patch] {
        let checked = check_with_applyrs(dir.path(), patch);
        assert_eq!(checked.status.code(), Some(0));
        let git_out = check_with_bare_git_unidiff_zero(dir.path(), patch);
        assert!(git_out.status.success());
        let applied = Command::new(sibling_applyrs())
            .arg("--patch")
            .arg(patch)
            .arg("--repo")
            .arg(dir.path())
            .output()
            .unwrap();
        assert_eq!(applied.status.code(), Some(0));
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f.rs")).unwrap(),
        "fn base() {}\n// staged\n// unstaged\n"
    );
}

// T246 session 19: the two-dot range form is forwarded verbatim to
// "git diff", so it converts and round-trips against the range's
// first-parent pre-image (materialized here as a fresh repo holding
// the v1 content, the same shape as the history_e2e archive recipe).
#[test]
fn two_dot_range_converts_and_round_trips() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "f.rs", "fn v1() {}\n");
    commit_all(dir.path());
    write(dir.path(), "f.rs", "fn v2() {}\n");
    commit_all(dir.path());

    let out = run(dir.path(), &["HEAD~1..HEAD"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+fn v2() {}"), "got: {patch}");
    assert!(!patch.contains("rule="), "got: {patch}");

    let pre = init_repo();
    write(pre.path(), "f.rs", "fn v1() {}\n");
    commit_all(pre.path());
    let checked = check_with_applyrs(pre.path(), &patch);
    assert_eq!(checked.status.code(), Some(0));
    let git_out = check_with_bare_git_unidiff_zero(pre.path(), &patch);
    assert!(git_out.status.success());
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(pre.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(pre.path().join("f.rs")).unwrap(),
        "fn v2() {}\n"
    );
}

// T246 session 19: an unresolvable REV must fail fast (exit 2, empty
// stdout) rather than convert the wrong scope or emit a silent
// "(no changes)" for a typo the user would never notice.
#[test]
fn unresolvable_rev_fails_fast() {
    let dir = init_repo();
    write(dir.path(), "f.rs", "fn v1() {}\n");
    commit_all(dir.path());

    let out = run(dir.path(), &["nosuchrev12345"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty(), "got: {:?}", out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("error:"), "got: {stderr}");
}
// T246 session 20 (T389): default `core.quotepath=true` C-quotes
// non-ASCII names in both `diff --git` headers and `??` status lines;
// the conversion must resolve the real filename on both sides.
// Hermetic: repo-local quotepath in case ambient config disables it.
#[test]
fn quoted_non_ascii_paths_round_trip() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    git(dir.path(), &["config", "core.quotepath", "true"]);
    write(dir.path(), "café.txt", "hello\n");
    commit_all(dir.path());
    write(dir.path(), "café.txt", "hello world\n");
    write(dir.path(), "neué.txt", "brand new\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+hello world"), "got: {patch}");
    assert!(patch.contains("+brand new"), "got: {patch}");

    // Back to the pre-change tree: check, bare-git check, real-apply.
    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(checked.status.code(), Some(0));
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(git_out.status.success());
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(dir.path().join("café.txt")).unwrap(),
        b"hello world\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("neué.txt")).unwrap(),
        b"brand new\n"
    );
}

// T246 session 20 (T390): `--repo <subdir>` converts — the tracked
// side was already repo-root-relative, but untracked reads joined
// root-relative `??` paths onto the subdir (`sub/sub/..`, exit 2).
// Output stays root-relative: check/apply run against the root.
#[test]
fn subdir_repo_converts_and_round_trips_against_root() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "top.txt", "top\n");
    write(dir.path(), "nested/inner.txt", "inner\n");
    commit_all(dir.path());
    write(dir.path(), "top.txt", "top2\n");
    write(dir.path(), "nested/inner.txt", "inner2\n");
    write(dir.path(), "nested/brand.txt", "newfile\n");

    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir.path().join("nested"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("nested/brand.txt"), "got: {patch}");
    assert!(patch.contains("+newfile"), "got: {patch}");

    // Back to the pre-change tree, then check + real-apply at the root.
    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(checked.status.code(), Some(0));
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("top.txt")).unwrap(),
        "top2\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("nested/inner.txt")).unwrap(),
        "inner2\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("nested/brand.txt")).unwrap(),
        "newfile\n"
    );
}
