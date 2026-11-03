//! WI-4 slice 1: whole-path `# symbol` matching for the general matcher
//! (SPEC 19.3).
//!
//! Split from `matcher_types.rs` per the D5 size gate. The general matcher
//! (`let`-init callees, general-expr callees, general-type paths) treats a
//! single-segment pattern path naming a `# symbol` declaration as a
//! whole-path use: with `# fallback=syntax` the site must spell the seed
//! path exactly (leading-`::` parity, no generic args either side); with a
//! bare `# symbol` the site's spelled path must resolve through the crate
//! graph to the same canonical definition (short spellings via imports
//! resolve; vendored copies with the same text do not). Plus-only seeds
//! (naming the rewrite target) are never verified. `qself` sites are
//! opaque and never match; turbofish/prefix uses are rejected at compile
//! time (pattern crate C14), with defense-in-depth `false` here.

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};

use crate::matcher_general::MatchCx;
use crate::spell_normalize::has_bare_token;

/// Verify minus-side seeds against the crate graph (plus-only seeds name
/// the rewrite target and are skipped — same law as the call matcher in
/// `matcher_expr`). Returns the seed → canonical map for per-site checks.
/// Unresolvable seeds fail with `RSMPL_SYMBOL_CONFLICT`, never silent.
pub(crate) fn seed_canons_for(
    symbols: &HashMap<String, String>,
    minus_sub: &str,
    graph: &crate::CrateGraph,
    rule: &str,
) -> Result<HashMap<String, String>, StructuredError> {
    let mut canons = HashMap::new();
    for (name, seed) in symbols {
        let token = format!("__RSMPL_{name}");
        if !has_bare_token(minus_sub, &token) {
            continue;
        }
        match graph.resolve_seed(seed) {
            Some(canon) => {
                canons.insert(name.clone(), canon);
            }
            None => {
                return Err(StructuredError::new(
                    ErrorCode::SymbolConflict,
                    format!("symbol `{name}` (`{seed}`) resolves to no definition in this crate"),
                )
                .with_rule(rule)
                .with_suggestion(format!(
                    "write the concrete path `{seed}` in the pattern, or add `# fallback=syntax` to allow syntactic matching"
                ))
                .with_repair(StructuredError::add_directive_repair(
                    "# fallback=syntax",
                )));
            }
        }
    }
    Ok(canons)
}

/// Whole-path symbol verdict for a pattern placeholder naming a symbol.
/// Returns `None` when `name` is an ordinary placeholder (the caller runs
/// its historical arm); `Some(verdict)` otherwise. Bindings are pre-bound
/// to seed paths by the hunk driver, so a verdict never mutates them —
/// mirroring the call matcher's resolved arm.
pub(crate) fn match_symbol_placeholder(
    cx: &MatchCx,
    name: &str,
    tgt_leading: bool,
    tgt_segs: &[String],
) -> Option<bool> {
    let seed = cx.symbols.get(name)?;
    if cx.fallback_syntax {
        let seed_leading = seed.starts_with("::");
        if seed_leading != tgt_leading {
            return Some(false);
        }
        let seed_segs: Vec<&str> = seed.trim_start_matches("::").split("::").collect();
        if seed_segs.len() != tgt_segs.len() {
            return Some(false);
        }
        return Some(seed_segs.iter().zip(tgt_segs.iter()).all(|(a, b)| a == b));
    }
    let (Some((graph, file)), Some(want)) = (cx.symbol_graph.as_ref(), cx.seed_canons.get(name))
    else {
        return Some(false);
    };
    match graph.resolve_call(file, tgt_leading, tgt_segs) {
        Some(got) => Some(got == *want),
        None => Some(false),
    }
}

/// Whole-type symbol verdict for `match_type` / `match_generic_arg`.
/// Only a `qself`-free `syn::Type::Path` can name a symbol; every other
/// target kind (references, tuples, `qself` types) is not a path use.
pub(crate) fn match_symbol_type(cx: &MatchCx, name: &str, tgt: &syn::Type) -> bool {
    if !cx.symbols.contains_key(name) {
        return false;
    }
    if let syn::Type::Path(tp) = tgt {
        if tp.qself.is_some() {
            return false;
        }
        if tp
            .path
            .segments
            .iter()
            .any(|s| !matches!(s.arguments, syn::PathArguments::None))
        {
            return false;
        }
        let segs: Vec<String> = tp
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        return match_symbol_placeholder(cx, name, tp.path.leading_colon.is_some(), &segs)
            .unwrap_or(false);
    }
    false
}

/// WI-4 slice 2a: prefix verdict for a pattern path whose FIRST segment
/// names a symbol and concrete suffix segments follow (`Old::suffix`,
/// `Old::<T>::suffix`). The compile gate (C14 skeleton check) admits
/// only a single leading symbol with an otherwise placeholder-free
/// suffix; this arm proves the site.
///
/// The pattern head is ONE segment that expands to the seed's N
/// segments on emission, so raw segment counts never compare directly.
/// Instead the target splits into head + tail, where the tail is the
/// last suffix.len() segments:
/// - fallback: the head must spell the seed exactly (leading-`::`
///   parity on the whole, head idents == seed segments) and the tail
///   must spell the suffix; stem turbofish/args compare structurally
///   (ordinary holes bind, nested symbols verify — the same arms as
///   whole-path types);
/// - resolved: `graph.resolve_call` on the target's full spelled path
///   must equal `seed_canon + "::" + suffix` (the head spelling may be
///   short via imports or full — resolution owns it); tail idents and
///   all args still compare as above.
///   `qself` never reaches here (callers compare it before `match_path`).
///   Returns `None` when the pattern path is not a symbol-led prefix
///   (caller falls through to the per-segment logic, which rejects
///   symbols).
pub(crate) fn match_symbol_prefix(
    cx: &mut crate::matcher_general::MatchCx,
    pat: &syn::Path,
    tgt: &syn::Path,
) -> Option<bool> {
    use crate::matcher_general::as_placeholder;
    use crate::matcher_types::match_path_args;
    if pat.segments.len() < 2 || tgt.segments.is_empty() {
        return None;
    }
    let head_id = pat.segments[0].ident.to_string();
    let name = as_placeholder(&cx.reverse, &head_id).cloned()?;
    if !cx.symbols.contains_key(&name) {
        return None;
    }
    // Suffix idents are compile-gated concrete; a placeholder ident in
    // the suffix is a loud `false` here — defense in depth, never a
    // bind. Placeholders inside suffix ARGS (`Old::make::<T>`) pass:
    // the tail comparison below binds ordinary holes and verifies
    // nested symbols structurally.
    for s in pat.segments.iter().skip(1) {
        let id = s.ident.to_string();
        if as_placeholder(&cx.reverse, &id).is_some() {
            return Some(false);
        }
    }
    let suffix: Vec<&syn::PathSegment> = pat.segments.iter().skip(1).collect();
    // The pattern head is ONE segment that expands to the seed's N
    // segments on emission, so raw segment counts never compare
    // directly. The target splits into head + tail, where the tail is
    // the last suffix.len() segments; the head spelling may be short
    // (via imports) or full — resolution (or exact head text) owns it.
    let tgt_all: Vec<&syn::PathSegment> = tgt.segments.iter().collect();
    if tgt_all.len() < suffix.len() + 1 {
        return Some(false);
    }
    let split = tgt_all.len() - suffix.len();
    let (tgt_head, tgt_tail) = (&tgt_all[..split], &tgt_all[split..]);
    // Tail idents compare textually (concrete in slice 2a); tail args
    // compare structurally (same verdict for concrete args).
    for (ps, ts) in suffix.iter().zip(tgt_tail.iter()) {
        if ps.ident != ts.ident {
            return Some(false);
        }
        if !match_path_args(&ps.arguments, &ts.arguments, cx) {
            return Some(false);
        }
    }
    // Stem turbofish/args ride the last head segment (structural —
    // ordinary holes bind, nested symbols verify).
    let Some(tgt_head_last) = tgt_head.last() else {
        return Some(false);
    };
    if !match_path_args(&pat.segments[0].arguments, &tgt_head_last.arguments, cx) {
        return Some(false);
    }
    let seed = cx.symbols.get(&name).cloned().unwrap_or_default();
    if cx.fallback_syntax {
        // Exact full-path text: the head spells the seed, the tail
        // spells the suffix (proven above), with whole-path leading
        // parity.
        let seed_leading = seed.starts_with("::");
        if seed_leading != tgt.leading_colon.is_some() {
            return Some(false);
        }
        let seed_segs: Vec<&str> = seed.trim_start_matches("::").split("::").collect();
        if seed_segs.len() != tgt_head.len() {
            return Some(false);
        }
        let tgt_head_ids: Vec<String> = tgt_head.iter().map(|s| s.ident.to_string()).collect();
        return Some(
            seed_segs
                .iter()
                .zip(tgt_head_ids.iter())
                .all(|(a, b)| a == b),
        );
    }
    let (graph, file) = match cx.symbol_graph.as_ref() {
        Some((g, f)) => (*g, *f),
        None => return Some(false),
    };
    let want = match cx.seed_canons.get(&name) {
        Some(w) => w.clone(),
        None => return Some(false),
    };
    let tgt_ids: Vec<String> = tgt_all.iter().map(|s| s.ident.to_string()).collect();
    let suffix_ids: Vec<String> = suffix.iter().map(|s| s.ident.to_string()).collect();
    let composed = format!("{want}::{}", suffix_ids.join("::"));
    match graph.resolve_call(file, tgt.leading_colon.is_some(), &tgt_ids) {
        Some(got) => Some(got == composed),
        None => Some(false),
    }
}
/// declared names pre-bound to their seed paths (so the plus side emits
/// the concrete seed, exactly the call-hunk posture), plus the graph and
/// verified canons for resolved per-site checks.
pub(crate) fn symbol_cx<'a>(
    original: &'a str,
    index: &'a crate::LineIndex,
    reverse: HashMap<String, String>,
    symbols: &HashMap<String, String>,
    fallback_syntax: bool,
    symbol_graph: Option<(&'a crate::CrateGraph, &'a str)>,
    seed_canons: &HashMap<String, String>,
) -> MatchCx<'a> {
    let mut cx = MatchCx::with_reverse(original, index, reverse);
    cx.symbols = symbols.clone();
    cx.fallback_syntax = fallback_syntax;
    cx.symbol_graph = symbol_graph;
    cx.seed_canons = seed_canons.clone();
    for (n, p) in symbols {
        cx.bindings.insert(n.clone(), p.clone());
    }
    cx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx_with<'a>(
        original: &'a str,
        index: &'a crate::LineIndex,
        symbols: &[(&str, &str)],
        fallback: bool,
        graph: Option<(&'a crate::CrateGraph, &'a str)>,
        canons: HashMap<String, String>,
    ) -> MatchCx<'a> {
        let mut cx = MatchCx::with_reverse(original, index, HashMap::new());
        for (n, p) in symbols {
            cx.symbols.insert(n.to_string(), p.to_string());
        }
        cx.fallback_syntax = fallback;
        cx.symbol_graph = graph;
        cx.seed_canons = canons;
        cx
    }

    #[test]
    fn fallback_exact_text_fires() {
        let index = crate::LineIndex::new("");
        let cx = cx_with(
            "",
            &index,
            &[("Old", "crate::old_api")],
            true,
            None,
            HashMap::new(),
        );
        assert_eq!(
            match_symbol_placeholder(
                &cx,
                "Old",
                false,
                &["crate".to_string(), "old_api".to_string()]
            ),
            Some(true)
        );
    }

    #[test]
    fn fallback_short_spelling_misses() {
        // Exact full-path text (SPEC 19.3): a bare `old_api` site does not
        // match `# symbol Old=crate::old_api` under fallback.
        let index = crate::LineIndex::new("");
        let cx = cx_with(
            "",
            &index,
            &[("Old", "crate::old_api")],
            true,
            None,
            HashMap::new(),
        );
        assert_eq!(
            match_symbol_placeholder(&cx, "Old", false, &["old_api".to_string()]),
            Some(false)
        );
    }

    #[test]
    fn fallback_leading_parity() {
        let index = crate::LineIndex::new("");
        let cx = cx_with(
            "",
            &index,
            &[("Old", "crate::old_api")],
            true,
            None,
            HashMap::new(),
        );
        assert_eq!(
            match_symbol_placeholder(
                &cx,
                "Old",
                true,
                &["crate".to_string(), "old_api".to_string()]
            ),
            Some(false)
        );
    }

    #[test]
    fn ordinary_placeholder_falls_through() {
        let index = crate::LineIndex::new("");
        let cx = cx_with("", &index, &[], true, None, HashMap::new());
        assert_eq!(
            match_symbol_placeholder(&cx, "E", false, &["u8".to_string()]),
            None
        );
    }

    #[test]
    fn resolved_short_spelling_via_import() {
        let files: HashMap<String, String> = [
            (
                "src/lib.rs".to_string(),
                "pub mod a;\npub fn old_api(x: i32) -> i32 { x }\n".to_string(),
            ),
            (
                "src/a.rs".to_string(),
                "use crate::old_api;\nfn f() { old_api(1); }\n".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        let graph = crate::CrateGraph::build(&files);
        let mut canons = HashMap::new();
        canons.insert("Old".to_string(), "crate::old_api".to_string());
        let index = crate::LineIndex::new("");
        let cx = cx_with(
            "",
            &index,
            &[("Old", "crate::old_api")],
            false,
            Some((&graph, "src/a.rs")),
            canons,
        );
        assert_eq!(
            match_symbol_placeholder(&cx, "Old", false, &["old_api".to_string()]),
            Some(true)
        );
    }

    #[test]
    fn resolved_vendored_copy_skipped() {
        // Same text, different definition: never a match (D2 honesty).
        let files: HashMap<String, String> = [
            (
                "src/lib.rs".to_string(),
                "pub mod vendored;\npub fn old_api(x: i32) -> i32 { x }\n".to_string(),
            ),
            (
                "src/vendored.rs".to_string(),
                "pub fn old_api(x: i32) -> i32 { x }\nfn g() { old_api(1); }\n".to_string(),
            ),
        ]
        .into_iter()
        .collect();
        let graph = crate::CrateGraph::build(&files);
        let mut canons = HashMap::new();
        canons.insert("Old".to_string(), "crate::old_api".to_string());
        let index = crate::LineIndex::new("");
        let cx = cx_with(
            "",
            &index,
            &[("Old", "crate::old_api")],
            false,
            Some((&graph, "src/vendored.rs")),
            canons,
        );
        assert_eq!(
            match_symbol_placeholder(&cx, "Old", false, &["old_api".to_string()]),
            Some(false)
        );
    }

    #[test]
    fn seed_verification_rejects_unknown() {
        let files: HashMap<String, String> =
            [("src/lib.rs".to_string(), "fn f() {}\n".to_string())]
                .into_iter()
                .collect();
        let graph = crate::CrateGraph::build(&files);
        let mut symbols = HashMap::new();
        symbols.insert("Old".to_string(), "crate::missing".to_string());
        let err = seed_canons_for(&symbols, "let x = __RSMPL_Old(1);", &graph, "r")
            .expect_err("unknown seed must fail");
        assert_eq!(err.code, rsmpl_patch::ErrorCode::SymbolConflict);
    }

    #[test]
    fn seed_verification_skips_plus_only() {
        // `New` names the rewrite target; the crate need not define it yet.
        let files: HashMap<String, String> =
            [("src/lib.rs".to_string(), "fn f() {}\n".to_string())]
                .into_iter()
                .collect();
        let graph = crate::CrateGraph::build(&files);
        let mut symbols = HashMap::new();
        symbols.insert("New".to_string(), "crate::new_api".to_string());
        let canons = seed_canons_for(&symbols, "let x = __RSMPL_Old(1);", &graph, "r")
            .expect("plus-only seed must not resolve");
        assert!(canons.is_empty());
    }
}
