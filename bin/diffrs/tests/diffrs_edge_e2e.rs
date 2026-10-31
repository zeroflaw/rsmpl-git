//! Edge-case e2e split out of `diffrs_e2e.rs` (gate headroom).
//!
//! `diffrs_e2e.rs` sat at exactly 2000 lines, at the
//! `no_rust_file_exceeds_2000_lines` gate; this file holds its 8-test
//! tail verbatim (space-path/staged/crlf/submodule/nested-repo edge
//! cases) plus the `check_with_bare_git_unidiff_zero` helper only the
//! tail uses, so the next diffrs test addition does not trip the gate.
//! No test logic altered; the eight helpers below are local copies of
//! the ones that stay in `diffrs_e2e.rs` (helpers duplicated, not
//! shared — T246 precedent).

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
    // Tolerate EPIPE: the child may exit before reading stdin.
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(patch.as_bytes()) {
            assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe);
        }
    }
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
