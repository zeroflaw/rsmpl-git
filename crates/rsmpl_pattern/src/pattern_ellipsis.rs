//! Trailing-block `...` validation (WI-5b): every substituted `...` marker
//! must sit as the final pseudo-statement of a `syn::Block`.
//!
//! Counts valid tails and flags anything elsewhere; callers compare
//! total-vs-valid so macro-opaque `...` positions fail loud at C1.

use rsmpl_patch::{ErrorCode, StructuredError};
use syn::visit::Visit;

use crate::expr_pattern::ExprPattern;
use crate::metavar::MetavarTable;
use crate::quote_utils::{
    count_ellipsis_markers, is_ellipsis_marker_ident, mask_literal_spans, unmask_literal_spans,
};

/// Reject `...` in positions SPEC 16 forbids (visibility/qualifier/generics
/// slots are handled by fn parser; here we reject stray `...` as path prefix
/// misuse like `...foo` without `::`).
pub(crate) fn validate_ellipsis_positions(minus: &str, plus: &str) -> Result<(), StructuredError> {
    for text in [minus, plus] {
        for line in text.lines() {
            let t = line.trim();
            // `...` must appear as a standalone token or inside delimiters.
            // A cheap check: no identifier char directly adjacent: `foo...bar`.
            // Byte-safe: `str::get` never panics on multi-byte text (an
            // em dash or CJK ident adjacent to `...` slices on char
            // boundaries, never inside a codepoint).
            let bytes = t.as_bytes();
            let mut i = 0;
            while i + 2 < bytes.len() {
                if t.get(i..i + 3) == Some("...") {
                    let before = if i > 0 {
                        t[..i].chars().next_back().unwrap_or(' ')
                    } else {
                        ' '
                    };
                    let after = t[i + 3..].chars().next().unwrap_or(' ');
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

/// WI-5b C1 diagnostic: `...` in general patterns is admitted only as
/// trailing block pseudo-statements.
pub(crate) fn c1_err() -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        "v1 general patterns support `...` only as trailing block statements (`{ prefix; ... }` replays the absorbed tail, omitting it drops the tail); any other `...` position needs a call pattern or a concrete shape",
    )
    .with_repair(StructuredError::fix_pattern_repair(
        "C1",
        "keep `...` only as the last statement of a block, or use a call pattern or a concrete shape",
    ))
}

/// True when the ident is a substituted `...` marker (`__RSMPL_ELLIPSIS_N`).
fn is_ellipsis_ident(ident: &str) -> bool {
    // WI-5b-follow: digit-suffixed only (see quote_utils).
    is_ellipsis_marker_ident(ident)
}

fn is_bare_ellipsis_path(path: &syn::Path) -> bool {
    path.leading_colon.is_none()
        && path.segments.len() == 1
        && path.segments[0].arguments.is_none()
        && is_ellipsis_ident(&path.segments[0].ident.to_string())
}

fn is_bare_ellipsis_expr(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Path(p) if is_bare_ellipsis_path(&p.path))
}

/// A trailing-block pseudo-statement: a bare `...` expression statement,
/// with or without its `;`.
fn is_block_ellipsis_stmt(stmt: &syn::Stmt) -> bool {
    matches!(stmt, syn::Stmt::Expr(e, _) if is_bare_ellipsis_expr(e))
}

/// WI-5b trailing-`...` validator: every substituted `...` marker must sit
/// as the final pseudo-statement of a `syn::Block`. Counts the valid tails;
/// flags anything elsewhere (call args, operands, types, patterns, match
/// arms, mid-block, macro token streams are opaque to the walk and trip
/// the total-vs-valid comparison at the call sites).
struct EllipsisCheck {
    valid: usize,
    misplaced: bool,
}

impl<'ast> Visit<'ast> for EllipsisCheck {
    fn visit_block(&mut self, node: &'ast syn::Block) {
        for (i, stmt) in node.stmts.iter().enumerate() {
            let last = i + 1 == node.stmts.len();
            if is_block_ellipsis_stmt(stmt) {
                if last {
                    self.valid += 1;
                } else {
                    self.misplaced = true;
                }
            }
        }
        for stmt in &node.stmts {
            if !is_block_ellipsis_stmt(stmt) {
                self.visit_stmt(stmt);
            }
        }
    }

    fn visit_path(&mut self, node: &'ast syn::Path) {
        if node
            .segments
            .iter()
            .any(|s| is_ellipsis_ident(&s.ident.to_string()))
        {
            self.misplaced = true;
            return;
        }
        syn::visit::visit_path(self, node);
    }

    fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
        if is_ellipsis_ident(&node.ident.to_string()) {
            self.misplaced = true;
            return;
        }
        syn::visit::visit_pat_ident(self, node);
    }
}

/// Count `syn::Block` nodes in a visited tree (v1 replay-alignment guard).
struct BlockCounter {
    count: usize,
}

impl<'ast> Visit<'ast> for BlockCounter {
    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.count += 1;
        syn::visit::visit_block(self, node);
    }
}

/// Substitute the plus side the same way the minus was substituted, so
/// ellipsis positions can be validated structurally.
fn plus_sub_for_ellipsis(
    plus_rest: &str,
    table: &MetavarTable,
) -> Result<(String, usize), StructuredError> {
    if plus_rest.trim().is_empty() {
        return Ok((String::new(), 0));
    }
    let (masked, sentinels) = mask_literal_spans(plus_rest)?;
    let (sub, maps) = ExprPattern::placeholder_expr(&masked, table);
    let total = maps.ellipsis_ids.len();
    Ok((unmask_literal_spans(&sub, &sentinels), total))
}

/// Shared tail-count rule (mirrors the SPEC 16.3 call-argument replay/drop
/// discipline): the plus side replays every minus tail (`J == K`) or drops
/// them all (`J == 0`); anything between cannot map tails to `...` in order
/// and fails loud. A plus `...` with no minus `...` names the replay fence.
fn check_tail_counts(minus_count: usize, plus_count: usize) -> Result<(), StructuredError> {
    if plus_count == 0 {
        return Ok(());
    }
    if minus_count == 0 {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`...` in `+` has no matching `- ...` to replay",
        )
        .with_suggestion("drop the `+ ...` or add a `- ...` tail absorbing the replayed statements")
        .with_repair(StructuredError::fix_pattern_repair(
            "C1",
            "drop the `+ ...` or add a `- ...` tail absorbing the replayed statements",
        )));
    }
    if plus_count != minus_count {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "trailing block `...` count differs (`-` has {minus_count}, `+` has {plus_count}); v1 replays all tails in order or drops them all — write the same number of `...` on both sides, or none on `+`"
            ),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C1",
            "write the same number of `...` on both sides, or none on `+`",
        )));
    }
    Ok(())
}

/// Validate a Stmt-shape (`let`) pair: minus must already parse as Local.
pub(crate) fn check_general_ellipsis_stmt(
    local: &syn::Local,
    plus_rest: &str,
    table: &MetavarTable,
) -> Result<(), StructuredError> {
    let mut ck = EllipsisCheck {
        valid: 0,
        misplaced: false,
    };
    ck.visit_local(local);
    let (plus_sub, plus_total) = plus_sub_for_ellipsis(plus_rest, table)?;
    let mut pk = EllipsisCheck {
        valid: 0,
        misplaced: false,
    };
    let mut plus_blocks = 0;
    if !plus_sub.trim().is_empty() {
        match syn::parse_str::<syn::Stmt>(&plus_sub) {
            Ok(syn::Stmt::Local(plus_local)) => {
                pk.visit_local(&plus_local);
                let mut bc = BlockCounter { count: 0 };
                bc.visit_local(&plus_local);
                plus_blocks = bc.count;
            }
            _ if plus_total > 0 => return Err(c1_err()),
            _ => {}
        }
    }
    // Total-vs-valid: macro token streams are opaque to the walk, so a
    // `...` hiding there counts in the total but never as valid.
    let minus_total = {
        let rendered = quote::quote!(#local).to_string();
        count_ellipsis_markers(&rendered)
    };
    if ck.misplaced || ck.valid != minus_total {
        return Err(c1_err());
    }
    if pk.misplaced || pk.valid != plus_total {
        return Err(c1_err());
    }
    check_tail_counts(ck.valid, pk.valid)?;
    if pk.valid > 0 {
        let mut bc = BlockCounter { count: 0 };
        bc.visit_local(local);
        if bc.count != plus_blocks {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "trailing block `...` replay needs the same block structure on both sides (`-` has {} blocks, `+` has {plus_blocks}); v1 maps tails to `...` in source order",
                    bc.count
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "C1",
                "keep the same block structure on both sides, or drop the `+ ...` tails",
            )));
        }
    }
    Ok(())
}

/// Validate an Expr-shape pair: minus must already parse as Expr.
pub(crate) fn check_general_ellipsis_expr(
    pat_expr: &syn::Expr,
    plus_rest: &str,
    table: &MetavarTable,
) -> Result<(), StructuredError> {
    let mut ck = EllipsisCheck {
        valid: 0,
        misplaced: false,
    };
    ck.visit_expr(pat_expr);
    let (plus_sub, plus_total) = plus_sub_for_ellipsis(plus_rest, table)?;
    let mut pk = EllipsisCheck {
        valid: 0,
        misplaced: false,
    };
    let mut plus_blocks = 0;
    if !plus_sub.trim().is_empty() {
        // Mirror the `had_semi` peel: `...;` statement forms compile as
        // their inner expression.
        let inner: String = match syn::parse_str::<syn::Stmt>(&plus_sub) {
            Ok(syn::Stmt::Expr(e, Some(_))) => quote::quote!(#e).to_string(),
            _ => plus_sub.clone(),
        };
        match syn::parse_str::<syn::Expr>(&inner) {
            Ok(plus_expr) => {
                pk.visit_expr(&plus_expr);
                let mut bc = BlockCounter { count: 0 };
                bc.visit_expr(&plus_expr);
                plus_blocks = bc.count;
            }
            _ if plus_total > 0 => return Err(c1_err()),
            _ => {}
        }
    }
    let minus_total = {
        let rendered = quote::quote!(#pat_expr).to_string();
        count_ellipsis_markers(&rendered)
    };
    if ck.misplaced || ck.valid != minus_total {
        return Err(c1_err());
    }
    if pk.misplaced || pk.valid != plus_total {
        return Err(c1_err());
    }
    check_tail_counts(ck.valid, pk.valid)?;
    if pk.valid > 0 {
        let mut bc = BlockCounter { count: 0 };
        bc.visit_expr(pat_expr);
        if bc.count != plus_blocks {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "trailing block `...` replay needs the same block structure on both sides (`-` has {} blocks, `+` has {plus_blocks}); v1 maps tails to `...` in source order",
                    bc.count
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "C1",
                "keep the same block structure on both sides, or drop the `+ ...` tails",
            )));
        }
    }
    Ok(())
}
