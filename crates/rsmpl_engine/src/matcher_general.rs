//! Generalized statement/expression/type matching and rewriting (T10).
//!
//! v1 supports three pattern shapes (see `rsmpl_pattern::GeneralShape`):
//!
//! - `Stmt`: a `let` statement (`syn::Stmt::Local`, no `let-else`). The bound
//!   pattern may be a whole-pattern placeholder (`# pat P`), a concrete
//!   pattern, or `P: T` with placeholder parts; the initializer is matched
//!   with the general expression matcher.
//! - `Expr`: a general Rust expression over a bounded variant set
//!   (paths, literals, binary/unary/cast/reference/paren, calls, method
//!   calls, assignment, field access, indexing, arrays, tuples, struct
//!   literals, ranges, `await`, `?`). Anything else rejects the candidate.
//! - `Type`: a general Rust type over a bounded variant set (paths with
//!   generic args, references, slices, arrays, tuples, parens, `_`).
//!
//! Matching is structural with placeholder binders (`__RSMPL_X` idents
//! standing for declared metavariables). Repeated occurrences must be
//! structurally equal (SPEC 15.7). Matching is a single top-down,
//! source-order, outermost-wins pass like `# allow=many` calls (SPEC 17.6).
//! Unknown shapes never guess: the candidate is rejected and ordinary
//! match-count validation reports the outcome.

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, GeneralShape, PatternKind};

use crate::edits::{fragments_equal, TextEdit};
use crate::lines::LineIndex;
use crate::matcher_expr::substitute_plus;
use crate::matcher_fn::delete_lines;

/// A match candidate with original byte range + line span.
#[derive(Debug, Clone)]
struct Node {
    start: usize,
    end: usize,
    start_line: u32,
    end_line: u32,
}

struct Collector<'ast, 'b> {
    index: &'b LineIndex,
    stmts: Vec<(syn::Local, Node)>,
    exprs: Vec<(&'ast syn::Expr, Node)>,
    types: Vec<(&'ast syn::Type, Node)>,
}

impl Collector<'_, '_> {
    fn span_range(&self, span: proc_macro2::Span) -> Option<(usize, usize)> {
        let s = span.start();
        let e = span.end();
        let start = self.index.offset(s.line as u32, s.column)?;
        let end = self.index.offset(e.line as u32, e.column)?;
        Some((start, end))
    }

    fn node_of(&self, span: proc_macro2::Span) -> Option<Node> {
        let (start, end) = self.span_range(span)?;
        Some(Node {
            start,
            end,
            start_line: span.start().line as u32,
            end_line: span.end().line as u32,
        })
    }
}

impl<'ast, 'b> Visit<'ast> for Collector<'ast, 'b> {
    fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
        if let syn::Stmt::Local(local) = node {
            // Attribute-aware start: slice from the first attribute (or the
            // `let` keyword) so preservation/deletion covers the envelope.
            // Cloning keeps the original spans for byte-exact binding.
            let attr_lc = local
                .attrs
                .first()
                .map(|a| a.span().start())
                .unwrap_or_else(|| local.let_token.span().start());
            let end_lc = local.span().end();
            if let (Some(start), Some(end)) = (
                self.index.offset(attr_lc.line as u32, attr_lc.column),
                self.index.offset(end_lc.line as u32, end_lc.column),
            ) {
                self.stmts.push((
                    (*local).clone(),
                    Node {
                        start,
                        end,
                        start_line: attr_lc.line as u32,
                        end_line: end_lc.line as u32,
                    },
                ));
            }
        }
        visit::visit_stmt(self, node);
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        // Keep the original AST node: re-parsing the slice would reset spans
        // to snippet coordinates and corrupt byte-exact bindings.
        if let Some(n) = self.node_of(node.span()) {
            self.exprs.push((node, n));
        }
        visit::visit_expr(self, node);
    }

    fn visit_type(&mut self, node: &'ast syn::Type) {
        // Same as expressions: match the original node, never a re-parse.
        if let Some(n) = self.node_of(node.span()) {
            self.types.push((node, n));
        }
        visit::visit_type(self, node);
    }
}

/// Per-candidate matching context: bindings plus placeholder maps.
struct MatchCx<'a> {
    original: &'a str,
    index: &'a LineIndex,
    /// placeholder ident (`__RSMPL_E`) -> declared name (`E`).
    reverse: HashMap<String, String>,
    /// declared name -> bound original text.
    bindings: HashMap<String, String>,
}

impl<'a> MatchCx<'a> {
    fn slice(&self, span: proc_macro2::Span) -> Option<String> {
        let s = span.start();
        let e = span.end();
        let start = self.index.offset(s.line as u32, s.column)?;
        let end = self.index.offset(e.line as u32, e.column)?;
        self.original.get(start..end).map(|t| t.to_string())
    }

    fn bind(&mut self, name: &str, text: &str) -> bool {
        match self.bindings.get(name) {
            Some(prev) => fragments_equal(prev, text),
            None => {
                self.bindings.insert(name.to_string(), text.to_string());
                true
            }
        }
    }

    fn bind_span(&mut self, name: &str, span: proc_macro2::Span) -> bool {
        match self.slice(span) {
            Some(t) => self.bind(name, &t),
            None => false,
        }
    }
}

/// Declared metavariable behind a placeholder ident, if any.
fn as_placeholder<'m>(reverse: &'m HashMap<String, String>, ident: &str) -> Option<&'m String> {
    reverse.get(ident)
}

fn render<T: quote::ToTokens>(node: &T) -> String {
    quote::quote!(#node).to_string()
}

fn render_eq<T: quote::ToTokens, U: quote::ToTokens>(a: &T, b: &U) -> bool {
    fragments_equal(&render(a), &render(b))
}

/// Match pattern path segments against target segments. A whole-segment
/// placeholder binds the target segment text; concrete segments must equal.
fn match_path(pat: &syn::Path, tgt: &syn::Path, cx: &mut MatchCx) -> bool {
    if pat.leading_colon.is_some() != tgt.leading_colon.is_some() {
        return false;
    }
    if pat.segments.len() != tgt.segments.len() {
        return false;
    }
    for (ps, ts) in pat.segments.iter().zip(tgt.segments.iter()) {
        let id = ps.ident.to_string();
        if let Some(name) = as_placeholder(&cx.reverse, &id).cloned() {
            if !matches!(ps.arguments, syn::PathArguments::None) {
                return false; // `P::<...>` shapes are not supported in v1
            }
            if !cx.bind(&name, &ts.ident.to_string()) {
                return false;
            }
            continue;
        }
        if ps.ident != ts.ident {
            return false;
        }
        if !match_path_args(&ps.arguments, &ts.arguments, cx) {
            return false;
        }
    }
    true
}

fn match_path_args(pat: &syn::PathArguments, tgt: &syn::PathArguments, cx: &mut MatchCx) -> bool {
    match (pat, tgt) {
        (syn::PathArguments::None, syn::PathArguments::None) => true,
        (syn::PathArguments::AngleBracketed(a), syn::PathArguments::AngleBracketed(b)) => {
            if a.args.len() != b.args.len() {
                return false;
            }
            if a.colon2_token.is_some() != b.colon2_token.is_some() {
                return false;
            }
            for (pa, ta) in a.args.iter().zip(b.args.iter()) {
                if !match_generic_arg(pa, ta, cx) {
                    return false;
                }
            }
            true
        }
        _ => render_eq(pat, tgt),
    }
}

fn match_generic_arg(
    pat: &syn::GenericArgument,
    tgt: &syn::GenericArgument,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        // Lifetimes are wildcards in v1 (documented).
        (syn::GenericArgument::Lifetime(_), syn::GenericArgument::Lifetime(_)) => true,
        (syn::GenericArgument::Type(pt), syn::GenericArgument::Type(tt)) => {
            if let Some(name) = single_placeholder_type(pt, cx) {
                return cx.bind_span(&name, tt.span());
            }
            match_type(pt, tt, cx)
        }
        (syn::GenericArgument::Const(a), syn::GenericArgument::Const(b)) => match_expr(a, b, cx),
        _ => render_eq(pat, tgt),
    }
}

/// Declared name if `t` is exactly one placeholder path (`__RSMPL_T`).
fn single_placeholder_type(t: &syn::Type, cx: &MatchCx) -> Option<String> {
    if let syn::Type::Path(tp) = t {
        if tp.qself.is_none() && tp.path.segments.len() == 1 {
            let seg = &tp.path.segments[0];
            if matches!(seg.arguments, syn::PathArguments::None) {
                return as_placeholder(&cx.reverse, &seg.ident.to_string()).cloned();
            }
        }
    }
    None
}

/// Match a single whole-expression placeholder (`E` alone) or recurse.
fn match_expr(pat: &syn::Expr, tgt: &syn::Expr, cx: &mut MatchCx) -> bool {
    if let syn::Expr::Path(ep) = pat {
        if ep.qself.is_none()
            && ep.path.segments.len() == 1
            && matches!(ep.path.segments[0].arguments, syn::PathArguments::None)
        {
            let id = ep.path.segments[0].ident.to_string();
            if let Some(name) = as_placeholder(&cx.reverse, &id).cloned() {
                return cx.bind_span(&name, tgt.span());
            }
        }
    }
    match (pat, tgt) {
        (syn::Expr::Path(a), syn::Expr::Path(b)) => match_path(&a.path, &b.path, cx),
        (syn::Expr::Lit(a), syn::Expr::Lit(b)) => render_eq(a, b),
        (syn::Expr::Binary(a), syn::Expr::Binary(b)) => {
            render(&a.op) == render(&b.op)
                && match_expr(&a.left, &b.left, cx)
                && match_expr(&a.right, &b.right, cx)
        }
        (syn::Expr::Unary(a), syn::Expr::Unary(b)) => {
            render(&a.op) == render(&b.op) && match_expr(&a.expr, &b.expr, cx)
        }
        (syn::Expr::Cast(a), syn::Expr::Cast(b)) => {
            match_expr(&a.expr, &b.expr, cx) && match_type(&a.ty, &b.ty, cx)
        }
        (syn::Expr::Reference(a), syn::Expr::Reference(b)) => {
            render(&a.mutability) == render(&b.mutability) && match_expr(&a.expr, &b.expr, cx)
        }
        (syn::Expr::Paren(a), syn::Expr::Paren(b)) => match_expr(&a.expr, &b.expr, cx),
        (syn::Expr::Group(a), syn::Expr::Group(b)) => match_expr(&a.expr, &b.expr, cx),
        (syn::Expr::Call(a), syn::Expr::Call(b)) => {
            a.args.len() == b.args.len()
                && match_expr(&a.func, &b.func, cx)
                && a.args
                    .iter()
                    .zip(b.args.iter())
                    .all(|(x, y)| match_expr(x, y, cx))
        }
        (syn::Expr::MethodCall(a), syn::Expr::MethodCall(b)) => {
            render(&a.turbofish) == render(&b.turbofish)
                && match_expr(&a.receiver, &b.receiver, cx)
                && method_name_matches(&a.method, &b.method, cx)
                && a.args.len() == b.args.len()
                && a.args
                    .iter()
                    .zip(b.args.iter())
                    .all(|(x, y)| match_expr(x, y, cx))
        }
        (syn::Expr::Assign(a), syn::Expr::Assign(b)) => {
            match_expr(&a.left, &b.left, cx) && match_expr(&a.right, &b.right, cx)
        }
        (syn::Expr::Field(a), syn::Expr::Field(b)) => {
            match_expr(&a.base, &b.base, cx) && match_member(&a.member, &b.member, cx)
        }
        (syn::Expr::Index(a), syn::Expr::Index(b)) => {
            match_expr(&a.expr, &b.expr, cx) && match_expr(&a.index, &b.index, cx)
        }
        (syn::Expr::Array(a), syn::Expr::Array(b)) => {
            a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_expr(x, y, cx))
        }
        (syn::Expr::Tuple(a), syn::Expr::Tuple(b)) => {
            a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_expr(x, y, cx))
        }
        (syn::Expr::Struct(a), syn::Expr::Struct(b)) => {
            match_path(&a.path, &b.path, cx)
                && a.fields.len() == b.fields.len()
                && a.fields.iter().zip(b.fields.iter()).all(|(x, y)| {
                    match_member(&x.member, &y.member, cx) && match_expr(&x.expr, &y.expr, cx)
                })
                && render(&a.rest) == render(&b.rest)
        }
        (syn::Expr::Range(a), syn::Expr::Range(b)) => {
            render(&a.limits) == render(&b.limits)
                && opt_expr_eq(&a.start, &b.start, cx)
                && opt_expr_eq(&a.end, &b.end, cx)
        }
        (syn::Expr::Await(a), syn::Expr::Await(b)) => match_expr(&a.base, &b.base, cx),
        (syn::Expr::Try(a), syn::Expr::Try(b)) => match_expr(&a.expr, &b.expr, cx),
        _ => false,
    }
}

fn opt_expr_eq(a: &Option<Box<syn::Expr>>, b: &Option<Box<syn::Expr>>, cx: &mut MatchCx) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => match_expr(x, y, cx),
        _ => false,
    }
}

fn method_name_matches(pat: &syn::Ident, tgt: &syn::Ident, cx: &mut MatchCx) -> bool {
    let id = pat.to_string();
    if let Some(name) = as_placeholder(&cx.reverse, &id).cloned() {
        return cx.bind(&name, &tgt.to_string());
    }
    pat == tgt
}

fn match_member(pat: &syn::Member, tgt: &syn::Member, cx: &mut MatchCx) -> bool {
    match (pat, tgt) {
        (syn::Member::Named(a), syn::Member::Named(b)) => {
            let id = a.to_string();
            if let Some(name) = as_placeholder(&cx.reverse, &id).cloned() {
                return cx.bind(&name, &b.to_string());
            }
            a == b
        }
        (syn::Member::Unnamed(a), syn::Member::Unnamed(b)) => a.index == b.index,
        _ => false,
    }
}

fn match_type(pat: &syn::Type, tgt: &syn::Type, cx: &mut MatchCx) -> bool {
    if let Some(name) = single_placeholder_type(pat, cx) {
        return cx.bind_span(&name, tgt.span());
    }
    match (pat, tgt) {
        (syn::Type::Path(a), syn::Type::Path(b)) => match_path(&a.path, &b.path, cx),
        (syn::Type::Reference(a), syn::Type::Reference(b)) => {
            // Lifetimes are wildcards in v1 (documented).
            render(&a.mutability) == render(&b.mutability) && match_type(&a.elem, &b.elem, cx)
        }
        (syn::Type::Slice(a), syn::Type::Slice(b)) => match_type(&a.elem, &b.elem, cx),
        (syn::Type::Array(a), syn::Type::Array(b)) => {
            match_type(&a.elem, &b.elem, cx) && render_eq(&a.len, &b.len)
        }
        (syn::Type::Tuple(a), syn::Type::Tuple(b)) => {
            a.elems.len() == b.elems.len()
                && a.elems
                    .iter()
                    .zip(b.elems.iter())
                    .all(|(x, y)| match_type(x, y, cx))
        }
        (syn::Type::Paren(a), syn::Type::Paren(b)) => match_type(&a.elem, &b.elem, cx),
        (syn::Type::Group(a), syn::Type::Group(b)) => match_type(&a.elem, &b.elem, cx),
        // `_` matches any type.
        (syn::Type::Infer(_), _) => true,
        (syn::Type::Never(_), syn::Type::Never(_)) => true,
        _ => false,
    }
}

/// Split a `let` pattern into (inner pattern, optional ascribed type).
fn decompose_pat(pat: &syn::Pat) -> (&syn::Pat, Option<&syn::Type>) {
    if let syn::Pat::Type(pt) = pat {
        (&pt.pat, Some(&pt.ty))
    } else {
        (pat, None)
    }
}

fn match_local(pat: &syn::Local, tgt: &syn::Local, cx: &mut MatchCx) -> bool {
    // v1 does not move into `let-else` targets.
    if tgt.init.as_ref().is_some_and(|i| i.diverge.is_some()) {
        return false;
    }
    let (pi, pty) = decompose_pat(&pat.pat);
    let (ti, tty) = decompose_pat(&tgt.pat);
    // Bound (inner) pattern.
    if let syn::Pat::Ident(pid) = pi {
        if let Some(name) = as_placeholder(&cx.reverse, &pid.ident.to_string()).cloned() {
            if pid.subpat.is_some() {
                return false; // `P @ ...` bindings are not supported in v1
            }
            if !cx.bind_span(&name, ti.span()) {
                return false;
            }
        } else if !render_eq(pi, ti) {
            return false;
        }
    } else if !fragments_equal(&render(&pat.pat), &render(&tgt.pat)) {
        return false;
    }
    // Ascribed-type agreement (strict: both absent, or both present and
    // matching). A bare `P` never silently drops an annotation.
    match (pty, tty) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            if let Some(name) = single_placeholder_type(a, cx) {
                if !cx.bind_span(&name, b.span()) {
                    return false;
                }
            } else if !match_type(a, b, cx) {
                return false;
            }
        }
        _ => return false,
    }
    // Initializer.
    match (&pat.init, &tgt.init) {
        (Some(a), Some(b)) => match_expr(&a.expr, &b.expr, cx),
        (None, None) => true,
        _ => false,
    }
}

fn in_scope(start_line: u32, end_line: u32, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    let hi = end.max(old_start);
    match scope {
        Scope::Contained => start_line >= old_start && end_line <= hi,
        Scope::Anchor => start_line >= old_start && start_line <= hi,
        Scope::Overlap => start_line <= hi && end_line >= old_start,
        Scope::Exact => old_len > 0 && start_line == old_start && end_line == end,
    }
}

fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn target_attrs_rendered(local: &syn::Local) -> Vec<String> {
    local.attrs.iter().map(|a| norm_ws(&render(a))).collect()
}

/// Original byte slice of the attribute envelope: from the first attribute
/// (or the `let` keyword) to the `let` keyword start.
fn attr_prefix<'a>(original: &'a str, index: &LineIndex, local: &syn::Local) -> Option<&'a str> {
    let attr_start = local
        .attrs
        .first()
        .map(|a| a.span().start())
        .unwrap_or_else(|| local.let_token.span().start());
    let let_start = local.let_token.span().start();
    let s = index.offset(attr_start.line as u32, attr_start.column)?;
    let e = index.offset(let_start.line as u32, let_start.column)?;
    original.get(s..e)
}

/// Substitute bound metavariables into the plus pattern, restoring backtick
/// literals (masked so substitution cannot touch them).
fn substitute_general_plus(
    plus_raw: &str,
    bindings: &HashMap<String, String>,
    sigiled: bool,
) -> Result<String, StructuredError> {
    let (masked, sentinels) = rsmpl_pattern::mask_literal_spans(plus_raw)?;
    let mut out = substitute_plus(&masked, bindings, "", sigiled);
    for (sent, lit) in &sentinels {
        out = out.replace(sent, lit);
    }
    Ok(out)
}

/// Apply a generalized Stmt/Expr/Type hunk. Returns the match count.
pub fn apply_general_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (shape, minus_sub, plus_raw, minus_attrs, plus_attrs, had_semi) = match &compiled.kind {
        PatternKind::General {
            shape,
            minus_sub,
            plus_raw,
            minus_attrs,
            plus_attrs,
            had_semi,
            ..
        } => (
            *shape,
            minus_sub.clone(),
            plus_raw.clone(),
            minus_attrs.clone(),
            plus_attrs.clone(),
            *had_semi,
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected generalized stmt/expr/type pattern".to_string(),
            ))
        }
    };
    let rule = hunk.rule_name();

    // Symbol gate (SPEC 19.3, same as call matching): without a semantic
    // environment, fail unless `# fallback=syntax` is set.
    let fallback_syntax = hunk
        .directives
        .iter()
        .any(|d| matches!(d, rsmpl_patch::Directive::FallbackSyntax(true)));
    if !compiled.table.symbols.is_empty() && !fallback_syntax {
        let name = compiled
            .table
            .symbols
            .keys()
            .next()
            .cloned()
            .unwrap_or_default();
        return Err(StructuredError::new(
            ErrorCode::SymbolConflict,
            format!("symbol `{name}` could not be resolved (no semantic environment in v1)"),
        )
        .with_rule(&rule)
        .with_suggestion("add `# fallback=syntax` to allow syntactic matching"));
    }

    // Attribute shape validation up front (clear errors, no guessing).
    if shape != GeneralShape::Stmt && (!minus_attrs.is_empty() || !plus_attrs.is_empty()) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "leading attributes are only supported on `let` statement patterns in v1",
        )
        .with_rule(&rule));
    }
    if !plus_attrs.is_empty() && minus_attrs.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`"));
    }
    if !minus_attrs.is_empty()
        && norm_ws(&plus_attrs.join("\n")) != norm_ws(&minus_attrs.join("\n"))
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
        )
        .with_rule(&rule)
        .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`"));
    }

    let reverse: HashMap<String, String> = match &compiled.kind {
        PatternKind::General { placeholders, .. } => placeholders
            .iter()
            .map(|(n, ph)| (ph.clone(), n.clone()))
            .collect(),
        _ => HashMap::new(),
    };

    // Parse the substituted minus pattern and collect candidates.
    let mut collector = Collector {
        index,
        stmts: Vec::new(),
        exprs: Vec::new(),
        types: Vec::new(),
    };
    collector.visit_file(parsed);
    collector.exprs.sort_by_key(|(_, n)| (n.start, n.end));
    collector.types.sort_by_key(|(_, n)| (n.start, n.end));
    collector.stmts.sort_by_key(|(_, n)| (n.start, n.end));

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;

    match shape {
        GeneralShape::Stmt => {
            let pat_stmt = syn::parse_str::<syn::Stmt>(&minus_sub).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid `let` pattern: {e}"),
                )
                .with_rule(&rule)
            })?;
            let syn::Stmt::Local(pat_local) = &pat_stmt else {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "general statement patterns must be `let` statements in v1",
                )
                .with_rule(&rule));
            };
            let deleting = plus_raw.trim().is_empty();
            for (tgt, node) in &collector.stmts {
                if accepted
                    .iter()
                    .any(|&(s, e)| s <= node.start && node.end <= e)
                {
                    continue;
                }
                if !in_scope(
                    node.start_line,
                    node.end_line,
                    hunk.scope(),
                    hunk.header.old_start,
                    hunk.header.old_len,
                ) {
                    continue;
                }
                let mut cx = MatchCx {
                    original,
                    index,
                    reverse: reverse.clone(),
                    bindings: HashMap::new(),
                };
                if !match_local(pat_local, tgt, &mut cx) {
                    continue;
                }
                // Attribute policy (mirrors fn matching: fail, never guess).
                let prefix = attr_prefix(original, index, tgt).unwrap_or("");
                match hunk.attrs_policy() {
                    AttrsPolicy::Preserve => {
                        if !minus_attrs.is_empty()
                            && norm_ws(&minus_attrs.join("\n"))
                                != norm_ws(&target_attrs_rendered(tgt).join("\n"))
                        {
                            return Err(StructuredError::new(
                                ErrorCode::MatchConflict,
                                "attached attributes do not match explicitly written pattern attributes",
                            )
                            .with_rule(&rule)
                            .with_suggestion("use `# attrs=preserve` (default) to ignore attributes"));
                        }
                    }
                    AttrsPolicy::Absent | AttrsPolicy::Explicit => {
                        if !minus_attrs.is_empty() {
                            if norm_ws(&minus_attrs.join("\n"))
                                != norm_ws(&target_attrs_rendered(tgt).join("\n"))
                            {
                                return Err(StructuredError::new(
                                    ErrorCode::MatchConflict,
                                    "attached attributes do not match explicitly written pattern attributes",
                                )
                                .with_rule(&rule));
                            }
                        } else if !tgt.attrs.is_empty() {
                            return Err(StructuredError::new(
                                ErrorCode::MatchConflict,
                                format!(
                                    "statement has attributes but `# attrs={}` requires none",
                                    match hunk.attrs_policy() {
                                        AttrsPolicy::Absent => "absent",
                                        _ => "explicit",
                                    }
                                ),
                            )
                            .with_rule(&rule));
                        }
                    }
                    AttrsPolicy::Strip => {}
                }
                if deleting {
                    edits.push(delete_lines(
                        original,
                        index,
                        node.start_line,
                        node.end_line,
                        &rule,
                    )?);
                } else {
                    let mut replacement =
                        substitute_general_plus(&plus_raw, &cx.bindings, compiled.table.sigiled)?;
                    if hunk.attrs_policy() == AttrsPolicy::Preserve && minus_attrs.is_empty() {
                        let mut with_prefix = prefix.to_string();
                        with_prefix.push_str(&replacement);
                        replacement = with_prefix;
                    }
                    // Strip: replacement already drops the envelope.
                    edits.push(TextEdit::new(node.start, node.end, replacement));
                }
                for &(s, e) in &accepted {
                    if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                        return Err(StructuredError::new(
                            ErrorCode::MatchConflict,
                            "partially overlapping matches cannot be merged",
                        )
                        .with_rule(&rule));
                    }
                }
                accepted.push((node.start, node.end));
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
        GeneralShape::Expr => {
            let pat_expr = syn::parse_str::<syn::Expr>(&minus_sub).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid expression pattern: {e}"),
                )
                .with_rule(&rule)
            })?;
            let deleting = plus_raw.trim().is_empty();
            for (tgt, node) in &collector.exprs {
                if accepted
                    .iter()
                    .any(|&(s, e)| s <= node.start && node.end <= e)
                {
                    continue;
                }
                if !in_scope(
                    node.start_line,
                    node.end_line,
                    hunk.scope(),
                    hunk.header.old_start,
                    hunk.header.old_len,
                ) {
                    continue;
                }
                let mut cx = MatchCx {
                    original,
                    index,
                    reverse: reverse.clone(),
                    bindings: HashMap::new(),
                };
                if !match_expr(&pat_expr, tgt, &mut cx) {
                    continue;
                }
                if deleting {
                    // Only `...;` statement shapes reach here (compile-gated).
                    edits.push(delete_lines(
                        original,
                        index,
                        node.start_line,
                        node.end_line,
                        &rule,
                    )?);
                } else {
                    let mut replacement =
                        substitute_general_plus(&plus_raw, &cx.bindings, compiled.table.sigiled)?;
                    if had_semi && !replacement.trim_end().ends_with(';') {
                        replacement.push(';');
                    }
                    edits.push(TextEdit::new(node.start, node.end, replacement));
                }
                for &(s, e) in &accepted {
                    if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                        return Err(StructuredError::new(
                            ErrorCode::MatchConflict,
                            "partially overlapping matches cannot be merged",
                        )
                        .with_rule(&rule));
                    }
                }
                accepted.push((node.start, node.end));
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
        GeneralShape::Type => {
            let pat_ty = syn::parse_str::<syn::Type>(&minus_sub).map_err(|e| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid type pattern: {e}"),
                )
                .with_rule(&rule)
            })?;
            for (tgt, node) in &collector.types {
                if accepted
                    .iter()
                    .any(|&(s, e)| s <= node.start && node.end <= e)
                {
                    continue;
                }
                if !in_scope(
                    node.start_line,
                    node.end_line,
                    hunk.scope(),
                    hunk.header.old_start,
                    hunk.header.old_len,
                ) {
                    continue;
                }
                let mut cx = MatchCx {
                    original,
                    index,
                    reverse: reverse.clone(),
                    bindings: HashMap::new(),
                };
                if !match_type(&pat_ty, tgt, &mut cx) {
                    continue;
                }
                let replacement =
                    substitute_general_plus(&plus_raw, &cx.bindings, compiled.table.sigiled)?;
                edits.push(TextEdit::new(node.start, node.end, replacement));
                for &(s, e) in &accepted {
                    if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                        return Err(StructuredError::new(
                            ErrorCode::MatchConflict,
                            "partially overlapping matches cannot be merged",
                        )
                        .with_rule(&rule));
                    }
                }
                accepted.push((node.start, node.end));
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rsmpl_patch::parse_patch;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    const LET_PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=rename_let\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n";

    #[test]
    fn renames_let_initializer() {
        let src = "fn a() {\n    let x = compute();\n    let other = 1;\n}\n";
        let out = apply(LET_PATCH, src).unwrap();
        assert!(out.contains("let y = compute();"), "got: {out}");
        assert!(out.contains("let other = 1;"), "got: {out}");
        assert!(!out.contains("let x ="), "got: {out}");
    }

    #[test]
    fn let_pattern_is_strict_about_annotations() {
        // `let x = E;` must not match `let x: i32 = 1;`.
        let src = "fn a() {\n    let x: i32 = 1;\n}\n";
        assert!(apply(LET_PATCH, src).is_err());
    }

    #[test]
    fn pat_placeholder_with_type_matches_annotated() {
        // A bare `P` never matches an annotated `x: T` (strict, like fn
        // slots); spell the ascription explicitly with `P: T`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=w\n# lang=rust\n# pat P\n# type T\n# expr E\n- let P: T = E;\n+ let P: T = E;\n";
        let src = "fn a() {\n    let x: i32 = 1;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let x: i32 = 1;"), "got: {out}");
    }

    #[test]
    fn wraps_binary_expr() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=wrap\n# lang=rust\n# allow=many\n# expr E\n- E + 1\n+ E.wrapping_add(1)\n";
        let src = "fn a() {\n    let x = y + 1;\n    let z = y + 2;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("y.wrapping_add(1)"), "got: {out}");
        assert!(out.contains("y + 2;"), "got: {out}");
    }

    #[test]
    fn repeated_metavar_must_agree() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=dedupe\n# lang=rust\n# expr E\n- E + E\n+ twice(E)\n";
        let same = "fn a() {\n    let v = x + x;\n}\n";
        let out = apply(patch, same).unwrap();
        assert!(out.contains("twice(x)"), "got: {out}");
        let diff = "fn a() {\n    let v = x + y;\n}\n";
        assert!(apply(patch, diff).is_err());
    }

    #[test]
    fn rewrites_types() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=boxit\n# lang=rust\n# type T\n- Vec<T>\n+ Box<[T]>\n";
        let src = "fn a(x: Vec<u8>) {}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("Box<[u8]>"), "got: {out}");
        assert!(!out.contains("Vec<u8>"), "got: {out}");
    }

    #[test]
    fn deletes_let_statement() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,0 @@ rule=cut\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n";
        let src = "fn a() {\n    let x = 1;\n    let y = 2;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(!out.contains("let x"), "got: {out}");
        assert!(out.contains("let y = 2;"), "got: {out}");
    }

    #[test]
    fn preserves_let_attributes() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,2 +2,2 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n";
        let src = "fn a() {\n    #[allow(let_unit_value)]\n    let x = compute();\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("#[allow(let_unit_value)]"), "got: {out}");
        assert!(out.contains("let y = compute();"), "got: {out}");
    }

    #[test]
    fn call_shapes_still_route_to_call_matcher() {
        // Regression: `foo(E)` must not become a General pattern.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# allow=many\n# expr E\n- foo(E);\n+ bar(E);\n";
        let src = "fn a() {\n    foo(1);\n    foo(2);\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(
            out.contains("bar(1);") && out.contains("bar(2);"),
            "got: {out}"
        );
    }

    #[test]
    fn coverage_table_runtime_rows_are_pinned() {
        // SPEC 18.6 runtime rows: silent candidate rejections surface as
        // `RSMPL_MATCH_CONFLICT` / `no match found`, and the attribute /
        // symbol gates fire with their exact table diagnostics. Pinning
        // these proves the table cannot drift from the code.
        struct Row {
            patch: &'static str,
            file: &'static str,
            code: &'static str,
            reason: &'static str,
            suggestion: Option<&'static str>,
        }
        const NO_MATCH: Option<&str> = Some("check the range and pattern, or add `# optional`");
        const ATTR_SUGGESTION: Option<&str> =
            Some("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`");
        let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let rows = [
            // R1: block as the general expression.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- { E }\n+ { E }\n",
                file: "fn a() {\n    let v = { 1 };\n    let _ = v;\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R1: `if` as the general expression.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- if E { 1 } else { 2 }\n+ if E { 1 } else { 3 }\n",
                file: "fn a(x: bool) {\n    let v = if x { 1 } else { 2 };\n    let _ = v;\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R1: closure as the general expression.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- |x| E\n+ |x| E\n",
                file: "fn a() {\n    let f = |x| x + 1;\n    let _ = f;\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R1: `match` as the general expression.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- match E { _ => 1 }\n+ match E { _ => 2 }\n",
                file: "fn a(x: bool) {\n    let v = match x { _ => 1 };\n    let _ = v;\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R1: `loop` as the general expression.
            Row {
                patch: "@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- loop { E }\n+ loop { E }\n",
                file: "fn a() {\n    loop { break; }\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R2: `let-else` in the target file.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n",
                file: "fn a(o: Option<i32>) -> i32 {\n    let x = o else { return 0 };\n    x\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R3: turbofish placeholder (placeholder path segment with args).
            Row {
                patch: "@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# type P\n- P<u8>\n+ Vec<u8>\n",
                file: "fn a(x: Vec<u8>) {}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R4: `P @ ...` subpattern binding in `let`.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# pat P\n# expr E\n- let P @ E = E;\n+ let P @ E = E;\n",
                file: "fn a() {\n    let q = 1;\n    let _ = q;\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R5: `dyn Trait` as the general type.
            Row {
                patch: "@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# type T\n- Box<dyn T>\n+ Box<dyn T>\n",
                file: "fn a(x: Box<dyn std::fmt::Display>) {}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // C10: leading attributes on a non-`let` shape (attribute lines
            // are split per line, so `#[attr]` stands on its own line).
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# expr E\n- #[attr]\n- E + 1\n+ E + 2\n",
                file: "fn a() {\n    let v = y + 1;\n    let _ = v;\n}\n",
                code: "RSMPL_PATTERN_CONFLICT",
                reason: "leading attributes are only supported on `let` statement patterns in v1",
                suggestion: None,
            },
            // C11: `+` attributes the `-` side did not match.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ #[attr]\n+ let y = E;\n",
                file: "fn a() {\n    let x = compute();\n    let _ = x;\n}\n",
                code: "RSMPL_PATTERN_CONFLICT",
                reason: "replacement writes attributes the pattern did not match",
                suggestion: ATTR_SUGGESTION,
            },
            // C12: mismatched attribute sets.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- #[a]\n- let x = E;\n+ #[b]\n+ let y = E;\n",
                file: "fn a() {\n    let x = compute();\n    let _ = x;\n}\n",
                code: "RSMPL_PATTERN_CONFLICT",
                reason: "replacement attributes differ from matched attributes",
                suggestion: ATTR_SUGGESTION,
            },
            // C13: `# symbol` without `# fallback=syntax` (general-matcher gate).
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# expr E\n# symbol Old=crate::old_api\n- E + 1\n+ E + 2\n",
                file: "fn a() {\n    let v = y + 1;\n    let _ = v;\n}\n",
                code: "RSMPL_SYMBOL_CONFLICT",
                reason: "symbol `Old` could not be resolved (no semantic environment in v1)",
                suggestion: Some("add `# fallback=syntax` to allow syntactic matching"),
            },
        ];
        assert_eq!(
            rows.len(),
            13,
            "SPEC 18.6 lists R1-R5 + C10-C13 runtime rows"
        );
        for row in rows {
            let patch = format!("{head}{}", row.patch);
            let err = apply(&patch, row.file).unwrap_err();
            assert!(
                err.contains(row.code),
                "SPEC 18.6 drift: want code {} in:\n{err}",
                row.code
            );
            assert!(
                err.contains(row.reason),
                "SPEC 18.6 drift: want reason {:?} in:\n{err}",
                row.reason
            );
            if let Some(s) = row.suggestion {
                assert!(
                    err.contains(s),
                    "SPEC 18.6 drift: want suggestion {s:?} in:\n{err}"
                );
            }
        }
    }

    #[test]
    fn concrete_generic_args_match_structurally() {
        // R3 counterpart (SPEC 18.6): concrete generic arguments compare
        // structurally — only placeholder segments carrying arguments are
        // rejected. A fully concrete `Vec<u8>` rewrite must succeed.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n- Vec<u8>\n+ Box<u8>\n";
        let out = apply(patch, "fn a(x: Vec<u8>) {}\n").unwrap();
        assert!(out.contains("Box<u8>"), "got: {out}");
        assert!(!out.contains("Vec<u8>"), "got: {out}");
    }
}
