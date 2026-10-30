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
                    assert!(val.as_str().unwrap().chars().count() <= 80, "got: {v}");
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
