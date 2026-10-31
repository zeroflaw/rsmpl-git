//! WI-4b e2e: trait+impl method rename compresses to one hunk per
//! enclosing block (verbatim headers — the engine filter matches only
//! the named block); the free-fn distractor stays untouched. All hunks
//! share the rule name; matches sum to 3, and real-apply is
//! byte-identical.
//!
//! Split out (like `wi4a_method_slot_e2e.rs`) to hold the 2000-line
//! budget (helpers duplicated, not shared — T246 precedent).

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

fn long_before() -> String {
    "trait Greeter {\n    fn greet(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String;\n}\n\nstruct A;\nimpl Greeter for A {\n    fn greet(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String {\n        \"a\".to_string()\n    }\n}\n\nstruct B;\nimpl Greeter for B {\n    fn greet(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String {\n        \"b\".to_string()\n    }\n}\n\nfn greet() -> i32 {\n    0\n}\n"
        .to_string()
}

fn renamed(before: &str) -> String {
    before
        .lines()
        .map(|l| {
            if l.contains("fn greet(") && l.starts_with([' ', '\t']) {
                l.replace("fn greet", "fn hello")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

// WI-4b: trait declaration + two impl definitions (one intent)
// compress to one hunk per enclosing block sharing the rule name;
// the free-fn distractor stays untouched.
#[test]
fn trait_method_rename_compresses_to_one_hunk_per_block() {
    let dir = init_repo();
    let before = long_before();
    write(dir.path(), "src/lib.rs", &before);
    commit_all(dir.path());
    let after = renamed(&before);
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(patch.contains("  trait Greeter {"), "got: {patch}");
    assert_eq!(
        patch.matches("  impl Greeter for ").count(),
        2,
        "got: {patch}"
    );
    assert_eq!(
        patch.matches("rule=rename_greet_to_hello").count(),
        3,
        "all hunks share the descriptive rule, got: {patch}"
    );
    assert!(
        patch.contains("-     V Q fn greet G (...) W ;"),
        "got: {patch}"
    );
    assert!(
        patch.contains("-     V Q fn greet G (...) W {"),
        "got: {patch}"
    );
    assert!(!patch.contains("fn greet() -> i32"), "got: {patch}");

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
    assert_eq!(total, 3, "got: {v}");

    // Real apply is byte-identical to the recorded post-tree.
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

// WI-4b: the same intent across two files groups under one shared
// rule name (definition-first); real-apply reproduces both files.
#[test]
fn trait_method_cross_file_groups_under_shared_rule() {
    let dir = init_repo();
    let mk = |ty: &str| {
        format!(
            "trait {ty} {{\n    fn old(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32;\n}}\nstruct S;\nimpl S {{\n    fn old(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32 {{\n        0\n    }}\n}}\n"
        )
    };
    let (ba, bb) = (mk("TA"), mk("TB"));
    write(dir.path(), "src/a.rs", &ba);
    write(dir.path(), "src/b.rs", &bb);
    commit_all(dir.path());
    write(dir.path(), "src/a.rs", &ba.replace("fn old", "fn new"));
    write(dir.path(), "src/b.rs", &bb.replace("fn old", "fn new"));

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        patch.matches("rule=rename_old_to_new").count(),
        4,
        "both files share the descriptive rule, got: {patch}"
    );
    assert!(
        !patch.contains("rule=r1") && !patch.contains("rule=r2"),
        "no positional fallback, got: {patch}"
    );

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
        std::fs::read_to_string(dir.path().join("src/a.rs")).unwrap(),
        ba.replace("fn old", "fn new")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/b.rs")).unwrap(),
        bb.replace("fn old", "fn new")
    );
}
#[test]
fn trait_decl_only_stays_ordinary() {
    // WI-4b: trait-side-only renames stay ordinary (no impl side
    // shares the intent) — pinned by design.
    let dir = init_repo();
    let before = "trait T {\n    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32;\n}\n\ntrait U {\n    fn old_name(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> i32;\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = before.replace("fn old_name", "fn new_name");
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("V Q fn"),
        "decl-only must stay ordinary, got: {patch}"
    );
}

// WI-4b: a divergent suffix (one impl evolves its signature too)
// vetoes the whole class to ordinary.
#[test]
fn trait_method_divergent_suffix_stays_ordinary() {
    let dir = init_repo();
    let before = long_before();
    write(dir.path(), "src/lib.rs", &before);
    commit_all(dir.path());
    let after = before
        .replace("fn greet(&self, first_argument", "fn hello(&self, first_argument")
        .replace(
            "impl Greeter for B {\n    fn hello(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument) -> String {",
            "impl Greeter for B {\n    fn hello(&self, first_argument: FirstLongTypeArgument, second_argument: SecondLongTypeArgument, third_argument: ThirdLongTypeArgument) -> String {",
        );
    write(dir.path(), "src/lib.rs", &after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("V Q fn"),
        "divergent class must stay ordinary, got: {patch}"
    );
}
