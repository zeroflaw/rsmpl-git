#![allow(dead_code)]
//! T250: shared e2e helpers for all `tests/*_e2e.rs` binaries.
//!
//! Dedupes the `binary` / `init_repo` / `write` / `read` / `run` /
//! `run_stdin` / `run_raw` copies that accumulated across ~15 files (third
//! copy added by the R2 stdin move, T172 precedent). Each test file does:
//!
//! ```rust
//! mod common;
//! use common::*;
//! ```
//!
//! and keeps its existing call-sites unchanged. Semantics are the superset
//! of all former copies: `init_repo` is bare `git init` (no `mkdir src/`),
//! `write` creates parent dirs (so `src/lib.rs` writes still land).

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_applyrs"))
}

pub fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(st.success());
    dir
}

pub fn write(dir: &Path, rel: &str, text: &str) {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
}

pub fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

pub fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

pub fn run_stdin(dir: &Path, patch: &str, extra: &[&str]) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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

pub fn run_raw(args: &[&str]) -> std::process::Output {
    // T254: isolate off default `--repo .` so `--patch` runs archive
    // `.history/` into a throwaway tempdir (auto-deleted), never into
    // `bin/gitrs/.history/`. Clap-usage failures exit before any
    // history write, so the extra `--repo` is harmless there. Skip
    // injection when the caller already passes `--repo`.
    if args.contains(&"--repo") {
        return Command::new(binary()).args(args).output().unwrap();
    }
    let dir = tempfile::tempdir().unwrap();
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap()
}

pub fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <root>/bin/gitrs
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .unwrap()
}
