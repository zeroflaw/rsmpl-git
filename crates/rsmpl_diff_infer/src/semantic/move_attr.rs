//! WI-14 slices 1-2: move + exactly one added single-line `#[...]`
//! attr line (D5 sibling of `move_scan.rs`).
//!
//! A verbatim item move whose plus side carries one extra `#[attr]`
//! line above an otherwise identical item re-diffs as delete-cut +
//! wholly ordinary plus: byte-identical `find_runs` matches the item
//! lines, but `plus_run_is_item` declines (sub-span of the attr+item
//! span) and the evolved cascade vetoes every attr delta. This module
//! admits exactly that shape: the claimed plus run is the item span
//! minus its one leading attr line, so the attr line falls into the
//! existing T432b plus-prefix leftover (ordinary `+#[attr]` hunk)
//! while the cut + bare `+ M1` paste carry the move — no pattern or
//! engine change, no new spelling.
//!
//! Scope is deliberately narrow: any move-eligible item kind, exactly
//! one added single-line `#[...]` line. Removed / replaced /
//! multi-line attrs, doc (`///`) additions, and rename+attr (two
//! deltas) all decline to ordinary (pinned below) — later WI-14
//! slices own those axes.

use syn::spanned::Spanned;

use super::move_parts::line_text;

/// True when `body[first..=last]` (0-based hunk-body lines) is a parsed
/// move-eligible item span minus exactly one leading single-line
/// `#[...]` attr line. The caller guarantees the run textually matches
/// a minus item; this checks the plus-side shape only: an item whose
/// span starts exactly one line above the run, that line is one
/// `#[...]` line, and the remaining item lines equal the run lines
/// verbatim. Content-bearing `mod` / method-bearing `trait` decline
/// via the shared `move_cut_eligible` gate (the cut cannot carry
/// them); ident-less items (use/macro/trait-impl) decline since no
/// minus item ever pairs them.
pub(crate) fn plus_run_is_attr_added_fn(body: &[String], first: usize, last: usize) -> bool {
    if first == 0 || last < first || last >= body.len() {
        return false;
    }
    let attr_idx = first - 1;
    let attr = line_text(&body[attr_idx]).trim();
    if !(attr.starts_with("#[") && attr.ends_with(']')) {
        return false;
    }
    let fragment: String = body
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return false;
    };
    for it in &file.items {
        if !super::move_scan::move_cut_eligible(it) {
            continue;
        }
        if super::move_scan::item_ident(it).is_none() {
            continue;
        }
        // syn spans are 1-based and include outer attrs: the item must
        // start on the attr line and end on the run's last line.
        if it.span().start().line != attr_idx + 1 || it.span().end().line != last + 1 {
            continue;
        }
        let item_lines: Vec<&str> = fragment
            .lines()
            .skip(attr_idx)
            .take(last - attr_idx + 1)
            .collect();
        if item_lines.len() != last - attr_idx + 1 {
            continue;
        }
        if item_lines[0].trim() != attr {
            continue;
        }
        let run: Vec<&str> = body[first..=last].iter().map(|l| line_text(l)).collect();
        if item_lines[1..] == run {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// T375-clearing `fn` body (the move pre-pass only fires when the
    /// cut+paste form is byte-smaller than the ordinary hunks).
    fn big_fn(name: &str) -> String {
        let mut s = format!("fn {name}() {{\n");
        for i in 0..40 {
            s.push_str(&format!("    let v{i} = {i};\n"));
        }
        s.push_str("}\n");
        s
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

    fn two_file_sections(
        a_old: &str,
        minus_body: &str,
        plus_body: &str,
    ) -> (String, String, HashMap<String, String>) {
        let a = format!("diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,42 +2,0 @@\n{minus_body}");
        let n = plus_body.lines().count();
        let b = format!("diff --git b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,0 +2,{n} @@\n{plus_body}");
        let pre = pre_map(&[("a.rs", a_old), ("b.rs", "use x;\n")]);
        (a, b, pre)
    }

    #[test]
    fn attr_added_fn_move_fires_with_ordinary_attr_prefix() {
        // WI-14 slice 1: `fn foo` moves a.rs -> b.rs gaining `#[inline]`.
        // The cut + bare paste carry the move; the attr line rides an
        // ordinary prefix hunk above the paste.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+#[inline]\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("attr-added fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("+ M1\n") || text.contains("+M1\n"),
            "paste must stay bare, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("-fn foo() {"),
            "minus lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn foo() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn attr_added_above_existing_attr_fires() {
        // Minus `#[allow]+fn`, plus `#[inline]+#[allow]+fn`: the needle
        // includes the old attr, the new attr is the one-line prefix.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{minus_item}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("attr-above-attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+#[inline]\n"),
            "new attr rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+fn foo() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn two_attrs_added_stays_ordinary() {
        // Exactly one attr line: two added attrs decline.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+#[inline]\n+#[allow(dead_code)]\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "two added attrs must stay ordinary"
        );
    }

    #[test]
    fn attr_replaced_stays_ordinary() {
        // Minus `#[a]+fn`, plus `#[b]+fn`: no textual hit, evolved arms
        // veto — later slice territory.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "replaced attr must stay ordinary"
        );
    }

    #[test]
    fn attr_removed_stays_ordinary() {
        // Minus `#[a]+fn`, plus bare `fn`: the bare paste would replay
        // the captured attr — decline (later slice strips on paste).
        let big = big_fn("foo");
        let minus_item = format!("#[inline]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "removed attr must stay ordinary"
        );
    }

    #[test]
    fn struct_attr_added_fires_with_ordinary_prefix() {
        // WI-14 slice 2: any move-eligible item kind rides the same
        // T432b leftover shape as `fn` — the attr line stays ordinary
        // above the bare paste.
        let mut strukt = String::from("struct Big {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let a_old = format!("fn keep() {{}}\n{strukt}");
        let a_minus: String = strukt.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(Clone)]\n{strukt}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("struct attr-added move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+#[derive(Clone)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+struct Big {"),
            "plus struct lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn enum_attr_added_fires_with_ordinary_prefix() {
        // Second-kind twin: enum + one added attr fires identically.
        let mut en = String::from("enum Big {\n");
        for i in 0..40 {
            en.push_str(&format!("    V{i}(u32),\n"));
        }
        en.push_str("}\n");
        let a_old = format!("fn keep() {{}}\n{en}");
        let a_minus: String = en.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(Clone)]\n{en}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("enum attr-added move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+#[derive(Clone)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn content_bearing_mod_attr_added_stays_ordinary() {
        // The shared `move_cut_eligible` gate still declines
        // content-bearing `mod` (the scalar cut cannot carry it).
        let modu = "mod m {\n    pub fn f() {}\n}\n";
        let a_old = format!("fn keep() {{}}\n{modu}");
        let a_minus: String = modu.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(dead_code)]\n{modu}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "content-bearing mod attr-added must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_attr_stays_ordinary() {
        // Two deltas (rename + attr add) never join in slices 1-2.
        let big = big_fn("foo");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used);
        assert!(
            out.is_none() || !out.unwrap().join("").contains("# bind="),
            "rename+attr must not claim a move"
        );
    }

    #[test]
    fn doc_comment_added_stays_ordinary() {
        // Slices 1-2 cover `#[...]` only; `///` additions stay ordinary.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+/// Adds one.\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "doc-comment addition must stay ordinary"
        );
    }

    #[test]
    fn multiline_attr_added_stays_ordinary() {
        // The added attr must fit on one line.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+#[allow(\n+    clippy::all,\n+)]\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "multi-line attr must stay ordinary"
        );
    }

    #[test]
    fn helper_single_line_gate() {
        // Direct helper pins: `#[x]` fires, `///` never does.
        let body: Vec<String> = ["+#[inline]", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(plus_run_is_attr_added_fn(&body, 1, 2));
        let doc: Vec<String> = ["+/// Doc.", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!plus_run_is_attr_added_fn(&doc, 1, 2));
        assert!(!plus_run_is_attr_added_fn(&body, 0, 2));
    }
}
