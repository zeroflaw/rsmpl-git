//! WI-8a `diffrs` move+evolve pins through the public
//! `infer_rsmpl_with_pre_images` entry point: an `fn` move whose header
//! evolves (vis/name) compresses to a cut + header-template paste and
//! round-trips byte-identical; everything outside the WI-8a shape
//! stays ordinary.

use std::collections::HashMap;

/// A file tree: (path, content) pairs.
type FileTree = Vec<(String, String)>;

fn big_fn(vis: &str, name: &str) -> String {
    let mut s = String::new();
    if !vis.is_empty() {
        s.push_str(vis);
        s.push(' ');
    }
    s.push_str(&format!("fn {name}() {{\n"));
    for i in 0..40 {
        s.push_str(&format!("    let v{i} = {i};\n"));
    }
    s.push_str("}\n");
    s
}

fn section(path: &str, body: &str) -> String {
    format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
}

fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Apply the inferred patch to the pre tree; the post tree must match
/// byte-for-byte (moves round-trip exactly, never approximately).
fn assert_round_trip(rsmpl: &str, pres: &[(&str, &str)], posts: &[(&str, &str)]) {
    let parsed = rsmpl_patch::parse_patch(rsmpl).expect("emitted patch must parse");
    let mut files = pre_map(pres);
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
        .expect("emitted patch must apply cleanly");
    for (path, want) in posts {
        assert_eq!(
            files.get(*path).map(String::as_str),
            Some(*want),
            "post tree mismatch for {path}"
        );
    }
}

fn evolved_move_fixture() -> (String, FileTree, FileTree) {
    let big = big_fn("", "big");
    let huge = big_fn("pub", "huge");
    let a_old = format!("fn keep() {{}}\n{big}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{huge}");
    let minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = huge.lines().map(|l| format!("+{l}\n")).collect();
    let a_sec = section(
        "a.rs",
        &format!("@@ -2,{} +2,0 @@\n{minus}", big.lines().count()),
    );
    let b_sec = section(
        "b.rs",
        &format!(
            "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
            huge.lines().count() + 1
        ),
    );
    (
        format!("{a_sec}{b_sec}"),
        vec![("a.rs".to_string(), a_old), ("b.rs".to_string(), b_old)],
        vec![("a.rs".to_string(), a_new), ("b.rs".to_string(), b_new)],
    )
}

#[test]
fn evolved_move_fires_with_template_paste_and_round_trips() {
    let (raw, pres, posts) = evolved_move_fixture();
    let pre: HashMap<String, String> = pres.iter().cloned().collect();
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind=M1"),
        "evolved cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub fn huge(...) { ... }"),
        "template paste missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        !out.rsmpl.contains("-fn big"),
        "minus lines must be consumed, got:\n{}",
        out.rsmpl
    );
    let post_refs: Vec<(&str, &str)> = posts
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let pre_refs: Vec<(&str, &str)> = pres.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn identical_move_stays_verbatim_no_template() {
    let big = big_fn("", "big");
    let a_old = format!("fn keep() {{}}\n{big}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", big.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", big.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(out.rsmpl.contains("+ M1"), "got:\n{}", out.rsmpl);
    assert!(
        !out.rsmpl.contains("..."),
        "identical moves never template, got:\n{}",
        out.rsmpl
    );
}

#[test]
fn body_edited_pair_stays_ordinary() {
    // Same header evolution, but the body gains a line: two changes,
    // not one move — stays a full delete+insert.
    let big = big_fn("", "big");
    let huge_owned = big_fn("pub", "huge");
    let mut huge_lines: Vec<&str> = huge_owned.lines().collect();
    huge_lines.insert(2, "    let extra = 1;");
    let huge = huge_lines.join("\n") + "\n";
    let a_old = format!("fn keep() {{}}\n{big}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = huge.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", big.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", huge.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "body edits veto the move, got:\n{}",
        out.rsmpl
    );
}

#[test]
fn param_edited_pair_stays_ordinary() {
    let minus_fn = "fn big(x: i32) {\n    x\n}\n".to_string()
        + &(0..40)
            .map(|i| format!("    let v{i} = {i};\n"))
            .collect::<String>();
    let plus_fn = "pub fn huge(x: i64) {\n    x\n}\n".to_string()
        + &(0..40)
            .map(|i| format!("    let v{i} = {i};\n"))
            .collect::<String>();
    let a_old = format!("fn keep() {{}}\n{minus_fn}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_fn.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_fn.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_fn.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_fn.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "param edits veto the move, got:\n{}",
        out.rsmpl
    );
}

#[test]
fn restricted_vis_only_move_raises_template_and_round_trips() {
    // WI-history-70d72c0: vis-only `fn big` → `pub(crate) fn big` (the
    // 70d72c0 `parse_diff_git_header` shape) must raise to cut +
    // header-template paste and apply byte-identical — the emitted
    // template must be accepted by the engine's own template gate.
    let big = big_fn("", "big");
    let big_pub = big_fn("pub(crate)", "big");
    let a_old = format!("fn keep() {{}}\n{big}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{big_pub}");
    let minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = big_pub.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", big.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", big_pub.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub(crate) fn big(...) { ... }"),
        "restricted-vis template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn struct_name_change_stays_ordinary_wi_8b() {
    // Non-`fn` evolution is WI-8b territory: stays ordinary.
    let minus_item = "struct Old {\n    x: i32,\n}\n";
    let plus_item = "struct New {\n    x: i32,\n}\n";
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section("a.rs", &format!("@@ -2,3 +2,0 @@\n{minus}")),
        section("b.rs", &format!("@@ -1,0 +2,4 @@\n+\n{plus}"))
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "struct evolution is WI-8b, got:\n{}",
        out.rsmpl
    );
}
