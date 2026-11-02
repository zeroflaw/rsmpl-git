//! WI-8 slice 9: signature-evolving `fn` move pairing.
//!
//! The vis/name path (`move_evolve::fn_evolved_match`) requires the
//! signature (generics, params, return, `where`) token-equal. A move
//! that also evolves the signature (`fn old(x: i32) -> i32` →
//! `pub fn new(x: i64) -> i64` in a new file) is the same one-change
//! shape — the paste template spells the full plus-side header
//! concretely (`+ pub fn new(x: i64) -> i64 { ... }`), so the new
//! signature rides verbatim from the plus side, never as a hole — but
//! needs its own matcher (the abstract `(...)` template keeps the
//! captured suffix verbatim and would silently keep the old signature).
//!
//! Narrowing (stated in SPEC 20): both sides must open their block
//! (a header line ending in `{`), both items must span ≥ 2 lines (a
//! single-line item has no body lines to ride), bodies and attrs stay
//! token-equal (a move that rewrites the body is two changes), and at
//! least the signature differs (sig-identical pairs belong to the
//! abstract path). Headers may span lines (WI-19 slice 2): the splice
//! replaces the whole header span, so no stale signature line stays
//! behind. All downstream gates (dedup, hunk holds, twin veto, T375)
//! apply unchanged — this module only produces candidate pairs.
//!
//! D5: this lives in a sibling module because `move_evolve.rs`
//! (1869 lines) and `moves.rs` (1977 lines) must not grow.

use super::move_evolve::{opt_tok, tok};
use syn::spanned::Spanned;

/// Signature-evolving `fn` pair check: same body and attrs, but the
/// signature (generics, params, return, `where`) differs. Vis/name may
/// evolve too (they ride in the concrete template verbatim); a
/// qualifier-only delta with identical vis/name/signature still vetoes,
/// mirroring the abstract path. Returns the plus-side ident when the
/// pair evolves.
pub(crate) fn fn_sig_evolved_match(minus: &syn::ItemFn, plus: &syn::ItemFn) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    if tok(&minus.block) != tok(&plus.block) {
        return None;
    }
    let (a, b) = (&minus.sig, &plus.sig);
    let sig_same = tok(&a.generics.params) == tok(&b.generics.params)
        && opt_tok(&a.generics.where_clause) == opt_tok(&b.generics.where_clause)
        && tok(&a.inputs) == tok(&b.inputs)
        && a.variadic.is_some() == b.variadic.is_some()
        && opt_tok(&a.variadic) == opt_tok(&b.variadic)
        && tok(&a.output) == tok(&b.output);
    // The delta that fires: without a signature delta the abstract
    // `(...)` path owns the pair (or it is byte-identical).
    if sig_same {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.sig.ident == plus.sig.ident;
    // Qualifiers ride verbatim in the template, so vis/name/sig deltas
    // in any combination fire — but a qualifier-only delta (vis, name,
    // and signature all same) stays ordinary, mirroring the abstract
    // path's `vis_same && name_same → None` veto.
    if vis_same && name_same && sig_same {
        return None;
    }
    Some(plus.sig.ident.to_string())
}

/// The verbatim prefix (vis + qualifiers, leading whitespace stripped)
/// and the concrete paste template of the `fn <name>` header *span*
/// inside `item_lines` (`None` for attr/doc/comment lines, or when no
/// `fn <name>` code line opens a block). The span runs from the first
/// `fn <name>` code line through the first line whose trimmed end
/// opens the block (`... {`), so single- and multi-line headers share
/// one gate; the span must be followed by body lines (a single-line
/// item has no body to ride). The template is the span with the
/// `{`-suffix replaced by `{ ... }`: the first line trimmed (the
/// engine re-indents it to the capture), continuation lines verbatim.
fn header_span(item_lines: &[String], name: &str) -> Option<(String, String, usize)> {
    let mut start = None;
    for (idx, line) in item_lines.iter().enumerate() {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if find_fn_name(text, name).is_some() {
            start = Some(idx);
            break;
        }
        // A `fn <name>` code line that holds no word-bounded pair is
        // not the header — keep scanning (doc lines are skipped
        // above; other code lines cannot precede the header).
    }
    let start = start?;
    // The header runs through the first line that opens the block.
    // Continuation lines (params, generics, `where` clauses) ride
    // inside the span; a span that never opens a block vetoes.
    let mut end = None;
    for (off, line) in item_lines[start..].iter().enumerate() {
        let text = line.get(1..).unwrap_or("");
        if text.trim_end().ends_with('{') {
            end = Some(start + off);
            break;
        }
    }
    let end = end?;
    if end + 1 >= item_lines.len() {
        return None;
    }
    let first_text = item_lines[start].get(1..).unwrap_or("");
    let fn_pos = find_fn_name(first_text, name)?;
    let prefix = first_text[..fn_pos].trim_start().to_string();
    let mut span: Vec<String> = item_lines[start..=end]
        .iter()
        .map(|l| l.get(1..).unwrap_or("").to_string())
        .collect();
    if let Some(first) = span.first_mut() {
        *first = first.trim().to_string();
    }
    if let Some(last) = span.last_mut() {
        let brace = last.find('{')?;
        *last = format!("{} {{ ... }}", last[..brace].trim_end());
    }
    let template = span.join("\n");
    Some((prefix, template, end - start + 1))
}

/// Byte offset of a word-bounded `fn <name>` pair (`None` when the line
/// holds no such pair: comments, strings, or other idents). Mirrors
/// `move_evolve::find_fn_name` (private there) over diff-body lines
/// (`+`/`-` prefix already stripped by the caller via `get(1..)`).
fn find_fn_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..].starts_with(b"fn")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 2;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// All signature-evolved `fn` hits of one minus item text in a pure-plus
/// hunk body. Mirrors `move_evolve::find_evolved_hits`; spans coincide
/// with parsed `fn` item spans by construction. Each hit carries the
/// plus-side header prefix, the concrete template (the plus header
/// span with the `{ ... }` marker — single- or multi-line) the
/// emission pastes verbatim, and the plus-side ident.
pub(crate) fn find_fn_sig_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemFn>(minus_text) else {
        return Vec::new();
    };
    // Minus-side header-span gate: the splice replaces the whole span,
    // so any span shape pairs (single- or multi-line).
    let minus_lines: Vec<String> = minus_text.lines().map(|l| format!("-{l}")).collect();
    if header_span(&minus_lines, &minus.sig.ident.to_string()).is_none() {
        return Vec::new();
    }
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Fn(plus) = it else {
            continue;
        };
        if fn_sig_evolved_match(&minus, plus).is_none() {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let name = plus.sig.ident.to_string();
        // Plus-side header-span gate: the template is the header span
        // with the `{ ... }` marker (single- or multi-line).
        let Some((prefix, template, _)) = header_span(&plus_body[(s - 1)..e], &name) else {
            continue;
        };
        out.push((s - 1, e - 1, prefix, template, name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_fn(text: &str) -> syn::ItemFn {
        syn::parse_str(text).expect("test fn must parse")
    }

    const BODY: &str = "{\n    let y = x + 1;\n    y * 2\n}";

    #[test]
    fn param_delta_fires() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("pub fn new(x: i64) -> i64 {BODY}"));
        assert_eq!(fn_sig_evolved_match(&minus, &plus), Some("new".to_string()));
    }

    #[test]
    fn return_only_delta_fires() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("fn old(x: i32) -> i64 {BODY}"));
        assert_eq!(fn_sig_evolved_match(&minus, &plus), Some("old".to_string()));
    }

    #[test]
    fn where_only_delta_fires() {
        let minus = item_fn("fn old<T>(x: T) -> T {\n    x\n}");
        let plus = item_fn("fn old<T>(x: T) -> T where T: Clone {\n    x\n}");
        assert_eq!(fn_sig_evolved_match(&minus, &plus), Some("old".to_string()));
    }

    #[test]
    fn generics_only_delta_fires() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("fn old<T>(x: T) -> T {BODY}"));
        assert_eq!(fn_sig_evolved_match(&minus, &plus), Some("old".to_string()));
    }

    #[test]
    fn sig_identical_pair_is_not_sig_evolved() {
        // The abstract `(...)` path owns sig-identical pairs (or the
        // byte-exact path owns identical items).
        let a = item_fn(&format!("pub fn new(x: i32) -> i32 {BODY}"));
        let b = item_fn(&format!("pub fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_sig_evolved_match(&a, &b), None);
        let c = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let d = item_fn(&format!("pub fn new(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_sig_evolved_match(&c, &d), None);
    }

    #[test]
    fn qualifier_only_delta_vetoes() {
        // Mirrors the abstract path: vis, name, and signature all same
        // but qualifiers differ stays ordinary.
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn(&format!("async fn old(x: i32) -> i32 {BODY}"));
        assert_eq!(fn_sig_evolved_match(&minus, &plus), None);
    }

    #[test]
    fn body_and_attr_deltas_veto() {
        let minus = item_fn(&format!("fn old(x: i32) -> i32 {BODY}"));
        let plus = item_fn("pub fn new(x: i64) -> i64 {\n    x\n}");
        assert_eq!(fn_sig_evolved_match(&minus, &plus), None);
        let plus_a = item_fn(&format!("#[inline]\npub fn new(x: i64) -> i64 {BODY}"));
        assert_eq!(fn_sig_evolved_match(&minus, &plus_a), None);
    }

    fn plus_body(text: &str) -> Vec<String> {
        text.lines().map(|l| format!("+{l}")).collect()
    }

    #[test]
    fn finder_builds_concrete_template() {
        let minus = &format!("fn old(x: i32) -> i32 {BODY}");
        let body = plus_body(&format!("pub fn new(x: i64) -> i64 {BODY}"));
        let hits = find_fn_sig_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1);
        let (first, last, prefix, tmpl, name) = &hits[0];
        assert_eq!((*first, *last), (0, 3));
        assert_eq!(prefix, "pub ");
        assert_eq!(tmpl, "pub fn new(x: i64) -> i64 { ... }");
        assert_eq!(name, "new");
    }

    #[test]
    fn finder_fires_multiline_plus_header() {
        // WI-19 slice 2: a multi-line plus header pairs — the template
        // is the plus header span with the `{ ... }` marker.
        let minus = &format!("fn old(x: i32) -> i32 {BODY}");
        let body =
            plus_body("pub fn new(\n    x: i64,\n) -> i64 {\n    let y = x + 1;\n    y * 2\n}");
        let hits = find_fn_sig_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].2, "pub ");
        assert_eq!(hits[0].3, "pub fn new(\n    x: i64,\n) -> i64 { ... }");
        assert_eq!(hits[0].4, "new");
    }

    #[test]
    fn finder_fires_multiline_minus_header() {
        // WI-19 slice 2: the span splice replaces the whole minus
        // header span, so a multi-line minus header pairs too.
        let minus = "fn old(\n    x: i32,\n) -> i32 {\n    x\n}";
        let body = plus_body("pub fn new(\n    x: i64,\n) -> i64 {\n    x\n}");
        let hits = find_fn_sig_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].3, "pub fn new(\n    x: i64,\n) -> i64 { ... }");
    }

    #[test]
    fn finder_vetoes_single_line_item() {
        // No body lines to ride: `fn old(x: i32) { x }` must not pair.
        let minus = "fn old(x: i32) -> i32 { x }";
        let body = plus_body("pub fn new(x: i64) -> i64 { x }");
        assert!(find_fn_sig_evolved_hits(minus, &body).is_empty());
    }

    #[test]
    fn finder_skips_doc_comment_fn_mention() {
        // A doc line mentioning `fn new` never wins over the header
        // (mirrors `header_prefix_skips_attrs_and_docs`): the identical
        // doc rides on both sides so attrs stay token-equal, and the
        // scan must skip it to find the real header line.
        let minus = &format!("/// calls fn new\nfn old(x: i32) -> i32 {BODY}");
        let body = plus_body(&format!(
            "/// calls fn new\npub fn new(x: i64) -> i64 {BODY}"
        ));
        let hits = find_fn_sig_evolved_hits(minus, &body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].3, "pub fn new(x: i64) -> i64 { ... }");
    }
}
