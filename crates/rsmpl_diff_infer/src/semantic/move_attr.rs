//! WI-14 slices 1-3 + slice 5: move + exactly one added `#[...]`
//! (single-line or one balanced multi-line) or outer-doc (`///`) line
//! (D5 sibling of `move_scan.rs`).
//!
//! A verbatim item move whose plus side carries one extra attr/doc
//! line above an otherwise identical item re-diffs as delete-cut +
//! wholly ordinary plus: byte-identical `find_runs` matches the item
//! lines, but `plus_run_is_item` declines (sub-span of the attr+item
//! span) and the evolved cascade vetoes every attr delta. This module
//! admits exactly that shape: the claimed plus run is the item span
//! minus its leading attr/doc lines, so the attr/doc lines fall into
//! the existing T432b plus-prefix leftover (ordinary `+#[attr]` / `+///`
//! hunk) while the cut + bare `+ M1` paste carry the move — no pattern
//! or engine change, no new spelling.
//!
//! Scope is deliberately narrow: any move-eligible item kind, exactly
//! one added single `#[...]` attr (one line, or one balanced attr
//! spanning up to `MAX_ATTR_LINES` lines — slice 5) or one `///` line.
//! Removed / replaced attrs, two added attrs, multi-line docs, `/* */` /
//! `//!` docs, attr interiors mentioning the moved ident, and rename+attr
//! (two deltas) all decline to ordinary (pinned below; slice 4 proves the
//! removed mirror needs engine work — the whole-span cut law — so it
//! stays ordinary by construction). Later WI-14 slices own those axes.

use syn::spanned::Spanned;

use super::move_parts::line_text;

/// Cap on the added-attr lookback (raising-scale bound, WI-21): a
/// single `#[...]` attr longer than this stays ordinary by construction.
const MAX_ATTR_LINES: usize = 10;

/// Balanced single `#[...]` attr over the candidate lines: exactly one
/// `#[` occurrence, first line opens it, last line closes it, and `[` /
/// `]` depth returns to zero exactly at the end without going negative.
/// String literals inside attrs are rare; the syn span check below
/// absorbs any textual false positive (unbalanced text never spans as
/// one attr+item), so a hostile interior stays ordinary rather than
/// misfiring.
fn is_balanced_single_attr(lines: &[&str]) -> bool {
    if lines.is_empty() {
        return false;
    }
    if !lines[0].trim_start().starts_with("#[") {
        return false;
    }
    if !lines[lines.len() - 1].trim_end().ends_with(']') {
        return false;
    }
    let joined = lines.join("\n");
    if joined.matches("#[").count() != 1 {
        return false;
    }
    // No doc lines inside an attr run (mixed doc+attr is two deltas).
    if lines.iter().any(|l| l.trim_start().starts_with("///")) {
        return false;
    }
    let mut depth: i32 = 0;
    for c in joined.chars() {
        if c == '[' {
            depth += 1;
        } else if c == ']' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
    }
    depth == 0
}

/// True when `body[first..=last]` (0-based hunk-body lines) is a parsed
/// move-eligible item span minus exactly one leading added attr/doc
/// block: either one single-line `///` outer-doc line (slices 1-3), or
/// one balanced single-line or multi-line `#[...]` attr (slice 5 widens
/// to multi-line). The caller guarantees the run textually matches a
/// minus item; this checks the plus-side shape only: an item whose span
/// starts exactly `k` lines above the run, those `k` lines form the one
/// added attr/doc block, and the remaining item lines equal the run
/// lines verbatim. Content-bearing `mod` / method-bearing `trait` decline
/// via the shared `move_cut_eligible` gate (the cut cannot carry them);
/// ident-less items (use/macro/trait-impl) decline since no minus item
/// ever pairs them.
pub(crate) fn plus_run_is_attr_added_item(body: &[String], first: usize, last: usize) -> bool {
    if first == 0 || last < first || last >= body.len() {
        return false;
    }
    // Fast path: exactly one single-line `///` doc line (unchanged).
    let one_above = line_text(&body[first - 1]).trim().to_string();
    let single_doc = one_above.starts_with("///");
    // Fast path: exactly one single-line `#[...]` attr (unchanged).
    let single_attr = one_above.starts_with("#[")
        && one_above.ends_with(']')
        && one_above.matches("#[").count() == 1;
    // Slice-5 path: one balanced multi-line `#[...]` attr over k lines.
    // `//!` is an inner doc of the enclosing scope (syn leaves the item
    // span starting at the item line), so it never satisfies the span
    // check below and stays ordinary by construction.
    let max_k = MAX_ATTR_LINES.min(first);
    let mut attr_ks: Vec<usize> = Vec::new();
    if single_doc || single_attr {
        attr_ks.push(1);
    }
    for k in 2..=max_k {
        let cand: Vec<&str> = body[first - k..first]
            .iter()
            .map(|l| line_text(l))
            .collect();
        if is_balanced_single_attr(&cand) {
            attr_ks.push(k);
        }
    }
    // No single-doc / single-attr / balanced multi-line attr above.
    if attr_ks.is_empty() {
        // Keep the old plain-`//` / `//!` decline loud: neither enters
        // `attr_ks` (no `#[`, no `///`), so they fall through here.
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
        };
        // syn spans are 1-based and include outer attrs: the item must
        // start on the attr-block first line and end on the run's last
        // line. Each candidate k is tried; the span check (exact) keeps
        // two-attr shapes ordinary (a k covering only the second attr
        // starts below the true item start; a k covering both carries
        // two `#[` and never enters `attr_ks`).
        for k in &attr_ks {
            let attr_start = first - k;
            if it.span().start().line != attr_start + 1 || it.span().end().line != last + 1 {
                continue;
            }
            let item_lines: Vec<&str> = fragment
                .lines()
                .skip(attr_start)
                .take(last - attr_start + 1)
                .collect();
            if item_lines.len() != last - attr_start + 1 {
                continue;
            }
            let attr_lines: Vec<&str> = body[attr_start..first]
                .iter()
                .map(|l| line_text(l))
                .collect();
            if item_lines[..*k] != attr_lines {
                continue;
            }
            let run: Vec<&str> = body[first..=last].iter().map(|l| line_text(l)).collect();
            if item_lines[*k..] != run {
                continue;
            }
            // No ident-mention veto on this axis: the added attr/doc
            // lines ride an ordinary T432b leftover (never rewritten by
            // the engine), so an interior mention reproduces byte-exact
            // via the ordinary hunk. The opaque-interior law binds the
            // future evolved-paste axes (replaced/joint), not the added
            // axis.
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
        // Minus `#[a]+fn`, plus bare `fn`: stays ordinary BY
        // CONSTRUCTION (WI-14 slice 4 span law). The engine whole-span
        // cut law blocks the T432c mirror: `- I` matches the full item
        // span *including* attrs, so a bare-span cut matches nothing
        // (probed: `no match found — nearest fn foo at line 3`), while
        // a full-span cut captures the attr and a bare `+ M1` paste
        // would replay it. Stripping needs engine work (sub-span cut
        // or strip-on-paste) — a future WI-14 slice owns it.
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
    fn doc_added_fn_move_fires_with_ordinary_doc_prefix() {
        // WI-14 slice 3: `fn foo` moves a.rs -> b.rs gaining one
        // single-line `///` doc line. The cut + bare paste carry the
        // move; the doc line rides an ordinary prefix hunk above the
        // paste (same T432b shape as `#[...]` slices 1-2).
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+/// Moved helper.\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("doc-added fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("+ M1\n") || text.contains("+M1\n"),
            "paste must stay bare, got:\n{text}"
        );
        assert!(
            text.contains("+/// Moved helper.\n"),
            "doc rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+fn foo() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn doc_added_struct_fires_with_ordinary_prefix() {
        // Second-kind twin: struct + one added `///` doc line fires
        // identically (any move-eligible kind rides the shape).
        let mut strukt = String::from("struct Big {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let a_old = format!("fn keep() {{}}\n{strukt}");
        let a_minus: String = strukt.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("/// Big container.\n{strukt}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("struct doc-added move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+/// Big container.\n"),
            "doc rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+struct Big {"),
            "plus struct lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn two_docs_added_stays_ordinary() {
        // Exactly one doc line: two added doc lines decline.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+/// First.\n+/// Second.\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "two added doc lines must stay ordinary"
        );
    }

    #[test]
    fn multiline_attr_added_fn_move_fires() {
        // WI-14 slice 5: one balanced multi-line `#[...]` attr joins the
        // same T432b shape as the single-line slices — cut + bare paste
        // carry the move, the 3 attr lines ride an ordinary prefix hunk.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+#[allow(\n+    clippy::all,\n+)]\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("multi-line attr-added fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("+ M1\n") || text.contains("+M1\n"),
            "paste must stay bare, got:\n{text}"
        );
        assert!(
            text.contains("+#[allow(\n"),
            "attr head rides ordinary, got:\n{text}"
        );
        assert!(
            text.contains("+)]\n"),
            "attr tail rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+fn foo() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn multiline_attr_added_struct_fires() {
        // Second-kind twin: struct + one multi-line attr fires identically.
        let mut strukt = String::from("struct Big {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let a_old = format!("fn keep() {{}}\n{strukt}");
        let a_minus: String = strukt.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(\n    Clone,\n)]\n{strukt}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("struct multi-line attr-added move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+#[derive(\n"),
            "attr head rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+struct Big {"),
            "plus struct lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn attr_mentioning_ident_still_fires_added_axis() {
        // Added-axis law: the added attr rides an ordinary T432b leftover
        // (never rewritten), so an interior mention of the moved ident
        // reproduces byte-exact via the ordinary hunk. The
        // opaque-interior veto binds the future replaced/joint axes, not
        // this one.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+#[allow(\n+    foo,\n+)]\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("ident-mentioning added attr must still fire (ordinary leftover)");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn two_attrs_one_multiline_stays_ordinary() {
        // Exactly one attr: a single-line plus a multi-line attr decline.
        let big = big_fn("foo");
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let mut b_plus = String::from("+#[inline]\n+#[allow(\n+    clippy::all,\n+)]\n");
        b_plus.push_str(&big.lines().map(|l| format!("+{l}\n")).collect::<String>());
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "two added attrs (one multi-line) must stay ordinary"
        );
    }

    #[test]
    fn helper_single_line_gate() {
        // Direct helper pins: `#[x]` and `///` fire; plain `//` never
        // does (trivia, outside the item span), and neither does `//!`
        // (inner doc of the enclosing scope, item span starts below it).
        // Slice 5 adds the balanced multi-line `#[...]` fire pin.
        let body: Vec<String> = ["+#[inline]", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(plus_run_is_attr_added_item(&body, 1, 2));
        let doc: Vec<String> = ["+/// Doc.", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(plus_run_is_attr_added_item(&doc, 1, 2));
        let multi: Vec<String> = ["+#[allow(", "+    clippy::all,", "+)]", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(plus_run_is_attr_added_item(&multi, 3, 4));
        let unbalanced: Vec<String> = ["+#[allow(", "+    clippy::all,", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!plus_run_is_attr_added_item(&unbalanced, 2, 3));
        let inner: Vec<String> = ["+//! Mod doc.", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!plus_run_is_attr_added_item(&inner, 1, 2));
        let plain: Vec<String> = ["+// lead", "+fn f() {", "+}"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!plus_run_is_attr_added_item(&plain, 1, 2));
        assert!(!plus_run_is_attr_added_item(&body, 0, 2));
    }

    #[test]
    fn doc_removed_struct_stays_ordinary_span_law() {
        // Struct twin of the span-law veto: `///`-doc removal rides the
        // same whole-span cut law (non-fn spans likewise include attrs),
        // so any move-eligible kind stays ordinary on the removed axis.
        let mut strukt = String::from("struct Big {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let minus_item = format!("/// Big container.\n{strukt}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = strukt.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "doc-removed move must stay ordinary (engine span law)"
        );
    }

    #[test]
    fn two_attrs_removed_stays_ordinary() {
        // Exactly one removed line: two removed attrs decline.
        let big = big_fn("foo");
        let minus_item = format!("#[inline]\n#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "two removed attrs must stay ordinary"
        );
    }

    #[test]
    fn multiline_attr_removed_stays_ordinary() {
        // The removed attr must fit on one line.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(\n    clippy::all,\n)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "multi-line removed attr must stay ordinary"
        );
    }
}
