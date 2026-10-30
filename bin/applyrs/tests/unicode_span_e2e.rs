//! History-sweep self-conversion gap: span columns are Unicode scalars.
//!
//! proc-macro2 fallback spans count columns in chars, not bytes
//! (SPEC 31.3). `LineIndex::offset` used to add the char column as a
//! byte offset, so a 3-byte em-dash before a span end shifted the
//! computed end two bytes short, leaving a dangling `";` —
//! `RSMPL_PARSE_CONFLICT ... cannot parse string into token stream`
//! on the raised `no_match_verbatim_e2e.rs` hunk of commit 095ca2d
//! (`history_sweep.py` 496/497). A 2-byte char left a stray `;` that
//! still parsed, which is why only 3+-byte text failed.
//!
//! Pins: single let rewrite plus the two-site `# allow=many` shape,
//! both with `—` and an inner `;` inside the string literal.

mod common;
use common::*;

const BEFORE_ONE: &str =
    "fn m() {\n    let reason = \"no match found \u{2014} nearest source at line 2: 'let x = 1;'\";\n}\n";
const AFTER_ONE: &str =
    "fn m() {\n    let reason = \"no match found \u{2014} nearest source at line 3: 'foo(1);'\";\n}\n";
const PATCH_ONE: &str = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n# allow=many\n- let reason = \"no match found \u{2014} nearest source at line 2: 'let x = 1;'\";\n+ let reason = \"no match found \u{2014} nearest source at line 3: 'foo(1);'\";\n";

const BEFORE_TWO: &str = "fn m() {\n    let reason = \"x \u{2014} y; z\";\n    let other = 1;\n    let reason = \"x \u{2014} y; z\";\n}\n";
const AFTER_TWO: &str = "fn m() {\n    let reason = \"x \u{2014} w; z\";\n    let other = 1;\n    let reason = \"x \u{2014} w; z\";\n}\n";
const PATCH_TWO: &str = "diff --git src/lib.rs\n@@ -2,3 @@ rule=r1\n# allow=many\n- let reason = \"x \u{2014} y; z\";\n+ let reason = \"x \u{2014} w; z\";\n";

#[test]
fn non_ascii_let_rewrite_applies() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE_ONE);
    let out = run_stdin(dir.path(), PATCH_ONE, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), AFTER_ONE);
}

#[test]
fn non_ascii_let_sweep_rewrites_both_sites() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE_TWO);
    let out = run_stdin(dir.path(), PATCH_TWO, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), AFTER_TWO);
}

#[test]
fn non_ascii_check_leaves_worktree_untouched() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", BEFORE_TWO);
    let out = run_stdin(dir.path(), PATCH_TWO, &["--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), BEFORE_TWO, "untouched");
}
