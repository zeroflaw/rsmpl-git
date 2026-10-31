//! WI-3: repair completeness — no patch failure without a next action.
//!
//! Every error class reachable from patch input carries a machine
//! `repair`; the single allow-list entry is the unreadable-file
//! `RSMPL_IO_CONFLICT` (environmental: the suggestion already names the
//! path, no patch spelling can fix it). CLI argv errors (bad flags) are
//! out of scope: they are not reachable from patch input, and their
//! suggestion names the exact next command.
//!
//! Probes marked "retry green" paste `write_instead` verbatim and apply
//! for real (T361 doctrine: `--check` never proves a sweep), asserting
//! worktree bytes — not just exit 0.

mod common;
use common::*;

fn check_json(patch: &str, file: &str, content: &str) -> serde_json::Value {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let got = read(dir.path(), file);
    assert_eq!(got, content, "worktree must be untouched");
    v
}

fn apply_real(patch: &str, file: &str, content: &str) -> String {
    let dir = init_repo();
    write(dir.path(), file, content);
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "mechanical retry must apply, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    read(dir.path(), file)
}

// --- WI-3 new producers: fix_range ---

#[test]
fn range_past_eof_carries_fix_range_and_retries_green() {
    let content = "fn other() {}\nfn foo() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -90,8 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_RANGE_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({
            "action": "fix_range",
            "file": "src/lib.rs",
            "file_lines": 2,
            "write_instead": "@@ -2,1 @@",
        }),
        "got: {}",
        v["error"]["repair"]
    );
    // Mechanical retry: paste the clamped header verbatim, apply for real.
    let retry = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let got = apply_real(retry, "src/lib.rs", content);
    assert!(got.contains("fn bar()"), "got: {got}");
    assert!(!got.contains("fn foo()"), "got: {got}");
    // Text mode carries the same repair line.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", content);
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("repair:"), "got: {stderr}");
    assert!(stderr.contains("fix_range"), "got: {stderr}");
    assert!(stderr.contains("file_lines"), "got: {stderr}");
}

// --- WI-3 new producers: fix_directive ---

#[test]
fn unknown_key_near_miss_carries_fix_directive_and_retries_green() {
    let content = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
    let patch =
        "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# alow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({
            "action": "fix_directive",
            "found": "# alow=many",
            "did_you_mean": "# allow=many",
            "write_instead": "# allow=many",
        }),
        "got: {}",
        v["error"]["repair"]
    );
    // Mechanical retry: paste the corrected directive verbatim, real apply.
    let retry =
        "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let got = apply_real(retry, "src/lib.rs", content);
    assert!(got.contains("bar(1)"), "got: {got}");
    assert!(got.contains("bar(2)"), "got: {got}");
    assert!(!got.contains("foo("), "got: {got}");
}

#[test]
fn unknown_key_far_miss_is_honest_null_with_repair() {
    let content = "fn a() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# zzzzzz=1\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_directive"));
    assert_eq!(repair["found"], serde_json::json!("# zzzzzz=1"));
    assert!(repair["did_you_mean"].is_null(), "got: {repair}");
    assert!(
        repair["write_instead"]
            .as_str()
            .unwrap()
            .contains("SPEC 9.3"),
        "far miss must point at the table, got: {repair}"
    );
}

#[test]
fn far_miss_key_suggests_nothing_even_at_distance_two() {
    // `# foo=bar`: `foo` is distance 2 from `root`, but shares no first
    // character — a `did_you_mean: root` would cost a retry round. The
    // T113/T274 pins (reason + suggestion) hold byte-identical beside
    // the new repair.
    let content = "fn a() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# foo=bar\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["reason"],
        serde_json::json!("unknown directive `#foo=bar`: # foo=bar")
    );
    assert_eq!(
        v["error"]["suggestion"],
        serde_json::json!("check SPEC 9.3 for the supported directive list")
    );
    assert!(v["error"]["repair"]["did_you_mean"].is_null(), "got: {v}");
    assert_eq!(
        v["error"]["repair"]["action"],
        serde_json::json!("fix_directive")
    );
}

#[test]
fn invalid_value_near_miss_repairs_to_corrected_line() {
    let content = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
    // `somany` is distance 2 from `many` (ungated value table).
    let patch =
        "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# allow=somany\n# expr E\n- foo(E)\n+ bar(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({
            "action": "fix_directive",
            "found": "# allow=somany",
            "did_you_mean": "# allow=many",
            "write_instead": "# allow=many",
        }),
        "got: {}",
        v["error"]["repair"]
    );
    let retry =
        "diff --git src/lib.rs\n@@ -1,4 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let got = apply_real(retry, "src/lib.rs", content);
    assert!(got.contains("bar(2)"), "got: {got}");
}

#[test]
fn invalid_value_far_miss_lists_every_option() {
    let content = "fn a() {}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# allow=zzz\n# expr E\n- a(E)\n+ b(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_directive"));
    assert!(repair["did_you_mean"].is_null(), "got: {repair}");
    let instead = repair["write_instead"].as_str().unwrap();
    assert!(instead.contains("# allow=one"), "got: {repair}");
    assert!(instead.contains("# allow=many"), "got: {repair}");
}

#[test]
fn header_tag_near_miss_repairs_and_retries_green() {
    let content = "fn a() {}\n";
    // `rul=x` keeps the first character and is distance 1 from `rule`.
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rul=x\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_directive"));
    assert_eq!(repair["found"], serde_json::json!("rul=x"));
    assert_eq!(repair["did_you_mean"], serde_json::json!("rule=x"));
    let retry =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let got = apply_real(retry, "src/lib.rs", content);
    assert!(got.contains("fn b()"), "got: {got}");
}

#[test]
fn header_tag_far_miss_repairs_to_removal() {
    let content = "fn a() {}\n";
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ foo=bar\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n";
    let v = check_json(patch, "src/lib.rs", content);
    let repair = &v["error"]["repair"];
    assert_eq!(repair["action"], serde_json::json!("fix_directive"));
    assert_eq!(repair["found"], serde_json::json!("foo=bar"));
    assert!(repair["did_you_mean"].is_null(), "got: {repair}");
    assert!(
        repair["write_instead"]
            .as_str()
            .unwrap()
            .contains("remove `foo=bar`"),
        "got: {repair}"
    );
}

#[test]
fn unknown_root_value_repairs_and_retries_green() {
    let content = "fn m() {\n    foo(1);\n}\n";
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# root=exrp\n# expr E\n- foo(E)\n+ bar(E)\n";
    let v = check_json(patch, "src/lib.rs", content);
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_PATTERN_CONFLICT")
    );
    assert_eq!(
        v["error"]["repair"],
        serde_json::json!({
            "action": "fix_directive",
            "found": "root=exrp",
            "did_you_mean": "# root=expr",
            "write_instead": "# root=expr",
        }),
        "got: {}",
        v["error"]["repair"]
    );
    let retry = "diff --git src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# root=expr\n# expr E\n- foo(E)\n+ bar(E)\n";
    let got = apply_real(retry, "src/lib.rs", content);
    assert!(got.contains("bar(1)"), "got: {got}");
}

#[test]
fn edit_distance_matrix_pins() {
    // Unit pins for the shared table (SPEC 9.3): typo classes that must
    // fire, and the far class that must stay null.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn m() {\n    foo(1);\n}\n");
    // `# scope=files` → `# scope=file` (value distance 1).
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=files\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["repair"]["did_you_mean"],
        serde_json::json!("# scope=file"),
        "got: {v}"
    );
    // `# repeat=yes` → bare `# repeat` (single spelling).
    let patch =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# repeat=yes\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["repair"]["write_instead"],
        serde_json::json!("# repeat"),
        "got: {v}"
    );
}

// --- WI-3 enumeration: every patch-input class carries a repair ---

/// (name, patch, file content, expected code, expected repair action).
/// The test fails if any pinned producer loses its repair — a new
/// null-repair error fails CI unless it joins the allow-list with a
/// documented reason (see `allow_list_unreadable_file_stays_null`).
fn enumeration_cases() -> Vec<(&'static str, String, String, String, String)> {
    vec![
        (
            "range past EOF",
            "diff --git src/lib.rs\n@@ -90,8 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n"
                .to_string(),
            "fn foo() {}\nfn other() {}\n".to_string(),
            "RSMPL_RANGE_CONFLICT".to_string(),
            "fix_range".to_string(),
        ),
        (
            "empty patch",
            "".to_string(),
            "fn a() {}\n".to_string(),
            "RSMPL_PARSE_CONFLICT".to_string(),
            "fix_pattern".to_string(),
        ),
        (
            "ordinary count mismatch",
            "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n mod old;\n+mod new;\n fn main() {}\n"
                .to_string(),
            "mod old;\n\nfn main() {}\n".to_string(),
            "RSMPL_PARSE_CONFLICT".to_string(),
            "fix_header".to_string(),
        ),
        (
            "unknown directive",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# alow=many\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n"
                .to_string(),
            "fn a() {}\n".to_string(),
            "RSMPL_PATTERN_CONFLICT".to_string(),
            "fix_directive".to_string(),
        ),
        (
            "unknown header tag",
            "diff --git src/lib.rs\n@@ -1,1 @@ foo=bar\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n"
                .to_string(),
            "fn a() {}\n".to_string(),
            "RSMPL_PATTERN_CONFLICT".to_string(),
            "fix_directive".to_string(),
        ),
        (
            "unknown root value",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n# root=bogus\n- foo\n+ bar\n"
                .to_string(),
            "fn a() {}\n".to_string(),
            "RSMPL_PATTERN_CONFLICT".to_string(),
            "fix_directive".to_string(),
        ),
        (
            "zero match",
            "diff --git src/lib.rs\n@@ -1,2 @@ rule=rename_fn\n# lang=rust\n- fn baz(...) {\n+ fn qux(...) {\n      ...\n  }\n"
                .to_string(),
            "fn foo() {}\nfn bar() {}\n".to_string(),
            "RSMPL_MATCH_CONFLICT".to_string(),
            "read_file".to_string(),
        ),
        (
            "too many matches",
            "diff --git src/lib.rs\n@@ -1,4 @@ rule=t\n# expr E\n- foo(E)\n+ bar(E)\n"
                .to_string(),
            "fn m() {\n    foo(1);\n    foo(2);\n}\n".to_string(),
            "RSMPL_MATCH_CONFLICT".to_string(),
            "add_directive".to_string(),
        ),
        (
            "symbol without fallback (C13)",
            "diff --git src/lib.rs\n@@ -1,3 @@ rule=r\n# lang=rust\n# symbol Old=crate::old_api\n# expr E\n- Old(E)\n+ crate::new_api(E)\n"
                .to_string(),
            "fn a() {\n    crate::old_api(1);\n}\n".to_string(),
            "RSMPL_SYMBOL_CONFLICT".to_string(),
            "add_directive".to_string(),
        ),
        (
            "unbound capture",
            "diff --git src/lib.rs\n@@ -2,0 @@ rule=paste\n# use=Helper\n+ Helper\n"
                .to_string(),
            "fn a() {}\nfn b() {}\n".to_string(),
            "RSMPL_CAPTURE_CONFLICT".to_string(),
            "fix_name".to_string(),
        ),
        (
            "ordinary context mismatch",
            "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-WRONG\n+RIGHT\n"
                .to_string(),
            "fn a() {}\n".to_string(),
            "GIT_HUNK_CONFLICT".to_string(),
            "fix_context".to_string(),
        ),
        (
            "absolute target path",
            "diff --git /tmp/evil.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n"
                .to_string(),
            "fn a() {}\n".to_string(),
            "RSMPL_IO_CONFLICT".to_string(),
            "fix_pattern".to_string(),
        ),
        (
            "target file does not parse",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn a(...) {\n+ fn b(...) {\n      ...\n  }\n"
                .to_string(),
            "this is not rust (((".to_string(),
            "RSMPL_PARSE_CONFLICT".to_string(),
            "fix_pattern".to_string(),
        ),
        (
            "attrs absent mismatch at match time",
            "diff --git src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n# attrs=absent\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n"
                .to_string(),
            "/// doc\nfn foo() {}\n".to_string(),
            "RSMPL_MATCH_CONFLICT".to_string(),
            "fix_pattern".to_string(),
        ),
        (
            "field type annotation mismatch",
            "diff --git src/lib.rs\n@@ -1,4 @@ rule=rename_field\n  struct S {\n- old: u64\n+ new: u64\n"
                .to_string(),
            "struct S {\n    old: u32,\n    keep: bool,\n}\n".to_string(),
            "RSMPL_MATCH_CONFLICT".to_string(),
            "fix_pattern".to_string(),
        ),
    ]
}

#[test]
fn enumeration_every_patch_input_class_carries_repair() {
    for (name, patch, content, code, action) in enumeration_cases() {
        let dir = init_repo();
        write(dir.path(), "src/lib.rs", &content);
        // Empty patches arrive as empty stdin; every other case as stdin
        // text — both are patch-input paths (SPEC D.1).
        let out = run_stdin(dir.path(), &patch, &["--format", "json", "--check"]);
        assert_eq!(out.status.code(), Some(2), "case `{name}` must fail");
        let v: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&out.stdout))
            .unwrap_or_else(|_| panic!("case `{name}`: stdout is not JSON"));
        assert_eq!(
            v["error"]["code"],
            serde_json::json!(code),
            "case `{name}` wrong code, got: {v}"
        );
        let repair = &v["error"]["repair"];
        assert!(
            !repair.is_null(),
            "case `{name}` carries no repair (add a producer or join the allow-list), got: {v}"
        );
        assert_eq!(
            repair["action"],
            serde_json::json!(action),
            "case `{name}` wrong repair action, got: {v}"
        );
        // Text mode prints the same payload as the trailing repair line.
        let out = run_stdin(dir.path(), &patch, &["--check"]);
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(
            stderr.contains("repair:"),
            "case `{name}` text mode hides the repair, got: {stderr}"
        );
        assert!(
            stderr.contains(action.as_str()),
            "case `{name}` text repair names the wrong action, got: {stderr}"
        );
    }
}

#[test]
fn allow_list_unreadable_file_stays_null() {
    // The single named allow-list entry: an unreadable patch file is
    // environmental (the suggestion already names the path); no patch
    // spelling can fix it, so `repair` stays null by decision.
    let dir = init_repo();
    let out = run(
        dir.path(),
        &[
            "--file",
            "/nonexistent-wi3-allow-list.rsmpl",
            "--format",
            "json",
            "--check",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert_eq!(
        v["error"]["code"],
        serde_json::json!("RSMPL_IO_CONFLICT"),
        "got: {v}"
    );
    assert!(v["error"]["repair"].is_null(), "got: {v}");
    assert!(
        v["error"]["suggestion"]
            .as_str()
            .unwrap()
            .contains("/nonexistent-wi3-allow-list.rsmpl"),
        "suggestion must name the path, got: {v}"
    );
}
