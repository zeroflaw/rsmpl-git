//! WI-2b follow-up: absolute report-budget pin on a multi-KB realistic report.
//!
//! Replaces the old "≤15% growth" relative bound (unachievable on sub-KB
//! reports, never normative) with the SPEC D.2 absolute budget:
//! overhead (bytes beyond the reported `diff`) ≤
//! `200 + 150·F + 150·R + 90·S` (WI-2b: 20 B per span + ~70 B per site
//! evidence). The fixture is deliberately multi-KB
//! (12 files × 10 call sites) so the pin applies where it matters.

mod common;
use common::*;

#[test]
fn json_report_overhead_within_absolute_budget() {
    let dir = init_repo();
    for i in 0..12 {
        let mut text = String::from("fn foo() {\n");
        for j in 0..10 {
            text.push_str(&format!("    old_call({j}, \"value{j}\");\n"));
        }
        text.push_str("}\n");
        write(dir.path(), &format!("src/f{i:02}.rs"), &text);
    }
    let mut patch = String::new();
    for i in 0..12 {
        let path = format!("src/f{i:02}.rs");
        patch.push_str(&format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -1,1 @@ rule=r{i}\n# sweep\n# expr E0\n# expr E1\n- old_call(E0, E1)\n+ new_call(E0, E1)\n"
        ));
    }
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
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    // Multi-KB realism: the report itself must exceed 2 KiB, so this pin
    // cannot pass on a toy single-hunk report.
    assert!(
        stdout.len() > 2048,
        "expected multi-KB report, got {} B: {v}",
        stdout.len()
    );
    let diff_len = v["diff"].as_str().unwrap().len();
    assert_eq!(diff_len, 0, "got: {v}");
    let overhead = stdout.len() - diff_len;
    let f = v["files"].as_array().unwrap().len();
    let rules = v["rules"].as_array().unwrap();
    let r = rules.len();
    let s: usize = rules
        .iter()
        .map(|e| e["spans"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    assert_eq!(f, 12, "got: {v}");
    assert_eq!(r, 12, "got: {v}");
    assert_eq!(s, 120, "got: {v}");
    let budget = 200 + 150 * f + 150 * r + 90 * s;
    assert!(
        overhead <= budget,
        "overhead {overhead} B exceeds budget {budget} B (F={f} R={r} S={s}): {v}"
    );
    // Token bounds still hold per entry (WI-2c v2: `sites` is the only
    // bindings evidence; first-site `bindings` removed).
    for e in rules {
        assert!(e.get("bindings").is_none(), "got: {v}");
        assert!(e.get("bindings_truncated").is_none(), "got: {v}");
        // WI-2b: per-site evidence mirrors spans; per-site bounds hold.
        if let Some(sites) = e.get("sites") {
            let arr = sites.as_array().unwrap();
            assert_eq!(arr.len(), e["spans"].as_array().unwrap().len(), "got: {v}");
            for site in arr {
                assert_eq!(site["span"].as_array().unwrap().len(), 2, "got: {v}");
                let map = site["bindings"].as_object().unwrap();
                assert!(map.len() <= 8, "got: {v}");
                for (_, val) in map {
                    // WI-1a: values are ≤ 80 chars, or an 80-char prefix
                    // plus the `…` truncation marker.
                    let s = val.as_str().unwrap();
                    if let Some(prefix) = s.strip_suffix('…') {
                        assert!(prefix.chars().count() <= 80, "got: {v}");
                    } else {
                        assert!(s.chars().count() <= 80, "got: {v}");
                    }
                }
            }
        }
    }
    assert_eq!(
        v["checked"],
        serde_json::json!({"lowering": true, "semantics": true, "build": "skipped"}),
        "got: {v}"
    );
}

#[test]
fn json_bindings_truncate_and_omit_empty() {
    // WI-2b/2c v2 token bounds: per-site values cut at 80 chars (ASCII
    // here, so chars == bytes) with a trailing `…` marker (WI-1a), and
    // shapes that bind nothing omit the `sites` key (first-site `bindings`
    // removed with `version: 2`).
    let dir = init_repo();
    let long_arg = "a".repeat(100);
    write(
        dir.path(),
        "src/lib.rs",
        &format!("fn foo() {{}}\nfn m() {{\n    foo({long_arg});\n    foo({long_arg});\n}}\n"),
    );
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    // WI-2b: per-site truncation; both sites truncate (WI-1a: 80-char
    // prefix plus the `…` marker, counted in `bindings_truncated_chars`).
    let sites = v["rules"][0]["sites"].as_array().unwrap();
    assert_eq!(sites.len(), 2, "got: {v}");
    for site in sites {
        let s = site["bindings"]["E"].as_str().unwrap();
        assert_eq!(s.chars().count(), 81, "got: {v}");
        assert!(s.ends_with('…'), "got: {v}");
        assert!(
            s.strip_suffix('…').unwrap().chars().all(|c| c == 'a'),
            "got: {v}"
        );
    }
    assert_eq!(
        v["rules"][0]["bindings_truncated_chars"],
        serde_json::json!(2),
        "got: {v}"
    );

    // A definition-only rename binds nothing: sites key absent.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "fn foo() {}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    assert!(v["rules"][0].get("sites").is_none(), "got: {v}");
    assert!(v["rules"][0].get("sites_truncated").is_none(), "got: {v}");
}
