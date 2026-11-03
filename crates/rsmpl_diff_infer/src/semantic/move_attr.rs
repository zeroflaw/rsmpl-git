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
//! Removed / replaced attrs, two added attrs, multi-line docs,
//! `/* */` / `//!` docs all decline to ordinary (pinned below; slice 4
//! proves the removed mirror needs engine work — the whole-span cut law
//! — so it stays ordinary by construction). Slices 7–8 extend the added
//! axis to move+rename (bare minus, one added leading block, renamed
//! item): `fn` in slice 7, every other move-eligible kind in slice 8,
//! empty `trait` in slice 9 —
//! the inner rename pairs through the abstract evolved matcher in
//! `move_evolve.rs` / `move_evolve_tail.rs` via `added_attr_block_len`
//! below (method-bearing traits veto inside the trait matcher and stay
//! engine-owned). Later WI-14 slices own the remaining axes.

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

/// Leading added attr/doc block length (WI-14 slice 7, evolved reuse):
/// `Some(k)` when the first `k` span lines form exactly one added block —
/// one single-line `///` outer-doc line, one single-line `#[...]` attr,
/// or one balanced multi-line `#[...]` attr (`k <= MAX_ATTR_LINES`) —
/// else `None`. The caller re-parses the remaining lines and proves the
/// pairing through its own matcher, so `Some` never fires alone:
/// two-added shapes fail the downstream inner match (the leftover inner
/// keeps one attr against a bare minus), and replaced/removed shapes
/// never reach here (their attr counts differ by zero, not plus one).
pub(crate) fn added_attr_block_len(span_lines: &[&str]) -> Option<usize> {
    if span_lines.len() < 2 {
        return None;
    }
    let first = span_lines[0].trim_start();
    if first.starts_with("///") {
        return Some(1);
    }
    if first.starts_with("#[")
        && first.trim_end().ends_with(']')
        && first.matches("#[").count() == 1
    {
        return Some(1);
    }
    let max_k = MAX_ATTR_LINES.min(span_lines.len() - 1);
    for k in 2..=max_k {
        if is_balanced_single_attr(&span_lines[..k]) {
            return Some(k);
        }
    }
    None
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
    fn attr_replaced_fn_stays_ordinary_span_law() {
        // WI-14 slice 6 triage: minus `#[old]+fn`, plus `#[new]+fn`
        // stays ordinary BY CONSTRUCTION under the whole-span cut law
        // (same wall as the removed axis, slice 4). `- I` matches the
        // full item span *including* attrs, so a bare-span cut over the
        // shared `fn` matches nothing, while a full-span cut captures
        // the old attr a bare paste would replay. A bare-pair attempt
        // was built, measured, REVERTED (two `validated_live`
        // `no match found` failures — the engine attaches the minus
        // attr to the item span). Stripping needs engine work
        // (sub-span cut or strip-on-paste) — a future WI-14 slice owns
        // it, shared with the removed mirror.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(unused_variables)]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "single-line attr-replaced fn move must stay ordinary (engine span law)"
        );
    }

    #[test]
    fn attr_replaced_struct_stays_ordinary_span_law() {
        // Struct twin of the span-law veto: non-fn spans likewise
        // include attrs, so any move-eligible kind stays ordinary on
        // the replaced axis (companion fire attempt failed
        // `validated_live` with `no match found — nearest struct Big`,
        // then reverted).
        let mut strukt = String::from("struct Big {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let minus_item = format!("#[allow(dead_code)]\n{strukt}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(Clone)]\n{strukt}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "struct attr-replaced move must stay ordinary (engine span law)"
        );
    }

    #[test]
    fn attr_replaced_tiny_stays_ordinary() {
        // Tiny shapes stay ordinary on the replaced axis too (no pair
        // ever forms — span law, so T375 is never even reached).
        let tiny = "fn foo() {}\n";
        let minus_item = format!("#[allow(dead_code)]\n{tiny}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{tiny}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "tiny attr-replaced move must stay ordinary (engine span law)"
        );
    }

    #[test]
    fn attr_replaced_multiline_minus_stays_ordinary() {
        // Multi-line `#[old]` against single-line `#[new]`: same span
        // law (the minus attr is part of the item span), so no pair
        // forms regardless of line counts.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(\n    dead_code,\n)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "multi-line minus replaced attr must stay ordinary"
        );
    }

    #[test]
    fn attr_replaced_doc_stays_ordinary() {
        // Doc replaced (`///` old → `///` new): docs attach to the item
        // span exactly like attrs, so the same span law keeps the pair
        // from forming.
        let big = big_fn("foo");
        let minus_item = format!("/// Old doc.\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("/// New doc.\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "doc-replaced move must stay ordinary"
        );
    }

    #[test]
    fn attr_replaced_two_attrs_minus_stays_ordinary() {
        // Exactly one minus attr: two `#[old]` lines decline even when
        // the plus side carries one `#[new]`.
        let big = big_fn("foo");
        let minus_item = format!("#[inline]\n#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(unused_variables)]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "two-attr minus replaced must stay ordinary"
        );
    }

    #[test]
    fn attr_replaced_multi_item_partial_fires() {
        // Two-item minus hunk with one replaced member: the replaced
        // member forms no pair (span law) but the byte-identical
        // companion still moves — partial rescue. `aaa` rides
        // cut+paste while `bbb` (both attrs + body) stays ordinary on
        // both sides.
        let big_a = big_fn("aaa");
        let big_b = big_fn("bbb");
        let minus_b = format!("#[allow(dead_code)]\n{big_b}");
        let minus_body = format!("{big_a}\n{minus_b}");
        let a_old = format!("fn keep() {{}}\n{minus_body}");
        let a_minus: String = minus_body.lines().map(|l| format!("-{l}\n")).collect();
        let plus_b = format!("#[inline]\n{big_b}");
        let plus_body = format!("{big_a}\n{plus_b}");
        let b_plus: String = plus_body.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("identical companion must still move");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            !text.contains("-fn aaa() {"),
            "moved companion consumed, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "replaced old attr stays ordinary, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "replaced new attr stays ordinary, got:\n{text}"
        );
        assert!(
            text.contains("-fn bbb() {") && text.contains("+fn bbb() {"),
            "replaced body stays ordinary both sides, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn attr_replaced_ident_mention_stays_ordinary_span_law() {
        // Interior mentions change nothing on this axis: the pair never
        // forms (span law), so `#[allow(foo)]` → `#[allow(bar)]` around
        // a moved `fn foo` stays ordinary like every other replaced
        // shape — no opaque-interior reasoning needed raiser-side.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(foo)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(bar)]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "ident-mentioning replaced attrs must stay ordinary"
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
    fn rename_plus_attr_fn_fires_with_ordinary_prefix() {
        // WI-14 slice 7: bare `fn foo` moves a.rs -> b.rs as
        // `#[inline] fn bar`. The minus cut is bare (span law
        // satisfied); the attr rides an ordinary T432b prefix while
        // the inner rename pairs through the abstract evolved matcher
        // — so the paste carries the evolution template, not a bare
        // bind (unlike the pure attr-added slices 1-3/5).
        let big = big_fn("foo");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("rename+attr fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+fn bar() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_doc_fn_fires_with_ordinary_prefix() {
        // Doc twin of the slice-7 shape: bare `fn foo` moves as
        // `/// Moved helper.` + `fn bar` (one added doc + rename).
        let big = big_fn("foo");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("/// Moved helper.\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("rename+doc fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("+/// Moved helper.\n"),
            "doc rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_multiline_attr_fn_fires() {
        // Multi-line twin: bare `fn foo` moves as balanced 3-line
        // attr + `fn bar`; the attr block rides one ordinary prefix.
        let big = big_fn("foo");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(\n    clippy::all,\n)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("rename+multi-line-attr fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("+#[allow(\n"),
            "attr head rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_attr_atop_existing_fires() {
        // Atop-existing twin: `#[allow]+foo` moves as
        // `#[inline]+#[allow]+bar` — the leading block strips, the
        // trailing attr proves equal through the inner match.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let plus_item = format!("#[inline]\n#[allow(dead_code)]\n{renamed}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("rename+attr atop-existing must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+#[inline]\n"),
            "added attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_two_attrs_stays_ordinary() {
        // Exactly one added block: `fn foo` -> two attrs + `fn bar`
        // stays ordinary (inner re-parse keeps one attr vs bare).
        let big = big_fn("foo");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{big}");
        let a_minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n#[allow(dead_code)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "two-added+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_replaced_attr_stays_ordinary() {
        // Replaced axis with rename: `#[old]+foo` -> `#[new]+bar`
        // stays ordinary (attr counts differ by zero, never plus one).
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let plus_item = format!("#[allow(unused_variables)]\n{renamed}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "replaced+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_removed_attr_stays_ordinary() {
        // Removed axis with rename: `#[old]+foo` -> bare `bar` stays
        // ordinary (deferred: stripping the minus side is engine
        // territory, same wall as slice 4's bare removed mirror).
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = renamed.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "removed+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_attr_struct_fires_with_ordinary_prefix() {
        // WI-14 slice 8: `struct Old` moves a.rs -> b.rs as
        // `#[derive(Clone)] struct New`. The minus cut is bare; the attr
        // rides an ordinary T432b prefix while the inner rename pairs
        // through the abstract struct evolved matcher.
        let mut strukt = String::from("struct Old {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let renamed = strukt.replacen("struct Old", "struct New", 1);
        let a_old = format!("fn keep() {{}}\n{strukt}");
        let a_minus: String = strukt.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(Clone)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("struct rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("struct New"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("+#[derive(Clone)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+struct New {"),
            "plus struct lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
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

    #[test]
    fn rename_plus_attr_enum_fires_with_ordinary_prefix() {
        // WI-14 slice 8: `enum Old` -> `#[derive] enum New` fires through
        // the enum evolved matcher with the attr as ordinary prefix.
        let mut en = String::from("enum Old {\n");
        for i in 0..40 {
            en.push_str(&format!("    V{i}(u32),\n"));
        }
        en.push_str("}\n");
        let renamed = en.replacen("enum Old", "enum New", 1);
        let a_old = format!("fn keep() {{}}\n{en}");
        let a_minus: String = en.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(Clone)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("enum rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("enum New"), "paste must evolve, got:\n{text}");
        assert!(
            text.contains("+#[derive(Clone)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_attr_union_fires_with_ordinary_prefix() {
        // WI-14 slice 8: `union Old` -> `#[derive] union New` fires.
        let mut un = String::from("union Old {\n");
        for i in 0..40 {
            un.push_str(&format!("    f{i}: u32,\n"));
        }
        un.push_str("}\n");
        let renamed = un.replacen("union Old", "union New", 1);
        let a_old = format!("fn keep() {{}}\n{un}");
        let a_minus: String = un.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[derive(Clone, Copy)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("union rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("union New"),
            "paste must evolve, got:\n{text}"
        );
        assert!(
            text.contains("+#[derive(Clone, Copy)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_attr_type_fires_with_ordinary_prefix() {
        // WI-14 slice 8 tail: `type Old = ...` -> `#[allow] type New`.
        // Long aliased type keeps the pair T375-clearing.
        let mut ty = String::from("type Old = (");
        for i in 0..30 {
            ty.push_str(&format!("u32, [u8; {i}], "));
        }
        ty.push_str("u64);\n");
        let renamed = ty.replacen("type Old", "type New", 1);
        let a_old = format!("fn keep() {{}}\n{ty}");
        let a_minus: String = ty.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(dead_code)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("type rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("type New"), "paste must evolve, got:\n{text}");
        assert!(
            text.contains("+#[allow(dead_code)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_attr_const_fires_with_ordinary_prefix() {
        // WI-14 slice 8 tail: `const OLD` -> `#[allow] const NEW`.
        let co = "const OLD: [u32; 64] = [1; 64];\n";
        let renamed = co.replacen("OLD", "NEW", 1);
        let a_old = format!("fn keep() {{}}\n{co}");
        let a_minus: String = co.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(dead_code)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("const rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("const NEW"),
            "paste must evolve, got:\n{text}"
        );
        assert!(
            text.contains("+#[allow(dead_code)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_attr_static_fires_with_ordinary_prefix() {
        // WI-14 slice 8 tail: `static OLD` -> `#[allow] static NEW`.
        let st = "static OLD: [u32; 64] = [1; 64];\n";
        let renamed = st.replacen("OLD", "NEW", 1);
        let a_old = format!("fn keep() {{}}\n{st}");
        let a_minus: String = st.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(dead_code)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("static rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("static NEW"),
            "paste must evolve, got:\n{text}"
        );
        assert!(
            text.contains("+#[allow(dead_code)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_attr_mod_stays_ordinary_tiny() {
        // WI-14 slice 8: empty-`mod` rename+attr carries the finder arm
        // (same composition as the other kinds) but stays ordinary under
        // the T375 byte guard — a 1-line `mod old;` plus one attr line
        // can never price the cut+paste below the ordinary hunks.
        // Content-bearing mods decline earlier via `move_cut_eligible`.
        let modu = "mod old {}\n";
        let renamed = modu.replacen("mod old", "mod new", 1);
        let a_old = format!("fn keep() {{}}\n{modu}");
        let a_minus: String = modu.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[allow(dead_code)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "tiny mod rename+attr must stay ordinary (T375)"
        );
    }

    #[test]
    fn rename_plus_two_attrs_struct_stays_ordinary() {
        // Exactly one added block: `struct Old` -> two attrs +
        // `struct New` stays ordinary (inner keeps one attr vs bare).
        let mut strukt = String::from("struct Old {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let renamed = strukt.replacen("struct Old", "struct New", 1);
        let a_old = format!("fn keep() {{}}\n{strukt}");
        let a_minus: String = strukt.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n#[allow(dead_code)]\n{renamed}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "struct two-added+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_replaced_attr_struct_stays_ordinary() {
        // Replaced axis with rename on struct stays ordinary (counts
        // differ by zero, never plus one).
        let mut strukt = String::from("struct Old {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let minus_item = format!("#[allow(dead_code)]\n{strukt}");
        let renamed = strukt.replacen("struct Old", "struct New", 1);
        let plus_item = format!("#[allow(unused)]\n{renamed}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "struct replaced+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_removed_attr_struct_stays_ordinary() {
        // Removed axis with rename on struct stays ordinary (engine
        // span law: the minus side carries the attr).
        let mut strukt = String::from("struct New {\n");
        for i in 0..40 {
            strukt.push_str(&format!("    f{i}: u32,\n"));
        }
        strukt.push_str("}\n");
        let mut old = strukt.clone().replacen("struct New", "struct Old", 1);
        old = format!("#[allow(dead_code)]\n{old}");
        let a_old = format!("fn keep() {{}}\n{old}");
        let a_minus: String = old.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = strukt.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "struct removed+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_attr_tiny_trait_stays_ordinary_t375() {
        // WI-14 slice 9: the finder arm lands, but a 1-line empty
        // trait plus one attr line can never price the cut+paste below
        // the ordinary hunks (T375) — same guard as the tiny-mod pin in
        // slice 8. Padded twins below fire; this pins the guard, not
        // the detector.
        let minus_item = "trait Old {}\n";
        let plus_item = "#[allow(dead_code)]\ntrait New {}\n";
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "tiny trait rename+attr must stay ordinary (T375 guard)"
        );
    }

    fn padded_trait_old() -> (String, String) {
        // 40 `///` pad lines (T375-clearing, same as the empty-trait
        // fire pin in `moves.rs`) + `trait Old {}`.
        let mut docs = String::new();
        for i in 0..40 {
            docs.push_str(&format!("/// doc line {i}\n"));
        }
        let minus_item = format!("{docs}trait Old {{}}\n");
        (docs, minus_item)
    }

    #[test]
    fn rename_plus_attr_trait_fires_with_ordinary_prefix() {
        // WI-14 slice 9: empty `trait Old` moves a.rs -> b.rs as
        // `#[allow(dead_code)] trait New` (docs padded to clear T375).
        // The minus cut is bare; the attr rides an ordinary T432b
        // prefix while the inner rename pairs through the abstract
        // trait evolved matcher.
        let (docs, minus_item) = padded_trait_old();
        let plus_item = format!("#[allow(dead_code)]\n{docs}trait New {{}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("trait rename+attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("trait New"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("+#[allow(dead_code)]\n"),
            "attr rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+trait New {}"),
            "plus trait lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_doc_trait_fires_with_ordinary_prefix() {
        // Doc twin: one added `///` line atop the padded docs + rename
        // fires identically through the same arm.
        let (docs, minus_item) = padded_trait_old();
        let plus_item = format!("/// Moved trait.\n{docs}trait New {{}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("trait rename+doc move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("+/// Moved trait.\n"),
            "doc rides ordinary, got:\n{text}"
        );
        assert!(
            !text.contains("+trait New {}"),
            "plus trait lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_multiline_attr_trait_fires() {
        // Multi-line twin: one balanced 3-line `#[derive(...)]` block
        // atop the padded docs + rename fires through the shared
        // `added_attr_block_len` cap.
        let (docs, minus_item) = padded_trait_old();
        let plus_item = format!("#[derive(\n    Clone,\n    Debug,\n)]\n{docs}trait New {{}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("trait rename+multiline-attr move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("trait New"),
            "paste must carry the evolved template, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_plus_two_attrs_trait_stays_ordinary() {
        // Two added attrs + rename stays ordinary (exactly-one-block
        // gate: the stripped inner keeps one attr against the bare
        // minus, so the inner evolved match vetoes).
        let (docs, minus_item) = padded_trait_old();
        let plus_item = format!("#[allow(dead_code)]\n#[inline]\n{docs}trait New {{}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "trait two-added+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_replaced_attr_trait_stays_ordinary() {
        // Replaced attr + rename stays ordinary (engine span law: the
        // counts are equal, never plus-one, so the arm never reached;
        // same wall as slices 4/6).
        let (docs, minus_item_base) = padded_trait_old();
        let minus_item = format!("#[allow(dead_code)]\n{minus_item_base}");
        let plus_item = format!("#[allow(unused)]\n{docs}trait New {{}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "trait replaced+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_removed_attr_trait_stays_ordinary() {
        // Removed attr + rename stays ordinary (engine span law: the
        // minus side carries the attr).
        let (docs, minus_item_base) = padded_trait_old();
        let minus_item = format!("#[allow(dead_code)]\n{minus_item_base}");
        let plus_item = format!("{docs}trait New {{}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "trait removed+rename must stay ordinary"
        );
    }

    #[test]
    fn rename_plus_attr_method_bearing_trait_stays_ordinary() {
        // Method-bearing trait rename+attr stays ordinary by the
        // `trait_evolved_match` empty-items gate (engine cut law: the
        // scalar `- I` cut cannot isolate a trait with methods).
        let mut docs = String::new();
        for i in 0..40 {
            docs.push_str(&format!("/// doc line {i}\n"));
        }
        let minus_item = format!("{docs}trait Old {{\n    fn foo(&self);\n}}\n");
        let plus_item =
            format!("#[allow(dead_code)]\n{docs}trait New {{\n    fn foo(&self);\n}}\n");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "method-bearing trait rename+attr must stay ordinary (engine-owned)"
        );
    }
}
