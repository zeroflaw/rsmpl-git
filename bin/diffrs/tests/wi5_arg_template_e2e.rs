//! WI-5 e2e: hetero call/method arg templates compress wrap/transform
//! migrations (`use_it(x)` → `use_it(x.clone())`) that the engine
//! applies but no detector raised; twins, drops, duplicates veto.
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

// WI-5: wrap migration compresses and real-applies byte-identical.
#[test]
fn call_wrap_clone_compresses_and_reapplies() {
    let dir = init_repo();
    let before = "fn m() {\n    use_it(alpha);\n    use_it(beta);\n    use_it(gamma);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn m() {\n    use_it(alpha.clone());\n    use_it(beta.clone());\n    use_it(gamma.clone());\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- use_it(E0)\n"), "got: {patch}");
    assert!(patch.contains("+ use_it(E0.clone())\n"), "got: {patch}");
    assert!(patch.contains("# allow=many\n"), "got: {patch}");
}

// WI-5: borrow migration compresses.
#[test]
fn call_borrow_compresses_and_reapplies() {
    let dir = init_repo();
    let before =
        "fn m() {\n    take(first_value);\n    take(second_value);\n    take(third_value);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn m() {\n    take(&first_value);\n    take(&second_value);\n    take(&third_value);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- take(E0)\n"), "got: {patch}");
    assert!(patch.contains("+ take(&E0)\n"), "got: {patch}");
}

// WI-5: permuted holes compress.
#[test]
fn call_permute_compresses_and_reapplies() {
    let dir = init_repo();
    let before =
        "fn m() {\n    combine(first_left, first_right);\n    combine(second_left, second_right);\n    combine(third_left, third_right);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn m() {\n    combine(first_right, first_left);\n    combine(second_right, second_left);\n    combine(third_right, third_left);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- combine(E0, E1)\n"), "got: {patch}");
    assert!(patch.contains("+ combine(E1, E0)\n"), "got: {patch}");
}

// WI-5: method sibling compresses with hetero receivers.
#[test]
fn method_wrap_compresses_and_reapplies() {
    let dir = init_repo();
    let before =
        "fn m() {\n    alpha.store(first_item);\n    beta.store(second_item);\n    gamma.store(third_item);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn m() {\n    alpha.store(first_item.clone());\n    beta.store(second_item.clone());\n    gamma.store(third_item.clone());\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- Receiver.store(E0)\n"), "got: {patch}");
    assert!(
        patch.contains("+ Receiver.store(E0.clone())\n"),
        "got: {patch}"
    );
}

// WI-5: callee delta plus arg wrapper compresses (disjoint from
// `call_sweep`: that detector needs identical args).
#[test]
fn callee_delta_with_wrapper_compresses() {
    let dir = init_repo();
    let before =
        "fn m() {\n    old_api(first_arg);\n    old_api(second_arg);\n    old_api(third_arg);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after =
        "fn m() {\n    new_api(first_arg.clone());\n    new_api(second_arg.clone());\n    new_api(third_arg.clone());\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let patch = diffrs_and_reapply(dir.path(), after);
    assert!(patch.contains("- old_api(E0)\n"), "got: {patch}");
    assert!(patch.contains("+ new_api(E0.clone())\n"), "got: {patch}");
}

// WI-5: an untouched twin vetoes (totality counts N+1).
#[test]
fn untouched_twin_stays_ordinary() {
    let dir = init_repo();
    let before = "fn m() {\n    use_it(alpha);\n    use_it(beta);\n    use_it(spare);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    // Only two of three sites migrate; the twin must veto the sweep.
    let after =
        "fn m() {\n    use_it(alpha.clone());\n    use_it(beta.clone());\n    use_it(spare);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!patch.contains("rule="), "twin must veto, got: {patch}");
}

// WI-5: hetero wrappers (different wrap per site) stay ordinary.
#[test]
fn hetero_wrapper_stays_ordinary() {
    let dir = init_repo();
    let before = "fn m() {\n    use_it(alpha);\n    use_it(beta);\n}\n";
    write(dir.path(), "src/lib.rs", before);
    commit_all(dir.path());
    let after = "fn m() {\n    use_it(alpha.clone());\n    use_it(&beta);\n}\n";
    write(dir.path(), "src/lib.rs", after);

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0));
    let patch = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !patch.contains("E0.clone()"),
        "hetero wrap must stay ordinary, got: {patch}"
    );
}
