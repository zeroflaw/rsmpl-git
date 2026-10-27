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
    assert!(patch.contains("diff --git a/src/lib.rs b/src/lib.rs"));
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
    assert!(patch.contains("diff --git a/src/new.rs b/src/new.rs"));
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
    assert!(patch.contains("rule=rename_foo_to_bar"), "got: {patch}");
    assert!(patch.contains("- fn foo(...) {"), "got: {patch}");
    assert!(!patch.contains("-fn foo"), "got: {patch}");

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
    assert!(patch.contains("# allow=many"), "got: {patch}");
    assert!(patch.contains("- foo(E0)"), "got: {patch}");
    assert_eq!(
        patch.matches("@@ -1,1 @@").count(),
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
    assert!(
        patch.contains("rule=calls_old_function_to_new_function\n"),
        "got: {patch}"
    );
    assert!(
        patch.contains("rule=calls_old_function_to_new_function_2\n"),
        "got: {patch}"
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
    assert!(patch.contains("# visibility V"), "got: {patch}");
    assert!(patch.contains("- V Q fn foo G (...) W {"), "got: {patch}");
    assert!(patch.contains("+ V Q fn bar G (...) W {"), "got: {patch}");
    assert_eq!(patch.matches("@@ -1,1 @@").count(), 1, "got: {patch}");

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
            if l.contains("fn old_name(&self)") {
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
    assert!(patch.contains("# expr Receiver"), "got: {patch}");
    assert!(patch.contains("- Receiver.foo(E0)"), "got: {patch}");
    assert!(patch.contains("+ Receiver.bar(E0)"), "got: {patch}");
    assert_eq!(patch.matches("@@ -1,1 @@").count(), 1, "got: {patch}");

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
    assert!(patch.contains("# expr Receiver"), "got: {patch}");
    assert!(patch.contains("- Receiver.old_method(E0)"), "got: {patch}");
    assert!(
        patch.contains("+ new_function(Receiver, E0)"),
        "got: {patch}"
    );
    assert_eq!(patch.matches("@@ -1,1 @@").count(), 1, "got: {patch}");

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
    assert!(patch.contains("- foo(E0)"), "got: {patch}");
    assert!(patch.contains("+ bar(E0)"), "got: {patch}");
    assert_eq!(patch.matches("@@ -1,1 @@").count(), 1, "got: {patch}");
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
    assert!(
        patch.contains("diff --git a/new.txt b/new.txt"),
        "got: {patch}"
    );
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
    assert!(
        patch.contains("diff --git a/link.txt b/link.txt"),
        "got: {patch}"
    );
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
    assert!(
        patch.contains("diff --git a/dangle.txt b/dangle.txt"),
        "got: {patch}"
    );
    assert!(patch.contains("+missing-target"), "got: {patch}");
    assert!(
        patch.contains("diff --git a/dirlink b/dirlink"),
        "got: {patch}"
    );
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
    assert!(patch.contains("rule=use_old_to_new"), "got: {patch}");
    assert!(patch.contains("- use crate::old\n"), "got: {patch}");
    assert!(patch.contains("+ use crate::new\n"), "got: {patch}");
    assert_eq!(
        patch.matches("@@ -1,1 @@").count(),
        1,
        "one hunk for five imports, got: {patch}"
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
    assert!(
        patch.contains("diff --git a/empty.txt b/empty.txt"),
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
    assert!(patch.contains("diff --git a/a.txt b/a.txt"), "got: {patch}");
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
    assert!(
        patch.contains("diff --git a/my file.txt b/my file.txt"),
        "got: {patch}"
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
