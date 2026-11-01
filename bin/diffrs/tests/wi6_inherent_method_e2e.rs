//! WI-6 e2e: inherent-method rename + anchored calls compress to one
//! follow hunk; unproven call sets stay ordinary. Real-apply is
//! byte-identical (T361 doctrine: `--check` alone never proves a sweep).

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

fn before() -> String {
    "struct Service;\nimpl Service {\n    fn run(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32 {\n        0\n    }\n}\nfn f(s: Service) {\n    s.run();\n}\nfn g(s: Service) {\n    s.run();\n}\n"
        .to_string()
}

fn renamed(before: &str) -> String {
    before
        .replace("fn run(", "fn execute(")
        .replace("s.run()", "s.execute()")
}

// WI-6: inherent def + two param-anchored calls compress to the single
// follow hunk; real-apply reproduces the post-tree byte-identical.
#[test]
fn inherent_method_rename_compresses_to_single_follow() {
    let dir = init_repo();
    let base = before();
    write(dir.path(), "src/lib.rs", &base);
    commit_all(dir.path());
    let after = renamed(&base);
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("  impl Service {"), "got: {patch}");
    assert!(patch.contains("# follow=uses"), "got: {patch}");
    assert!(
        patch.contains("-     V Q fn run G (...) W {"),
        "got: {patch}"
    );
    assert_eq!(
        patch.matches("rule=").count(),
        1,
        "single follow hunk, got: {patch}"
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
    assert_eq!(
        v["checked"]["semantics"].as_str().unwrap(),
        "verified",
        "single follow hunk must verify, got: {v}"
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
    let got = std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap();
    assert_eq!(got, after, "real-apply must reproduce the post-tree");
}

// WI-6: an unanchored same-name call fails the scratch probe, so the
// class stays ordinary — never a def-only partial follow.
#[test]
fn inherent_method_unproven_call_stays_ordinary() {
    let dir = init_repo();
    let base = "struct Service;\nimpl Service {\n    fn run(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32 {\n        0\n    }\n}\nstruct Other;\nfn f(o: Other) {\n    o.run();\n}\n";
    write(dir.path(), "src/lib.rs", base);
    commit_all(dir.path());
    // The def renames AND the unproven-receiver call renames: the call
    // fails the anchor probe, so no follow may claim the class.
    let after = base
        .replace("fn run(", "fn execute(")
        .replace("o.run()", "o.execute()");
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("# follow=uses"),
        "unproven receivers never ride the follow, got: {patch}"
    );
}
