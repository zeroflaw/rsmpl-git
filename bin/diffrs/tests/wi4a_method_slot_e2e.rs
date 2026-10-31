//! WI-4a e2e: heterogeneous method rename compresses to one hunk per
//! enclosing `impl` block (verbatim headers — the engine filter
//! matches only the named block); the free-fn distractor stays
//! untouched. All four hunks share the rule name; matches sum to 4.
//!
//! Split out of `diffrs_e2e.rs` to hold that file's 2000-line budget
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

// WI-4a: heterogeneous method rename compresses to one hunk per
// enclosing `impl` block (verbatim headers); matches sum to 4.
#[test]
fn method_slot_sweep_compresses_to_one_hunk_per_block() {
    let dir = init_repo();
    // Long heterogeneous headers: each per-block hunk must clear its
    // own T375 guard (short methods stay ordinary — one shared
    // advisory hunk can no longer pool them).
    let before = "struct A;\nstruct B<T>(T);\nstruct C;\nstruct D;\n\nimpl A {\n    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32 {\n        1\n    }\n}\n\nimpl B<i32> {\n    pub fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32 {\n        2\n    }\n}\n\nimpl C {\n    pub async fn old_name<T>(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> T where T: Clone {\n        x.clone()\n    }\n}\n\nimpl crate::T for D {\n    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument, third_argument: ThirdLongTypeArgument) -> i32 {\n        4\n    }\n}\n\nfn old_name() -> i32 {\n    0\n}\n";
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
    assert_eq!(patch.matches("  impl ").count(), 4, "got: {patch}");
    assert!(!patch.contains("impl Svc"), "got: {patch}");
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
    let total: i64 = v["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["matches"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 4, "got: {v}");
}
