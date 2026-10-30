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

fn big_struct(vis: &str, name: &str) -> String {
    let mut s = String::new();
    if !vis.is_empty() {
        s.push_str(vis);
        s.push(' ');
    }
    s.push_str(&format!("struct {name} {{\n"));
    for i in 0..40 {
        s.push_str(&format!("    v{i}: u32,\n"));
    }
    s.push_str("}\n");
    s
}

#[test]
fn struct_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 1: `struct Old` → `pub struct New` (same fields) moving
    // files compresses to a cut + struct-template paste and round-trips
    // byte-identical.
    let minus_item = big_struct("", "Old");
    let plus_item = big_struct("pub", "New");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved struct cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub struct New { ... }"),
        "struct template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn struct_field_edited_pair_stays_ordinary() {
    // Same vis/name evolution, but a field changes type: two changes,
    // not one move — stays a full delete+insert.
    let minus_item = big_struct("", "Old");
    let mut plus_lines: Vec<String> = big_struct("pub", "New")
        .lines()
        .map(str::to_string)
        .collect();
    plus_lines[2] = "    v1: i64,".to_string();
    let plus_item = plus_lines.join("\n") + "\n";
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "field edits veto the struct move, got:\n{}",
        out.rsmpl
    );
}

/// A long-enough enum body to clear the T375 byte-win guard.
fn big_enum(vis: &str, name: &str) -> String {
    let pre = if vis.is_empty() {
        String::new()
    } else {
        format!("{vis} ")
    };
    let mut s = format!("{pre}enum {name} {{\n");
    for i in 0..30 {
        s.push_str(&format!("    V{i}(u32),\n"));
    }
    s.push_str("}\n");
    s
}

#[test]
fn enum_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 2: `enum Old` → `pub enum New` (same variants) moving
    // files compresses to a cut + enum-template paste and round-trips
    // byte-identical.
    let minus_item = big_enum("", "Old");
    let plus_item = big_enum("pub", "New");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved enum cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub enum New { ... }"),
        "enum template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn enum_variant_edited_pair_stays_ordinary() {
    // Same vis/name evolution, but a variant changes payload: two
    // changes, not one move — stays a full delete+insert.
    let minus_item = big_enum("", "Old");
    let plus_item = big_enum("pub", "New").replace("V1(u32),", "V1(i64),");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "variant edits veto the enum move, got:\n{}",
        out.rsmpl
    );
}

/// A long-enough union body to clear the T375 byte-win guard.
fn big_union(vis: &str, name: &str) -> String {
    let pre = if vis.is_empty() {
        String::new()
    } else {
        format!("{vis} ")
    };
    let mut s = format!("{pre}union {name} {{\n");
    for i in 0..30 {
        s.push_str(&format!("    v{i}: u32,\n"));
    }
    s.push_str("}\n");
    s
}

#[test]
fn union_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 3: `union Old` → `pub union New` (same fields) moving
    // files compresses to a cut + union-template paste and round-trips
    // byte-identical.
    let minus_item = big_union("", "Old");
    let plus_item = big_union("pub", "New");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved union cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub union New { ... }"),
        "union template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn union_field_edited_pair_stays_ordinary() {
    // Same vis/name evolution, but a field changes type: two changes,
    // not one move — stays a full delete+insert.
    let minus_item = big_union("", "Old");
    let plus_item = big_union("pub", "New").replace("v1: u32,", "v1: i64,");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "field edits veto the union move, got:\n{}",
        out.rsmpl
    );
}

/// A long-enough type-alias body to clear the T375 byte-win guard
/// (doc lines are attrs, so they ride with the alias item).
fn big_alias(vis: &str, name: &str) -> String {
    let pre = if vis.is_empty() {
        String::new()
    } else {
        format!("{vis} ")
    };
    let mut s = String::new();
    for i in 0..12 {
        s.push_str(&format!("/// pad {i} for byte-win padding text\n"));
    }
    s.push_str(&format!("{pre}type {name} = u32;\n"));
    s
}

#[test]
fn type_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 4: `type Old` → `pub type New` (same aliased type)
    // moving files compresses to a cut + type-template paste and
    // round-trips byte-identical.
    let minus_item = big_alias("", "Old");
    let plus_item = big_alias("pub", "New");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved type cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub type New = ...;"),
        "type template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn type_aliased_type_edited_pair_stays_ordinary() {
    // Same vis/name evolution, but the aliased type changes: two
    // changes, not one move — stays a full delete+insert.
    let minus_item = big_alias("", "Old");
    let plus_item = big_alias("pub", "New").replace("type New = u32;", "type New = i64;");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "aliased-type edits veto the type move, got:\n{}",
        out.rsmpl
    );
}

/// A long-enough const body to clear the T375 byte-win guard
/// (doc lines are attrs, so they ride with the const item).
fn big_const(vis: &str, name: &str) -> String {
    let pre = if vis.is_empty() {
        String::new()
    } else {
        format!("{vis} ")
    };
    let mut s = String::new();
    for i in 0..12 {
        s.push_str(&format!("/// pad {i} for byte-win padding text\n"));
    }
    s.push_str(&format!("{pre}const {name}: u32 = 1;\n"));
    s
}

#[test]
fn const_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 5: `const Old` → `pub const New` (same type + expr)
    // moving files compresses to a cut + const-template paste and
    // round-trips byte-identical.
    let minus_item = big_const("", "Old");
    let plus_item = big_const("pub", "New");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved const cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub const New = ...;"),
        "const template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn const_expr_edited_pair_stays_ordinary() {
    // Same vis/name evolution, but the initializer changes: two
    // changes, not one move — stays a full delete+insert.
    let minus_item = big_const("", "Old");
    let plus_item = big_const("pub", "New").replace("const New: u32 = 1;", "const New: u32 = 2;");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "expr edits veto the const move, got:\n{}",
        out.rsmpl
    );
}

/// A long-enough static body to clear the T375 byte-win guard
/// (doc lines are attrs, so they ride with the static item).
fn big_static(vis: &str, name: &str, is_mut: bool) -> String {
    let mut s = String::new();
    for i in 0..12 {
        s.push_str(&format!("/// pad {i} for byte-win padding text\n"));
    }
    let pre = if vis.is_empty() {
        String::new()
    } else {
        format!("{vis} ")
    };
    let mut_word = if is_mut { "mut " } else { "" };
    s.push_str(&format!("{pre}static {mut_word}{name}: u32 = 1;\n"));
    s
}

#[test]
fn static_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 6: `static Old` → `pub static New` (same type + expr)
    // moving files compresses to a cut + static-template paste and
    // round-trips byte-identical.
    let minus_item = big_static("", "Old", false);
    let plus_item = big_static("pub", "New", false);
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved static cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub static New = ...;"),
        "static template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn static_mut_gain_fires_with_mut_template_and_round_trips() {
    // WI-8 slice 6: `static Old` → `pub static mut New` rides the
    // mutability verbatim in the template, like `fn` qualifiers ride
    // the prefix.
    let minus_item = big_static("", "Old", false);
    let plus_item = big_static("pub", "New", true);
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("+ pub static mut New = ...;"),
        "mut static template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn static_expr_edited_pair_stays_ordinary() {
    // Same vis/name evolution, but the initializer changes: two
    // changes, not one move — stays a full delete+insert.
    let minus_item = big_static("", "Old", false);
    let plus_item =
        big_static("pub", "New", false).replace("static New: u32 = 1;", "static New: u32 = 2;");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "expr edits veto the static move, got:\n{}",
        out.rsmpl
    );
}

/// A long-enough mod to clear the T375 byte-win guard: doc attrs pad
/// the item (they ride with it), while the inline body stays empty —
/// inline mods with nested items stay ordinary (the scalar `- I` cut
/// cannot isolate them).
fn big_mod(vis: &str, name: &str) -> String {
    let mut s = String::new();
    for i in 0..12 {
        s.push_str(&format!("/// pad {i} for byte-win padding text\n"));
    }
    if !vis.is_empty() {
        s.push_str(vis);
        s.push(' ');
    }
    s.push_str(&format!("mod {name} {{}}\n"));
    s
}

#[test]
fn mod_name_change_fires_with_template_paste_and_round_trips() {
    // WI-8 slice 7: `mod old {}` → `pub mod new {}` (doc-padded empty mod)
    // moving files compresses to a cut + mod-template paste and
    // round-trips byte-identical.
    let minus_item = big_mod("", "old");
    let plus_item = big_mod("pub", "new");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("# bind="),
        "evolved mod cut missing, got:\n{}",
        out.rsmpl
    );
    assert!(
        out.rsmpl.contains("+ pub mod new { ... }"),
        "mod template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn mod_unsafe_gain_fires_with_unsafe_template_and_round_trips() {
    // WI-8 slice 7: `mod old` → `pub unsafe mod new` rides the
    // `unsafe` verbatim in the prefix, like `fn` qualifiers ride
    // the prefix.
    let minus_item = big_mod("", "old");
    let plus_item = big_mod("pub unsafe", "new");
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let a_new = "fn keep() {}\n".to_string();
    let b_new = format!("fn other() {{}}\n\n{plus_item}");
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus_blank = "+\n".to_string();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!(
                "@@ -1,0 +2,{} @@\n{plus_blank}{plus}",
                plus_item.lines().count() + 1
            )
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        out.rsmpl.contains("+ pub unsafe mod new { ... }"),
        "unsafe mod template paste missing, got:\n{}",
        out.rsmpl
    );
    let pre_refs: Vec<(&str, &str)> = vec![("a.rs", a_old.as_str()), ("b.rs", b_old.as_str())];
    let post_refs: Vec<(&str, &str)> = vec![("a.rs", a_new.as_str()), ("b.rs", b_new.as_str())];
    assert_round_trip(&out.rsmpl, &pre_refs, &post_refs);
}

#[test]
fn mod_content_edited_pair_stays_ordinary() {
    // A nested item on either side vetoes the mod move: the scalar
    // `- I` cut would match the nested item too, so the pair stays a
    // full delete+insert.
    let minus_item = big_mod("", "old");
    let plus_item = "pub mod new {\n    pub fn f() {}\n}\n".to_string();
    let a_old = format!("fn keep() {{}}\n{minus_item}");
    let b_old = "fn other() {}\n".to_string();
    let minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
    let plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
    let raw = format!(
        "{}{}",
        section(
            "a.rs",
            &format!("@@ -2,{} +2,0 @@\n{minus}", minus_item.lines().count())
        ),
        section(
            "b.rs",
            &format!("@@ -1,0 +2,{} @@\n+\n{plus}", plus_item.lines().count() + 1)
        )
    );
    let pre = pre_map(&[("a.rs", &a_old), ("b.rs", &b_old)]);
    let out = rsmpl_diff_infer::infer_rsmpl_with_pre_images(&raw, &pre).unwrap();
    assert!(
        !out.rsmpl.contains("# bind="),
        "nested items veto the mod move, got:\n{}",
        out.rsmpl
    );
}
