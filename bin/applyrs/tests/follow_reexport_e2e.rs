//! WI-7b: one-hop `pub use` re-export following for `# follow=uses`.
//!
//! A `pub use crate::a::Old` re-export that names the definition module is
//! a proven hop: the re-export line plus uses spelled through the
//! re-exporter rewrite in the same hunk. A second hop (re-export through
//! another re-exporter) or an off-target re-export (`pub use other::Old`)
//! vetoes loud without `# fallback=syntax`, or skips all use rewriting
//! with per-site `follow_skipped` notes under it.

mod common;
use common::*;

const HOP_PATCH: &str =
    "diff --git src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

#[test]
fn one_hop_reexport_rewrites_reexport_and_through_uses() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(dir.path(), "src/b.rs", "pub use crate::a::Old;\n");
    write(
        dir.path(),
        "src/c.rs",
        "use crate::b::Old;\nfn f(x: Old) -> Old {\n    let y = b::Old;\n    y\n}\n",
    );
    let out = run_stdin(
        dir.path(),
        HOP_PATCH,
        &["--format", "json", "--diff-max-lines", "0"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/a.rs"), "pub struct New;\n");
    assert_eq!(read(dir.path(), "src/b.rs"), "pub use crate::a::New;\n");
    assert_eq!(
        read(dir.path(), "src/c.rs"),
        "use crate::b::New;\nfn f(x: New) -> New {\n    let y = b::New;\n    y\n}\n"
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert!(files.contains(&"src/a.rs"), "got: {v}");
    assert!(files.contains(&"src/b.rs"), "got: {v}");
    assert!(files.contains(&"src/c.rs"), "got: {v}");
}

#[test]
fn second_hop_reexport_vetoes_without_fallback() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(dir.path(), "src/b.rs", "pub use crate::a::Old;\n");
    write(dir.path(), "src/c.rs", "pub use crate::b::Old;\n");
    let out = run_stdin(dir.path(), HOP_PATCH, &["--check"]);
    assert!(!out.status.success(), "second-hop veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("second"), "got: {stderr}");
}

#[test]
fn second_hop_reexport_skips_with_fallback_and_reports() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(dir.path(), "src/b.rs", "pub use crate::a::Old;\n");
    write(dir.path(), "src/c.rs", "pub use crate::b::Old;\n");
    let patch = HOP_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let before_b = read(dir.path(), "src/b.rs");
    let before_c = read(dir.path(), "src/c.rs");
    let out = run_stdin(
        dir.path(),
        &patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Check mode plans only: the definition entry still carries its rename
    // while every use site (incl. the re-exports) is listed untouched.
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let skipped = v["rules"][0]["follow_skipped"]
        .as_str()
        .expect("follow_skipped present");
    assert!(skipped.contains("src/c.rs"), "got: {v}");
    assert_eq!(read(dir.path(), "src/b.rs"), before_b);
    assert_eq!(read(dir.path(), "src/c.rs"), before_c);
}

#[test]
fn off_target_reexport_vetoes_without_fallback() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "pub use other::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
    );
    let out = run_stdin(dir.path(), HOP_PATCH, &["--check"]);
    assert!(!out.status.success(), "off-target veto expected");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("RSMPL_SYMBOL_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("does not name"), "got: {stderr}");
}
