//! WI-2: assertable evidence — binding digests, structured skips, span labels.
//!
//! Every binding an agent might act on is either complete or accompanied
//! by its length + SHA-256; follow skips and orphans carry machine reason
//! codes; identical multi-edit spans are distinguishable. All keys are
//! additive under report `version: 3` (WI-20 owns the `follow_skipped`
//! string removal).

mod common;
use common::*;

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON")
}

#[test]
fn binding_digest_equality_is_assertable_without_full_bytes() {
    // Two sites bind the same 100-char expression: both `binding_meta`
    // digests agree, so an agent asserts equality without re-reading.
    let dir = init_repo();
    let long_arg = "a".repeat(100);
    write(
        dir.path(),
        "src/lib.rs",
        &format!("fn foo() {{}}\nfn m() {{\n    foo({long_arg});\n    foo({long_arg});\n}}\n"),
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    let sites = v["rules"][0]["sites"].as_array().unwrap();
    assert_eq!(sites.len(), 2, "got: {v}");
    let meta0 = &sites[0]["binding_meta"]["E"];
    let meta1 = &sites[1]["binding_meta"]["E"];
    assert_eq!(meta0["len"], serde_json::json!(100), "got: {v}");
    assert_eq!(meta0["sha256"], meta1["sha256"], "got: {v}");
    // Display values stay truncated with the `…` marker.
    for site in sites {
        let shown = site["bindings"]["E"].as_str().unwrap();
        assert_eq!(shown.chars().count(), 81, "got: {v}");
        assert!(shown.ends_with('…'), "got: {v}");
    }
    assert_eq!(
        v["rules"][0]["bindings_truncated_chars"],
        serde_json::json!(2),
        "got: {v}"
    );
    // WI-2 budget: digest keys ride the `110·C` term (measured ~91 B per
    // cut here — len digits + 64-hex sha256 + keys — pinned at 110).
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let diff_len = v["diff"].as_str().unwrap().len();
    let overhead = stdout.len() - diff_len;
    let f = v["files"].as_array().unwrap().len();
    let rules = v["rules"].as_array().unwrap();
    let r = rules.len();
    let s: usize = rules
        .iter()
        .map(|e| e["spans"].as_array().unwrap().len())
        .sum();
    let c: usize = rules
        .iter()
        .map(|e| {
            e.get("bindings_truncated_chars")
                .and_then(|n| n.as_u64())
                .unwrap_or(0) as usize
        })
        .sum();
    let budget = 200 + 150 * f + 175 * r + 90 * s + 110 * c;
    assert!(
        overhead <= budget,
        "overhead {overhead} B exceeds budget {budget} B (F={f} R={r} S={s} C={c}): {v}"
    );
}

#[test]
fn bindings_full_lifts_the_cut_for_that_run_only() {
    let dir = init_repo();
    let long_arg = "a".repeat(100);
    write(
        dir.path(),
        "src/lib.rs",
        &format!("fn foo() {{}}\nfn m() {{\n    foo({long_arg});\n}}\n"),
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--check", "--bindings", "full"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    let shown = v["rules"][0]["sites"][0]["bindings"]["E"].as_str().unwrap();
    assert_eq!(shown.chars().count(), 100, "got: {v}");
    assert!(!shown.ends_with('…'), "got: {v}");
    // Full run carries no cut evidence at all.
    assert!(
        v["rules"][0].get("binding_meta").is_none()
            && v["rules"][0]["sites"][0].get("binding_meta").is_none(),
        "got: {v}"
    );
    assert!(
        v["rules"][0].get("bindings_truncated_chars").is_none(),
        "got: {v}"
    );
}

#[test]
fn bindings_flag_rejects_unknown_modes() {
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let out = run_stdin(
        dir.path(),
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        &["--format", "json", "--check", "--bindings", "everything"],
    );
    assert_eq!(out.status.code(), Some(2), "got: {out:?}");
    let v = json_of(&out);
    assert_eq!(v["error"]["code"], "RSMPL_IO_CONFLICT", "got: {v}");
}

#[test]
fn short_bindings_carry_no_digest_keys() {
    // Uncut reports stay byte-identical to the pre-WI-2 shape: no meta.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {}\nfn m() {\n    foo(1);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert!(
        v["rules"][0]["sites"][0].get("binding_meta").is_none(),
        "got: {v}"
    );
    assert!(
        v["rules"][0].get("bindings_truncated_chars").is_none(),
        "got: {v}"
    );
}

#[test]
fn follow_skip_shadowed_carries_structured_reason() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn old() {}\nfn old() {}\nfn g() {\n    old();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- fn old(...) {\n+ fn new(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    // Compat string is unchanged prose.
    assert!(
        v["rules"][0]["follow_skipped"]
            .as_str()
            .unwrap()
            .contains("shadowed definition"),
        "got: {v}"
    );
    // Structured triple sits beside it.
    assert_eq!(
        v["rules"][0]["follow_skipped_details"],
        serde_json::json!([{"file": "src/lib.rs", "line": 2, "reason": "shadowed"}]),
        "got: {v}"
    );
}

#[test]
fn follow_skip_macro_hidden_carries_structured_reason() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct Old;\nfn f() {\n    m!(Old);\n    let x: Old = make();\n    x\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert_eq!(
        v["rules"][0]["follow_skipped_details"],
        serde_json::json!([{"file": "src/lib.rs", "line": 3, "reason": "macro-hidden"}]),
        "got: {v}"
    );
}

#[test]
fn follow_skip_glob_reexport_carries_structured_reason() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "use m::*;\nfn f(x: Old) -> Old {\n    x\n}\n",
    );
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert_eq!(
        v["rules"][0]["follow_skipped_details"],
        serde_json::json!([{"file": "src/b.rs", "line": 1, "reason": "glob-reexport"}]),
        "got: {v}"
    );
}

#[test]
fn follow_skip_off_target_carries_structured_reason() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "pub use crate::other::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
    );
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert_eq!(
        v["rules"][0]["follow_skipped_details"],
        serde_json::json!([{"file": "src/b.rs", "line": 1, "reason": "off-target"}]),
        "got: {v}"
    );
}

#[test]
fn follow_skip_second_hop_carries_structured_reason() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(dir.path(), "src/re.rs", "pub use crate::b::Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "pub use crate::re::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
    );
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    let details = v["rules"][0]["follow_skipped_details"].as_array().unwrap();
    assert_eq!(details.len(), 2, "got: {v}");
    for d in details {
        assert_eq!(d["reason"], "second-hop", "got: {v}");
    }
}

#[test]
fn text_mode_prints_skip_triples_one_per_line() {
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn old() {}\nfn old() {}\nfn g() {\n    old();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- fn old(...) {\n+ fn new(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("follow-skipped: src/lib.rs:2:shadowed"),
        "got: {stderr}"
    );
}

#[test]
fn orphans_carry_machine_reasons_in_json_and_text() {
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "fn f(x: Old) -> Old {\n    m!(Old);\n    x\n}\n",
    );
    let patch = "diff --git src/a.rs\n@@ -1,1 @@ rule=r\n- struct Old\n+ struct New\n";
    let out = run_stdin(
        dir.path(),
        patch,
        &["--format", "json", "--diff-max-lines", "0", "--check"],
    );
    assert!(out.status.success());
    let v = json_of(&out);
    let orphans = v["rules"][0]["orphans"].as_array().unwrap();
    assert_eq!(orphans.len(), 2, "got: {v}");
    // Provable use vs macro interior.
    assert_eq!(orphans[0]["reason"], "off-target", "got: {v}");
    assert_eq!(orphans[1]["reason"], "macro-hidden", "got: {v}");
    // Text line names the same triples.
    let out = run_stdin(dir.path(), patch, &["--check"]);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("src/b.rs:1 [off-target]"), "got: {stderr}");
    assert!(
        stderr.contains("src/b.rs:2 [macro-hidden]"),
        "got: {stderr}"
    );
}

#[test]
fn span_edits_distinguish_same_line_sig_evolution_slots() {
    // One match, three edits, all on line 1: without labels the three
    // `[1,1]` spans are indistinguishable.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo<T>(x: T) -> T {\n    x\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=evo\n- fn foo<T>(...) -> T {\n+ fn bar<T: Clone>(...) -> String {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(&out);
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[1, 1], [1, 1], [1, 1]]),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["span_edits"],
        serde_json::json!(["rename", "generics", "return"]),
        "got: {v}"
    );
}

#[test]
fn span_edits_label_follow_use_spans() {
    // Rename + same-file follow: definition span is the rename, the use
    // span is the follow.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct Old;\nfn f() {\n    m!(Old);\n    let x: Old = make();\n    x\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[1, 1], [4, 4]]),
        "got: {v}"
    );
    assert_eq!(
        v["rules"][0]["span_edits"],
        serde_json::json!(["rename", "follow"]),
        "got: {v}"
    );
}

#[test]
fn span_edits_omit_for_single_edit_rules() {
    // One span = nothing to disambiguate, even with a label present.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert_eq!(
        v["rules"][0]["spans"],
        serde_json::json!([[1, 1]]),
        "got: {v}"
    );
    assert!(v["rules"][0].get("span_edits").is_none(), "got: {v}");
    // Call sweeps bind sites, not slots: no labels at all.
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo() {}\nfn m() {\n    foo(1);\n    foo(2);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    assert!(v["rules"][0].get("span_edits").is_none(), "got: {v}");
}

#[test]
fn follow_only_extras_carry_follow_labels() {
    // Cross-file follow: the extra entry's spans are all use sites.
    let dir = init_repo();
    write(dir.path(), "src/a.rs", "pub struct Old;\n");
    write(
        dir.path(),
        "src/b.rs",
        "use crate::Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
    );
    let patch =
        "diff --git src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(out.status.success());
    let v = json_of(&out);
    let rules = v["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 2, "got: {v}");
    assert_eq!(rules[1]["file"], "src/b.rs", "got: {v}");
    assert_eq!(
        rules[1]["span_edits"],
        serde_json::json!(["follow", "follow", "follow"]),
        "got: {v}"
    );
}
