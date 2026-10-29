//! WI-2 provenance pins per detector family (SPEC D.2).
//!
//! Each detector family records exact provenance — detector identity,
//! file, covered sites, totality evidence, and byte win — through the
//! public `infer_rsmpl_with_pre_images` entry point. Sections are
//! native-shape (with `---`/`+++` markers, as `git diff` emits them).

use std::collections::HashMap;

fn section(path: &str, body: &str) -> String {
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
}

fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn validated_live(patch: &str, pre: &HashMap<String, String>) {
    let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
        .expect("emitted patch must apply cleanly");
}

#[test]
fn provenance_pins_per_detector_family() {
    // WI-2: every detector family records exact provenance — detector
    // identity, file, covered sites, totality evidence, and byte win —
    // through the public `infer_rsmpl_with_pre_images` entry point over
    // native-shape sections (with `---`/`+++` markers, as `git diff` emits).
    struct Case {
        sections: Vec<String>,
        infer_pre: Vec<(&'static str, &'static str)>,
        apply_pre: Vec<(&'static str, &'static str)>,
        expect: Vec<(&'static str, &'static str)>,
        pre_image: bool,
    }
    fn case(
        sections: Vec<String>,
        infer_pre: Vec<(&'static str, &'static str)>,
        apply_pre: Vec<(&'static str, &'static str)>,
        expect: Vec<(&'static str, &'static str)>,
        pre_image: bool,
    ) -> Case {
        Case {
            sections,
            infer_pre,
            apply_pre,
            expect,
            pre_image,
        }
    }
    // Section-visible cases still need the real old tree to apply
    // against (`validated_live`); only the inference input is empty.
    fn novec(
        sections: Vec<String>,
        apply_pre: Vec<(&'static str, &'static str)>,
        expect: Vec<(&'static str, &'static str)>,
    ) -> Case {
        case(sections, vec![], apply_pre, expect, false)
    }
    // Pre-image cases: inference and application share one tree.
    fn both(
        sections: Vec<String>,
        pre: Vec<(&'static str, &'static str)>,
        expect: Vec<(&'static str, &'static str)>,
    ) -> Case {
        case(sections, pre.clone(), pre, expect, true)
    }
    let cases = vec![
        // Whole-section single-hunk sweeps (section-visible evidence).
        novec(
            vec![section(
                "f.rs",
                "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n+    new_function(a, 1);\n+    new_function(b, 2);\n+    new_function(c, 3);\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    a.old_method(1);\n    b.old_method(2);\n    c.old_method(3);\n}\n",
            )],
            vec![("method_to_fn", "f.rs")],
        ),
        novec(
            vec![section(
                "f.rs",
                "@@ -1,6 +1,6 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n-    d.old_method(4);\n+    a.new_method(1);\n+    b.new_method(2);\n+    c.new_method(3);\n+    d.new_method(4);\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    a.old_method(1);\n    b.old_method(2);\n    c.old_method(3);\n    d.old_method(4);\n}\n",
            )],
            vec![("method_to_method", "f.rs")],
        ),
        novec(
            vec![section(
                "f.rs",
                "@@ -1,2 +1,2 @@\n- use crate::old::Thing;\n- use crate::old::Other;\n+ use crate::new::Thing;\n+ use crate::new::Other;\n",
            )],
            vec![(
                "f.rs",
                "use crate::old::Thing;\nuse crate::old::Other;\n",
            )],
            vec![("use_prefix", "f.rs")],
        ),
        novec(
            vec![section(
                "f.rs",
                "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-async fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n+async fn bar(x: i32) -> i32 {\n",
            )],
            vec![(
                "f.rs",
                "fn foo(x: i32) -> i32 {\n}\nasync fn foo(x: i32) -> i32 {\n}\n",
            )],
            vec![("fn_slot_sweep", "f.rs")],
        ),
        // Pre-image evidence: the caller supplied the old file text.
        both(
            vec![section(
                "f.rs",
                "@@ -1,5 +1,5 @@\n fn m() {\n-    old_api(a1);\n-    old_api(a2);\n-    old_api(a3);\n+    new_api(a1);\n+    new_api(a2);\n+    new_api(a3);\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    old_api(a1);\n    old_api(a2);\n    old_api(a3);\n}\n",
            )],
            vec![("call_sweep", "f.rs")],
        ),
        both(
            vec![section(
                "f.rs",
                "@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, true);\n}\n",
            )],
            vec![("call_literal", "f.rs")],
        ),
        both(
            vec![section(
                "f.rs",
                "@@ -1,3 +1,3 @@\n fn m() {}\n-struct Old;\n+struct New;\n fn n() {}\n",
            )],
            vec![("f.rs", "fn m() {}\nstruct Old;\nfn n() {}\n")],
            vec![("item_rename", "f.rs")],
        ),
        // Verbatim identical-pair sweep (cross-hunk, pre-image).
        both(
            vec![section(
                "f.rs",
                "@@ -2,1 +2,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -9,1 +10,1 @@\n-        &state.parsed,\n+        parsed,\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    let _ = &state.parsed;\n    x();\n    x();\n    x();\n    x();\n    x();\n    let _ = &state.parsed;\n}\n",
            )],
            vec![("verbatim", "f.rs")],
        ),
        // Whole-file item deletion (section-visible: no pre-image use).
        novec(
            vec!["diff --git a.rs\ndeleted file mode 100644\n--- a/a.rs\n+++ /dev/null\n@@ -1,5 +0,0 @@\n-fn gone() {\n-    let a = 1;\n-    let b = 2;\n-    a + b\n-}\n".to_string()],
            vec![("a.rs", "fn gone() {\n    let a = 1;\n    let b = 2;\n    a + b\n}\n")],
            vec![("delete", "a.rs")],
        ),
        // Same-literal method sweep (hetero receivers, shared literals).
        novec(
            vec![section(
                "f.rs",
                "@@ -1,5 +1,5 @@\n fn m() {\n-    agents.contains(\"skills/rsmpl-git/SKILL.md\"),\n-    readme.contains(\"skills/rsmpl-git/SKILL.md\"),\n-    spec.contains(\"skills/rsmpl-git/SKILL.md\"),\n+    agents.contains(\"skills/gitrs/SKILL.md\"),\n+    readme.contains(\"skills/gitrs/SKILL.md\"),\n+    spec.contains(\"skills/gitrs/SKILL.md\"),\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    check(\n        agents.contains(\"skills/rsmpl-git/SKILL.md\"),\n        readme.contains(\"skills/rsmpl-git/SKILL.md\"),\n        spec.contains(\"skills/rsmpl-git/SKILL.md\"),\n    );\n}\n",
            )],
            vec![("method_same_literal", "f.rs")],
        ),
        // &-receiver strip sweep.
        novec(
            vec![section(
                "f.rs",
                "@@ -1,5 +1,5 @@\n fn m() {\n-    &dir.path().join(\u{22}\u{22}),\n-    &dir.path().join(\u{22}\u{22}),\n-    write(&dir.path().join(\u{22}\u{22}), \u{22}a\u{22});\n+    dir.path(),\n+    dir.path(),\n+    write(dir.path(), \u{22}a\u{22});\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    check(\n        &dir.path().join(\"\"),\n        &dir.path().join(\"\"),\n        write(&dir.path().join(\u{22}\u{22}), \u{22}a\u{22}),\n    );\n}\n",
            )],
            vec![("ref_strip", "f.rs")],
        ),
        // Same-callee trailing-arg addition sweep.
        both(
            vec![section(
                "f.rs",
                "@@ -1,4 +1,4 @@\n fn m() {\n-    run(first_long_argument_name);\n-    run(second_long_argument_name);\n+    run(first_long_argument_name, extra_long_tail_argument);\n+    run(second_long_argument_name, extra_long_tail_argument);\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    run(first_long_argument_name);\n    run(second_long_argument_name);\n}\n",
            )],
            vec![("call_arg_add", "f.rs")],
        ),
        // Array-head-peel call migration sweep (same head, hetero tail).
        both(
            vec![section(
                "f.rs",
                "@@ -1,4 +1,4 @@\n fn m() {\n-    f(dst, &[\"h\", m1]);\n-    f(dst, &[\"h\", m2]);\n+    g(dst, m1, &[]);\n+    g(dst, m2, &[]);\n }\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    f(dst, &[\"h\", m1]);\n    f(dst, &[\"h\", m2]);\n}\n",
            )],
            vec![("call_peel", "f.rs")],
        ),
        // Cross-hunk peel section (two tail-arity classes + leftover).
        both(
            vec![section(
                "f.rs",
                "@@ -2,1 +2,1 @@\n-    f(dst, &[\"h1\", m1]);\n+    g(dst, m1, &[]);\n@@ -5,1 +6,1@@\n-    f(dst, &[\"h1\", m2]);\n+    g(dst, m2, &[]);\n@@ -9,1 +10,1 @@\n-    f(dst, &[\"h2\", n1, t1]);\n+    g(dst, n1, &[t1]);\n@@ -12,1 +13,1@@\n-    f(dst, &[\"h2\", n2, t1]);\n+    g(dst, n2, &[t1]);\n@@ -20,0 +21,1 @@\n+let cached = extra();\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    f(dst, &[\"h1\", m1]);\n    x();\n    x();\n    f(dst, &[\"h1\", m2]);\n    x();\n    x();\n    x();\n    f(dst, &[\"h2\", n1, t1]);\n    x();\n    x();\n    f(dst, &[\"h2\", n2, t1]);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n}\n",
            )],
            vec![("call_peel_section", "f.rs"), ("call_peel_section", "f.rs")],
        ),
        // Cross-hunk literal section (hetero trailing literals + leftover).
        both(
            vec![section(
                "f.rs",
                "@@ -2,1 +2,1@@\n-    run(a1, \"old_alpha_literal_value\");\n+    run(a1, \"new_alpha_literal_value\");\n@@ -5,1 +6,1@@\n-    run(a2, \"old_alpha_literal_value\");\n+    run(a2, \"new_alpha_literal_value\");\n@@ -9,1 +10,1@@\n-    run(b1, \"old_beta_literal_value\");\n+    run(b1, \"new_beta_literal_value\");\n@@ -12,1 +13,1@@\n-    run(b2, \"old_beta_literal_value\");\n+    run(b2, \"new_beta_literal_value\");\n@@ -20,0 +21,1 @@\n+let cached = extra();\n",
            )],
            vec![(
                "f.rs",
                "fn m() {\n    run(a1, \"old_alpha_literal_value\");\n    x();\n    x();\n    run(a2, \"old_alpha_literal_value\");\n    x();\n    x();\n    x();\n    run(b1, \"old_beta_literal_value\");\n    x();\n    x();\n    run(b2, \"old_beta_literal_value\");\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n}\n",
            )],
            vec![("call_literal_section", "f.rs"), ("call_literal_section", "f.rs")],
        ),
        // Cross-file whole-item move (cut + paste).
        both(
            vec![
                "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,7 +1,0 @@\n-fn big() {\n-    let a = 1;\n-    let b = 2;\n-    let c = 3;\n-    let d = 4;\n-    a + b + c + d\n-}\n".to_string(),
                "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 +2,7 @@\n+fn big() {\n+    let a = 1;\n+    let b = 2;\n+    let c = 3;\n+    let d = 4;\n+    a + b + c + d\n+}\n".to_string(),
            ],
            vec![
                (
                    "a.rs",
                    "fn keep() {}\nfn big() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    a + b + c + d\n}\n"
                ),
                ("b.rs", "use x;\n"),
            ],
            vec![("move", "a.rs"), ("move", "b.rs")],
        ),
    ];
    for (i, c) in cases.iter().enumerate() {
        let to_map = |pairs: &[(&'static str, &'static str)]| {
            pre_map(&pairs.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>())
        };
        let infer_pre = to_map(&c.infer_pre);
        let apply_pre = to_map(&c.apply_pre);
        let raw: String = c.sections.concat();
        let inferred = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &infer_pre)
            .expect("infer must succeed");
        let prov = inferred.hunks;
        assert!(
            !prov.is_empty(),
            "case {i} must compress, got passthrough:\n{}",
            inferred.rsmpl
        );
        validated_live(&inferred.rsmpl, &apply_pre);
        assert_eq!(
            prov.len(),
            c.expect.len(),
            "case {i}: expected {} hunks, got {:?}",
            c.expect.len(),
            prov
        );
        if c.expect.iter().any(|(d, _)| *d == "move") {
            assert!(
                prov.iter().any(|h| !h.sites.is_empty()),
                "case {i}: move cut must carry sites: {prov:?}"
            );
        }
        for (h, (detector, file)) in prov.iter().zip(c.expect.iter()) {
            assert_eq!(h.detector, *detector, "case {i}: {h:?}");
            assert_eq!(h.file, *file, "case {i}: {h:?}");
            assert!(h.rule.starts_with('r'), "case {i}: {h:?}");
            assert_eq!(
                h.totality,
                if c.pre_image {
                    "pre-image"
                } else {
                    "section-visible"
                },
                "case {i}: {h:?}"
            );
            assert!(h.saved_bytes > 0, "case {i}: {h:?}");
            // Anchor-only move pastes report no sites; every other
            // hunk covers at least one changed line.
            if h.detector != "move" {
                assert!(!h.sites.is_empty(), "case {i}: {h:?}");
            }
        }
    }
}
