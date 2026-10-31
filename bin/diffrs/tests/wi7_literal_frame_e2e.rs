//! WI-7 e2e: literal-frame raising compresses lone-literal deltas in
//! hetero frames (`"old"` → `"new"`) that the engine applies but no
//! detector raised; twins veto, single sites stay ordinary.
//!
//! Binary-level proofs via the real `diffrs`/`applyrs` binaries
//! (helpers duplicated, not shared — T246 precedent).

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

/// `diffrs` the worktree change, then prove the semantic patch
/// real-applies byte-identical: stash, apply for real, compare bytes.
fn diffrs_and_reapply(dir: &Path, after: &str) -> String {
    let out = run(dir, &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
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

// WI-7: the F5 string probe compresses and real-applies byte-identical.
#[test]
fn literal_string_frames_compress_and_reapply() {
    let dir = init_repo();
    let before = "fn a() -> &'static str {\n    return \"old\";\n}\nfn b(x: i32) {\n    log(\"old\", x);\n}\nfn c() {\n    let m = \"old\";\n    send(m);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a() -> &'static str {\n    return \"new\";\n}\nfn b(x: i32) {\n    log(\"new\", x);\n}\nfn c() {\n    let m = \"new\";\n    send(m);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- \"old\"\n"), "got: {patch}");
    assert!(patch.contains("+ \"new\"\n"), "got: {patch}");
    assert!(patch.contains("# allow=many\n"), "got: {patch}");
}

// WI-7: a numeric probe fires across hetero frames.
#[test]
fn literal_numeric_frames_compress_and_reapply() {
    let dir = init_repo();
    let before =
        "fn a() -> i32 {\n    let n = 10;\n    n\n}\nfn b() -> i32 {\n    let m = 10 + 1;\n    m\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn a() -> i32 {\n    let n = 20;\n    n\n}\nfn b() -> i32 {\n    let m = 20 + 1;\n    m\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- 10\n"), "got: {patch}");
    assert!(patch.contains("+ 20\n"), "got: {patch}");
}

// WI-7: an untouched twin matching the minus template vetoes to ordinary.
#[test]
fn literal_untouched_twin_vetoes() {
    let dir = init_repo();
    let before = "fn a() -> &'static str {\n    return \"old\";\n}\nfn b(x: i32) {\n    log(\"old\", x);\n}\nfn untouched(y: i32) {\n    log(\"old\", y);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn a() -> &'static str {\n    return \"new\";\n}\nfn b(x: i32) {\n    log(\"new\", x);\n}\nfn untouched(y: i32) {\n    log(\"old\", y);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("- \"old\"\n"),
        "twin must veto, got: {patch}"
    );
}
