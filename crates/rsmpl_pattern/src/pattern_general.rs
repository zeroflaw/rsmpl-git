//! Generalized Stmt/Expr/Type + statement-sequence compilation (T10, WI-5c).
//!
//! Upgrades non-call `Raw` shapes to structural patterns; the engine performs
//! the actual target matching.

use rsmpl_patch::{ErrorCode, MetavarKind, StructuredError};
use syn::visit::{self, Visit};

use crate::expr_pattern::{ExprPattern, PlaceholderMaps};
use crate::metavar::MetavarTable;
use crate::pattern::{GeneralShape, PatternKind};
use crate::pattern_ellipsis::{c1_err, check_general_ellipsis_expr, check_general_ellipsis_stmt};
use crate::quote_utils::{has_ellipsis_marker, mask_literal_spans, unmask_literal_spans};

/// WI-5c sequence member: a plain `let` with an initializer, no
/// attributes, no `let-else` diverge. The sequence arm matches members
/// with the existing structural `match_local`; anything else stays
/// outside the v1 sequence shape (C18).
fn is_sequence_let(stmt: &syn::Stmt) -> bool {
    match stmt {
        syn::Stmt::Local(l) => {
            l.attrs.is_empty() && l.init.as_ref().is_some_and(|i| i.diverge.is_none())
        }
        _ => false,
    }
}

/// C18 for a minus that is sequence-shaped but whose plus side is not:
/// the minus parses as K plain lets while the plus side has a different
/// count or a non-`let` member. Same code as the generic C18 (one C-table
/// row), with the admitted shape named.
fn sequence_shape_err(k: usize) -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        format!(
            "v1 statement sequences need 2–4 plain `let` statements on both sides with equal counts (the minus side has {k} plain `let` statements, the plus side differs); write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk"
        ),
    )
    .with_repair(StructuredError::fix_pattern_repair(
        "C18",
        "write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk",
    ))
}

/// WI-5c: compile a repeated-block statement sequence (2–4 plain `let`
/// statements, exact count both sides) to `GeneralShape::Sequence.
///
/// Returns `None` when the minus side is not sequence-shaped (caller
/// keeps the C18 diagnostic or `Raw`). A sequence-shaped minus with a
/// mismatched plus side is a loud C18, never a silent `Raw` miss. `...`
/// anywhere in the minus/plus sides is a C1: sequences are exact-K in
/// v1 (no trailing-block tail inside a sequence member — each member
/// matches via `match_stmt`, never `match_block`).
#[allow(clippy::too_many_arguments)]
fn try_sequence(
    minus_sub: &str,
    plus_raw: &str,
    table: &MetavarTable,
    maps: &PlaceholderMaps,
    minus_attrs: Vec<String>,
    plus_attrs: Vec<String>,
) -> Result<Option<PatternKind>, StructuredError> {
    let Ok(minus_block) = syn::parse_str::<syn::Block>(&format!("{{ {minus_sub} }}")) else {
        return Ok(None);
    };
    if !(2..=4).contains(&minus_block.stmts.len()) {
        return Ok(None);
    }
    if !minus_block.stmts.iter().all(is_sequence_let) {
        return Ok(None);
    }
    // Ellipsis precision: placeholder substitution rewrote every genuine
    // `...` (outside strings/comments, which ride sentinels) to an
    // `__RSMPL_ELLIPSIS_N` marker, so a marker here is a real `...`
    // position — rejected at C1, never a silent match-time miss.
    if has_ellipsis_marker(minus_sub) {
        return Err(c1_err());
    }
    for stmt in &minus_block.stmts {
        if let syn::Stmt::Local(l) = stmt {
            check_let_pat(l)?;
        }
    }
    let (_, plus_rest) = split_leading_attrs(plus_raw);
    if plus_rest.trim().is_empty() {
        // Deletion: an empty `+` side deletes the K statements (Stmt parity).
        return Ok(Some(build_general(
            GeneralShape::Sequence,
            minus_sub.to_string(),
            plus_raw,
            maps,
            minus_attrs,
            plus_attrs,
            false,
        )));
    }
    let (masked_p, sentinels_p) = mask_literal_spans(&plus_rest)?;
    let (mut sub_p, _) = ExprPattern::placeholder_expr(&masked_p, table);
    sub_p = unmask_literal_spans(&sub_p, &sentinels_p);
    if has_ellipsis_marker(&sub_p) {
        return Err(c1_err());
    }
    let k = minus_block.stmts.len();
    let Ok(plus_block) = syn::parse_str::<syn::Block>(&format!("{{ {sub_p} }}")) else {
        return Err(sequence_shape_err(k));
    };
    if plus_block.stmts.len() != k || !plus_block.stmts.iter().all(is_sequence_let) {
        return Err(sequence_shape_err(k));
    }
    Ok(Some(build_general(
        GeneralShape::Sequence,
        minus_sub.to_string(),
        plus_raw,
        maps,
        minus_attrs,
        plus_attrs,
        false,
    )))
}

/// Upgrade a non-call `Raw` shape to a generalized Stmt/Expr/Type pattern.
///
/// Returns `None` when the text is none of those (caller keeps `Raw`, which
/// the engine rejects with an unsupported-shape diagnostic).
///
/// Routing (forced by `# root=` or heuristic): a `syn::Stmt::Local` (`let`,
/// with or without `let-else`) becomes `Stmt`; otherwise a Rust `Type` when forced or when
/// a type-level metavariable is declared; otherwise a Rust `Expr`.
/// WI-5b: `...` is admitted only as trailing block pseudo-statements
/// (replayed/dropped tails); any other position fails at C1.
#[allow(clippy::too_many_lines)]
pub(crate) fn try_general(
    minus_raw: &str,
    plus_raw: &str,
    table: &MetavarTable,
    forced: Option<GeneralShape>,
) -> Result<Option<PatternKind>, StructuredError> {
    let has_ellipsis = minus_raw.contains("...") || plus_raw.contains("...");
    let (minus_attrs, minus_rest) = split_leading_attrs(minus_raw);
    let (plus_attrs, plus_rest) = split_leading_attrs(plus_raw);
    // A leading `/*` the peel cannot normalize (multi-line block doc,
    // ordinary block comment) is never a valid general-pattern start —
    // fail fast instead of a misleading match-time error (T156).
    for rest in [minus_rest.trim_start(), plus_rest.trim_start()] {
        if rest.starts_with("/*") {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "block comments (`/* ... */`) are not supported in patterns in v1 (mirror doc comments with `#[doc = \"...\"]` or use an ordinary hunk)",
            ));
        }
    }
    // Mask backtick literals so substitution cannot mistake them for
    // metavariables, then restore them in the substituted pattern.
    let (masked, sentinels) = mask_literal_spans(&minus_rest)?;
    let (mut sub, maps) = ExprPattern::placeholder_expr(&masked, table);
    sub = unmask_literal_spans(&sub, &sentinels);
    // WI-2b C19: macro bodies and `Verbatim` nodes never match (verbatim-only
    // engine); fail at compile time with the exact node kind.
    check_c19(&sub)?;

    let want = |s: GeneralShape| forced.is_none_or(|f| f == s);
    let has_type_decl = table.declared.iter().any(|(k, _)| {
        matches!(
            k,
            MetavarKind::Type | MetavarKind::Lifetime | MetavarKind::GenericParams
        )
    });

    // 1. `let` statements (only `Stmt::Local`). Other statement shapes fall
    // through so existing routing is untouched. `let-else` diverge
    // blocks ride along structurally (WI-5a: the engine matches them
    // as `Block`s, both sides or neither).
    if want(GeneralShape::Stmt) {
        match syn::parse_str::<syn::Stmt>(&sub) {
            Ok(syn::Stmt::Local(local)) => {
                check_let_pat(&local)?;
                if has_ellipsis {
                    check_general_ellipsis_stmt(&local, &plus_rest, table)?;
                }
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
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` with or without `let-else`)",
                    )
                    .with_repair(StructuredError::fix_pattern_repair(
                        "C3",
                        "write a `let` statement pattern",
                    )));
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
    // WI-5c: keep `sub` alive past the expression-shape move below (the
    // sequence arm reuses the substituted minus text).

    // 3. Types before expressions when the author declared type-level
    // metavariables (so `Vec<T>` with `# type T` is a type pattern).
    let type_first = forced == Some(GeneralShape::Type) || (forced.is_none() && has_type_decl);
    if want(GeneralShape::Type) && type_first && syn::parse_str::<syn::Type>(&sub).is_ok() {
        if has_ellipsis {
            // Types never contain blocks: any `...` here is misplaced.
            return Err(c1_err());
        }
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
        // WI-5c: clone (not move) so the WI-5c sequence arm below can
        // reuse the substituted minus text.
        let src = expr_src.unwrap_or_else(|| sub.clone());
        if syn::parse_str::<syn::Expr>(&src).is_ok() {
            if plus_rest.trim().is_empty() && !had_semi {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement",
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "C6",
                    "match a `let` statement or write an explicit replacement",
                )));
            }
            if has_ellipsis {
                let pat_expr: syn::Expr = syn::parse_str(&src).map_err(|e| {
                    StructuredError::new(
                        ErrorCode::PatternConflict,
                        format!("invalid expression pattern: {e}"),
                    )
                })?;
                check_general_ellipsis_expr(&pat_expr, &plus_rest, table)?;
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
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "C4",
                "write a Rust expression pattern",
            )));
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
            if has_ellipsis {
                // Types never contain blocks: any `...` here is misplaced.
                return Err(c1_err());
            }
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
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C5",
            "write a Rust type pattern",
        )));
    }
    // WI-5c: repeated-block statement sequences (2–4 plain `let`
    // statements, exact count both sides) compile to `GeneralShape::Sequence`.
    // Anything else multi-statement keeps the C18 diagnostic below.
    if let Some(seq) = try_sequence(&sub, plus_raw, table, &maps, minus_attrs, plus_attrs)? {
        return Ok(Some(seq));
    }
    // WI-9c(c): a minus side that parses as two or more Rust statements
    // outside the WI-5c sequence shape cannot be expressed. Fail fast at
    // compile time naming the pattern instead of degrading to `Raw` and
    // dying at match with a misleading "no match found" that points at
    // the file. Single-statement text (including multi-line single shapes
    // such as line-continued calls) and unparsable wraps still fall
    // through to `Ok(None)` — never a false positive. C1 precedent: the
    // repair rides in the reason, no separate suggestion.
    let (masked_seq, sentinels_seq) = mask_literal_spans(&minus_rest)?;
    let (sub_seq, _) = ExprPattern::placeholder_expr(&masked_seq, table);
    let sub_seq = unmask_literal_spans(&sub_seq, &sentinels_seq);
    if let Ok(block) = syn::parse_str::<syn::Block>(&format!("{{ {sub_seq} }}")) {
        if block.stmts.len() >= 2 {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "v1 statement sequences need 2–4 plain `let` statements on both sides with equal counts (the minus side parses as {} Rust statements, not a plain-`let` sequence); write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk",
                    block.stmts.len()
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "C18",
                "write 2–4 plain `let` statements, one hunk per statement or item, or use an ordinary hunk",
            )));
        }
    }
    if has_ellipsis {
        // No Stmt/Expr/Type shape parsed, yet `...` is present: it sits in
        // no trailing-block position (those parse). Fail at C1, not as a
        // misleading `Raw` match-time miss.
        return Err(c1_err());
    }
    Ok(None)
}

/// C19 (WI-2b): reject pattern positions the general matcher cannot bind.
///
/// Macro invocations match verbatim (`render_eq`, never holed), and
/// `Verbatim` nodes have no matcher arm at all — so a declared hole inside
/// a macro body can never bind and a `Verbatim` node can never match. Both
/// fail here at compile time with the exact node kind instead of a
/// match-time `no match found` that points at the file. Plus sides are not
/// checked: writing a bound hole into a macro body is textual substitution
/// and applies today.
fn check_c19(minus_sub: &str) -> Result<(), StructuredError> {
    // Shape is unset this early, so walk every root that parses (`E + 1`
    // parses as both `Stmt` and `Expr`; the walk is idempotent). Sequence
    // members ride the `Block` root.
    if let Ok(stmt) = syn::parse_str::<syn::Stmt>(minus_sub) {
        let mut walk = C19Walk::default();
        walk.visit_stmt(&stmt);
        if let Some(err) = walk.into_err() {
            return Err(err);
        }
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(minus_sub) {
        let mut walk = C19Walk::default();
        walk.visit_expr(&expr);
        if let Some(err) = walk.into_err() {
            return Err(err);
        }
    }
    if let Ok(ty) = syn::parse_str::<syn::Type>(minus_sub) {
        let mut walk = C19Walk::default();
        walk.visit_type(&ty);
        if let Some(err) = walk.into_err() {
            return Err(err);
        }
    }
    if let Ok(block) = syn::parse_str::<syn::Block>(&format!("{{ {minus_sub} }}")) {
        let mut walk = C19Walk::default();
        walk.visit_block(&block);
        if let Some(err) = walk.into_err() {
            return Err(err);
        }
    }
    Ok(())
}

/// C19 walker: records the first unbindable position (macro hole wins over
/// `Verbatim`; either fails the compile).
#[derive(Default)]
struct C19Walk {
    hole_macro: Option<String>,
    verbatim: Option<String>,
}

impl C19Walk {
    fn into_err(self) -> Option<StructuredError> {
        if let Some(name) = self.hole_macro {
            return Some(
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "v1 general patterns match macro invocations verbatim (a hole inside `{name}` never binds); write the macro body concretely, use a `# macro=token_tree` hunk for whole-macro rewrites, or use an ordinary hunk"
                    ),
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "C19",
                    "write the macro body concretely, use a `# macro=token_tree` hunk for whole-macro rewrites, or use an ordinary hunk",
                )),
            );
        }
        if let Some(node) = self.verbatim {
            return Some(
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "v1 general patterns do not match `{node}` nodes (engine-opaque); write a concrete shape or use an ordinary hunk"
                    ),
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "C19",
                    "write a concrete shape or use an ordinary hunk",
                )),
            );
        }
        None
    }

    /// A macro invocation carrying a declared hole (placeholder or `...`
    /// sentinel) in its path or body can never bind; records `NAME!`.
    fn check_macro(&mut self, path: &syn::Path, tokens: &proc_macro2::TokenStream) {
        if self.hole_macro.is_some() {
            return;
        }
        if !tokens.to_string().contains("__RSMPL_") {
            return;
        }
        let name = path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_else(|| "macro".to_string());
        self.hole_macro = Some(format!("{name}!"));
    }
}

impl<'ast> Visit<'ast> for C19Walk {
    fn visit_expr(&mut self, e: &'ast syn::Expr) {
        match e {
            syn::Expr::Verbatim(_) => {
                if self.verbatim.is_none() {
                    self.verbatim = Some("Verbatim expression".to_string());
                }
            }
            syn::Expr::Macro(m) => self.check_macro(&m.mac.path, &m.mac.tokens),
            _ => {}
        }
        // Macro token streams are opaque: never recurse into them.
        if !matches!(e, syn::Expr::Macro(_)) {
            visit::visit_expr(self, e);
        }
    }

    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        // `Stmt::Macro` members inside block patterns (same opacity).
        self.check_macro(&m.path, &m.tokens);
    }

    fn visit_type(&mut self, t: &'ast syn::Type) {
        match t {
            syn::Type::Verbatim(_) => {
                if self.verbatim.is_none() {
                    self.verbatim = Some("Verbatim type".to_string());
                }
            }
            syn::Type::Macro(m) => self.check_macro(&m.mac.path, &m.mac.tokens),
            _ => {}
        }
        if !matches!(t, syn::Type::Macro(_)) {
            visit::visit_type(self, t);
        }
    }

    fn visit_pat(&mut self, p: &'ast syn::Pat) {
        if matches!(p, syn::Pat::Verbatim(_)) && self.verbatim.is_none() {
            self.verbatim = Some("Verbatim pattern".to_string());
        }
        visit::visit_pat(self, p);
    }
}
fn check_type_deletion(
    plus_trimmed: &str,
    build: impl FnOnce() -> PatternKind,
) -> Result<Option<PatternKind>, StructuredError> {
    if plus_trimmed.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type",
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C7",
            "write an explicit replacement type",
        )));
    }
    Ok(Some(build()))
}

/// Strip the one-space diff-prefix artifact from each pattern line (T153).
/// Agents author `- let x` (prefix + space + content) and `classify` trims
/// only the outer ends of the full text, so interior lines of multi-line
/// patterns keep one leading space. Matching is whitespace-insensitive (syn),
/// but replacements are byte splices: emitting the artifact would silently
/// re-indent the target. Genuine continuation indent (2+) survives minus one
/// space, preserving relative indentation.
fn strip_artifact_space(text: &str) -> String {
    text.lines()
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
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
    // Attribute-stripped plus for the Stmt mirrored-attrs path (the envelope
    // is re-supplied from the original source, indent included).
    let (_, plus_rest) = split_leading_attrs(plus_raw);
    PatternKind::General {
        shape,
        minus_sub,
        plus_raw: plus_raw.to_string(),
        plus_body: strip_artifact_space(&plus_rest),
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
/// inside the bound pattern (other than a whole-pattern `# pat P` placeholder,
/// a `P @ sub` shape whose head is a whole-pattern placeholder and whose
/// subpattern is a single placeholder or concrete, a concrete pattern, or a
/// `P: T` shape whose parts are each whole-placeholder/concrete).
fn check_let_pat(local: &syn::Local) -> Result<(), StructuredError> {
    let pat = &local.pat;
    let (inner, ty) = match pat {
        syn::Pat::Type(pt) => (&*pt.pat, Some(&*pt.ty)),
        _ => (pat, None),
    };
    if let syn::Pat::Ident(pid) = inner {
        // WI-5a `P @ sub` admission: a fully concrete `@` keeps the
        // historical literal behavior; a placeholder head admits a
        // single-placeholder or concrete subpattern. Placeholders
        // anywhere else (concrete head + placeholder sub, or a
        // composite-placeholder sub) stay C8.
        if let Some((_, sub)) = &pid.subpat {
            let head_is_ph = pid.ident.to_string().starts_with("__RSMPL_");
            if head_is_ph {
                if contains_placeholder(sub) && !is_single_placeholder_pat(sub) {
                    return Err(embedded_pat_err());
                }
            } else if contains_placeholder(sub) {
                return Err(embedded_pat_err());
            }
        }
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

/// True when the subpattern carries any placeholder ident.
fn contains_placeholder(pat: &syn::Pat) -> bool {
    quote::quote!(#pat).to_string().contains("__RSMPL_")
}

/// True when the subpattern is exactly one placeholder ident (`Q`).
fn is_single_placeholder_pat(pat: &syn::Pat) -> bool {
    if let syn::Pat::Ident(pid) = pat {
        return pid.subpat.is_none() && pid.ident.to_string().starts_with("__RSMPL_");
    }
    false
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
    .with_repair(StructuredError::fix_pattern_repair(
        "C8",
        "use a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts",
    ))
}

/// Split leading attribute lines (`#[...]`, possibly multi-line, and
/// `///`/`//!` doc-comment lines normalized to `#[doc = "..."]`) from
/// pattern text; returns (attrs, rest).
///
/// `pub` for the `tests/pattern_compilation.rs` integration suite (T171),
/// which pins the peeling contract directly.
pub fn split_leading_attrs(text: &str) -> (Vec<String>, String) {
    let lines: Vec<&str> = text.lines().collect();
    let mut attrs = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].trim_start().starts_with("#[") {
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
        } else if let Some(attr) = crate::fn_pattern::doc_comment_to_attr(lines[idx].trim()) {
            attrs.push(attr);
            idx += 1;
        } else {
            break;
        }
    }
    (attrs, lines[idx..].join("\n"))
}
