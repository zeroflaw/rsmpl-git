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

#[test]
fn headers_only_sections_keep_two_path_and_bare_apply() {
    // T393: headers-only sections (mode flips, empty creates) carry
    // no `---`/`+++` markers, so the T381 single-path collapse must
    // not fire — bare `git apply` resolves filenames from the
    // markers and rejects a collapsed single path (`lacks filename
    // information`). Both cells stay two-path, `--check` clean,
    // bare-git clean, and real-apply exact.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    // Cell 1: staged chmod-only.
    let dir = init_repo();
    write(dir.path(), "f.sh", "x\n");
    commit_all(dir.path());
    git(dir.path(), &["update-index", "--chmod=+x", "f.sh"]);
    let out = run(dir.path(), &["--staged"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("diff --git a/f.sh b/f.sh\n"), "got: {patch}");
    assert!(!patch.contains("diff --git f.sh\n"), "got: {patch}");
    // Back to HEAD: the mode patch must check clean both ways.
    git(dir.path(), &["stash", "-q"]);
    #[cfg(unix)]
    let before = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(dir.path().join("f.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let bare = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        bare.status.success(),
        "bare git apply --check failed: {}",
        String::from_utf8_lossy(&bare.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    // The flip took effect: the mode moved off the pre-image and
    // owner-exec is set. No absolute value asserted — `git apply`
    // itself ORs exec bits onto the worktree mode (664 -> 775 under
    // umask 002, 644 -> 755 under 022), so mirroring git is the
    // contract and the pre/post delta proves it here.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let after = std::fs::metadata(dir.path().join("f.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_ne!(after, before, "mode flip had no effect");
        assert_ne!(after & 0o100, 0, "owner-exec bit not set");
    }
    // Cell 2: empty (0-byte) untracked create.
    let dir2 = init_repo();
    write(dir2.path(), "a.txt", "base\n");
    commit_all(dir2.path());
    write(dir2.path(), "empty.txt", "");
    let out = run(dir2.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        patch.contains("diff --git a/empty.txt b/empty.txt\n"),
        "got: {patch}"
    );
    // Back to the pre-change tree: check clean both ways, then the
    // real apply must recreate the 0-byte file.
    std::fs::remove_file(dir2.path().join("empty.txt")).unwrap();
    let checked = check_with_applyrs(dir2.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let bare = check_with_bare_git_unidiff_zero(dir2.path(), &patch);
    assert!(
        bare.status.success(),
        "bare git apply --check failed: {}",
        String::from_utf8_lossy(&bare.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir2.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::metadata(dir2.path().join("empty.txt"))
            .unwrap()
            .len(),
        0
    );
}

// T246 session 27: staged binary-only change under `--format json`.
// Text mode strips to `(no changes)` rc 0 with a warning (T378); the
// JSON contract is the envelope with `files: []` and an empty patch,
// warning on stderr.
#[test]
fn staged_binary_only_json_envelope_is_empty() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() {}\n");
    commit_all(dir.path());
    std::fs::write(dir.path().join("blob.bin"), [0u8, 1, 2, 3, 255, 254]).unwrap();
    git(dir.path(), &["add", "blob.bin"]);
    let out = run(dir.path(), &["--staged", "--format", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("\"files\":[]"), "got: {stdout}");
    assert!(stdout.contains("\"patch\":\"\""), "got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("blob.bin"), "got: {stderr}");
}

// T246 session 28 (T395): staged binary CREATE without NUL bytes.
// Native `git diff --cached` dumps literal non-UTF-8 bytes for creates
// (only modifies get the `Binary … differ` marker), which used to leak
// through as `+` lines with no warning and corrupt the lowering
// (`--check` failed `corrupt patch` where bare git passes). Now the
// section strips + warns like marker binaries: rc 0, empty patch,
// warning names the file. Bytes avoid NUL (NUL'd creates already take
// the marker path) and LF (a literal 0x0A would start a prefix-less
// line, a different shape).
#[test]
fn staged_binary_create_without_nul_is_stripped_and_warned() {
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() {}\n");
    commit_all(dir.path());
    let bytes: Vec<u8> = (1u8..=200).filter(|&b| b != 10).collect();
    std::fs::write(dir.path().join("blob.bin"), &bytes).unwrap();
    git(dir.path(), &["add", "blob.bin"]);
    let out = run(dir.path(), &["--staged"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!stdout.contains("blob.bin"), "got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("blob.bin"), "got: {stderr}");
}

// T395 follow-up cell: staged binary create ALONGSIDE a text modify —
// only the binary section strips; the text side converts and
// round-trips `--check` + real-apply byte-exact.
#[test]
fn staged_binary_create_with_text_modify_keeps_text() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() {}\n");
    commit_all(dir.path());
    write(dir.path(), "a.rs", "fn main() { 1 }\n");
    let bytes: Vec<u8> = (1u8..=200).filter(|&b| b != 10).collect();
    std::fs::write(dir.path().join("blob.bin"), &bytes).unwrap();
    git(dir.path(), &["add", "-A"]);
    let out = run(dir.path(), &["--staged"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+fn main() { 1 }"), "got: {patch}");
    assert!(!patch.contains("blob.bin"), "got: {patch}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("blob.bin"), "got: {stderr}");
    let pre = init_repo();
    write(pre.path(), "a.rs", "fn main() {}\n");
    commit_all(pre.path());
    let checked = check_with_applyrs(pre.path(), &patch);
    assert_eq!(checked.status.code(), Some(0));
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(pre.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(pre.path().join("a.rs")).unwrap(),
        "fn main() { 1 }\n"
    );
}

#[test]
fn tracked_text_with_replacement_char_converts_and_round_trips() {
    // T396: a literal U+FFFD char (valid UTF-8 text, e.g. a
    // string-literal fixture) on a changed line is genuine content —
    // the old infer U+FFFD arm stripped the whole section, silently
    // dropping the file (caught on 62fae28's own lib.rs). Now it
    // converts + round-trips byte-exact.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "f.rs", "const S: &str = \"�\";\nfn main() {}\n");
    commit_all(dir.path());
    write(
        dir.path(),
        "f.rs",
        "const S: &str = \"�!\";\nfn main() {}\n",
    );
    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("const S"), "got: {patch}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("warning"), "got: {stderr}");
    let pre = init_repo();
    write(pre.path(), "f.rs", "const S: &str = \"�\";\nfn main() {}\n");
    commit_all(pre.path());
    let checked = check_with_applyrs(pre.path(), &patch);
    assert_eq!(checked.status.code(), Some(0));
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(pre.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(pre.path().join("f.rs")).unwrap(),
        std::fs::read(dir.path().join("f.rs")).unwrap()
    );
}

#[test]
fn untracked_non_utf8_without_nul_is_stripped_and_warned() {
    // T396: an untracked no-NUL non-UTF-8 file takes the source
    // `Binary` marker (the T395 staged shape, now detected at the byte
    // level): rc 0, empty patch, warning names the file.
    let dir = init_repo();
    write(dir.path(), "a.rs", "fn main() {}\n");
    commit_all(dir.path());
    let bytes: Vec<u8> = (0x80u8..=0xC8).collect();
    std::fs::write(dir.path().join("blob.bin"), &bytes).unwrap();
    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!stdout.contains("blob.bin"), "got: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("blob.bin"), "got: {stderr}");
}

#[test]
fn tab_filename_stays_quoted_through_lowering() {
    // T398: native git C-quotes control chars (`"tab\tname.txt"`).
    // diffrs passes the quoted header through, but the old lowering
    // re-emitted a literal TAB unquoted, which bare git rejects
    // (`error: tab: No such file`). The lowering now C-quotes too, so
    // the lowered artifact itself is pinned here via `--output`.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "tab\tname.txt", "l1\nl2\nl3\n");
    commit_all(dir.path());
    write(dir.path(), "tab\tname.txt", "l1\nL2\nl3\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("\"a/tab\\tname.txt\""), "got: {patch}");

    git(dir.path(), &["stash", "-q", "-u"]);
    let lowered_path = dir.path().join("lowered.patch");
    let checked = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--check")
        .arg("--output")
        .arg(&lowered_path)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let lowered = std::fs::read_to_string(&lowered_path).unwrap();
    assert!(lowered.contains("\"a/tab\\tname.txt\""), "got: {lowered}");
    assert!(!lowered.contains('\t'), "literal TAB leaked: {lowered}");
    // The lowered artifact is what bare git must accept.
    let git_out = Command::new("git")
        .args([
            "apply",
            "--check",
            "--unidiff-zero",
            lowered_path.to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        git_out.status.success(),
        "{}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    std::fs::remove_file(&lowered_path).unwrap();
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(dir.path().join("tab\tname.txt")).unwrap(),
        b"l1\nL2\nl3\n"
    );
}

#[test]
fn space_path_with_b_segment_applies_at_right_path() {
    // T397: `split_git_paths` split at the FIRST `b/`-led part, so a
    // path holding an interior `b/…` segment mis-targeted — silently
    // creating `c d/e f.txt b/a b/c d/e f.txt`, rc 0. The equal-sides
    // split targets the real file; no stray dirs may appear.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "top.txt", "x\n");
    commit_all(dir.path());
    write(dir.path(), "a b/c d/e f.txt", "deep\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("+deep"), "got: {patch}");

    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "{}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(dir.path().join("a b/c d/e f.txt")).unwrap(),
        b"deep\n"
    );
    assert!(!dir.path().join("c d").exists(), "stray mis-split dirs");
}

#[test]
fn spaced_rename_resolves_from_rename_lines() {
    // T397: `diff --git a/old b/c b/new d` halves mis-split (the new
    // side read `c b/new d`); bare git resolves via `rename to`. The
    // `rename from/to` override carries the exact paths instead.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "old b/c", "one\ntwo\n");
    commit_all(dir.path());
    std::fs::rename(dir.path().join("old b/c"), dir.path().join("new d")).unwrap();
    git(dir.path(), &["add", "-A"]);

    let out = run(dir.path(), &["--staged"]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("rename to new d"), "got: {patch}");

    git(dir.path(), &["stash", "-q"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "{}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(dir.path().join("new d")).unwrap(),
        b"one\ntwo\n"
    );
    assert!(!dir.path().join("old b").exists(), "old side not removed");
}

#[test]
fn untracked_quote_name_text_create_round_trips() {
    // T399: `new_file_section` interpolated the path raw, so
    // `q"ute2.txt` emitted `diff --git a/q"ute2.txt b/q"ute2.txt` —
    // the splitter mis-read the interior `"` and applyrs silently
    // wrote `qute2.txt b/qute2.txt`, rc 0. Quoted emission targets
    // the real file; no stray dirs may appear.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "top.txt", "x\n");
    commit_all(dir.path());
    std::fs::write(dir.path().join("q\"ute2.txt"), "hello\n").unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("q\\\"ute2.txt"), "got: {patch}");
    assert!(
        !patch.contains("a/q\"ute2.txt b/"),
        "raw unquoted header: {patch}"
    );

    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let git_out = check_with_bare_git_unidiff_zero(dir.path(), &patch);
    assert!(
        git_out.status.success(),
        "{}",
        String::from_utf8_lossy(&git_out.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(dir.path().join("q\"ute2.txt")).unwrap(),
        b"hello\n"
    );
    assert!(
        !dir.path().join("qute2.txt").exists(),
        "mis-split stray file"
    );
}

#[test]
fn untracked_quote_name_binary_strips_with_quoted_warning() {
    // T399: the untracked binary marker interpolated the path raw, so
    // `q"bin.txt` warned as `a/qbin.txt b/qbin.txt` (quote dropped by
    // the mis-split). Quoted emission keeps the real name in the
    // warning; the binary still strips and the text mix round-trips.
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    write(dir.path(), "top.txt", "x\n");
    commit_all(dir.path());
    std::fs::write(dir.path().join("q\"bin.txt"), [0u8, 1, 2, 255, 254]).unwrap();
    write(dir.path(), "keep.txt", "keep me\n");

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(patch.contains("+keep me"), "got: {patch}");
    assert!(!patch.contains("qbin"), "binary leaked: {patch}");
    assert!(
        stderr.contains("q\"bin.txt"),
        "warning lost the quote: {stderr}"
    );

    git(dir.path(), &["stash", "-q", "-u"]);
    let checked = check_with_applyrs(dir.path(), &patch);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    assert_eq!(
        std::fs::read(dir.path().join("keep.txt")).unwrap(),
        b"keep me\n"
    );
    assert!(
        !dir.path().join("q\"bin.txt").exists(),
        "stripped binary must not materialize"
    );
}
