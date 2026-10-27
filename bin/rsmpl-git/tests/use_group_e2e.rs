//! T183: use-path group-member matching (SPEC 14.9).
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

mod common;
use common::*;

#[test]
fn group_member_prefix_rewrites_text() {
    // T183 dogfood probe: a `mod old` → `mod new` rename orphaning a
    // grouped member import — one hunk moves it back, siblings preserved,
    // `x::old` (not the prefix) untouched.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "use crate::{old, other as o};\nuse crate::old::Thing;\nuse crate::{old::Deep, x::old};\nfn m() {}\n",
    );
    let patch =
        "diff --git a.rs\n@@ -1,4 @@ rule=u\n# allow=many\n- use crate::old\n+ use crate::new\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "a.rs"),
        "use crate::{new, other as o};\nuse crate::new::Thing;\nuse crate::{new::Deep, x::old};\nfn m() {}\n"
    );
}

#[test]
fn group_member_prefix_rewrites_json() {
    // T183 JSON pin: two member edits report `matches:2` with per-rule
    // spans.
    let dir = init_repo();
    write(
        dir.path(),
        "a.rs",
        "use crate::{old, other};\nuse crate::{old::T, x};\n",
    );
    let patch =
        "diff --git a.rs\n@@ -1,2 @@ rule=u\n# allow=many\n- use crate::old\n+ use crate::new\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("\"matches\":2"), "got: {stdout}");
    assert!(stdout.contains("\"check\":true"), "got: {stdout}");
}
