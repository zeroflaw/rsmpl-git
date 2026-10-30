//! T297: engine-verbatim attrs match-time + trait-vis + path-ellipsis pins.
//!
//! Three clusters the engine emits but SPEC stated only in prose:
//! (a) the `RSMPL_MATCH_CONFLICT` attribute match-time family (`function`/
//! `item`/`statement has attributes but ... requires none`,
//! `attached attributes do not match ...` + suggestion; rule + file, null
//! range, suggestion set only on some paths — SPEC 14.6); (b) the
//! `RSMPL_PATTERN_CONFLICT` trait-method visibility gate (SPEC 13.4);
//! (c) the `RSMPL_PATTERN_CONFLICT` `+`-path `...` rejection, distinct
//! from the Section 16.3 argument-`...` replay fence (SPEC 16.5).
//! Every conflict path leaves the worktree untouched. A closing rot-gate
//! pins the shared spellings across matcher sites and SPEC.

mod common;
use common::*;

const FN_ABSENT: &str = "function has attributes but `# attrs=absent` requires none";
const ITEM_ABSENT: &str = "item has attributes but `# attrs=absent` requires none";
const STMT_ABSENT: &str = "statement has attributes but `# attrs=absent` requires none";
const STMT_EXPLICIT: &str = "statement has attributes but `# attrs=explicit` requires none";
const ATTACHED: &str = "attached attributes do not match explicitly written pattern attributes";
const ATTACHED_SUGGESTION: &str = "use `# attrs=preserve` (default) to ignore attributes";
const TRAIT_VIS: &str =
    "trait methods cannot declare visibility: drop `pub` (or `pub(...)`) on the `+` side";
const PATH_ELLIPSIS: &str =
    "`...` in `+` path has nothing to replay (plus side must be concrete, SPEC 16.5)";
const PATH_SUGGESTION: &str = "write a concrete `+` path (e.g. `+ crate::new(E)`) — path `...` replays nothing, only argument `...` replays";

fn check_text(
    patch: &str,
    file: &str,
    content: &str,
    code: &str,
    reason: &str,
    want_suggestion: Option<&str>,
) {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains(code), "got: {stderr}");
    assert!(stderr.contains("rule: r"), "got: {stderr}");
    assert!(stderr.contains("file: src/lib.rs"), "got: {stderr}");
    assert!(stderr.contains(reason), "got: {stderr}");
    assert!(!stderr.contains("old_range:"), "got: {stderr}");
    match want_suggestion {
        Some(s) => assert!(stderr.contains(s), "got: {stderr}"),
        None => assert!(!stderr.contains("suggestion:"), "got: {stderr}"),
    }
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

fn check_json(
    patch: &str,
    file: &str,
    content: &str,
    code: &str,
    reason: &str,
    want_suggestion: Option<&str>,
) {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(v["error"]["code"], serde_json::json!(code));
    assert_eq!(v["error"]["rule"], serde_json::json!("r"));
    assert_eq!(v["error"]["file"], serde_json::json!("src/lib.rs"));
    assert_eq!(v["error"]["reason"], serde_json::json!(reason));
    assert!(v["error"]["range"].is_null(), "got: {v}");
    match want_suggestion {
        Some(s) => assert_eq!(v["error"]["suggestion"], serde_json::json!(s)),
        None => assert!(v["error"]["suggestion"].is_null(), "got: {v}"),
    }
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
}

#[test]
fn fn_attrs_absent_rejected_verbatim() {
    let content = "/// doc\nfn foo() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=r\n# attrs=absent\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        FN_ABSENT,
        None,
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        FN_ABSENT,
        None,
    );
}

#[test]
fn item_attrs_absent_rejected_verbatim() {
    let content = "/// doc\nstruct Old;\n";
    let patch =
        "diff --git src/lib.rs\n@@ -1,2 @@ rule=r\n# attrs=absent\n- struct Old\n+ struct New\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        ITEM_ABSENT,
        None,
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        ITEM_ABSENT,
        None,
    );
}

#[test]
fn use_attrs_absent_reports_item_wording() {
    // `use` items route through the item matcher, so the reason says `item`.
    let content = "#[allow(x)]\nuse foo::Bar;\nfn a() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,2 @@ rule=r\n# attrs=absent\n- use foo::Bar\n+ use foo::Baz\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        ITEM_ABSENT,
        None,
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        ITEM_ABSENT,
        None,
    );
}

#[test]
fn stmt_attrs_absent_rejected_verbatim() {
    let content = "fn a() {\n    #[allow(x)]\n    let v = 1;\n    let _ = v;\n}\n";
    let patch = "diff --git src/lib.rs\n@@ -2,3 @@ rule=r\n# attrs=absent\n# expr E\n- let v = E;\n+ let w = E;\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        STMT_ABSENT,
        None,
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        STMT_ABSENT,
        None,
    );
}

#[test]
fn stmt_attrs_policy_echoes_explicit() {
    // The `statement` reason echoes the active policy name.
    let content = "fn a() {\n    #[allow(x)]\n    let v = 1;\n    let _ = v;\n}\n";
    let patch = "diff --git src/lib.rs\n@@ -2,3 @@ rule=r\n# attrs=explicit\n# expr E\n- let v = E;\n+ let w = E;\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        STMT_EXPLICIT,
        None,
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        STMT_EXPLICIT,
        None,
    );
}

#[test]
fn attached_mismatch_carries_preserve_suggestion() {
    // Written `#[...]` vs unattributed target under `explicit` hard-fails
    // (under default `preserve` a fn/item candidate silently skips instead).
    let content = "fn foo() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# attrs=explicit\n- #[inline]\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        ATTACHED,
        Some(ATTACHED_SUGGESTION),
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_MATCH_CONFLICT",
        ATTACHED,
        Some(ATTACHED_SUGGESTION),
    );
}

#[test]
fn trait_method_visibility_rejected_verbatim() {
    let content = "trait T {\n    fn m();\n}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r\n# lang=rust\n  trait T {\n-     fn m(...) {\n+     pub fn m2(...) {\n           ...\n       }\n  }\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_PATTERN_CONFLICT",
        TRAIT_VIS,
        None,
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_PATTERN_CONFLICT",
        TRAIT_VIS,
        None,
    );
}

#[test]
fn plus_path_ellipsis_rejected_verbatim() {
    // A `+`-path `...` has nothing to replay (only argument remainders
    // replay, SPEC 16.3) — distinct from the arg-`...` fence.
    let content = "fn a() {\n    let v = crate::foo(1);\n    let _ = v;\n}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# allow=many\n# expr E\n- crate::foo(E)\n+ crate::...::new(E)\n";
    check_text(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_PATTERN_CONFLICT",
        PATH_ELLIPSIS,
        Some(PATH_SUGGESTION),
    );
    check_json(
        patch,
        "src/lib.rs",
        content,
        "RSMPL_PATTERN_CONFLICT",
        PATH_ELLIPSIS,
        Some(PATH_SUGGESTION),
    );
}

#[test]
fn shared_spellings_rot_gate() {
    // The attached-mismatch wording is shared across the fn, item, and
    // general matchers; the absent-wordings and the two new fences must
    // appear in SPEC exactly as the engine emits them.
    let root = workspace_root();
    let attached_sites = [
        "crates/rsmpl_engine/src/matcher_fn.rs",
        "crates/rsmpl_engine/src/matcher_item.rs",
        "crates/rsmpl_engine/src/matcher_general_apply.rs",
    ];
    for rel in attached_sites {
        let text = std::fs::read_to_string(root.join(rel)).expect("engine source readable");
        assert!(
            text.contains(ATTACHED),
            "{rel} must carry the verbatim attached-mismatch reason"
        );
    }
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md readable");
    for needle in [
        FN_ABSENT,
        ITEM_ABSENT,
        "statement has attributes but",
        ATTACHED,
        TRAIT_VIS,
        PATH_ELLIPSIS,
    ] {
        assert!(
            spec.contains(needle),
            "SPEC.md must carry the verbatim fence: {needle}"
        );
    }
}
