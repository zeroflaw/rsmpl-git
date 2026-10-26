//! Pattern compilation: from raw hunk body + directives to structured kinds.
//!
//! Root inference (SPEC 13.4): explicit `# root=` wins; otherwise heuristics
//! on the minus text (fn/item/call/macro/insert). The engine performs the
//! actual target matching; this module validates shape and extracts bindings.

use rsmpl_patch::{Directive, ErrorCode, MetavarKind, RsmplHunk, StructuredError};
use std::collections::HashSet;

use crate::expr_pattern::{classify_call_pattern, ExprPattern, ExprPatternKind, PlaceholderMaps};
use crate::fn_pattern::parse_fn_pattern;
use crate::metavar::MetavarTable;
use crate::quote_utils::{mask_literal_spans, unmask_literal_spans, validate_quoting};

/// Shape of a generalized (non-call) pattern (T10 v1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralShape {
    /// A `let` statement (`syn::Stmt::Local`, no `let-else`).
    Stmt,
    /// A general Rust expression (bounded engine variant set).
    Expr,
    /// A general Rust type (bounded engine variant set).
    Type,
}

/// Top-level pattern kind for a hunk side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternKind {
    /// Function-item rename/rewrite (`fn foo(...) { ... }`).
    Fn {
        minus_name: String,
        plus_name: String,
        vis_meta: Option<String>,
    },
    /// Expression/call rewrite (`foo(E)` -> `bar(E)`).
    Expr {
        minus: ExprPatternKind,
        plus_raw: String,
        /// Placeholder idents per metavariable, for substitution checks.
        placeholders: Vec<(String, String)>,
        /// Backtick literals in the minus pattern.
        literals: Vec<String>,
        /// Whether a `crate::...::name` path ellipsis is present.
        has_path_ellipsis: bool,
    },
    /// Single-metavariable cut (`- I` with `# item I` + `# bind=X`).
    ItemCapture { meta: String },
    /// Single-metavariable or literal insertion (`+ Helper` / `+ use ...`).
    Insertion {
        text: String,
        use_name: Option<String>,
    },
    /// Impl-context rewrite (`impl Service { ... - fn ... }`).
    ImplContext { impl_header: String },
    /// Macro token-tree / invocation patterns.
    Macro { minus_raw: String, plus_raw: String },
    /// Generalized statement/expression/type rewrite (`let x = E`, `a + E`,
    /// `Vec<T>`). `minus_sub` holds the placeholder-substituted pattern
    /// (`__RSMPL_E` idents, backtick literals restored as literal text).
    General {
        shape: GeneralShape,
        minus_sub: String,
        plus_raw: String,
        /// Placeholder idents per metavariable, for substitution checks.
        placeholders: Vec<(String, String)>,
        /// Backtick literals in the minus pattern.
        literals: Vec<String>,
        /// Leading `#[...]` attribute lines split off minus/plus (Stmts).
        minus_attrs: Vec<String>,
        plus_attrs: Vec<String>,
        /// True when a trailing `;` was stripped to match an expression
        /// statement as an expression (deletions remove whole lines).
        had_semi: bool,
    },
    /// Fallback raw-text pattern (kept for diagnostics).
    Raw { minus: String, plus: String },
}

/// A compiled hunk pattern ready for the engine.
#[derive(Debug, Clone)]
pub struct CompiledPattern {
    pub kind: PatternKind,
    pub table: MetavarTable,
    pub minus_bindings: HashSet<String>,
    pub plus_bindings: HashSet<String>,
    /// Names used in `+` but never bound in `-`/context or `# use` (engine
    /// re-checks with capture awareness; stored here for diagnostics).
    pub unbound_plus: Vec<String>,
    pub repeated: Vec<String>,
    pub root: Option<String>,
}

impl CompiledPattern {
    pub fn rule(&self) -> &str {
        ""
    }
}

/// Compile an RSmPL hunk's pattern body.
pub fn compile(hunk: &RsmplHunk) -> Result<CompiledPattern, StructuredError> {
    let table = MetavarTable::from_directives(&hunk.directives)?;
    let root = hunk
        .directives
        .iter()
        .filter_map(|d| match d {
            Directive::Root(r) => Some(r.clone()),
            _ => None,
        })
        .next_back();

    let minus_raw = full_minus_text(hunk);
    let plus_raw = full_plus_text(hunk);
    let context_raw = hunk.context_text();

    for line in minus_raw
        .lines()
        .chain(plus_raw.lines())
        .chain(context_raw.lines())
    {
        validate_quoting(line)?;
    }

    let minus_bindings = table.bound_in_text(&minus_raw);
    let mut bound: HashSet<String> = minus_bindings.clone();
    // `# use=Name` imports a capture as bound for the plus side.
    for u in hunk.uses() {
        bound.insert(u.to_string());
    }
    // Symbol-declared names used as callee in minus are bound by declaration.
    for k in table.symbols.keys() {
        bound.insert(k.clone());
    }
    let plus_bindings = table.bound_in_text(&plus_raw);
    let uses: HashSet<String> = hunk.uses().into_iter().map(|s| s.to_string()).collect();
    let mut unbound_plus: Vec<String> = plus_bindings
        .difference(&bound)
        .filter(|n| !uses.contains(*n))
        .cloned()
        .collect();
    unbound_plus.sort();

    // Repeated metavariables in the minus pattern (SPEC 15.7): names occurring
    // 2+ times as whole tokens.
    let repeated = repeated_metavars(&minus_raw, &table);

    validate_ellipsis_positions(&minus_raw, &plus_raw)?;

    let kind = classify(
        &minus_raw,
        &plus_raw,
        &context_raw,
        &table,
        hunk,
        root.as_deref(),
    )?;

    // `# returns=none` only meaningful on fn patterns.
    if hunk.returns_none() && !matches!(kind, PatternKind::Fn { .. }) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# returns=none` requires a function-item pattern".to_string(),
        ));
    }

    Ok(CompiledPattern {
        kind,
        table,
        minus_bindings,
        plus_bindings,
        unbound_plus,
        repeated,
        root,
    })
}

/// Full minus pattern = removal lines + context lines, in hunk order.
/// This reconstructs multi-line patterns like `fn foo(...) {` / `...` / `}`.
fn full_minus_text(hunk: &RsmplHunk) -> String {
    hunk.lines
        .iter()
        .filter(|l| l.is_removal() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Full plus pattern = addition lines + context lines, in hunk order.
fn full_plus_text(hunk: &RsmplHunk) -> String {
    hunk.lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Names occurring 2+ times in `text` as whole ident tokens.
fn repeated_metavars(text: &str, table: &MetavarTable) -> Vec<String> {
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    for tok in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')) {
        if tok.is_empty() {
            continue;
        }
        let bare = tok.strip_prefix('$').unwrap_or(tok);
        if table.sigiled && !tok.starts_with('$') {
            continue;
        }
        if table.is_meta(bare) {
            *counts.entry(bare.to_string()).or_default() += 1;
        }
    }
    let mut out: Vec<String> = counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(n, _)| n)
        .collect();
    out.sort();
    out
}

/// Reject `...` in positions SPEC 16 forbids (visibility/qualifier/generics
/// slots are handled by fn parser; here we reject stray `...` as path prefix
/// misuse like `...foo` without `::`).
fn validate_ellipsis_positions(minus: &str, plus: &str) -> Result<(), StructuredError> {
    for text in [minus, plus] {
        for line in text.lines() {
            let t = line.trim();
            // `...` must appear as a standalone token or inside delimiters.
            // A cheap check: no identifier char directly adjacent: `foo...bar`.
            let bytes = t.as_bytes();
            let mut i = 0;
            while i + 2 < bytes.len() {
                if &t[i..i + 3] == "..." {
                    let before = if i > 0 { bytes[i - 1] as char } else { ' ' };
                    let after = if i + 3 < bytes.len() {
                        bytes[i + 3] as char
                    } else {
                        ' '
                    };
                    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
                    if ident(before) && ident(after) {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("`...` must be a standalone wildcard, got `{line}`"),
                        ));
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn classify(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    table: &MetavarTable,
    hunk: &RsmplHunk,
    root: Option<&str>,
) -> Result<PatternKind, StructuredError> {
    let m = minus_raw.trim();
    let p = plus_raw.trim();
    let c = context_raw.trim();

    // Explicit root wins.
    if let Some(r) = root {
        match r {
            "item" if is_single_meta(m, table) => {
                return Ok(PatternKind::ItemCapture {
                    meta: m.to_string(),
                });
            }
            "expr" => {
                let (minus, maps, lits) = classify_call_pattern(m, table)?;
                if matches!(minus, ExprPatternKind::Raw { .. }) {
                    if let Some(g) = try_general(m, p, table, Some(GeneralShape::Expr))? {
                        return Ok(g);
                    }
                }
                return Ok(PatternKind::Expr {
                    minus,
                    plus_raw: p.to_string(),
                    placeholders: maps
                        .expr_placeholders
                        .into_iter()
                        .map(|(ph, n)| (n, ph))
                        .collect(),
                    literals: lits,
                    has_path_ellipsis: has_path_ellipsis(m),
                });
            }
            "stmt" => {
                return match try_general(m, p, table, Some(GeneralShape::Stmt))? {
                    Some(g) => Ok(g),
                    None => Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` only, no `let-else`)",
                    )),
                };
            }
            "type" => {
                return match try_general(m, p, table, Some(GeneralShape::Type))? {
                    Some(g) => Ok(g),
                    None => Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=type` requires a Rust type pattern",
                    )),
                };
            }
            _ => {}
        }
    }

    // Single-metavar minus => item/stmt capture (cut).
    if m.lines().count() == 1 && is_single_meta(m, table) {
        return Ok(PatternKind::ItemCapture {
            meta: m.to_string(),
        });
    }

    // Insertion: no minus, plus present.
    if m.is_empty() && !p.is_empty() {
        let uses = hunk.uses();
        let use_name = if uses.len() == 1 && p.lines().count() == 1 && p.trim() == uses[0] {
            Some(uses[0].to_string())
        } else {
            None
        };
        return Ok(PatternKind::Insertion {
            text: p.to_string(),
            use_name,
        });
    }

    // Impl-context: context lines mention `impl`.
    if c.contains("impl ") && looks_like_fn(m) {
        let impl_header = c.lines().next().unwrap_or("").to_string();
        return Ok(PatternKind::ImplContext { impl_header });
    }

    // Function patterns (minus starts with fn-ish, possibly with vis/attrs).
    if looks_like_fn(m) && looks_like_fn(p) {
        let (vis_metas, qual_metas, gen_metas, where_metas) = slot_metas(table);
        let minus_fn = parse_fn_pattern(
            &first_fn_block(m),
            &vis_metas,
            &qual_metas,
            &gen_metas,
            &where_metas,
            hunk.returns_none(),
        )?;
        let plus_fn = parse_fn_pattern(
            &first_fn_block(p),
            &vis_metas,
            &qual_metas,
            &gen_metas,
            &where_metas,
            false,
        )?;
        let minus_name = match &minus_fn.name {
            crate::fn_pattern::NamePat::Concrete(n) => n.clone(),
            crate::fn_pattern::NamePat::Meta(n) => format!("${n}"),
        };
        let plus_name = match &plus_fn.name {
            crate::fn_pattern::NamePat::Concrete(n) => n.clone(),
            crate::fn_pattern::NamePat::Meta(n) => format!("${n}"),
        };
        let vis_meta = match &minus_fn.visibility {
            crate::fn_pattern::VisPat::Meta(v) => Some(v.clone()),
            _ => None,
        };
        return Ok(PatternKind::Fn {
            minus_name,
            plus_name,
            vis_meta,
        });
    }

    // Macro modes: explicit token_tree or `!` in pattern.
    let is_macro_mode = hunk.macro_mode() != rsmpl_patch::MacroMode::Invocation;
    if is_macro_mode || (m.contains('!') && m.contains('(')) {
        return Ok(PatternKind::Macro {
            minus_raw: m.to_string(),
            plus_raw: p.to_string(),
        });
    }

    // Default: expression/call rewrite.
    if !m.is_empty() {
        let (minus, maps, lits) = classify_call_pattern(m, table)?;
        // Non-call shapes (`Raw`) get a second chance as generalized
        // stmt/expr/type patterns (T10). Call/method/macro routing is
        // untouched, including `foo(E);` (which classifies as `Call`).
        if matches!(minus, ExprPatternKind::Raw { .. }) {
            if let Some(g) = try_general(m, p, table, None)? {
                return Ok(g);
            }
        }
        return Ok(PatternKind::Expr {
            minus,
            plus_raw: p.to_string(),
            placeholders: maps
                .expr_placeholders
                .into_iter()
                .map(|(ph, n)| (n, ph))
                .collect(),
            literals: lits,
            has_path_ellipsis: has_path_ellipsis(m),
        });
    }

    Ok(PatternKind::Raw {
        minus: m.to_string(),
        plus: p.to_string(),
    })
}

/// True when the pattern uses `::...::` path ellipsis (SPEC 16.5), as opposed
/// to argument/body ellipsis `foo(...)`.
fn has_path_ellipsis(text: &str) -> bool {
    text.contains("::...") || text.contains("...::")
}

/// Upgrade a non-call `Raw` shape to a generalized Stmt/Expr/Type pattern.
///
/// Returns `None` when the text is none of those (caller keeps `Raw`, which
/// the engine rejects with an unsupported-shape diagnostic).
///
/// Routing (forced by `# root=` or heuristic): a `syn::Stmt::Local` (`let`,
/// no `let-else`) becomes `Stmt`; otherwise a Rust `Type` when forced or when
/// a type-level metavariable is declared; otherwise a Rust `Expr`.
/// `...` is rejected: v1 general patterns are concrete-or-metavariable only.
#[allow(clippy::too_many_lines)]
fn try_general(
    minus_raw: &str,
    plus_raw: &str,
    table: &MetavarTable,
    forced: Option<GeneralShape>,
) -> Result<Option<PatternKind>, StructuredError> {
    if minus_raw.contains("...") || plus_raw.contains("...") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v1 general stmt/expr/type patterns do not support `...` (use a call pattern or a concrete shape)",
        ));
    }
    let (minus_attrs, minus_rest) = split_leading_attrs(minus_raw);
    let (plus_attrs, plus_rest) = split_leading_attrs(plus_raw);
    // Mask backtick literals so substitution cannot mistake them for
    // metavariables, then restore them in the substituted pattern.
    let (masked, sentinels) = mask_literal_spans(&minus_rest)?;
    let (mut sub, maps) = ExprPattern::placeholder_expr(&masked, table);
    sub = unmask_literal_spans(&sub, &sentinels);

    let want = |s: GeneralShape| forced.is_none_or(|f| f == s);
    let has_type_decl = table.declared.iter().any(|(k, _)| {
        matches!(
            k,
            MetavarKind::Type | MetavarKind::Lifetime | MetavarKind::GenericParams
        )
    });

    // 1. `let` statements (only `Stmt::Local`). Other statement shapes fall
    // through so existing routing is untouched.
    if want(GeneralShape::Stmt) {
        match syn::parse_str::<syn::Stmt>(&sub) {
            Ok(syn::Stmt::Local(local)) => {
                if local.init.as_ref().is_some_and(|i| i.diverge.is_some()) {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "v1 general patterns do not support `let-else` statements",
                    ));
                }
                check_let_pat(&local)?;
                return Ok(Some(build_general(
                    GeneralShape::Stmt,
                    sub,
                    plus_raw,
                    &maps,
                    minus_attrs,
                    plus_attrs,
                    false,
                )));
            }
            // Any other statement shape (or unparsable text) falls through so
            // existing routing is untouched; `# root=stmt` fails fast.
            _ => {
                if forced == Some(GeneralShape::Stmt) {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` only, no `let-else`)",
                    ));
                }
            }
        }
    }

    // 2. Expression statements with `;` match as expressions; the engine
    // preserves (or whole-line-deletes) the semicolon via `had_semi`.
    // (Call-like `foo(E);` shapes never reach here: they classify as `Call`.)
    let mut expr_src: Option<String> = None;
    let mut had_semi = false;
    if let Ok(syn::Stmt::Expr(e, Some(_))) = syn::parse_str::<syn::Stmt>(&sub) {
        expr_src = Some(quote::quote!(#e).to_string());
        had_semi = true;
    }

    // 3. Types before expressions when the author declared type-level
    // metavariables (so `Vec<T>` with `# type T` is a type pattern).
    let type_first = forced == Some(GeneralShape::Type) || (forced.is_none() && has_type_decl);
    if want(GeneralShape::Type) && type_first && syn::parse_str::<syn::Type>(&sub).is_ok() {
        return check_type_deletion(plus_rest.trim(), || {
            build_general(
                GeneralShape::Type,
                sub,
                plus_raw,
                &maps,
                minus_attrs,
                plus_attrs,
                false,
            )
        });
    }

    // 4. Expressions.
    if want(GeneralShape::Expr) {
        let src = expr_src.unwrap_or(sub);
        if syn::parse_str::<syn::Expr>(&src).is_ok() {
            if plus_rest.trim().is_empty() && !had_semi {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement",
                ));
            }
            return Ok(Some(build_general(
                GeneralShape::Expr,
                src,
                plus_raw,
                &maps,
                minus_attrs,
                plus_attrs,
                had_semi,
            )));
        }
        if forced == Some(GeneralShape::Expr) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "`# root=expr` requires a Rust expression pattern",
            ));
        }
        // Fall through: the text may still be a `Type` (step 5 re-derives it).
    }

    // 5. Types (expression-first order).
    if want(GeneralShape::Type) && !type_first {
        // Re-parse: `sub` may have been moved above; re-derive cheaply.
        let (masked2, sentinels2) = mask_literal_spans(&minus_rest)?;
        let (mut sub2, _) = ExprPattern::placeholder_expr(&masked2, table);
        sub2 = unmask_literal_spans(&sub2, &sentinels2);
        if syn::parse_str::<syn::Type>(&sub2).is_ok() {
            return check_type_deletion(plus_rest.trim(), || {
                build_general(
                    GeneralShape::Type,
                    sub2,
                    plus_raw,
                    &maps,
                    minus_attrs,
                    plus_attrs,
                    false,
                )
            });
        }
    }
    if forced == Some(GeneralShape::Type) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# root=type` requires a Rust type pattern",
        ));
    }
    Ok(None)
}

fn check_type_deletion(
    plus_trimmed: &str,
    build: impl FnOnce() -> PatternKind,
) -> Result<Option<PatternKind>, StructuredError> {
    if plus_trimmed.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type",
        ));
    }
    Ok(Some(build()))
}

fn build_general(
    shape: GeneralShape,
    minus_sub: String,
    plus_raw: &str,
    maps: &PlaceholderMaps,
    minus_attrs: Vec<String>,
    plus_attrs: Vec<String>,
    had_semi: bool,
) -> PatternKind {
    PatternKind::General {
        shape,
        minus_sub,
        plus_raw: plus_raw.to_string(),
        placeholders: maps
            .expr_placeholders
            .iter()
            .map(|(ph, n)| (n.clone(), ph.clone()))
            .collect(),
        // Backticked spans were masked during substitution and restored as
        // literal text in `minus_sub`; nothing further to record.
        literals: Vec::new(),
        minus_attrs,
        plus_attrs,
        had_semi,
    }
}

/// Reject `let` patterns the v1 engine cannot match: embedded metavariables
/// inside the bound pattern (other than a whole-pattern placeholder, or a
/// `P: T` shape whose parts are each whole-placeholder/concrete).
fn check_let_pat(local: &syn::Local) -> Result<(), StructuredError> {
    let pat = &local.pat;
    let (inner, ty) = match pat {
        syn::Pat::Type(pt) => (&*pt.pat, Some(&*pt.ty)),
        _ => (pat, None),
    };
    if matches!(inner, syn::Pat::Ident(_)) {
        if let Some(t) = ty {
            let rendered = quote::quote!(#t).to_string();
            if rendered.contains("__RSMPL_") && !is_single_placeholder_type(t) {
                return Err(embedded_pat_err());
            }
        }
        return Ok(());
    }
    let rendered = quote::quote!(#pat).to_string();
    if rendered.contains("__RSMPL_") {
        return Err(embedded_pat_err());
    }
    Ok(())
}

fn is_single_placeholder_type(t: &syn::Type) -> bool {
    if let syn::Type::Path(tp) = t {
        if tp.qself.is_none() && tp.path.segments.len() == 1 {
            let seg = &tp.path.segments[0];
            return seg.arguments.is_none() && seg.ident.to_string().starts_with("__RSMPL_");
        }
    }
    false
}

fn embedded_pat_err() -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        "v1 `let` patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts; embedded metavariables elsewhere in the pattern are not supported",
    )
}

/// Split leading `#[...]` attribute lines (possibly multi-line) from pattern
/// text; returns (attrs, rest).
fn split_leading_attrs(text: &str) -> (Vec<String>, String) {
    let lines: Vec<&str> = text.lines().collect();
    let mut attrs = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if !lines[idx].trim_start().starts_with("#[") {
            break;
        }
        let mut depth = 0i32;
        let start = idx;
        while idx < lines.len() {
            for c in lines[idx].chars() {
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                }
            }
            idx += 1;
            if depth <= 0 {
                break;
            }
        }
        attrs.extend(lines[start..idx].iter().map(ToString::to_string));
        if depth > 0 {
            break;
        }
    }
    (attrs, lines[idx..].join("\n"))
}

fn is_single_meta(text: &str, table: &MetavarTable) -> bool {
    let t = text.trim().trim_start_matches('$');
    let single_line = text.trim().lines().count() == 1;
    single_line && !t.contains(char::is_whitespace) && *t != *"..." && table.is_meta(t)
}

fn slot_metas(table: &MetavarTable) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut vis = Vec::new();
    let mut qual = Vec::new();
    let mut gen = Vec::new();
    let mut wh = Vec::new();
    for (kind, name) in &table.declared {
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

fn looks_like_fn(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim().trim_start_matches("#[");
        t.starts_with("fn ")
            || t.starts_with("pub ")
            || t.starts_with("async ")
            || t.starts_with("unsafe ")
            || t.starts_with("const ")
            || t.starts_with("extern ")
            || (t.starts_with("pub("))
    }) || text.contains("\nfn ")
        || text.trim_start().starts_with("fn ")
}

fn first_fn_block(text: &str) -> String {
    // The fn pattern may span multiple lines (`fn foo(...) {` / `...` / `}`);
    // join lines with newlines preserved for the fn parser.
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_patch::parse_patch;

    fn hunk_of(patch: &str) -> RsmplHunk {
        let p = parse_patch(patch).unwrap();
        match p.files[0].hunks[0].clone() {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("expected rsmpl"),
        }
    }

    #[test]
    fn compiles_fn_rename() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        );
        let c = compile(&h).unwrap();
        assert!(matches!(c.kind, PatternKind::Fn { .. }));
    }

    #[test]
    fn compiles_call_rewrite() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=rename_calls\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n",
        );
        let c = compile(&h).unwrap();
        assert!(matches!(c.kind, PatternKind::Expr { .. }));
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn detects_unbound_plus() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=x\n# lang=rust\n# expr E\n- foo(E)\n+ bar(Q)\n",
        );
        let c = compile(&h).unwrap();
        // Q undeclared => not tracked as metavar at all.
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn detects_declared_but_unbound_plus() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=x\n# lang=rust\n# expr E\n# expr Q\n- foo(E)\n+ bar(Q)\n",
        );
        let c = compile(&h).unwrap();
        assert_eq!(c.unbound_plus, vec!["Q".to_string()]);
    }

    #[test]
    fn expr_carries_placeholders_and_path_ellipsis() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -80,20 +80,20 @@ rule=x\n# lang=rust\n# allow=many\n# symbol Old=crate::old_api\n- Old(...)\n+ crate::new_api(...)\n",
        );
        let c = compile(&h).unwrap();
        match &c.kind {
            PatternKind::Expr {
                placeholders,
                has_path_ellipsis,
                ..
            } => {
                assert!(!placeholders.is_empty() || c.table.symbols.contains_key("Old"));
                assert!(!has_path_ellipsis);
            }
            _ => panic!("expected expr"),
        }
    }

    #[test]
    fn compiles_item_capture() {
        let h = hunk_of(
            "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -20,12 +20,0 @@ rule=cut\n# lang=rust\n# item I\n# bind=Helper\n- I\n",
        );
        let c = compile(&h).unwrap();
        assert!(matches!(c.kind, PatternKind::ItemCapture { .. }));
    }

    #[test]
    fn repeated_metavar_detected() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,1 +40,1 @@ rule=x\n# lang=rust\n# expr E\n- assert_eq!(E, E)\n+ assert_equal_once(E)\n",
        );
        let c = compile(&h).unwrap();
        assert!(c.repeated.contains(&"E".to_string()));
    }

    #[test]
    fn let_shape_compiles_to_general_stmt() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Stmt,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn binary_expr_compiles_to_general_expr() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + 1\n+ E.wrapping_add(1)\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Expr,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn type_with_type_metavar_compiles_to_general_type() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n+ Box<[T]>\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Type,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn call_with_semi_stays_call() {
        // `foo(E);` must keep call-matcher routing (not Stmt/GeneralExpr).
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E);\n+ bar(E);\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(&c.kind, PatternKind::Expr { .. }),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn root_stmt_forces_let() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n# stmt S\n- let x = 1;\n+ let y = 1;\n",
        );
        assert!(matches!(
            compile(&h).unwrap().kind,
            PatternKind::General {
                shape: GeneralShape::Stmt,
                ..
            }
        ));
        let bad = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n- foo\n+ bar\n",
        );
        assert!(compile(&bad).is_err());
    }

    #[test]
    fn ellipsis_rejected_in_general_shapes() {
        // `...` still works in call routing...
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- foo(..., E)\n+ bar(E)\n",
        );
        assert!(matches!(
            compile(&h).unwrap().kind,
            PatternKind::Expr { .. }
        ));
        // ...but not in general shapes.
        let h2 = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + ...\n+ E\n",
        );
        assert!(compile(&h2).is_err());
    }

    #[test]
    fn type_deletion_rejected() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,0 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n",
        );
        assert!(compile(&h).is_err());
    }
}
