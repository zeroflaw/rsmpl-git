//! Function-item matching and rewriting (SPEC 14).
//!
//! v0.1 supports: rename of plain/explicit functions (minus/plus differ only
//! in the function name), item deletion via bare `- I`, literal insertion,
//! and rename inside `impl` context blocks. Format is preserved by replacing
//! only the name token (or deleting/inserting whole line spans).

use syn::spanned::Spanned;

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{
    CompiledPattern, FnPattern, GenericPat, PatternKind, RetPat, VisPat, WherePat,
};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// Whitespace-insensitive normalization for signature slot comparison.
/// Both sides are token text; stripping all whitespace makes `pub(crate)`,
/// `x : i32`, and `< T >` compare equal to their canonical spellings.
fn norm_slot(s: &str) -> String {
    s.split_whitespace().collect()
}
/// List-slot normalization: `norm_slot` plus trailing-comma tolerance.
/// In comma-separated positions (parameter lists, `where` predicates) a
/// trailing comma is a separator, never a semantic difference, so the
/// rustfmt-canonical multi-line spelling (`x: i32,`) compares equal to
/// the single-line pattern spelling (`x: i32`) (SPEC 14.7).
fn norm_list_slot(s: &str) -> String {
    norm_slot(s).trim_end_matches(',').to_string()
}

/// Compact pattern slot text into valid Rust (`< T >` -> `<T>`,
/// `x : i32` -> `x: i32`). Input is token-joined pattern text.
fn compact_slot(s: &str) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    t.replace("< ", "<")
        .replace(" >", ">")
        .replace(" ,", ",")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" :", ":")
        .replace(" ::", "::")
        .replace(":: ", "::")
}

/// A function item with its source span lines and attribute lines.
struct FnCandidate<'a> {
    item: &'a syn::ItemFn,
    start_line: u32,
    end_line: u32,
    fn_line: u32,
    attr_start_line: u32,
}

fn collect_fns<'a>(file: &'a syn::File) -> Vec<FnCandidate<'a>> {
    let mut out = Vec::new();
    collect_from_items(&file.items, &mut out);
    out
}

fn collect_from_items<'a>(items: &'a [syn::Item], out: &mut Vec<FnCandidate<'a>>) {
    for it in items {
        match it {
            syn::Item::Fn(f) => out.push(candidate_of(f)),
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_from_items(inner, out);
                }
            }
            _ => {}
        }
    }
}

fn candidate_of(f: &syn::ItemFn) -> FnCandidate<'_> {
    let s = f.span().start();
    let e = f.span().end();
    let fl = f.sig.fn_token.span().start().line as u32;
    // Attribute envelope starts at the item span start (syn includes attrs).
    FnCandidate {
        item: f,
        start_line: s.line as u32,
        end_line: e.line as u32,
        fn_line: fl,
        attr_start_line: s.line as u32,
    }
}

/// Scope filtering (SPEC 17.3). `anchor` uses the `fn` keyword line as the
/// first concrete non-ellipsis token.
fn in_scope(c: &FnCandidate<'_>, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.fn_line >= old_start && c.fn_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
        Scope::File => true,
    }
}

fn check_attrs(
    c: &FnCandidate<'_>,
    policy: AttrsPolicy,
    pattern_attrs: &[String],
    rule: &str,
    file: &str,
) -> Result<(), StructuredError> {
    let has = !c.item.attrs.is_empty();
    match policy {
        AttrsPolicy::Preserve => Ok(()),
        AttrsPolicy::Absent => {
            if has {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "function has attributes but `# attrs=absent` requires none",
                )
                .with_rule(rule)
                .with_file(file));
            }
            Ok(())
        }
        AttrsPolicy::Explicit => {
            if !attrs_equal(pattern_attrs, &c.item.attrs) {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "attached attributes do not match explicitly written pattern attributes",
                )
                .with_rule(rule)
                .with_file(file)
                .with_suggestion("use `# attrs=preserve` (default) to ignore attributes"));
            }
            Ok(())
        }
        AttrsPolicy::Strip => Ok(()), // eligibility like preserve; stripped at rewrite
    }
}

/// Strip-all attribute comparison shared by the Explicit arm and the
/// Preserve site filter: pattern text (token-joined at parse) vs
/// `quote!`-rendered target attributes. Whitespace-insensitive so
/// `///`/`#[doc]` mirrors (normalized at parse) compare equal.
fn attrs_equal(pattern_attrs: &[String], item_attrs: &[syn::Attribute]) -> bool {
    let want: Vec<String> = pattern_attrs.iter().map(|a| norm_slot(a)).collect();
    let have: Vec<String> = item_attrs
        .iter()
        .map(|a| norm_slot(&quote::quote!(#a).to_string()))
        .collect();
    want == have
}

/// Reconstruct the `FnPattern` for the minus side (also validates it parses).
///
/// For method-context hunks the full minus text includes the surrounding
/// `impl`/`trait` header lines, so only the function block (from the first
/// fn-like line, balanced to its closing brace — or to `;` for trait
/// declarations) is parsed.
fn minus_fn_pattern(
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
) -> Result<FnPattern, StructuredError> {
    let (vis, qual, gen, wh) = slot_metas(compiled);
    // Rebuild full minus text (removals + context) as the pattern crate does.
    let full: String = hunk
        .lines
        .iter()
        .filter(|l| l.is_removal() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let block = match &compiled.kind {
        PatternKind::ImplContext { .. } => extract_fn_block(&full),
        _ => full,
    };
    rsmpl_pattern::parse_fn_pattern(&block, &vis, &qual, &gen, &wh, hunk.returns_none())
}

/// Extract the first function block from text that may be prefixed by
/// context lines (e.g. `impl Service {`). Scans for the first fn-like line
/// and takes lines until braces balance back to zero — or until a `;`
/// declaration terminator (T182: `trait` methods may be `fn foo();`
/// declarations with no body, so the `;` must end the block before trailing
/// `...`/`}` context lines trip the trailing-tokens gate). The `;` only
/// terminates at zero nesting (parens + brackets + braces) and before any
/// `{` opens, so `[u8; N]` lengths and fn-pointer params never cut the block
/// short, and mixed bodies still reach the T160 parser gate.
fn extract_fn_block(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut start = None;
    for (idx, line) in lines.iter().enumerate() {
        let t = line.trim();
        let t = t.strip_prefix("#[").unwrap_or(t);
        // T185: `unsafe trait T {` / `unsafe impl … {` headers open method
        // context but are never the fn pattern — yet they start with the
        // fn-like `unsafe ` prefix. Skip block headers before testing.
        let u = t.strip_prefix("unsafe ").unwrap_or(t);
        if u.starts_with("impl ") || u.starts_with("trait ") {
            continue;
        }
        if t.starts_with("fn ")
            || t.starts_with("pub ")
            || t.starts_with("async ")
            || t.starts_with("unsafe ")
            || t.starts_with("const ")
            || t.starts_with("extern ")
            || t.starts_with("pub(")
            || t.starts_with("fn(")
        {
            start = Some(idx);
            break;
        }
    }
    let Some(s) = start else {
        return text.to_string();
    };
    let mut depth = 0i32;
    let mut parens = 0i32;
    let mut brackets = 0i32;
    let mut seen_open = false;
    let mut end = lines.len();
    for (idx, line) in lines.iter().enumerate().skip(s) {
        let mut terminated = false;
        for c in line.chars() {
            match c {
                '{' => {
                    depth += 1;
                    seen_open = true;
                }
                '}' => {
                    depth -= 1;
                }
                '(' => {
                    parens += 1;
                }
                ')' => {
                    parens -= 1;
                }
                '[' => {
                    brackets += 1;
                }
                ']' => {
                    brackets -= 1;
                }
                ';' if !seen_open && depth == 0 && parens == 0 && brackets == 0 => {
                    terminated = true;
                    break;
                }
                _ => {}
            }
        }
        if terminated {
            end = idx + 1;
            break;
        }
        if seen_open && depth == 0 {
            end = idx + 1;
            break;
        }
    }
    lines[s..end].join("\n")
}

fn slot_metas(compiled: &CompiledPattern) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    use rsmpl_patch::MetavarKind;
    let mut vis = Vec::new();
    let mut qual = Vec::new();
    let mut gen = Vec::new();
    let mut wh = Vec::new();
    for (kind, name) in &compiled.table.declared {
        match kind {
            MetavarKind::Visibility => vis.push(name.clone()),
            MetavarKind::Qualifier | MetavarKind::QualifierList => qual.push(name.clone()),
            MetavarKind::GenericParams => gen.push(name.clone()),
            MetavarKind::WhereClause => wh.push(name.clone()),
            _ => {}
        }
    }
    (vis, qual, gen, wh)
}

/// Byte range of `span` in the original text, or `None` when the span
/// falls outside the file (never guesses).
fn span_bytes(
    original: &str,
    index: &LineIndex,
    span: proc_macro2::Span,
) -> Option<(usize, usize)> {
    let s = span.start();
    let e = span.end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

/// Slice original text covered by `span`.
fn span_text(original: &str, index: &LineIndex, span: proc_macro2::Span) -> Option<String> {
    let (a, b) = span_bytes(original, index, span)?;
    original.get(a..b).map(|t| t.to_string())
}

/// Start line/column and end line/column of the qualifier token sequence
/// (`const`/`async`/`unsafe`/`extern ["ABI"]`), or `None` when the
/// signature has no qualifiers.
fn qualifier_span_ends(sig: &syn::Signature) -> Option<(proc_macro2::Span, proc_macro2::Span)> {
    let mut starts: Vec<proc_macro2::Span> = Vec::new();
    let mut ends: Vec<proc_macro2::Span> = Vec::new();
    if let Some(c) = &sig.constness {
        starts.push(c.span());
        ends.push(c.span());
    }
    if let Some(a) = &sig.asyncness {
        starts.push(a.span());
        ends.push(a.span());
    }
    if let Some(u) = &sig.unsafety {
        starts.push(u.span());
        ends.push(u.span());
    }
    if let Some(abi) = &sig.abi {
        starts.push(abi.span());
        ends.push(abi.span());
    }
    if starts.is_empty() {
        return None;
    }
    // Signatures store qualifiers in fixed order; first start / last end
    // covers the whole qualifier run.
    let first = starts.into_iter().next()?;
    let last = ends.into_iter().next_back()?;
    Some((first, last))
}

/// Byte range covering the qualifier run, or `None` when absent.
fn qualifier_bytes(
    original: &str,
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    let (first, last) = qualifier_span_ends(sig)?;
    let s = first.start();
    let e = last.end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

/// Byte range inside the parameter parentheses (empty for `()`).
fn params_inner_bytes(index: &LineIndex, sig: &syn::Signature) -> Option<(usize, usize)> {
    let open_end = sig.paren_token.span.open().end();
    let close_start = sig.paren_token.span.close().start();
    let a = index.offset(open_end.line as u32, open_end.column)?;
    let b = index.offset(close_start.line as u32, close_start.column)?;
    if a <= b {
        Some((a, b))
    } else {
        None
    }
}

/// Byte range of `<...>` (inclusive), or `None` when no generics.
fn generics_bytes(
    original: &str,
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    let (lt, gt) = (
        sig.generics.lt_token.as_ref()?,
        sig.generics.gt_token.as_ref()?,
    );
    let s = lt.span().start();
    let e = gt.span().end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

/// Byte range of `-> Ty` (inclusive), or `None` when no return type.
fn return_bytes(original: &str, index: &LineIndex, sig: &syn::Signature) -> Option<(usize, usize)> {
    match &sig.output {
        syn::ReturnType::Type(arrow, ty) => {
            let s = arrow.span().start();
            let e = ty.span().end();
            let a = index.offset(s.line as u32, s.column)?;
            let b = index.offset(e.line as u32, e.column)?;
            if a <= b && b <= original.len() {
                Some((a, b))
            } else {
                None
            }
        }
        syn::ReturnType::Default => None,
    }
}

/// Byte range of the `where ...` clause, or `None` when absent.
fn where_bytes(original: &str, index: &LineIndex, sig: &syn::Signature) -> Option<(usize, usize)> {
    let w = sig.generics.where_clause.as_ref()?;
    span_bytes(original, index, w.span())
}

/// Extend `end` over one following ASCII space so slot deletions do not
/// leave doubled spacing (`fn foo()  {`).
fn swallow_one_space(original: &str, end: usize) -> usize {
    if original.as_bytes().get(end) == Some(&b' ') {
        end + 1
    } else {
        end
    }
}

/// Strict minus-side slot check (SPEC 14 explicit-syntax rule).
///
/// `matches_item_fn` already enforces presence/absence and qualifier
/// identity; this refines `Concrete` slots to normalized text equality
/// and adds parameter-list checking (previously unchecked), slicing the
/// target slot text from the original. Any failure rejects the candidate
/// (fail over guessing); un-sliceable spans also reject.
fn minus_matches_strict(
    minus: &FnPattern,
    vis: &syn::Visibility,
    sig: &syn::Signature,
    original: &str,
    index: &LineIndex,
) -> bool {
    // Visibility: Concrete requires normalized equality (stricter than the
    // presence-only check in `matches_item_fn`).
    if let rsmpl_pattern::VisPat::Concrete(want) = &minus.visibility {
        match vis {
            syn::Visibility::Inherited => return false,
            _ => {
                let Some(got) = span_text(original, index, vis.span()) else {
                    return false;
                };
                if norm_slot(&got) != norm_slot(want) {
                    return false;
                }
            }
        }
    }
    // Generics: Concrete requires normalized equality.
    if let GenericPat::Concrete(want) = &minus.generics {
        let Some((a, b)) = generics_bytes(original, index, sig) else {
            return false;
        };
        let got = original.get(a..b).unwrap_or("");
        if norm_slot(got) != norm_slot(want) {
            return false;
        }
    }
    // Parameters: concrete (including empty `()`) requires normalized
    // equality with the sliced parameter list.
    if !minus.params_ellipsis {
        let Some((a, b)) = params_inner_bytes(index, sig) else {
            return false;
        };
        let got = original.get(a..b).unwrap_or("");
        let want = minus.params_text.as_deref().unwrap_or("");
        if norm_list_slot(got) != norm_list_slot(want) {
            return false;
        }
    }
    // Return type: Concrete requires normalized equality of the type
    // (sliced text minus the leading `->`).
    if let RetPat::Concrete(want) = &minus.ret {
        let Some((a, b)) = return_bytes(original, index, sig) else {
            return false;
        };
        let got = original.get(a..b).unwrap_or("");
        let got_ty = norm_slot(got).strip_prefix("->").unwrap_or("").to_string();
        if got_ty != norm_slot(want) {
            return false;
        }
    }
    // Where clause: Concrete requires normalized equality of the
    // predicates (sliced text minus the leading `where`).
    if let rsmpl_pattern::WherePat::Concrete(want) = &minus.where_clause {
        let Some((a, b)) = where_bytes(original, index, sig) else {
            return false;
        };
        let got = original.get(a..b).unwrap_or("");
        let got_preds = norm_list_slot(got)
            .strip_prefix("where")
            .unwrap_or("")
            .to_string();
        if got_preds != norm_list_slot(want) {
            return false;
        }
    }
    true
}

/// Plan signature edits for one matched function (free or `impl` method).
///
/// Compares the minus/plus `FnPattern` slot pairs and emits minimal
/// disjoint byte-range edits over the original text:
/// - identical (normalized) slots: no edit (rename fast-path preserved);
/// - plus `...`/Meta-same/Any: preserve original (no edit);
/// - plus explicit text differing from target: insert/replace/delete.
///
/// `minus_name`/`plus_name` carry the `$Meta` convention (`$Name` means
/// "keep original"). Returns the edits (possibly only the name edit).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
fn plan_sig_edits(
    vis: &syn::Visibility,
    sig: &syn::Signature,
    minus: &FnPattern,
    plus: &FnPattern,
    minus_name: &str,
    plus_name: &str,
    original: &str,
    index: &LineIndex,
    rule: &str,
) -> Result<Vec<TextEdit>, StructuredError> {
    let mut edits = Vec::new();
    let range_err = |line: u32| {
        StructuredError::new(
            ErrorCode::RangeConflict,
            format!("span line {line} outside file"),
        )
        .with_rule(rule)
    };

    // ---- Name (unchanged rename semantics). ----
    if !plus_name.starts_with('$') && minus_name != plus_name {
        let ns = sig.ident.span();
        let (sl, sc, el, ec) = (
            ns.start().line as u32,
            ns.start().column,
            ns.end().line as u32,
            ns.end().column,
        );
        let start = index.offset(sl, sc).ok_or_else(|| range_err(sl))?;
        let end = index.offset(el, ec).ok_or_else(|| range_err(el))?;
        if sig.ident != original[start..end] {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "function name span mismatch (file changed during matching?)",
            )
            .with_rule(rule));
        }
        edits.push(TextEdit::new(start, end, plus_name.to_string()));
    }

    // ---- Visibility. ----
    // Plus-side `V` preserves (covers both-sides-`V`); a minus-side `V`
    // with explicit plus text sets that text. Omission preserves.
    if !matches!(plus.visibility, VisPat::Meta(_)) {
        let plus_vis = match &plus.visibility {
            VisPat::Concrete(t) => Some(t.clone()),
            VisPat::Absent => None,
            VisPat::Meta(_) => None,
        };
        let minus_vis = match &minus.visibility {
            VisPat::Concrete(t) => Some(t.clone()),
            VisPat::Absent => None,
            VisPat::Meta(_) => None,
        };
        if plus_vis != minus_vis {
            let next_start = qualifier_span_ends(sig)
                .map(|(first, _)| first.start())
                .unwrap_or_else(|| sig.fn_token.span().start());
            let next_off = index
                .offset(next_start.line as u32, next_start.column)
                .ok_or_else(|| range_err(next_start.line as u32))?;
            match (vis, plus_vis) {
                (syn::Visibility::Inherited, Some(want)) => {
                    edits.push(TextEdit::new(next_off, next_off, format!("{want} ")));
                }
                (syn::Visibility::Inherited, None) => {}
                (_, Some(want)) => {
                    let (a, b) = span_bytes(original, index, vis.span())
                        .ok_or_else(|| range_err(vis.span().start().line as u32))?;
                    if norm_slot(original.get(a..b).unwrap_or("")) != norm_slot(&want) {
                        edits.push(TextEdit::new(a, b, want));
                    }
                }
                (_, None) => {
                    let (a, _) = span_bytes(original, index, vis.span())
                        .ok_or_else(|| range_err(vis.span().start().line as u32))?;
                    if next_off > a {
                        edits.push(TextEdit::new(a, next_off, String::new()));
                    }
                }
            }
        }
    }

    // ---- Qualifiers. ----
    // Plus-side `Q` preserves; minus-side `Q` with explicit plus text sets
    // it. Omission preserves. (A concrete-minus/plus-`Q` mix is rejected
    // earlier as an unbound metavariable.)
    if plus.qualifier_meta.is_none() {
        let minus_q = minus.qualifiers.join(" ");
        let plus_q = plus.qualifiers.join(" ");
        if norm_slot(&minus_q) != norm_slot(&plus_q) {
            let fn_start = sig.fn_token.span().start();
            let fn_off = index
                .offset(fn_start.line as u32, fn_start.column)
                .ok_or_else(|| range_err(fn_start.line as u32))?;
            match qualifier_bytes(original, index, sig) {
                Some((a, b)) => {
                    if plus_q.trim().is_empty() {
                        if fn_off > a {
                            edits.push(TextEdit::new(a, fn_off, String::new()));
                        }
                    } else if norm_slot(original.get(a..b).unwrap_or("")) != norm_slot(&plus_q) {
                        edits.push(TextEdit::new(a, b, compact_slot(&plus_q)));
                    }
                }
                None => {
                    if !plus_q.trim().is_empty() {
                        let text = format!("{} ", compact_slot(&plus_q));
                        // A same-anchor visibility insertion may already exist
                        // (target had neither vis nor qualifiers); merge to keep
                        // canonical `vis quals` order in the single combined
                        // edit (same-point inserts materialize in document
                        // order, SPEC 23.5 — merging keeps order explicit).
                        if let Some(v) = edits
                            .iter_mut()
                            .find(|e| e.is_insertion() && e.start == fn_off)
                        {
                            v.replacement = format!("{}{}", v.replacement, text);
                        } else {
                            edits.push(TextEdit::new(fn_off, fn_off, text));
                        }
                    }
                }
            }
        }
    }

    // ---- Generics. ----
    // Plus-side `G` preserves; minus-side `G` with an explicit plus side
    // sets it. Omission preserves.
    if !matches!(plus.generics, GenericPat::Meta(_)) {
        let minus_g = match &minus.generics {
            GenericPat::Concrete(t) => Some(t.clone()),
            _ => None,
        };
        let plus_g = match &plus.generics {
            GenericPat::Concrete(t) => Some(t.clone()),
            _ => None,
        };
        if minus_g.as_deref().map(norm_slot) != plus_g.as_deref().map(norm_slot) {
            let ident_end = sig.ident.span().end();
            let ident_off = index
                .offset(ident_end.line as u32, ident_end.column)
                .ok_or_else(|| range_err(ident_end.line as u32))?;
            match (generics_bytes(original, index, sig), plus_g) {
                (Some((a, b)), Some(want)) => {
                    let want_c = compact_slot(&want);
                    if norm_slot(original.get(a..b).unwrap_or("")) != norm_slot(&want_c) {
                        edits.push(TextEdit::new(a, b, want_c));
                    }
                }
                (Some((a, b)), None) => {
                    edits.push(TextEdit::new(a, b, String::new()));
                }
                (None, Some(want)) => {
                    edits.push(TextEdit::new(ident_off, ident_off, compact_slot(&want)));
                }
                (None, None) => {}
            }
        }
    }

    // ---- Parameters. ----
    // Plus `...` preserves; minus `...` matches any. Otherwise the sliced
    // inner list must equal the minus text (checked in
    // `minus_matches_strict`); replace when the plus text differs.
    if !plus.params_ellipsis {
        let want = plus.params_text.as_deref().unwrap_or("");
        let (a, b) = params_inner_bytes(index, sig)
            .ok_or_else(|| range_err(sig.paren_token.span.open().start().line as u32))?;
        if norm_list_slot(original.get(a..b).unwrap_or("")) != norm_list_slot(want) {
            edits.push(TextEdit::new(a, b, compact_slot(want)));
        }
    }
    // ---- Return type. ----
    // Plus-side `R` preserves; minus-side `R` with an explicit plus side
    // sets it.
    if !matches!(plus.ret, RetPat::Meta(_)) {
        let minus_r = match &minus.ret {
            RetPat::Concrete(t) => Some(t.clone()),
            _ => None,
        };
        let plus_is_any = matches!(plus.ret, RetPat::Any);
        let plus_r = match &plus.ret {
            RetPat::Concrete(t) => Some(t.clone()),
            _ => None,
        };
        let cur = return_bytes(original, index, sig);
        if plus_is_any {
            // Plus wrote no `->`: preserve, except explicit removal when
            // the minus side named a concrete return type.
            if minus_r.is_some() {
                if let Some((a, b)) = cur {
                    edits.push(TextEdit::new(
                        a,
                        swallow_one_space(original, b),
                        String::new(),
                    ));
                }
            }
        } else if minus_r.as_deref().map(norm_slot) != plus_r.as_deref().map(norm_slot) {
            match (cur, plus_r) {
                (Some((a, b)), Some(want)) => {
                    let rep = format!("-> {}", compact_slot(&want));
                    if norm_slot(original.get(a..b).unwrap_or("")) != norm_slot(&rep) {
                        edits.push(TextEdit::new(a, b, rep));
                    }
                }
                (Some((a, b)), None) => {
                    edits.push(TextEdit::new(
                        a,
                        swallow_one_space(original, b),
                        String::new(),
                    ));
                }
                (None, Some(want)) => {
                    let close_end = sig.paren_token.span.close().end();
                    let anchor = index
                        .offset(close_end.line as u32, close_end.column)
                        .ok_or_else(|| range_err(close_end.line as u32))?;
                    edits.push(TextEdit::new(
                        anchor,
                        anchor,
                        format!(" -> {}", compact_slot(&want)),
                    ));
                }
                (None, None) => {}
            }
        }
    }

    // ---- Where clause. ----
    // Plus-side `W` preserves; minus-side `W` with an explicit plus side
    // sets it. Omission preserves.
    if !matches!(plus.where_clause, rsmpl_pattern::WherePat::Meta(_)) {
        let minus_w = match &minus.where_clause {
            rsmpl_pattern::WherePat::Concrete(t) => Some(t.clone()),
            _ => None,
        };
        let plus_w = match &plus.where_clause {
            rsmpl_pattern::WherePat::Concrete(t) => Some(t.clone()),
            rsmpl_pattern::WherePat::Absent => None,
            rsmpl_pattern::WherePat::Meta(_) => None,
            rsmpl_pattern::WherePat::Any => None,
        };
        // Plus Absent with minus Absent/Any: preserve (no delete signal).
        // Plus Absent with minus Concrete: delete the clause.
        let plus_absent = matches!(plus.where_clause, WherePat::Absent);
        if minus_w.as_deref().map(norm_list_slot) != plus_w.as_deref().map(norm_list_slot) {
            match (where_bytes(original, index, sig), plus_w) {
                (Some((a, b)), Some(want)) => {
                    let rep = format!("where {}", compact_slot(&want));
                    if norm_list_slot(original.get(a..b).unwrap_or("")) != norm_list_slot(&rep) {
                        edits.push(TextEdit::new(a, b, rep));
                    }
                }
                (Some((a, b)), None) => {
                    if plus_absent && minus_w.is_some() {
                        edits.push(TextEdit::new(
                            a,
                            swallow_one_space(original, b),
                            String::new(),
                        ));
                    }
                }
                (None, Some(want)) => {
                    let anchor = match &sig.output {
                        syn::ReturnType::Type(_, ty) => {
                            let e = ty.span().end();
                            index
                                .offset(e.line as u32, e.column)
                                .ok_or_else(|| range_err(e.line as u32))?
                        }
                        syn::ReturnType::Default => {
                            let e = sig.paren_token.span.close().end();
                            index
                                .offset(e.line as u32, e.column)
                                .ok_or_else(|| range_err(e.line as u32))?
                        }
                    };
                    let text = format!(" where {}", compact_slot(&want));
                    // A same-anchor return-type insertion may already exist
                    // (plus adds both `-> R` and `where ...`); merge so the
                    // return type stays before the where clause.
                    if let Some(r) = edits
                        .iter_mut()
                        .find(|e| e.is_insertion() && e.start == anchor)
                    {
                        r.replacement = format!("{}{}", r.replacement, text);
                    } else {
                        edits.push(TextEdit::new(anchor, anchor, text));
                    }
                }
                (None, None) => {}
            }
        }
    }

    Ok(edits)
}

/// Apply a function-rename hunk. Returns the number of matches rewritten.
pub fn apply_fn_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (minus_name, plus_name) = match &compiled.kind {
        PatternKind::Fn {
            minus_name,
            plus_name,
            ..
        } => (minus_name.clone(), plus_name.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected function pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();
    let file = "";
    let minus_pat = minus_fn_pattern(hunk, compiled).map_err(|e| e.with_rule(&rule))?;
    let full_plus: String = hunk
        .lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let (vis, qual, gen, wh) = slot_metas(compiled);
    let plus_pat = rsmpl_pattern::parse_fn_pattern(&full_plus, &vis, &qual, &gen, &wh, false)
        .map_err(|e| e.with_rule(&rule))?;

    // Bodies are preserved, never rewritten: both sides must be `...`
    // (or `;`). A concrete body would be silently ignored, so fail fast.
    if !minus_pat.body_ellipsis || !plus_pat.body_ellipsis {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v1 function hunks require `...` bodies on both sides (body evolution is not supported; the original body is always preserved)",
        )
        .with_rule(&rule));
    }

    // Attribute mirror gates (same contract as the general-Stmt matcher:
    // plus-side attribute text must mirror the minus side, never invent
    // or drop — silently ignoring written attributes would violate SPEC
    // 2.6). The plus side is otherwise dead text: `plan_sig_edits`
    // preserves the original envelope and never reads plus attrs.
    if !plus_pat.attrs_written.is_empty() && minus_pat.attrs_written.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`"));
    }
    if hunk.attrs_policy() != AttrsPolicy::Strip
        && !minus_pat.attrs_written.is_empty()
        && minus_pat
            .attrs_written
            .iter()
            .map(|a| norm_slot(a))
            .collect::<Vec<_>>()
            != plus_pat
                .attrs_written
                .iter()
                .map(|a| norm_slot(a))
                .collect::<Vec<_>>()
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`"));
    }

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for c in collect_fns(parsed) {
        if !in_scope(&c, hunk.scope(), old_start, old_len) {
            continue;
        }
        // Name must equal the minus name (or bind if minus is `$Meta`).
        let name_ok = if minus_name.starts_with('$') {
            true
        } else {
            c.item.sig.ident == minus_name.as_str()
        };
        if !name_ok {
            continue;
        }
        if !minus_pat.matches_item_fn(c.item) {
            continue;
        }
        // Strict slot refinement: explicit minus-side slot text must equal
        // the target (fail over guessing).
        if !minus_matches_strict(&minus_pat, &c.item.vis, &c.item.sig, original, index) {
            continue;
        }
        // Written attributes constrain even under the default preserve
        // policy (SPEC 14.6): a site lacking them is a non-match like any
        // other slot mismatch, so skip it — count validation (plus the
        // T62 whole-file hint) keeps typos fail-safe.
        if hunk.attrs_policy() == AttrsPolicy::Preserve && !minus_pat.attrs_written.is_empty() {
            if !attrs_equal(&minus_pat.attrs_written, &c.item.attrs) {
                continue;
            }
        } else {
            check_attrs(
                &c,
                hunk.attrs_policy(),
                &minus_pat.attrs_written,
                &rule,
                file,
            )?;
        }
        if hunk.returns_none() && !matches!(c.item.sig.output, syn::ReturnType::Default) {
            continue;
        }
        // Signature evolution: name plus any explicitly changed slots.
        // Plus name may be `$Meta` (abstracted) meaning "keep original".
        let mut new_edits = plan_sig_edits(
            &c.item.vis,
            &c.item.sig,
            &minus_pat,
            &plus_pat,
            &minus_name,
            &plus_name,
            original,
            index,
            &rule,
        )?;
        edits.append(&mut new_edits);
        // `# attrs=strip`: delete attribute lines above the fn item.
        if hunk.attrs_policy() == AttrsPolicy::Strip && !c.item.attrs.is_empty() {
            let start = index
                .line_start(c.attr_start_line)
                .ok_or_else(|| range_err(&rule, c.attr_start_line))?;
            let end = index
                .line_start(c.fn_line)
                .ok_or_else(|| range_err(&rule, c.fn_line))?;
            if end > start {
                edits.push(TextEdit::new(start, end, String::new()));
            }
        }
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

/// Find item targets (function or other items) for an item-capture hunk.
///
/// v0.1 matches any item (fn, struct, mod, use, ...) by line-span
/// containment; only functions are structurally verified elsewhere.
pub(crate) fn find_item_targets(
    parsed: &syn::File,
    scope: Scope,
    old_start: u32,
    old_len: u32,
) -> Vec<(u32, u32)> {
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for c in collect_fns(parsed) {
        if in_scope(&c, scope, old_start, old_len) {
            candidates.push((c.start_line, c.end_line));
        }
    }
    candidates.extend(collect_non_fn_item_spans(parsed, scope, old_start, old_len));
    candidates.sort();
    candidates.dedup();
    candidates
}

/// Delete the single matched item for a bare `- I` hunk (SPEC 18.2).
pub fn apply_item_delete(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    match &compiled.kind {
        PatternKind::ItemCapture { .. } => {}
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected item capture pattern".to_string(),
            ))
        }
    }
    let rule = hunk.rule_name();
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let candidates = find_item_targets(parsed, hunk.scope(), old_start, old_len);
    if candidates.is_empty() {
        return Ok(0);
    }
    // allow=one requires exactly one; for deletion, `many` deletes all
    // contained items.
    let targets: Vec<(u32, u32)> = match hunk.allow() {
        rsmpl_patch::Allow::One => vec![candidates[0]],
        rsmpl_patch::Allow::Many => candidates,
    };
    let n = targets.len();
    for (sl, el) in targets {
        edits.push(delete_lines(original, index, sl, el, &rule)?);
    }
    Ok(n)
}

/// Whole-line deletion edit from `start_line` through `end_line` (inclusive),
/// swallowing the trailing newline.
///
/// To avoid leaving stray blank separator lines, the range additionally
/// absorbs one adjacent blank line: the following blank line when one exists,
/// otherwise one preceding blank line at EOF. Comments and code are never
/// absorbed — only a fully blank line.
pub(crate) fn delete_lines(
    original: &str,
    index: &LineIndex,
    start_line: u32,
    end_line: u32,
    rule: &str,
) -> Result<TextEdit, StructuredError> {
    let mut start = index
        .line_start(start_line)
        .ok_or_else(|| range_err(rule, start_line))?;
    // End at the start of the following line (this swallows the trailing
    // newline); at EOF use EOF.
    let mut end = index
        .line_start(end_line + 1)
        .unwrap_or_else(|| index.eof());
    if end < original.len() {
        // Absorb one following blank line (a lone `\n` right at `end`).
        if original.as_bytes().get(end) == Some(&b'\n') {
            end += 1;
        }
    } else if start > 0 {
        // At EOF: absorb one preceding blank line.
        let before = &original[..start];
        if before.ends_with("\n\n") {
            // Move `start` back over the blank line.
            let trimmed = &before[..before.len() - 1];
            if let Some(pos) = trimmed.rfind('\n') {
                start = pos + 1;
            } else {
                start = 0;
            }
        }
    }
    Ok(TextEdit::new(start, end, String::new()))
}

/// Non-function item spans (struct/enum/mod/use/...) via syn item spans.
fn collect_non_fn_item_spans(
    parsed: &syn::File,
    scope: Scope,
    old_start: u32,
    old_len: u32,
) -> Vec<(u32, u32)> {
    let end = old_start + old_len.saturating_sub(1);
    let mut out = Vec::new();
    for it in &parsed.items {
        if matches!(it, syn::Item::Fn(_)) {
            continue;
        }
        let s = it.span().start().line as u32;
        let e = it.span().end().line as u32;
        let inside = match scope {
            Scope::File => true,
            Scope::Contained => s >= old_start && e <= end.max(old_start),
            Scope::Anchor => s >= old_start && s <= end.max(old_start),
            Scope::Overlap => s <= end.max(old_start) && e >= old_start,
            Scope::Exact => old_len > 0 && s == old_start && e == end,
        };
        if inside {
            out.push((s, e));
        }
    }
    out
}

/// Insert literal text at a zero-length anchor (SPEC 18.3, 10.5).
pub fn apply_insertion(
    _original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (text, _use_name) = match &compiled.kind {
        PatternKind::Insertion { text, use_name } => (text.clone(), use_name.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected insertion pattern".to_string(),
            ))
        }
    };
    let anchor = hunk.header.old_start;
    let pos = if anchor == 0 {
        0
    } else {
        index.line_start(anchor + 1).unwrap_or_else(|| index.eof())
    };
    // Ensure the inserted text ends with a newline so it forms whole lines.
    let mut ins = text;
    if !ins.ends_with('\n') {
        ins.push('\n');
    }
    edits.push(TextEdit::new(pos, pos, ins));
    Ok(1)
}

/// Rename a function inside an `impl` or `trait` context block (SPEC 29.9).
pub fn apply_impl_context_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let impl_header = match &compiled.kind {
        PatternKind::ImplContext { impl_header } => impl_header.clone(),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected impl-context pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();
    // Method-context covers `impl` blocks (existing behavior) and `trait`
    // blocks (T182: `;` declarations + default bodies via `TraitItem::Fn`).
    // The pattern header picks the half: an `impl` header never matches a
    // trait block and vice versa.
    let want_trait = rsmpl_pattern::is_trait_header(&impl_header);
    // Find the block whose header matches (whitespace-normalized).
    let want = impl_header.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut matched = 0;
    for it in &parsed.items {
        // Resolve each item to a borrowable method list; other items (incl.
        // the opposite block kind) are skipped. `inherited` backs the
        // trait-method view (trait methods carry no `vis` field).
        let inherited = syn::Visibility::Inherited;
        let mut methods: Vec<(
            &syn::Visibility,
            &syn::Signature,
            &[syn::Attribute],
            proc_macro2::Span,
        )> = Vec::new();
        match it {
            syn::Item::Impl(imp) if !want_trait => {
                // T336: under `# scope=file` the header is purely advisory
                // (SPEC 13.4) — every `impl` block is scanned and the inner
                // fn match decides, giving impl/trait sweep parity.
                if hunk.scope() != Scope::File {
                    let header_tokens: Vec<&str> = want.split_whitespace().collect();
                    // Verify the impl's self type + optional trait match the pattern by
                    // comparing normalized renderings.
                    let pat_self = header_tokens.join(" ");
                    let self_ty = &imp.self_ty;
                    let got_self = quote::quote!(#self_ty)
                        .to_string()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    let got_trait = imp
                        .trait_
                        .as_ref()
                        .map(|(_, p, _)| {
                            quote::quote!(#p)
                                .to_string()
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    let pat_has_trait = pat_self.contains(&got_trait) && !got_trait.is_empty();
                    let pat_mentions_self = header_tokens.iter().any(|t| {
                        got_self.split_whitespace().any(|g| g == *t) || pat_self.contains(&got_self)
                    });
                    if !pat_has_trait && !pat_mentions_self {
                        // Fallback: accept impl blocks overlapping the range and let the
                        // inner fn match decide (range-gated).
                        let s = it.span().start().line as u32;
                        let e = it.span().end().line as u32;
                        let end = hunk.header.old_start + hunk.header.old_len.saturating_sub(1);
                        if !(s <= end.max(hunk.header.old_start) && e >= hunk.header.old_start) {
                            continue;
                        }
                    }
                }
                for item in &imp.items {
                    if let syn::ImplItem::Fn(m) = item {
                        methods.push((&m.vis, &m.sig, &m.attrs, m.span()));
                    }
                }
            }
            syn::Item::Trait(t) if want_trait => {
                // T336: under `# scope=file` the header is purely advisory
                // (SPEC 13.4) — every `trait` block is scanned, impl parity.
                if hunk.scope() != Scope::File {
                    // T182: the header names the trait ident
                    // (`trait T<U>: B ...` → `T`). Anything else (attribute
                    // lines, garbage) falls back to the range-overlap check so
                    // a mistyped header fails as no-match, never a wrong match.
                    let ident_ok = rsmpl_pattern::trait_header_ident(&impl_header)
                        .is_some_and(|w| t.ident == w);
                    if !ident_ok {
                        let s = it.span().start().line as u32;
                        let e = it.span().end().line as u32;
                        let end = hunk.header.old_start + hunk.header.old_len.saturating_sub(1);
                        if !(s <= end.max(hunk.header.old_start) && e >= hunk.header.old_start) {
                            continue;
                        }
                    }
                }
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item {
                        methods.push((&inherited, &f.sig, &f.attrs, f.span()));
                    }
                }
            }
            _ => continue,
        }
        // Match methods inside (same slot-evolution semantics as free
        // functions, via the shared planner).
        let minus_pat = minus_fn_pattern(hunk, compiled).map_err(|e| e.with_rule(&rule))?;
        let (pvis, pqual, pgen, pwh) = slot_metas(compiled);
        let full_plus_all: String = hunk
            .lines
            .iter()
            .filter(|l| l.is_addition() || l.is_context())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let plus_pat = rsmpl_pattern::parse_fn_pattern(
            &extract_fn_block(&full_plus_all),
            &pvis,
            &pqual,
            &pgen,
            &pwh,
            false,
        )
        .map_err(|e| e.with_rule(&rule))?;
        if !minus_pat.body_ellipsis || !plus_pat.body_ellipsis {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "v1 function hunks require `...` bodies on both sides (body evolution is not supported; the original body is always preserved)",
            )
            .with_rule(&rule));
        }
        // Attribute mirror gates (same contract as `apply_fn_hunk`: the
        // plus side must mirror the minus side, never invent or drop).
        if !plus_pat.attrs_written.is_empty() && minus_pat.attrs_written.is_empty() {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them",
            )
            .with_rule(&rule)
            .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`"));
        }
        if hunk.attrs_policy() != AttrsPolicy::Strip
            && !minus_pat.attrs_written.is_empty()
            && minus_pat
                .attrs_written
                .iter()
                .map(|a| norm_slot(a))
                .collect::<Vec<_>>()
                != plus_pat
                    .attrs_written
                    .iter()
                    .map(|a| norm_slot(a))
                    .collect::<Vec<_>>()
        {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
            )
            .with_rule(&rule)
            .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`"));
        }
        let minus_name = pattern_name(&minus_pat.name);
        let plus_name = pattern_name(&plus_pat.name);
        // T182: trait methods never carry visibility qualifiers, so a `+`
        // side writing `pub` could only emit invalid Rust (the reparse
        // backstop would catch it with a confusing message). Fail fast with
        // the repair; the minus side needs no gate (explicit `pub` simply
        // never matches inherited trait visibility).
        if want_trait && matches!(plus_pat.visibility, rsmpl_pattern::VisPat::Concrete(_)) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "trait methods cannot declare visibility: drop `pub` (or `pub(...)`) on the `+` side",
            )
            .with_rule(&rule));
        }
        for (vis, sig, attrs, span) in methods {
            let name_ok = if minus_name.starts_with('$') {
                true
            } else {
                sig.ident == minus_name.as_str()
            };
            if !name_ok {
                continue;
            }
            // Scope: method span within old range.
            let ms = span.start().line as u32;
            let me = span.end().line as u32;
            let end = hunk.header.old_start + hunk.header.old_len.saturating_sub(1);
            let inside = match hunk.scope() {
                Scope::File => true,
                Scope::Contained => {
                    ms >= hunk.header.old_start && me <= end.max(hunk.header.old_start)
                }
                _ => ms >= hunk.header.old_start && ms <= end.max(hunk.header.old_start),
            };
            if !inside {
                continue;
            }
            // Qualifier/visibility strictness via minus pattern on a
            // synthetic ItemFn. The synth carries the method's real
            // visibility: `Signature` alone renders inherited vis, which
            // would fail every explicit-`pub` pattern (T175).
            let synth: syn::ItemFn =
                syn::parse_str(&format!("{} {{}}", full_sig_without_body(vis, sig))).map_err(
                    |_| {
                        StructuredError::new(
                            ErrorCode::PatternConflict,
                            "cannot analyze method signature",
                        )
                        .with_rule(&rule)
                    },
                )?;
            if !minus_pat.matches_item_fn(&synth) {
                continue;
            }
            if !minus_matches_strict(&minus_pat, vis, sig, original, index) {
                continue;
            }
            // Written attributes constrain under default preserve, like
            // free functions (site-level filter, not an error).
            if hunk.attrs_policy() == AttrsPolicy::Preserve
                && !minus_pat.attrs_written.is_empty()
                && !attrs_equal(&minus_pat.attrs_written, attrs)
            {
                continue;
            }
            if hunk.returns_none() && !matches!(sig.output, syn::ReturnType::Default) {
                continue;
            }
            // Signature evolution: name plus any explicitly changed slots.
            let mut new_edits = plan_sig_edits(
                vis,
                sig,
                &minus_pat,
                &plus_pat,
                &minus_name,
                &plus_name,
                original,
                index,
                &rule,
            )?;
            edits.append(&mut new_edits);
            matched += 1;
        }
    }
    Ok(matched)
}

/// Pattern function name as match text (`$Meta` stays sigiled).
fn pattern_name(name: &rsmpl_pattern::NamePat) -> String {
    match name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(n) => format!("${n}"),
    }
}

/// Render a signature without its body for synthetic parsing, carrying the
/// item's real visibility: `Signature` alone has none, so without `vis` the
/// synth would read as inherited and fail every explicit-`pub` pattern.
fn full_sig_without_body(vis: &syn::Visibility, sig: &syn::Signature) -> String {
    let mut s = quote::quote!(#sig).to_string();
    // `quote` renders `fn foo (x : i32)` etc.; parse needs no trailing body.
    // Drop anything from the first `{` (shouldn't be present for Signature).
    if let Some(pos) = s.find('{') {
        s.truncate(pos);
    }
    // Ensure it starts with `fn` (methods have receiver; keep as-is for
    // qualifier checks by wrapping in a trait? No: parse as ItemFn requires
    // no `self`. Replace `self` params textually for the check.)
    // NOTE: the `self` rewrite stays scoped to the sig text -- it runs
    // before the vis prepend so `pub(in self::...)` paths are untouched.
    s = replace_self_receiver(&s);
    // T190: `quote!(#sig)` renders qualifiers (`const fn …`, `async fn …`,
    // `unsafe fn …`, `extern "ABI" fn …`), so gating the defensive `fn `
    // prepend on a bare-`fn` prefix turned every qualified method synth into
    // `fn const fn …` (unparsable → bare `cannot analyze method signature`).
    // Accept the qualifier prefixes; the prepend stays for whatever
    // non-`fn` rendering needed it.
    let t = s.trim_start();
    if !t.starts_with("fn ")
        && !t.starts_with("const ")
        && !t.starts_with("async ")
        && !t.starts_with("unsafe ")
        && !t.starts_with("extern ")
    {
        s = format!("fn {s}");
    }
    let vis_text = quote::quote!(#vis).to_string();
    if vis_text.trim().is_empty() {
        s
    } else {
        format!("{vis_text} {s}")
    }
}

/// Replace the `self` receiver for synthetic parsing, word-aware (T221).
/// The impl/trait-context presence gate parses a synthetic free `fn`
/// built from the method signature, and a bare `self` receiver does not
/// parse there -- so the receiver is rewritten to a placeholder param.
/// The old blind `str::replace("self", …)` broke three valid spellings:
/// lifetime receivers (`&'a self` -- reference patterns take no
/// lifetime), idents merely containing `self` (`myself`), and typed
/// receivers (`self: Box<Self>`). All three failed with the bare
/// `cannot analyze method signature` needle. Scan rules: `& 'lt [mut]
/// self` drops the lifetime (reference pattern + `: u8` type, as
/// before); a `self` word followed by `:` (but not `::`) keeps its type
/// (`self: Box<Self>` becomes the placeholder with the same type); any
/// other `self` word becomes the placeholder param. `self::` paths and
/// `Self` are never touched.
fn replace_self_receiver(s: &str) -> String {
    fn is_ident(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < b.len() {
        // `& 'lt [mut] self`: drop the lifetime, keep `& [mut]`.
        if b[i] == b'&' {
            let mut j = i + 1;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b'\'' {
                j += 1;
                while j < b.len() && is_ident(b[j]) {
                    j += 1;
                }
                while j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                let mut has_mut = false;
                if s[j..].starts_with("mut") && (j + 3 >= b.len() || !is_ident(b[j + 3])) {
                    has_mut = true;
                    j += 3;
                    while j < b.len() && b[j] == b' ' {
                        j += 1;
                    }
                }
                if s[j..].starts_with("self") && (j + 4 >= b.len() || !is_ident(b[j + 4])) {
                    out.push_str("& ");
                    if has_mut {
                        out.push_str("mut ");
                    }
                    out.push_str("_self_placeholder : u8");
                    i = j + 4;
                    continue;
                }
            }
            out.push('&');
            i += 1;
            continue;
        }
        if s[i..].starts_with("self")
            && (i == 0 || !is_ident(b[i - 1]))
            && (i + 4 >= b.len() || !is_ident(b[i + 4]))
        {
            // Typed receiver (`self: Type`, not `self::path`): keep the type.
            let mut j = i + 4;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b':' {
                if j + 1 >= b.len() || b[j + 1] != b':' {
                    out.push_str("_self_placeholder");
                    i += 4;
                    continue;
                }
                // `self::…`: not a receiver; leave untouched.
                out.push_str("self");
                i += 4;
                continue;
            }
            out.push_str("_self_placeholder : u8");
            i += 4;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    const RENAME: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";

    #[test]
    fn renames_plain_fn_preserving_body_and_attrs() {
        let src = "/// Adds one.\n#[inline]\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n";
        let out = apply(RENAME, src).unwrap();
        assert!(out.contains("fn bar(x: i32) -> i32 {"), "got: {out}");
        assert!(out.contains("/// Adds one."));
        assert!(out.contains("#[inline]"));
        assert!(!out.contains("fn foo"));
    }

    #[test]
    fn rejects_pub_fn_for_plain_pattern() {
        let src = "pub fn foo() {}\n";
        let patch = RENAME.replace("@@ -1,5 +1,5 @@", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn rejects_generic_fn_for_plain_pattern() {
        let src = "fn foo<T>(x: T) {}\n";
        let patch = RENAME.replace("@@ -1,5 +1,5 @@", "@@ -1,1 +1,1 @@");
        assert!(apply(&patch, src).is_err());
    }

    #[test]
    fn returns_none_constrains() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# returns=none\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        assert!(apply(patch, "fn foo() {}\n").is_ok());
        assert!(apply(patch, "fn foo() -> i32 {\n    1\n}\n").is_err());
    }

    #[test]
    fn attrs_absent_rejects_attributed() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# attrs=absent\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        assert!(apply(patch, "fn foo() {}\n").is_ok());
        let patch2 = patch.replace("@@ -1,1 +1,1 @@", "@@ -1,2 +1,2 @@");
        assert!(apply(&patch2, "#[inline]\nfn foo() {}\n").is_err());
    }

    #[test]
    fn attrs_explicit_mirrored_applies() {
        // T153: the fn attribute loop detects the `#` + `[` token pair, so
        // mirrored explicit attributes compare whitespace-insensitively and
        // the rename lands byte-exact (plain and indented targets).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# attrs=explicit\n- #[inline]\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
        let out = apply(patch, "#[inline]\nfn foo() {}\n").unwrap();
        assert_eq!(out, "#[inline]\nfn bar() {}\n", "got: {out:?}");
        let out2 = apply(patch, "#[inline]\n    fn foo() {}\n").unwrap();
        assert_eq!(out2, "#[inline]\n    fn bar() {}\n", "got: {out2:?}");
        // Wrong attribute still fails instead of silently matching.
        let wrong = patch.replace("- #[inline]", "- #[other]");
        assert!(apply(&wrong, "#[inline]\nfn foo() {}\n").is_err());
    }

    #[test]
    fn attrs_explicit_doc_comment_mirror_applies() {
        // T155: `///` doc lines normalize to `#[doc = "..."]`, so explicit
        // doc-comment mirroring parses and compares like `#[...]` attrs.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n# attrs=explicit\n- /// Doc comment.\n- fn foo(...) {\n+ /// Doc comment.\n+ fn bar(...) {\n      ...\n  }\n";
        let out = apply(patch, "/// Doc comment.\nfn foo() {}\n").unwrap();
        assert_eq!(out, "/// Doc comment.\nfn bar() {}\n", "got: {out:?}");
        // Wrong doc text still fails instead of silently matching.
        let wrong = patch.replace("- /// Doc comment.", "- /// Other text.");
        assert!(apply(&wrong, "/// Doc comment.\nfn foo() {}\n").is_err());
    }

    #[test]
    fn attrs_preserve_written_constrains_site() {
        // T165: explicitly written attributes constrain even under the
        // default preserve policy — other sites are skipped like any slot
        // mismatch (count validation keeps typos fail-safe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n- #[inline]\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
        let out = apply(patch, "#[inline]\nfn foo() {}\n").unwrap();
        assert_eq!(out, "#[inline]\nfn bar() {}\n", "got: {out:?}");
        // Wrong attribute: site skipped → no match → error, not a silent
        // rename of the wrong item.
        let err = apply(patch, "#[other]\nfn foo() {}\n").unwrap_err();
        assert!(err.contains("no match found"), "got: {err}");
    }

    #[test]
    fn attrs_preserve_written_filters_many() {
        // T165: with `allow=many` over mixed targets only the attributed
        // site rewrites; the bare `fn foo` is skipped, not renamed.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 @@ rule=x\n# lang=rust\n# allow=many\n- #[inline]\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
        let src = "#[inline]\nfn foo() {}\nfn foo() {}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out, "#[inline]\nfn bar() {}\nfn foo() {}\n", "got: {out:?}");
    }

    #[test]
    fn attrs_mirror_gates_fail_fast() {
        // T165: the plus side must mirror the minus side — inventing or
        // changing attributes is a pattern error, never a silent drop.
        let add = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- fn foo(...) {\n+ #[inline]\n+ fn bar(...) {\n      ...\n  }\n";
        let err = apply(add, "fn foo() {}\n").unwrap_err();
        assert!(
            err.contains("replacement writes attributes the pattern did not match"),
            "got: {err}"
        );
        let diff = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=x\n# lang=rust\n- #[inline]\n- fn foo(...) {\n+ #[other]\n+ fn bar(...) {\n      ...\n  }\n";
        let err = apply(diff, "#[inline]\nfn foo() {}\n").unwrap_err();
        assert!(
            err.contains("replacement attributes differ from matched attributes"),
            "got: {err}"
        );
    }

    #[test]
    fn optional_no_match_ok() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# optional\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let out = apply(patch, "fn a() {}\n").unwrap();
        assert_eq!(out, "fn a() {}\n");
    }

    #[test]
    fn deletes_item() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,0 @@ rule=cut\n# lang=rust\n# item I\n- I\n";
        let src = "fn a() {}\nfn dead() {}\nfn b() {}\n";
        // Range covers only `fn dead` (line 2); single-item delete.
        let patch2 = patch.replace("@@ -1,3 +1,0 @@", "@@ -2,1 +2,0 @@");
        let out = apply(&patch2, src).unwrap();
        assert!(!out.contains("fn dead"), "got: {out}");
        assert!(out.contains("fn a"));
        assert!(out.contains("fn b"));
    }

    #[test]
    fn inserts_use_item() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,1 @@ rule=ins\n# lang=rust\n+ use crate::new_module;\n";
        let src = "fn a() {}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("use crate::new_module;"), "got: {out}");
    }

    #[test]
    fn renames_inside_impl() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=x\n# lang=rust\n  impl Service {\n      ...\n-     fn old_name(...) {\n+     fn new_name(...) {\n           ...\n       }\n  }\n";
        let src = "impl Service {\n    fn old_name(&self) {}\n    fn other(&self) {}\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("fn new_name(&self)"), "got: {out}");
        assert!(out.contains("fn other(&self)"));
    }

    #[test]
    fn renames_trait_declaration() {
        // T182: `;` declarations route through the trait half of
        // method-context matching (previously no-match).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n  trait T {\n      ...\n-     fn foo();\n+     fn bar();\n      ...\n  }\n";
        let src = "trait T {\n    fn foo();\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out, "trait T {\n    fn bar();\n}\n", "got: {out}");
    }

    #[test]
    fn renames_unsafe_trait_declaration() {
        // T185: `unsafe trait` headers start with the fn-like `unsafe `
        // prefix, so the block scan must skip them (previously failed
        // with "must contain `fn` keyword").
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n  unsafe trait T {\n      ...\n-     fn foo();\n+     fn bar();\n      ...\n  }\n";
        let src = "unsafe trait T {\n    fn foo();\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out, "unsafe trait T {\n    fn bar();\n}\n", "got: {out}");
    }

    #[test]
    fn renames_inside_unsafe_impl() {
        // T185: same `unsafe `-prefix hole on the impl half.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,3 +3,3 @@ rule=x\n# lang=rust\n  unsafe impl U for S {\n      ...\n-     fn m(...) {\n+     fn n(...) {\n          ...\n      }\n  }\n";
        let src = "trait U { fn m(x: i32) -> i32; }\nstruct S;\nunsafe impl U for S {\n    fn m(x: i32) -> i32 { x }\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("fn n(x: i32) -> i32 { x }"), "got: {out}");
    }

    #[test]
    fn renames_trait_default_body() {
        // T182: default bodies rename with the body preserved.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n  trait T {\n      ...\n-     fn foo(...) {\n+     fn bar(...) {\n          ...\n      }\n  }\n";
        let src = "trait T {\n    fn foo() {}\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out, "trait T {\n    fn bar() {}\n}\n", "got: {out}");
    }

    #[test]
    fn evolves_trait_method_sig() {
        // T182: `&self` methods evolve params/return through the shared
        // slot planner; default bodies are preserved.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n  trait T {\n      ...\n-     fn get(&self, x: i32) -> i32 {\n+     fn get(&self, x: i64) -> i64 {\n          ...\n      }\n  }\n";
        let src = "trait T {\n    fn get(&self, x: i32) -> i32 { x }\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(
            out, "trait T {\n    fn get(&self, x: i64) -> i64 { x }\n}\n",
            "got: {out}"
        );
    }

    #[test]
    fn free_pattern_ignores_trait_methods() {
        // T182: no widening — a bare free-fn pattern never matches inside a
        // trait block; only a `trait` context header routes there.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# scope=file\n- fn foo();\n+ fn bar();\n";
        let src = "trait T {\n    fn foo();\n}\n";
        assert!(apply(patch, src).is_err());
    }

    #[test]
    fn trait_plus_pub_fails_fast() {
        // T182: trait methods never carry visibility — a `+` side writing
        // `pub` fails with the repair needle, not a reparse error.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=x\n# lang=rust\n  trait T {\n      ...\n-     fn foo();\n+     pub fn bar();\n      ...\n  }\n";
        let src = "trait T {\n    fn foo();\n}\n";
        let err = apply(patch, src).unwrap_err();
        assert!(err.contains("visibility"), "got: {err}");
    }

    #[test]
    fn mixed_ordinary_and_semantic_disjoint_ok() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,8 +1,8 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\n@@ -8,3 +8,3 @@\n fn calls() {\n-    old_call();\n+    new_call();\n }\n";
        let src = "fn foo() {}\nfn other() {}\nfn more() {}\nfn extra() {}\nfn tail() {}\nfn t2() {}\nfn t3() {}\nfn calls() {\n    old_call();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("fn bar()"), "got: {out}");
        assert!(out.contains("new_call();"), "got: {out}");
    }

    #[test]
    fn mixed_overlap_rejected() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\n@@ -1,1 +1,1 @@\n-fn foo() {}\n+fn foo() { 1; }\n";
        let src = "fn foo() {}\n";
        assert!(apply(patch, src).is_err());
    }

    fn sig_patch(minus: &str, plus: &str, range: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=evolve\n# lang=rust\n{minus}\n{plus}\n      ...\n  }}\n"
        )
    }

    #[test]
    fn adds_visibility() {
        let patch = sig_patch("- fn foo(...) {", "+ pub fn foo(...) {", "@@ -1,1 +1,1 @@");
        let out = apply(&patch, "fn foo() {}\n").unwrap();
        assert_eq!(out, "pub fn foo() {}\n");
    }

    #[test]
    fn removes_visibility() {
        let patch = sig_patch("- pub fn foo(...) {", "+ fn foo(...) {", "@@ -1,1 +1,1 @@");
        let out = apply(&patch, "pub fn foo() {}\n").unwrap();
        assert_eq!(out, "fn foo() {}\n");
    }

    #[test]
    fn changes_visibility() {
        let patch = sig_patch(
            "- pub fn foo(...) {",
            "+ pub(crate) fn foo(...) {",
            "@@ -1,1 +1,1 @@",
        );
        let out = apply(&patch, "pub fn foo() {}\n").unwrap();
        assert_eq!(out, "pub(crate) fn foo() {}\n");
    }

    #[test]
    fn rejects_wrong_concrete_visibility() {
        // Strictness (was presence-only): `pub` does not match `pub(crate)`.
        let patch = sig_patch(
            "- pub fn foo(...) {",
            "+ pub fn bar(...) {",
            "@@ -1,1 +1,1 @@",
        );
        assert!(apply(&patch, "pub(crate) fn foo() {}\n").is_err());
    }

    #[test]
    fn adds_and_removes_qualifier() {
        let add = sig_patch(
            "- fn foo(...) {",
            "+ async fn foo(...) {",
            "@@ -1,1 +1,1 @@",
        );
        assert_eq!(apply(&add, "fn foo() {}\n").unwrap(), "async fn foo() {}\n");
        let del = sig_patch(
            "- async fn foo(...) {",
            "+ fn foo(...) {",
            "@@ -1,1 +1,1 @@",
        );
        assert_eq!(apply(&del, "async fn foo() {}\n").unwrap(), "fn foo() {}\n");
    }

    #[test]
    fn adds_and_removes_generics() {
        let add = sig_patch("- fn foo(...) {", "+ fn foo<T>(...) {", "@@ -1,1 +1,1 @@");
        assert_eq!(apply(&add, "fn foo() {}\n").unwrap(), "fn foo<T>() {}\n");
        let del = sig_patch("- fn foo<T>(...) {", "+ fn foo(...) {", "@@ -1,1 +1,1 @@");
        assert_eq!(apply(&del, "fn foo<T>() {}\n").unwrap(), "fn foo() {}\n");
    }

    #[test]
    fn rejects_wrong_concrete_generics() {
        let patch = sig_patch(
            "- fn foo<T>(...) {",
            "+ fn foo<T>(...) {",
            "@@ -1,1 +1,1 @@",
        );
        // Target has different generics: no match (fail over guessing).
        assert!(apply(&patch, "fn foo<U>() {}\n").is_err());
    }

    #[test]
    fn adds_changes_removes_return() {
        let add = sig_patch(
            "- fn foo(...) {",
            "+ fn foo(...) -> i32 {",
            "@@ -1,1 +1,1 @@",
        );
        assert_eq!(
            apply(&add, "fn foo() {}\n").unwrap(),
            "fn foo() -> i32 {}\n"
        );
        let chg = sig_patch(
            "- fn foo(...) -> i32 {",
            "+ fn foo(...) -> String {",
            "@@ -1,1 +1,1 @@",
        );
        assert_eq!(
            apply(&chg, "fn foo() -> i32 { 1 }\n").unwrap(),
            "fn foo() -> String { 1 }\n"
        );
        // Minus names a concrete return, plus omits it: explicit removal.
        let del = sig_patch(
            "- fn foo(...) -> i32 {",
            "+ fn foo(...) {",
            "@@ -1,1 +1,1 @@",
        );
        assert_eq!(
            apply(&del, "fn foo() -> i32 { 1 }\n").unwrap(),
            "fn foo() { 1 }\n"
        );
    }

    #[test]
    fn preserves_return_when_both_omit() {
        // Rename fast-path: no `->` on either side keeps the return type.
        let patch = sig_patch("- fn foo(...) {", "+ fn bar(...) {", "@@ -1,1 +1,1 @@");
        assert_eq!(
            apply(&patch, "fn foo() -> i32 { 1 }\n").unwrap(),
            "fn bar() -> i32 { 1 }\n"
        );
    }

    #[test]
    fn evolves_where_clause() {
        let src = "fn foo<T>(x: T) {\n    x\n}\n";
        let add = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evolve\n# lang=rust\n- fn foo<T>(...) {\n+ fn foo<T>(...) where T: Clone {\n      ...\n  }\n";
        assert_eq!(
            apply(add, src).unwrap(),
            "fn foo<T>(x: T) where T: Clone {\n    x\n}\n"
        );
        let del = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evolve\n# lang=rust\n- fn foo<T>(...) where T: Clone {\n+ fn foo<T>(...) {\n      ...\n  }\n";
        let src2 = "fn foo<T>(x: T) where T: Clone {\n    x\n}\n";
        assert_eq!(apply(del, src2).unwrap(), "fn foo<T>(x: T) {\n    x\n}\n");
    }

    #[test]
    fn concretizes_and_preserves_params() {
        // Minus `...`, plus concrete: replace the parameter list.
        let patch = sig_patch("- fn foo(...) {", "+ fn foo(x: i32) {", "@@ -1,3 +1,3 @@");
        let out = apply(&patch, "fn foo(a: i32) {\n    a\n}\n").unwrap();
        assert_eq!(out, "fn foo(x: i32) {\n    a\n}\n");
        // Plus `...` preserves the original parameters (rename only).
        let keep = sig_patch("- fn foo(a: i32) {", "+ fn bar(...) {", "@@ -1,3 +1,3 @@");
        let out = apply(&keep, "fn foo(a: i32) {\n    a\n}\n").unwrap();
        assert_eq!(out, "fn bar(a: i32) {\n    a\n}\n");
    }

    #[test]
    fn rejects_wrong_concrete_params() {
        let patch = sig_patch(
            "- fn foo(a: i32) {",
            "+ fn bar(a: i32) {",
            "@@ -1,1 +1,1 @@",
        );
        assert!(apply(&patch, "fn foo(b: String) {}\n").is_err());
    }

    #[test]
    fn combined_rename_vis_qual_ret() {
        let patch = sig_patch(
            "- fn foo(...) {",
            "+ pub async fn bar(...) -> i32 {",
            "@@ -1,1 +1,1 @@",
        );
        assert_eq!(
            apply(&patch, "fn foo() {}\n").unwrap(),
            "pub async fn bar() -> i32 {}\n"
        );
    }

    #[test]
    fn rejects_concrete_body() {
        // Body evolution is unsupported: fail fast instead of silently
        // preserving the body.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evolve\n# lang=rust\n- fn foo(...) {\n-     1\n- }\n+ fn bar(...) {\n      ...\n  }\n";
        assert!(apply(patch, "fn foo() {\n    1\n}\n").is_err());
    }

    #[test]
    fn visibility_metavar_sets_and_preserves() {
        // Minus `V`, plus concrete: set visibility and rename.
        let set = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# visibility V\n- V fn foo(...) {\n+ pub fn bar(...) {\n      ...\n  }\n";
        assert_eq!(apply(set, "fn foo() {}\n").unwrap(), "pub fn bar() {}\n");
        // Same `V` on both sides: preserve the original visibility.
        let keep = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# visibility V\n- V fn foo(...) {\n+ V fn bar(...) {\n      ...\n  }\n";
        assert_eq!(
            apply(keep, "pub(crate) fn foo() {}\n").unwrap(),
            "pub(crate) fn bar() {}\n"
        );
    }

    #[test]
    fn qualifier_and_generic_metavars_set() {
        let qual = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# qualifier_list Q\n- Q fn foo(...) {\n+ async fn foo(...) {\n      ...\n  }\n";
        assert_eq!(apply(qual, "fn foo() {}\n").unwrap(), "async fn foo() {}\n");
        let gen = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# generic_params G\n- fn foo G (...) {\n+ fn foo<T>(...) {\n      ...\n  }\n";
        assert_eq!(apply(gen, "fn foo() {}\n").unwrap(), "fn foo<T>() {}\n");
    }

    #[test]
    fn evolves_method_inside_impl() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=x\n# lang=rust\n  impl Service {\n      ...\n-     fn old(...) {\n+     fn old(...) -> i32 {\n           ...\n       }\n  }\n";
        let src = "impl Service {\n    fn old(&self) {}\n    fn other(&self) {}\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("fn old(&self) -> i32"), "got: {out}");
        assert!(out.contains("fn other(&self)"));
    }
}
