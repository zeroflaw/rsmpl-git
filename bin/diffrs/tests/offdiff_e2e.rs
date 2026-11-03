//! DISCOVERED off-diff gap: a workspace intent proven over the
//! changed-files-only `pre` map silently rewrites files the native diff
//! never touches. The facade widens the D13 domain to off-diff files
//! that mention a followed ident, so the intent declines to ordinary.
//!
//! Requires a workspace build so the sibling `applyrs` binary sits
//! next to `diffrs` (`cargo test --workspace`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn sibling_applyrs() -> PathBuf {
    let me = binary();
    me.parent().map(|p| p.join("applyrs")).unwrap()
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

fn rename_change(dir: &Path) {
    let a = std::fs::read_to_string(dir.join("src/a.rs")).unwrap();
    write(
        dir,
        "src/a.rs",
        &a.replace("pub struct Old;", "pub struct New;")
            .replace("-> Old { Old }", "-> New { New }"),
    );
    let b = std::fs::read_to_string(dir.join("src/b.rs")).unwrap();
    write(
        dir,
        "src/b.rs",
        &b.replace("crate::Old", "crate::New")
            .replace("-> Old { Old }", "-> New { New }"),
    );
}

fn diffrs(dir: &Path, extra: &[&str]) -> String {
    let out = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn seed_three_file_repo(dir: &Path) {
    write(
        dir,
        "src/a.rs",
        "pub struct Old;\npub fn make() -> Old { Old }\n",
    );
    write(
        dir,
        "src/b.rs",
        "use crate::Old;\npub fn use_b() -> Old { Old }\n",
    );
    write(
        dir,
        "src/c.rs",
        "use crate::Old;\npub fn use_c() -> Old { Old }\n",
    );
    commit_all(dir);
}

#[test]
fn off_diff_use_forces_ordinary_raise() {
    assert!(sibling_applyrs().exists(), "sibling applyrs missing");
    let dir = init_repo();
    seed_three_file_repo(dir.path());
    rename_change(dir.path());

    // c.rs is untouched by the native diff but names Old: the follow
    // intent must decline to ordinary sections for the touched files.
    let patch = diffrs(dir.path(), &[]);
    assert!(
        !patch.contains("# follow=uses"),
        "off-diff c.rs must veto the follow intent, got:\n{patch}"
    );
    assert!(patch.contains("diff --git src/a.rs"), "got:\n{patch}");
    assert!(patch.contains("diff --git src/b.rs"), "got:\n{patch}");

    // T390: `--repo` may be a subdirectory — pathspec roots at the
    // worktree top, so the subdir raise declines identically.
    let sub_patch = diffrs(&dir.path().join("src"), &[]);
    assert!(
        !sub_patch.contains("# follow=uses"),
        "subdir raise must also decline, got:\n{sub_patch}"
    );
    assert!(
        sub_patch.contains("diff --git src/a.rs"),
        "got:\n{sub_patch}"
    );

    // Back on the pre tree the raised patch applies to exactly the two
    // native files; c.rs keeps Old (no silent beyond-diff rewrite).
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
    let c = std::fs::read_to_string(dir.path().join("src/c.rs")).unwrap();
    assert!(c.contains("Old"), "c.rs must keep Old, got:\n{c}");
    assert!(!c.contains("New"), "c.rs must not gain New, got:\n{c}");
    let a = std::fs::read_to_string(dir.path().join("src/a.rs")).unwrap();
    assert!(a.contains("pub struct New;"), "got:\n{a}");
}

#[test]
fn no_off_diff_use_raises_follow_control() {
    let dir = init_repo();
    seed_three_file_repo(dir.path());
    // Control: c.rs leaves the workspace, so the site set is complete.
    git(dir.path(), &["rm", "-q", "src/c.rs"]);
    git(
        dir.path(),
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "drop c",
        ],
    );
    rename_change(dir.path());

    let patch = diffrs(dir.path(), &[]);
    assert!(
        patch.contains("# follow=uses"),
        "without off-diff uses the follow intent must fire, got:\n{patch}"
    );
}
