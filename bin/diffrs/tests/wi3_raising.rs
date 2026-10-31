//! WI-3 raising pin: a 3-site typed destructuring `let` migration
//! compresses to one structural template hunk and real-applies
//! byte-identical (T361 doctrine: `--check` alone never proves it).

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
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const BASE: &str = "fn a(o: Option<i32>, p: Option<i32>, q: Option<i32>) {\n    let Some(v): Option<i32> = o;\n    let Some(w): Option<i32> = p;\n    let Some(z): Option<i32> = q;\n    let _ = (v, w, z);\n}\n";

fn migrated() -> String {
    BASE.replace(
        "let Some(v): Option<i32> = o;",
        "let Some(v): Option<i32> = o.unwrap_or(0);",
    )
    .replace(
        "let Some(w): Option<i32> = p;",
        "let Some(w): Option<i32> = p.unwrap_or(0);",
    )
    .replace(
        "let Some(z): Option<i32> = q;",
        "let Some(z): Option<i32> = q.unwrap_or(0);",
    )
}

#[test]
fn wi3_destructuring_let_migration_compresses_and_real_applies() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), BASE).unwrap();
    git(dir, &["init", "-q"]);
    git(
        dir,
        &[
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "add",
            "-A",
        ],
    );
    git(
        dir,
        &[
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    );
    let want = migrated();
    std::fs::write(dir.join("src/lib.rs"), &want).unwrap();

    // Native → semantic: one structural template hunk, not ordinary.
    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "diffrs failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let semantic = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        semantic.contains("- let Some(P0): Option<i32> = E0;"),
        "no structural minus template, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let Some(P0): Option<i32> = E0.unwrap_or(0);"),
        "no structural plus template, got: {semantic}"
    );
    assert!(semantic.contains("# allow=many"), "got: {semantic}");

    // Semantic → native for real: byte-identical post-tree (T361).
    git(dir, &["checkout", "-q", "--", "src/lib.rs"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&semantic)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "real apply failed: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let got = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert_eq!(got, want, "round-trip must be byte-identical");
}

const STRUCT_BASE: &str = "fn a(s: S, t: S, u: S) {\n    let S { f: v }: S = foo(s);\n    let S { f: w }: S = foo(t);\n    let S { f: z }: S = foo(u);\n    let _ = (v, w, z);\n}\n";

fn struct_migrated() -> String {
    STRUCT_BASE
        .replace(
            "let S { f: v }: S = foo(s);",
            "let S { f: v }: S = foo(s) + 1;",
        )
        .replace(
            "let S { f: w }: S = foo(t);",
            "let S { f: w }: S = foo(t) + 1;",
        )
        .replace(
            "let S { f: z }: S = foo(u);",
            "let S { f: z }: S = foo(u) + 1;",
        )
}

#[test]
fn wi3_struct_let_migration_compresses_and_real_applies() {
    // Struct patterns raise through the same middle tier; the bare
    // `S` ascription infers `# type S` and the struct path binds it
    // (a type-hole path is not an `Or`-style veto).
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), STRUCT_BASE).unwrap();
    git(dir, &["init", "-q"]);
    git(
        dir,
        &[
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "add",
            "-A",
        ],
    );
    git(
        dir,
        &[
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    );
    let want = struct_migrated();
    std::fs::write(dir.join("src/lib.rs"), &want).unwrap();

    let out = Command::new(binary())
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "diffrs failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let semantic = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        semantic.contains("- let S { f: P0 }: S = E0;"),
        "no structural minus template, got: {semantic}"
    );
    assert!(
        semantic.contains("+ let S { f: P0 }: S = E0 + 1;"),
        "no structural plus template, got: {semantic}"
    );

    git(dir, &["checkout", "-q", "--", "src/lib.rs"]);
    let applied = Command::new(sibling_applyrs())
        .arg("--patch")
        .arg(&semantic)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap();
    assert_eq!(
        applied.status.code(),
        Some(0),
        "real apply failed: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let got = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert_eq!(got, want, "round-trip must be byte-identical");
}
