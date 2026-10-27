//! Reference-wrapped call e2e: a leading `&` is a general-expression
//! reference wrapper, not part of a call receiver (T252).
//!
//! Before the T252 router fix, `- &E.join("")` classified as a method call
//! with composite receiver `&E` (verbatim-compare per SPEC 18.5) and never
//! matched; now it bypasses call routing and matches structurally.

mod common;
use common::*;

#[test]
fn amp_wrapped_call_metavar_applies() {
    // T252: the exact failing shape — placeholder receiver under `&`.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f(v: &std::path::PathBuf) {\n    let x = &v.join(\"\");\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# allow=many\n# expr E\n- &E.join(\"\")\n+ E\n";
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
    assert!(read(dir.path(), "src/lib.rs").contains("let x = v;"));
}

#[test]
fn amp_wrapped_call_concrete_applies() {
    // T252 phase 2 shape: concrete reference-wrapped call.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn f(v: &std::path::PathBuf) {\n    let x = &v.join(\"\");\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# allow=many\n- &v.join(\"\")\n+ v\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(read(dir.path(), "src/lib.rs").contains("let x = v;"));
}
