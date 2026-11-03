//! E1 slices 1-2: removed/replaced-axis moves (minus attr+item → plus item).
//!
//! The mirror of the added axis (`move_attr.rs`): a whole-item move whose
//! minus side carries exactly one extra leading attr/doc block re-diffs as
//! delete-cut + wholly ordinary plus. The pairing here claims the stripped
//! minus remainder against the plus run, so the existing rebuild emits a
//! sub-span cut (stripped lines only, new engine
//! `find_stripped_item_target`) with the attr line as an ordinary minus
//! prefix leftover (`-#[attr]`) and a bare `+ M1` paste (slice 1) or an
//! evolved paste (slice 2: the stripped remainder pairs through the shared
//! evolved cascade, so removed+rename hits the abstract arm and
//! replaced+rename hits the plus-added arm with the new attr as an
//! ordinary plus prefix). No pattern or engine spelling beyond the sub-span
//! cut; no new spelling at all.
//!
//! Scope is deliberately narrow: exactly one removed block (single `///`,
//! single `#[...]`, or one balanced multi-line `#[...]` ≤10 lines — the
//! `added_attr_block_len` predicate reused verbatim), any move-eligible
//! kind. Slice 1 covers same-ident bare (removed AND replaced-bare, the
//! latter composing via the plus-side added prefix); slice 2 covers
//! removed/replaced + rename via the stripped evolved delegation below.
//! Slice 3 covered hunk-interior removed (one removed attr between
//! companion cuts, bare or +rename). Slice 4 covers hunk-interior replaced
//! (one removed attr minus-side + one added attr plus-side between
//! companions, bare or +rename): both pairs form regardless of hunk
//! position; both gates admit one single-attr interior gap and both
//! rebuilds emit it as an interior ordinary leftover. Item-interior stays
//! ordinary by design.

use super::move_parts::{MoveItem, MovePair, SectionParts};

/// Raw stripped minus parts: `Some((k, lines, text))` when the first `k`
/// minus-item lines form exactly one attr/doc block and the remainder
/// parses as a single item (any ident — the caller proves the pairing).
/// `None` otherwise (bare minus, unparseable remainder). Kept docs/attrs
/// in the remainder are allowed (the cut cites the stripped span including
/// them; only the first block rides the ordinary minus prefix).
pub(crate) fn stripped_minus_raw(item: &MoveItem) -> Option<(usize, Vec<String>, String)> {
    let refs: Vec<&str> = item.lines.iter().map(|s| s.as_str()).collect();
    let k = super::move_attr::added_attr_block_len(&refs)?;
    if k >= item.lines.len() {
        return None;
    }
    let stripped: Vec<String> = item.lines[k..].to_vec();
    let text = stripped.join("\n");
    let file: syn::File = syn::parse_str(&text).ok()?;
    if file.items.len() != 1 {
        return None;
    }
    super::move_scan::item_ident(&file.items[0])?;
    Some((k, stripped, text))
}

/// Stripped minus parts for the bare (same-ident) axis: the raw strip
/// whose remainder is bare (no leading attr) with the same ident.
/// `None` otherwise (two attrs, bare minus, rename — the rename twins
/// ride the evolved delegation below, not this byte-identical arm).
pub(crate) fn stripped_minus_parts(item: &MoveItem) -> Option<(usize, Vec<String>, String)> {
    let (k, stripped, text) = stripped_minus_raw(item)?;
    // The bare remainder must itself be bare: a second attr would need a
    // second ordinary leftover the rebuild never emits (the cut cites one
    // span). The evolved arm below allows kept attrs (it pairs through
    // the attr-aware matchers, not byte runs).
    if let Some(first) = stripped.first() {
        let t = first.trim_start();
        if t.starts_with("#[") || t.starts_with("///") {
            return None;
        }
    }
    let file: syn::File = syn::parse_str(&text).ok()?;
    if super::move_scan::item_ident(&file.items[0]).as_deref() != Some(item.ident.as_str()) {
        return None;
    }
    Some((k, stripped, text))
}

/// Bare removed pairs for one minus item with no byte-identical plus run.
pub(crate) fn removed_pairs_for_item(
    item: &MoveItem,
    parsed: &[SectionParts],
    plus_hunks: &[(usize, usize)],
    minus_key: (usize, usize),
) -> Vec<MovePair> {
    let Some((k, stripped_lines, stripped_text)) = stripped_minus_parts(item) else {
        return Vec::new();
    };
    // E1 slice 4: no hunk-position gate and no plus-side filter. The strip
    // itself proves the removed block is the first block OF ITS ITEM; the
    // hunk position (top vs interior between companion cuts) is owned by
    // `minus_hunk_holds` + `plus_runs_hold` + both rebuilt interior
    // ordinary leftovers.
    let mut out = Vec::new();
    for (psi, phi) in plus_hunks {
        let pbody = &parsed[*psi].hunks[*phi].body;
        for (first, last) in super::move_scan::find_runs(pbody, &stripped_lines) {
            if !super::move_scan::plus_run_is_item(pbody, first, last) {
                continue;
            }
            out.push(MovePair {
                minus_hunk_key: minus_key,
                plus_hunk_key: (*psi, *phi),
                item: MoveItem {
                    ident: item.ident.clone(),
                    text: stripped_text.clone(),
                    lines: stripped_lines.clone(),
                    first: item.first + k,
                    last: item.last,
                },
                plus_first: first,
                plus_last: last,
                evolved: None,
            });
        }
    }
    out
}

/// Stripped evolved pairs (E1 slice 2): removed/replaced + rename.
///
/// The raw stripped remainder (which may keep docs/attrs and carries the
/// old ident) pairs through the shared evolved cascade against each plus
/// hunk: stripped-bare vs bare-plus hits the abstract arm (removed+rename);
/// stripped-bare vs attr-plus hits the plus-added arm (replaced+rename,
/// the new attr riding the existing ordinary plus prefix). The returned
/// pairs carry the stripped cut item plus the evolved paste template, so
/// the existing rebuild emits a sub-span cut (served by the slice-1 engine
/// `find_stripped_item_target`) with ordinary attr prefixes both sides.
/// The prefix-only guard mirrors the bare arm: interior attrs would strand
/// as uncovered non-blank and veto the sweep, so they stay ordinary.
pub(crate) fn removed_evolved_pairs_for_item(
    item: &MoveItem,
    parsed: &[SectionParts],
    plus_hunks: &[(usize, usize)],
    minus_key: (usize, usize),
) -> Vec<MovePair> {
    let Some((k, _stripped_lines, stripped_text)) = stripped_minus_raw(item) else {
        return Vec::new();
    };
    let file: syn::File = match syn::parse_str(&stripped_text) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    if file.items.len() != 1 {
        return Vec::new();
    }
    let Some(stripped_ident) = super::move_scan::item_ident(&file.items[0]) else {
        return Vec::new();
    };
    // E1 slice 4: no hunk-position gate and no plus-side filter here
    // either (symmetric with the bare arm); position is owned by
    // `minus_hunk_holds` + `plus_runs_hold` + both rebuilt leftovers.
    let stripped = MoveItem {
        ident: stripped_ident,
        text: stripped_text,
        lines: item.lines[k..].to_vec(),
        first: item.first + k,
        last: item.last,
    };
    super::move_pair_evolve::evolved_pairs_for_item(&stripped, parsed, plus_hunks, minus_key)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

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
    fn attr_removed_fn_fires_with_ordinary_minus_prefix() {
        // Minus `#[inline]+fn`, plus bare `fn`: the stripped remainder
        // pairs byte-identical; the cut cites the stripped span while the
        // attr rides an ordinary minus prefix leftover.
        let big = big_fn("foo");
        let minus_item = format!("#[inline]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("attr-removed fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(text.contains("# use="), "paste must fire, got:\n{text}");
        assert!(
            text.contains("-#[inline]\n"),
            "removed attr rides ordinary minus, got:\n{text}"
        );
        assert!(
            text.contains("+ M1\n") || text.contains("+M1\n"),
            "paste must stay bare, got:\n{text}"
        );
        assert!(
            !text.contains("-fn foo() {"),
            "minus fn lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn foo() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn doc_removed_struct_fires() {
        // Struct twin with one removed `///` line.
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
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("doc-removed struct move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("-/// Big container.\n"),
            "removed doc rides ordinary minus, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn multiline_attr_removed_fn_fires() {
        // One balanced 3-line `#[...]` removed joins the same shape.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(\n    clippy::all,\n)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("multi-line removed fn move must fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("-#[allow(\n"),
            "removed attr head rides ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn two_attrs_removed_stays_ordinary() {
        // Exactly one removed block: two removed attrs decline.
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
    fn replaced_axis_fires_with_both_prefixes() {
        // `#[old]+fn` → `#[new]+fn` (same ident) composes for free: the
        // stripped minus remainder hits the bare plus run inside the
        // attr-added shape, so the cut cites the stripped span while the
        // old attr rides an ordinary minus prefix and the new attr rides
        // an ordinary plus prefix around a bare paste.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[inline]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("replaced bare move must fire via stripped cut");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "old attr rides ordinary minus, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "new attr rides ordinary plus, got:\n{text}"
        );
        assert!(
            text.contains("+ M1\n") || text.contains("+M1\n"),
            "paste must stay bare, got:\n{text}"
        );
        assert!(
            !text.contains("-fn foo() {"),
            "minus fn lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn foo() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn removed_plus_rename_fn_fires_with_evolved_paste() {
        // E1 slice 2: removed + rename (`#[old]+foo` → bare `bar`) fires
        // via the stripped evolved delegation (abstract arm): the cut
        // cites the stripped span while the old attr rides an ordinary
        // minus prefix and the paste carries the evolved template.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = renamed.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("removed+rename fn move must fire via stripped evolved");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "removed attr rides ordinary minus, got:\n{text}"
        );
        assert!(
            !text.contains("-fn foo() {"),
            "minus fn lines must be consumed, got:\n{text}"
        );
        assert!(
            !text.contains("+fn bar() {"),
            "plus fn lines must be consumed, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn replaced_plus_rename_fn_fires_with_both_prefixes() {
        // E1 slice 2: replaced + rename (`#[old]+foo` → `#[new]+bar`)
        // fires via the stripped evolved delegation (plus-added arm):
        // stripped-bare pairs past the plus attr block, so both attrs
        // ride ordinary prefixes around an evolved paste.
        let big = big_fn("foo");
        let minus_item = format!("#[allow(dead_code)]\n{big}");
        let renamed = big.replacen("fn foo()", "fn bar()", 1);
        let plus_item = format!("#[inline]\n{renamed}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("replaced+rename fn move must fire via stripped evolved");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut must fire, got:\n{text}");
        assert!(
            text.contains("+ fn bar(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "old attr rides ordinary minus, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "new attr rides ordinary plus, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn hunk_interior_removed_second_moves_with_interior_leftover() {
        // E1 slice 3: two-item minus hunk whose second item carries one
        // removed attr (`[aaa, #[old]+bbb]` → `[aaa, bbb]`). Both move as
        // two cuts; the removed attr rides an interior ordinary minus
        // leftover between them (no partial rescue needed).
        let big_a = big_fn("aaa");
        let big_b = big_fn("bbb");
        let minus_b = format!("#[allow(dead_code)]\n{big_b}");
        let minus_body = format!("{big_a}\n{minus_b}");
        let a_old = format!("fn keep() {{}}\n{minus_body}");
        let a_minus: String = minus_body.lines().map(|l| format!("-{l}\n")).collect();
        let plus_body = format!("{big_a}\n{big_b}");
        let b_plus: String = plus_body.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("hunk-interior removed must fire both moves");
        let text = out.join("");
        assert_eq!(
            text.matches("# bind=").count(),
            2,
            "two cuts must fire, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "removed attr rides interior ordinary minus, got:\n{text}"
        );
        assert!(
            !text.contains("-fn bbb() {"),
            "bbb body must be consumed by its cut, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn hunk_interior_removed_plus_rename_fires() {
        // E1 slice 3 evolved twin: `[aaa, #[old]+bbb]` → `[aaa, ccc]`.
        // aaa rides a bare cut; the renamed bbb rides a stripped evolved
        // cut with the same interior ordinary `-#[old]` leftover.
        let big_a = big_fn("aaa");
        let big_b = big_fn("bbb");
        let minus_b = format!("#[allow(dead_code)]\n{big_b}");
        let minus_body = format!("{big_a}\n{minus_b}");
        let a_old = format!("fn keep() {{}}\n{minus_body}");
        let a_minus: String = minus_body.lines().map(|l| format!("-{l}\n")).collect();
        let renamed = big_b.replacen("fn bbb()", "fn ccc()", 1);
        let plus_body = format!("{big_a}\n{renamed}");
        let b_plus: String = plus_body.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("hunk-interior removed+rename must fire");
        let text = out.join("");
        assert_eq!(
            text.matches("# bind=").count(),
            2,
            "two cuts must fire, got:\n{text}"
        );
        assert!(
            text.contains("+ fn ccc(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "removed attr rides interior ordinary minus, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn hunk_interior_two_removed_stays_partial() {
        // Two removed attrs between companions: the stripped arm declines
        // (inner attr-count mismatch), so only the companion moves —
        // partial rescue preserved, bbb wholly ordinary both sides.
        let big_a = big_fn("aaa");
        let big_b = big_fn("bbb");
        let minus_b = format!("#[a]\n#[b]\n{big_b}");
        let minus_body = format!("{big_a}\n{minus_b}");
        let a_old = format!("fn keep() {{}}\n{minus_body}");
        let a_minus: String = minus_body.lines().map(|l| format!("-{l}\n")).collect();
        let plus_body = format!("{big_a}\n{big_b}");
        let b_plus: String = plus_body.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("companion must still move");
        let text = out.join("");
        assert_eq!(
            text.matches("# bind=").count(),
            1,
            "only the companion cut must fire, got:\n{text}"
        );
        assert!(
            text.contains("-fn bbb() {") && text.contains("+fn bbb() {"),
            "bbb stays ordinary both sides, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn hunk_interior_replaced_moves_with_both_leftovers() {
        // E1 slice 4: `[aaa, #[old]+bbb]` → `[aaa, #[new]+bbb]` — both move
        // as two cuts; the removed attr rides an interior ordinary minus
        // leftover, the added attr an interior ordinary plus leftover.
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
            .expect("hunk-interior replaced must fire both moves");
        let text = out.join("");
        assert_eq!(
            text.matches("# bind=").count(),
            2,
            "two cuts must fire, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "removed attr rides interior ordinary minus, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "added attr rides interior ordinary plus, got:\n{text}"
        );
        assert!(
            !text.contains("-fn bbb() {"),
            "bbb body must be consumed by its cut, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn hunk_interior_replaced_plus_rename_fires() {
        // E1 slice 4 evolved twin: `[aaa, #[old]+bbb]` → `[aaa, #[new]+ccc]`.
        // aaa rides a bare cut; the renamed bbb rides a stripped evolved cut
        // with both interior ordinary leftovers.
        let big_a = big_fn("aaa");
        let big_b = big_fn("bbb");
        let minus_b = format!("#[allow(dead_code)]\n{big_b}");
        let minus_body = format!("{big_a}\n{minus_b}");
        let a_old = format!("fn keep() {{}}\n{minus_body}");
        let a_minus: String = minus_body.lines().map(|l| format!("-{l}\n")).collect();
        let renamed = big_b.replacen("fn bbb()", "fn ccc()", 1);
        let plus_b = format!("#[inline]\n{renamed}");
        let plus_body = format!("{big_a}\n{plus_b}");
        let b_plus: String = plus_body.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&[a, b], &pre, &mut used)
            .expect("hunk-interior replaced+rename must fire");
        let text = out.join("");
        assert_eq!(
            text.matches("# bind=").count(),
            2,
            "two cuts must fire, got:\n{text}"
        );
        assert!(
            text.contains("+ fn ccc(...) { ... }"),
            "paste must carry the evolved template, got:\n{text}"
        );
        assert!(
            text.contains("-#[allow(dead_code)]\n"),
            "removed attr rides interior ordinary minus, got:\n{text}"
        );
        assert!(
            text.contains("+#[inline]\n"),
            "added attr rides interior ordinary plus, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn single_item_kept_plus_removed_stays_ordinary() {
        // Item-interior (`#[keep]+#[old]+fn` → `#[keep]+fn`): no
        // middle-strip exists — the raw strip removes the KEPT leading
        // block, not the interior one — so no pair forms and the move
        // stays wholly ordinary. By design (needs discontiguous cuts).
        let big = big_fn("foo");
        let minus_item = format!("#[keep_attr]\n#[allow(dead_code)]\n{big}");
        let a_old = format!("fn keep() {{}}\n{minus_item}");
        let a_minus: String = minus_item.lines().map(|l| format!("-{l}\n")).collect();
        let plus_item = format!("#[keep_attr]\n{big}");
        let b_plus: String = plus_item.lines().map(|l| format!("+{l}\n")).collect();
        let (a, b, pre) = two_file_sections(&a_old, &a_minus, &b_plus);
        let mut used = Vec::new();
        assert!(
            super::super::moves::try_move_sweep(&[a, b], &pre, &mut used).is_none(),
            "item-interior removed stays ordinary by design"
        );
    }
}
