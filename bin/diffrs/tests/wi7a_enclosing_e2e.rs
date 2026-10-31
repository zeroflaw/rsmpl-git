//! WI-7a e2e: enclosing-macro interior sweep (changed lines inside one
//! shared macro invocation compress to a single concrete token-tree
//! hunk) plus the authored token-tree per-arg hole round-trip through
//! the same shape. Tiny templates stay ordinary (T375 floor).

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

fn diffrs(dir: &Path) -> String {
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// `diffrs` the worktree change, then prove the semantic patch
/// real-applies byte-identical: stash, apply for real, compare bytes.
fn diffrs_and_reapply(dir: &Path, after: &str) -> String {
    let patch = diffrs(dir);
    git(dir, &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let back = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert_eq!(back, after, "real-apply diverged");
    patch
}

fn three_site_before() -> String {
    "fn render(path: &str) -> String {\n    let a = format!(\n        \"{header}\\nrest\\n\",\n        header = super::old_header(path)\n    );\n    let b = format!(\n        \"{header}\\nrest\\n\",\n        header = super::old_header(path)\n    );\n    let c = format!(\n        \"{header}\\nrest\\n\",\n        header = super::old_header(path)\n    );\n    format!(\"{a}{b}{c}\")\n}\n"
        .to_string()
}

fn three_site_after() -> String {
    three_site_before().replace("super::old_header(path)", "super::new_header(path)")
}

// WI-7a: three identical interior lines inside one shared `format!`
// compress to one enclosing sweep and real-apply byte-identical.
#[test]
fn enclosing_three_identical_sites_compress_and_reapply() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", &three_site_before());
    commit_all(dir.path());
    let after = three_site_after();
    write(dir.path(), "src/lib.rs", &after);

    let patch = diffrs_and_reapply(dir.path(), &after);
    assert!(patch.contains("# sweep\n"), "got: {patch}");
    assert!(patch.contains("-format!(\n"), "got: {patch}");
    assert!(
        patch.contains("header = super::new_header(path)"),
        "got: {patch}"
    );
    assert!(!patch.contains("@@ -4 +4 @@"), "got: {patch}");
}

// WI-7a: two short interior lines cost less as ordinary hunks — the
// T375 floor keeps them ordinary (no enclosing sweep emitted).
#[test]
fn enclosing_two_tiny_sites_stay_ordinary() {
    let dir = init_repo();
    let before =
        "fn m() {\n    let a = wrap!(\n        \"x\",\n        key = old_val\n    );\n    let b = wrap!(\n        \"x\",\n        key = old_val\n    );\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = before.replace("key = old_val", "key = new_val");
    write(dir.path(), "src/lib.rs", &after);

    let patch = diffrs(dir.path());
    assert!(!patch.contains("-wrap!("), "got: {patch}");
    // Still a faithful diff: real-apply reproduces the post-image.
    git(dir.path(), &["stash", "-q"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&patch)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(applied.status.code(), Some(0));
    let back = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert_eq!(back, after);
}
