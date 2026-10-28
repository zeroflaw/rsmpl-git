//! T431: a trailing blank context line before a `@@` header (or at EOF)
//! belongs to the ordinary hunk body. Raw `git diff -U3` emits ` ` as
//! the last context line; before the fix the parser dropped it (T212
//! lookahead only covers the leading side) and T367 failed with a
//! spurious count mismatch.

mod common;
use common::*;

fn fixture_pre() -> String {
    // Blank line after L4 becomes the trailing U3 context of hunk 1;
    // L18 is far enough for a second hunk.
    let mut s = String::new();
    for i in 1..=22 {
        s.push_str(&format!("L{i}\n"));
        if i == 4 {
            s.push('\n');
        }
    }
    s
}

#[test]
fn trailing_blank_before_header_applies() {
    let dir = init_repo();
    write(dir.path(), "f.txt", &fixture_pre());
    let patch = "diff --git f.txt\n@@ -1,5 +1,5 @@\n L1\n-L2\n+L2-MOD\n L3\n L4\n \n@@ -16,7 +16,7 @@\n L15\n L16\n L17\n-L18\n+L18-MOD\n L19\n L20\n L21\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let got = read(dir.path(), "f.txt");
    assert!(got.contains("L2-MOD\n"), "got: {got}");
    assert!(got.contains("L18-MOD\n"), "got: {got}");
    assert!(got.contains("L4\n\n"), "blank context kept, got: {got}");
}

#[test]
fn trailing_blank_at_eof_applies() {
    let dir = init_repo();
    write(dir.path(), "f.txt", "L1\nL2\nL3\nL4\n\nL6\nL7\n");
    let patch = "diff --git f.txt\n@@ -1,5 +1,5 @@\n L1\n-L2\n+L2-MOD\n L3\n L4\n \n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "f.txt"), "L1\nL2-MOD\nL3\nL4\n\nL6\nL7\n");
}
