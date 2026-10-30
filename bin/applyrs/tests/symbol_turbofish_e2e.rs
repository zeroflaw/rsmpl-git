//! T199: `# symbol` seeds compose with explicit turbofish (`Old::<u8>(E)`
//! matches `crate::old_api::<u8>(1)` via sole-symbol expansion to the
//! seeded path — SPEC 19.3 + 18.6 R3).
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

use std::fs;
mod common;
use common::*;

const PATCH: &str = "diff --git src/a.rs\n@@ -1,1 @@ rule=symt\n# scope=file\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old::<u8>(E)\n+ New::<u8>(E)\n";

const SRC: &str = "fn f() {\n    crate::old_api::<u8>(1);\n    crate::old_api(2);\n}\n";

#[test]
fn symbol_turbofish_applies_byte_exact() {
    // T199: the symbol+turbofish site rewrites; the bare site is
    // untouched (T177 parity: an explicit-turbofish minus matches only
    // turbofish sites).
    let dir = init_repo();
    write(dir.path(), "src/a.rs", SRC);
    let check = run_stdin(dir.path(), PATCH, &["--check"]);
    assert!(
        check.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&check.stderr)
    );
    let out = run_stdin(dir.path(), PATCH, &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = fs::read_to_string(dir.path().join("src/a.rs")).unwrap();
    assert!(text.contains("crate::new_api::<u8>(1);"), "got: {text}");
    assert!(text.contains("crate::old_api(2);"), "got: {text}");
}

#[test]
fn symbol_turbofish_json_spans() {
    // T199: JSON report carries the match count + spans (T27).
    let dir = init_repo();
    write(dir.path(), "src/a.rs", SRC);
    let out = run_stdin(dir.path(), PATCH, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": "verified", "build": "skipped"})
    );
    assert_eq!(v["rules"][0]["matches"], serde_json::json!(1));
    assert_eq!(v["rules"][0]["spans"], serde_json::json!([[2, 2]]));
}
