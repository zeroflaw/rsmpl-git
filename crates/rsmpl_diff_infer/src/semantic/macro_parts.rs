//! Macro parse/anatomy helpers (split from `macros.rs` per the
//! D5 hygiene gate — `macros.rs` was at 1930/2000). Pure
//! `syn`-over-diff-line parsing shared by the rename sweep, the
//! interior sweep (`macros.rs`), and cross-site hetero grouping
//! (`macro_hetero.rs`). No detector or emission logic lives here.

use super::ident_len;

/// Parse a diff line as a macro invocation: strip one terminator, take
/// the whole line as `Expr::Macro`, else unwrap a `let`-init macro
/// (T449 precedent — `let x = old!(a);` matches `- old!(...)`,
/// probed). `let-else` never qualifies.
pub(crate) fn parse_macro_expr(line: &str) -> Option<syn::ExprMacro> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    if let Ok(syn::Expr::Macro(mac)) = syn::parse_str::<syn::Expr>(text) {
        return Some(mac);
    }
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            match *init.expr {
                syn::Expr::Macro(mac) => Some(mac),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Split a rendered macro path into (prefix segments, last segment).
pub(crate) fn split_macro_path(path: &str) -> Option<(Vec<String>, String)> {
    let mut segs: Vec<String> = path.split("::").map(str::to_string).collect();
    if segs.is_empty() {
        return None;
    }
    let last = segs.pop()?;
    Some((segs, last))
}

/// Parse a `-`/`+` line pair as a macro rename; return (old path, new
/// path) rendered with `::` joins when both sides are macro invocations
/// with a shared prefix, a last-segment delta, generic-free segments,
/// no leading `::`, and per-site token-equal bodies.
pub(crate) fn parse_macro_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(old_line)?;
    let n_mac = parse_macro_expr(new_line)?;
    let old = macro_path(&o_mac)?;
    let new = macro_path(&n_mac)?;
    if o_mac.mac.tokens.to_string() != n_mac.mac.tokens.to_string() {
        return None;
    }
    Some((old, new))
}

/// The rendered path of a macro invocation (`old!` or `prefix::old!`);
/// `None` for leading `::` (no engine spelling, probed — declined like
/// qualified calls), segment-count mismatch is checked by the caller via
/// exact rendered-path equality, and generic args.
pub(crate) fn macro_path(mac: &syn::ExprMacro) -> Option<String> {
    if mac.mac.path.leading_colon.is_some() {
        return None;
    }
    let mut segs = Vec::new();
    for seg in mac.mac.path.segments.iter() {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return None;
        }
        let name = seg.ident.to_string();
        if ident_len(&name)? != name.len() {
            return None;
        }
        segs.push(name);
    }
    if segs.is_empty() {
        return None;
    }
    // Bare (len 1) and qualified (len >= 2) both qualify; the sweep
    // requires a uniform class (exact rendered-path equality above), so
    // mixed bare/qualified classes stay ordinary by construction.
    Some(segs.join("::"))
}

/// Open delimiter character of a macro invocation (`(` / `[` / `{`).
pub(crate) fn macro_delim_open(mac: &syn::ExprMacro) -> Option<char> {
    match mac.mac.delimiter {
        syn::MacroDelimiter::Paren(_) => Some('('),
        syn::MacroDelimiter::Bracket(_) => Some('['),
        syn::MacroDelimiter::Brace(_) => Some('{'),
    }
}

/// Close delimiter matching [`macro_delim_open`].
pub(crate) fn delim_close(open: char) -> char {
    match open {
        '[' => ']',
        '{' => '}',
        _ => ')',
    }
}

/// Old-side lines for group member pairs, deduped, first-occurrence
/// order (WI-1b precise emission sites, shared by whole-line and
/// WI-9b2b nested groups).
pub(crate) fn member_sites(members: &[usize], pair_old: &[usize]) -> Vec<usize> {
    let mut sites = Vec::new();
    for &idx in members {
        let old_line = pair_old.get(idx).copied().unwrap_or(0);
        if !sites.contains(&old_line) {
            sites.push(old_line);
        }
    }
    sites
}

/// Parse a `-`/`+` line pair as a same-name concrete macro interior:
/// both sides render (via the rename sweep's `parse_macro_expr` —
/// whole-line invocations and `let`-init unwraps, `let-else` never) to
/// one identical path with the same delimiter on both sides (paren,
/// bracket, or brace — the engine splice emits with the plus pattern's
/// delimiter, WI-9b2-delim) and a body delta. Returns the rendered
/// `(minus, plus)` invocation pair (`path!(args)` / `path![args]` /
/// `path!{args}`). `None` for renames (name delta — the rename sweep
/// owns those), mixed delimiters (no cross-delimiter class — each side
/// keeps its own spelling), leading-`::`/generic paths (no engine
/// spelling), and identical bodies (no change).
pub(crate) fn parse_macro_interior_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(String, String)> {
    let o_mac = parse_macro_expr(old_line)?;
    let n_mac = parse_macro_expr(new_line)?;
    let (Some(o_open), Some(n_open)) = (macro_delim_open(&o_mac), macro_delim_open(&n_mac)) else {
        return None;
    };
    if o_open != n_open {
        return None;
    }
    let old_path = macro_path(&o_mac)?;
    let new_path = macro_path(&n_mac)?;
    if old_path != new_path {
        return None;
    }
    let old_args = o_mac.mac.tokens.to_string();
    let new_args = n_mac.mac.tokens.to_string();
    if old_args == new_args {
        return None;
    }
    let close = delim_close(o_open);
    Some((
        format!("{old_path}!{o_open}{old_args}{close}"),
        format!("{new_path}!{n_open}{new_args}{close}"),
    ))
}

/// Expression-position macro invocations of one diff line in visit
/// (source) order. `syn::visit` never descends into macro bodies
/// (opaque token streams), so macros nested inside other macro
/// arguments are invisible by construction (WI-9b2 non-goal) — only
/// macros in real expression positions (call args, `let` inits,
/// operands) collect.
struct MacroNodes {
    out: Vec<syn::ExprMacro>,
}

impl<'ast> syn::visit::Visit<'ast> for MacroNodes {
    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.out.push(node.clone());
        syn::visit::visit_expr_macro(self, node);
    }
}

/// Expression-position macros of one diff line in source order, via one
/// `Stmt` parse (fallback: `Expr`). `None` when neither parses.
pub(crate) fn line_macros(line: &str) -> Option<Vec<syn::ExprMacro>> {
    use syn::visit::Visit as _;
    let mut out = MacroNodes { out: Vec::new() };
    if let Ok(stmt) = syn::parse_str::<syn::Stmt>(line.trim()) {
        out.visit_stmt(&stmt);
        return Some(out.out);
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(line.trim()) {
        out.visit_expr(&expr);
        return Some(out.out);
    }
    None
}

// Raw source text of a parsed macro invocation: the Span byte_range
// slice of the trimmed diff line the node was parsed from (WI-9b2
// nested-raw). str::get is the fail-safe -- non-ASCII boundaries or
// out-of-range spans yield None and callers fall back to the
// token-normalized rendering, never loud on a rendering detail.
pub(crate) fn macro_raw_text(trimmed: &str, mac: &syn::ExprMacro) -> Option<String> {
    use syn::spanned::Spanned as _;
    trimmed.get(mac.span().byte_range()).map(str::to_string)
}

/// Sub-line nested-macro interior pair (WI-9b2b, bench-17 `tool.rs`
/// shape): the whole-line interior parse fails, but both lines parse
/// as one `Stmt` (fallback `Expr`) holding the same count (≥1) of
/// expression-position macros, all token-equal except exactly one delta
/// with the same rendered path, the same delimiter on both sides, and
/// different tokens (paren, bracket, or brace — WI-9b2-delim). The
/// rendered delta texts must each occur
/// byte-exact exactly once in their line, and the plus line must equal
/// the minus line with exactly that one substring replaced (the rest of
/// the line is byte-identical). Returns the rendered
/// `(macro_minus, macro_plus)` pair. `None` for renames (path delta —
/// the rename sweep owns whole-line renames; nested renames stay
/// ordinary), mixed delimiters (no cross-delimiter class), leading-`::`/
/// generic paths (no engine spelling), multi-delta lines, and any
/// outside-macro difference.
pub(crate) fn parse_nested_macro_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    // Whole-line pairs belong to the whole-line sweep (no double-claim).
    if parse_macro_interior_pair(old_line, new_line).is_some() {
        return None;
    }
    let old_macros = line_macros(old_line)?;
    let new_macros = line_macros(new_line)?;
    if old_macros.len() != new_macros.len() || old_macros.is_empty() {
        return None;
    }
    let mut delta: Option<usize> = None;
    for (i, (o, n)) in old_macros.iter().zip(new_macros.iter()).enumerate() {
        let (Some(op), Some(np)) = (macro_path(o), macro_path(n)) else {
            return None;
        };
        let (Some(o_open), Some(n_open)) = (macro_delim_open(o), macro_delim_open(n)) else {
            return None;
        };
        if op == np && o_open == n_open {
            if o.mac.tokens.to_string() == n.mac.tokens.to_string() {
                continue;
            }
            if delta.is_some() {
                return None;
            }
            delta = Some(i);
            continue;
        }
        return None;
    }
    let i = delta?;
    let (o, n) = (&old_macros[i], &new_macros[i]);
    let (Some(o_open), Some(n_open)) = (macro_delim_open(o), macro_delim_open(n)) else {
        return None;
    };
    let close = delim_close(o_open);
    // WI-9b2 nested-raw: render the delta pair from raw source bytes via
    // the nodes' spans (multi-token TokenStream display like 1 , 2
    // never occurs byte-exact in the line). Slicing failures fall back
    // to the token-normalized rendering (single-token bodies render
    // identically either way, so existing groups are byte-stable).
    let (minus, plus) = match (
        macro_raw_text(old_line.trim(), o),
        macro_raw_text(new_line.trim(), n),
    ) {
        (Some(m), Some(p)) => (m, p),
        _ => (
            format!("{}!{o_open}{}{close}", macro_path(o)?, o.mac.tokens),
            format!("{}!{n_open}{}{close}", macro_path(n)?, n.mac.tokens),
        ),
    };
    if minus == plus {
        return None;
    }
    if old_line.match_indices(&minus).count() != 1 || new_line.match_indices(&plus).count() != 1 {
        return None;
    }
    if *new_line != old_line.replacen(&minus, &plus, 1) {
        return None;
    }
    Some((minus, plus))
}

/// Split macro inner-args text into top-level comma-group byte spans
/// (string-aware like the engine `split_top_level`: commas/parens inside
/// `"..."` split nothing). Returns `None` on unbalanced delimiters
/// (e.g. a `'('` char literal — the engine deliberately skips `'` for
/// lifetime ambiguity, T419); hetero then bails to concrete.
pub(crate) fn split_top_spans(body: &str) -> Option<Vec<(usize, usize)>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if matches!(b, b'(' | b'[' | b'{') {
            depth += 1;
        } else if matches!(b, b')' | b']' | b'}') {
            depth -= 1;
            if depth < 0 {
                return None;
            }
        } else if b == b',' && depth == 0 {
            out.push((start, i));
            start = i + 1;
        }
        i += 1;
    }
    if in_str || depth != 0 {
        return None;
    }
    out.push((start, body.len()));
    // Drop empty groups (leading/trailing/double commas carry no arg —
    // engine `split_macro_args` retains the same way).
    let spans: Vec<(usize, usize)> = out
        .into_iter()
        .filter(|(s, e)| !body[*s..*e].trim().is_empty())
        .collect();
    Some(spans)
}

/// Trimmed byte range of a group span (leading/trailing whitespace excluded).
pub(crate) fn trim_range(s: &str, ms: usize, me: usize) -> (usize, usize) {
    let bytes = s.as_bytes();
    let mut ts = ms;
    while ts < me && bytes[ts].is_ascii_whitespace() {
        ts += 1;
    }
    let mut te = me;
    while te > ts && bytes[te - 1].is_ascii_whitespace() {
        te -= 1;
    }
    (ts, te)
}

/// True for string-literal args (`".."`, `r".."`, `b".."`, `br".."`):
/// strings are opaque positions (WI-9 string-literal-only rule) and never
/// hole, even when stable. Char literals and lifetimes hole like any other
/// non-string arg under totality.
pub(crate) fn is_string_arg(arg: &str) -> bool {
    let t = arg.trim();
    let u = t
        .strip_prefix("br\"")
        .or_else(|| t.strip_prefix("rb\""))
        .or_else(|| t.strip_prefix('r'))
        .or_else(|| t.strip_prefix('b'))
        .unwrap_or(t);
    u.len() >= 2 && u.starts_with('"') && u.ends_with('"')
}

/// Scan ident-like tokens (mirrors the inference candidate scan).
pub(crate) fn scan_arg_idents(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '$' || c.is_ascii_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(text[i..j].to_string());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The raw argument text between a line's macro-invocation delimiters:
/// everything after the open delimiter past `!` up to the matching
/// close (mod a trailing `;`). Exact because the line parsed as a
/// complete invocation — nothing trails the close delimiter but
/// `;`/whitespace — so interior closers inside strings cannot mislead
/// the `strip_suffix`.
pub(crate) fn macro_raw_args(line: &str) -> Option<String> {
    let bang = line.find('!')?;
    let after = line[bang + 1..].trim_start();
    let (open, close) = match after.chars().next()? {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        '{' => ('{', '}'),
        _ => return None,
    };
    let mut inner = after[open.len_utf8()..].trim_end();
    inner = inner.strip_suffix(';').unwrap_or(inner).trim_end();
    Some(inner.strip_suffix(close)?.to_string())
}
