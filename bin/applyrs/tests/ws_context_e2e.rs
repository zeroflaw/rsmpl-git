//! T388: whitespace-only context lines keep content byte-exact.
//! A native `git diff -U2` patch whose context carries `"    "`
//! (`" "` prefix + 3-space file line) used to fail `applyrs --check`
//! (`context mismatch ... expected ```) while `git apply` succeeded.
//! `--check` + real apply are byte-exact (trailing spaces kept).

mod common;
use common::*;

#[test]
fn ws_only_context_checks_and_applies() {
    // T388: `f.txt` holds a 3-space line; the U2 patch carries it as
    // a 4-space context line. `--check` must pass and apply must keep
    // every byte, trailing spaces included.
    let dir = init_repo();
    write(dir.path(), "f.txt", "line1\n   \nline3\nline4\nline5\n");
    let patch = "diff --git f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,5 +1,5 @@\n line1\n    \n-line3\n+line3b\n line4\n line5\n";
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
    assert_eq!(
        read(dir.path(), "f.txt"),
        "line1\n   \nline3b\nline4\nline5\n"
    );
}
