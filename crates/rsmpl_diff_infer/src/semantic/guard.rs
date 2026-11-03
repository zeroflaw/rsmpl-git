//! Byte-guard discipline (WI-19, SPEC D.4): one decision point for
//! which detectors ride precision vs strict T375.
//!
//! Split from `semantic/mod.rs` under the 2000-line gate: pure move
//! of the WI-19 guard vocabulary, no behavior change. `mod.rs` keeps
//! the facade (`infer_semantic*`) and re-exports this module's names
//! so `super::`/`semantic::` paths keep working.

/// Byte-win guard discipline (WI-19, SPEC D.4): definition shapes are
/// proved by construction (the engine re-verifies the plus at the
/// reported spans), so they fire under the bounded precision guard
/// (`semantic <= ordinary + PRECISION_GUARD`); sweep shapes whose
/// minus pattern could match wider than the diff keep strict T375
/// (`semantic < ordinary`), where bytes remain the honest proxy for
/// over-match risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Guard {
    Precision,
    Strict,
}

/// One decision point for the guard discipline above (WI-19): every
/// single/pair caller consults this instead of open-coding a byte
/// comparison, so a detector's guard can only change here. Unknown
/// names (future detectors, `prov.rs` lag) default to [`Guard::Strict`].
pub(crate) fn guard_for(detector: &str) -> Guard {
    match detector {
        "item_rename"
        | "member_rename"
        | "assoc_rename"
        | "macro_rules_rename"
        | "fn_slot_sweep"
        | "sig_evolution"
        | "header_evolution"
        | "definition_evolution"
        | "definition_addition"
        | "definition_addition_follow"
        | "variant_addition_follow"
        | "delete"
        | "move"
        | "mod_rename"
        | "item_intent" => Guard::Precision,
        _ => Guard::Strict,
    }
}

/// WI-2b: rendering-normalized native hunk length for guard budgets.
///
/// Real `git diff -U0` elides `,1` (`@@ -1 +1 @@`) while unit
/// convention and the synthetic builders write it explicitly
/// (`@@ -1,1 +1,1 @@`) — 4 bytes that flipped a per-hunk precision
/// verdict by 1 (vis+ret single: body 89 vs elided 72+16=88 veto vs
/// explicit 76+16=92 fire). Guards must price the hunk *span*, not
/// its rendering, so every guard measures the canonical
/// explicit-length header plus the verbatim body/title bytes.
/// Unparseable headers (or a header-only string) fall back to
/// `hunk.len()` — normalization never vetoes. Canonical is the
/// explicit form, so budgets only grow to the unit-test convention
/// and no firing emission can newly veto.
pub(crate) fn normalized_hunk_len(hunk: &str) -> usize {
    let Some((header, rest)) = hunk.split_once('\n') else {
        return hunk.len();
    };
    if !header.starts_with("@@") {
        return hunk.len();
    }
    let Some((old_start, old_len, new_start, new_len, title)) = parse_hunk_header(header) else {
        return hunk.len();
    };
    let canonical = format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@{title}\n");
    canonical.len() + rest.len()
}

/// `@@ -o[,ol] +n[,nl] @@{title}` shapes the differ emits; elided
/// lengths default to 1 (git's `,1` elision). `None` on anything else
/// (the caller falls back to the raw length).
fn parse_hunk_header(header: &str) -> Option<(usize, usize, usize, usize, &str)> {
    let inner = header.strip_prefix("@@")?.split("@@").next()?;
    let mut parts = inner.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let (old_start, old_len) = parse_range(old)?;
    let (new_start, new_len) = parse_range(new)?;
    let title = header.split("@@").nth(2).unwrap_or("");
    Some((old_start, old_len, new_start, new_len, title))
}

/// `start[,len]` with git's elided-`,1` default.
fn parse_range(range: &str) -> Option<(usize, usize)> {
    let mut it = range.split(',');
    let start: usize = it.next()?.parse().ok()?;
    let len: usize = match it.next() {
        Some(l) => l.parse().ok()?,
        None => 1,
    };
    if it.next().is_some() {
        return None;
    }
    Some((start, len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_for_precision_set_matches_spec() {
        // WI-19: definition shapes (proved by construction via the WI-1
        // plus re-match) ride the precision guard; everything else
        // keeps strict T375. This list is the SPEC D.4 vocabulary —
        // adding a detector without classifying it here fails CI.
        for d in [
            "item_rename",
            "member_rename",
            "assoc_rename",
            "macro_rules_rename",
            "fn_slot_sweep",
            "sig_evolution",
            "header_evolution",
            "definition_evolution",
            "definition_addition",
            "definition_addition_follow",
            "variant_addition_follow",
            "delete",
            "move",
            "mod_rename",
            "item_intent",
        ] {
            assert_eq!(guard_for(d), Guard::Precision, "{d} must be precision");
        }
        for d in [
            "call_sweep",
            "call_arg_template",
            "method_arg_template",
            "call_arg_add",
            "call_literal",
            "call_peel",
            "control_template",
            "general_expr",
            "literal_frame",
            "let_template",
            "let_type_add",
            "type_sweep",
            "sequence",
            "repeat_fold",
            "partial_strip",
            "single_block",
            "use_prefix",
            "verbatim",
            "macro_rename",
            "macro_interior",
            "macro_rule_body",
            "macro_enclosing",
            "qualified_call",
            "turbofish",
            "ref_strip",
            "method_same_literal",
            "method_to_fn",
            "method_to_method",
            "trait_method_rename",
            "inherent_method_rename",
            "attr_sweep",
        ] {
            assert_eq!(guard_for(d), Guard::Strict, "{d} must be strict");
        }
        // Unknown detectors (future work, `prov.rs` lag) default strict.
        assert_eq!(guard_for("not_a_detector"), Guard::Strict);
    }

    #[test]
    fn normalized_hunk_len_elided_equals_explicit() {
        // WI-2b live case: the 113-shape native hunk prices 72 elided,
        // 76 explicit — the precision verdict must not depend on it.
        let body = "-fn helper(x: i32) -> i32 {\n+pub fn helper(x: i32) -> i64 {\n";
        let elided = format!("@@ -1 +1 @@\n{body}");
        let explicit = format!("@@ -1,1 +1,1 @@\n{body}");
        assert_eq!(normalized_hunk_len(&elided), explicit.len());
        assert_eq!(normalized_hunk_len(&explicit), explicit.len());
    }

    #[test]
    fn normalized_hunk_len_keeps_title_bytes() {
        // Titles (function context) ride verbatim on both sides of the
        // comparison — only the range rendering canonicalizes.
        let body = "-    fn alpha(x: i32) {\n+    pub fn alpha(x: i32) {\n";
        let elided = format!("@@ -2 +2 @@ impl Svc {{\n{body}");
        let explicit = format!("@@ -2,1 +2,1 @@ impl Svc {{\n{body}");
        assert_eq!(normalized_hunk_len(&elided), normalized_hunk_len(&explicit));
        assert_eq!(normalized_hunk_len(&elided), explicit.len());
        assert!(
            normalized_hunk_len(&elided) > normalized_hunk_len(&format!("@@ -2 +2 @@\n{body}"))
        );
    }

    #[test]
    fn normalized_hunk_len_zero_sides_and_fallback() {
        // Pure-insertion `,0` sides parse (already explicit, identity);
        // garbage and header-only input fall back to the raw length.
        let ins = "@@ -1,0 +1,2 @@\n+a\n+b\n";
        assert_eq!(normalized_hunk_len(ins), ins.len());
        let garbage = "not a hunk at all\n-x\n";
        assert_eq!(normalized_hunk_len(garbage), garbage.len());
        let header_only = "@@ -1 +1 @@";
        assert_eq!(normalized_hunk_len(header_only), header_only.len());
    }
}
