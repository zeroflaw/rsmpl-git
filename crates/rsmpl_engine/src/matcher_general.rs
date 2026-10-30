//! Generalized statement/expression/type matching and rewriting (T10).
//!
//! v1 supports three pattern shapes (see `rsmpl_pattern::GeneralShape`):
//!
//! - `Stmt`: a `let` statement (`syn::Stmt::Local`, with or without a
//!   `let-else` diverge block — both sides must agree structurally). The bound
//!   pattern may be a whole-pattern placeholder (`# pat P`), a concrete
//!   pattern, `P: T` with placeholder parts, or `P @ sub` (WI-5a: `P`
//!   binds the whole target pattern text, `sub` matches recursively);
//!   the initializer is matched with the general expression matcher.
//! - `Expr`: a general Rust expression over a bounded variant set
//!   (paths, literals, binary/unary/cast/reference/paren, calls, method
//!   calls, assignment, field access, indexing, arrays, tuples, struct
//!   literals, ranges, `await`, `?`, `return`, `if` with condition +
//!   then-block + else branch, blocks as concrete-length statement
//!   sequences (WI-5a), plus `match` (scrutinee + arms), `loop`/`while`/
//!   `for` (label-strict, bodies structural), closures (qualifiers
//!   strict, params/return/body structural), and `let` expressions
//!   (WI-5b)). Anything else rejects the candidate.
//! - `Type`: a general Rust type over a bounded variant set (paths with
//!   generic args, references, slices, arrays, tuples, parens, raw
//!   pointers, fn pointers, `impl Trait`, `dyn Trait`, `_`).
//!
//! Matching is structural with placeholder binders (`__RSMPL_X` idents
//! standing for declared metavariables). Repeated occurrences must be
//! structurally equal (SPEC 15.7). Matching is a single top-down,
//! source-order, outermost-wins pass like `# allow=many` calls (SPEC 17.6).
//! Unknown shapes never guess: the candidate is rejected and ordinary
//! match-count validation reports the outcome.

use std::cmp::Reverse;
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
pub(crate) struct MatchCx<'a> {
    original: &'a str,
    index: &'a LineIndex,
    /// placeholder ident (`__RSMPL_E`) -> declared name (`E`).
    pub(crate) reverse: HashMap<String, String>,
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

    pub(crate) fn bind(&mut self, name: &str, text: &str) -> bool {
        match self.bindings.get(name) {
            Some(prev) => fragments_equal(prev, text),
            None => {
                self.bindings.insert(name.to_string(), text.to_string());
                true
            }
        }
    }

    pub(crate) fn bind_span(&mut self, name: &str, span: proc_macro2::Span) -> bool {
        match self.slice(span) {
            Some(t) => self.bind(name, &t),
            None => false,
        }
    }
}

/// Declared metavariable behind a placeholder ident, if any.
pub(crate) fn as_placeholder<'m>(
    reverse: &'m HashMap<String, String>,
    ident: &str,
) -> Option<&'m String> {
    reverse.get(ident)
}

pub(crate) fn render<T: quote::ToTokens>(node: &T) -> String {
    quote::quote!(#node).to_string()
}

pub(crate) fn render_eq<T: quote::ToTokens, U: quote::ToTokens>(a: &T, b: &U) -> bool {
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
            if !cx.bind(&name, &ts.ident.to_string()) {
                return false;
            }
            // Placeholder segments may carry generic arguments (T144):
            // `P<u8>`, `P::<T>` match structurally via `match_path_args`.
            // Bare `P` (no args) keeps the wildcard behavior below since
            // `match_path_args(None, <anything>)` only accepts `None`.
            // To preserve the historical bare-`P` wildcard (matches any
            // target args), skip the args check when the pattern has none.
            if !matches!(ps.arguments, syn::PathArguments::None)
                && !match_path_args(&ps.arguments, &ts.arguments, cx)
            {
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

/// Structural method-call turbofish comparison (T144): `None/None`
/// matches, `Some/Some` compares `::` presence + each generic argument
/// via `match_generic_arg` (so `collect::<T>` binds `T`). Anything else
/// (including `None` vs `Some`) fails.
fn match_turbofish(
    pat: &Option<syn::AngleBracketedGenericArguments>,
    tgt: &Option<syn::AngleBracketedGenericArguments>,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.colon2_token.is_some() != b.colon2_token.is_some() {
                return false;
            }
            if a.args.len() != b.args.len() {
                return false;
            }
            for (pa, ta) in a.args.iter().zip(b.args.iter()) {
                if !match_generic_arg(pa, ta, cx) {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

/// Declared name if `t` is exactly one placeholder path (`__RSMPL_T`).
pub(crate) fn single_placeholder_type(t: &syn::Type, cx: &MatchCx) -> Option<String> {
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
pub(crate) fn match_expr(pat: &syn::Expr, tgt: &syn::Expr, cx: &mut MatchCx) -> bool {
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
            match_turbofish(&a.turbofish, &b.turbofish, cx)
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
        (syn::Expr::Return(a), syn::Expr::Return(b)) => opt_expr_eq(&a.expr, &b.expr, cx),
        // WI-5a/5b: `if` + blocks + `match`/loops/closures/`let`
        // expressions match structurally in `matcher_general_ctrl`.
        // Anything else (async/try/unsafe blocks, yield, macro types)
        // still rejects here and stays an ordinary hunk.
        (syn::Expr::If(a), syn::Expr::If(b)) => crate::matcher_general_ctrl::match_if(a, b, cx),
        (syn::Expr::Block(a), syn::Expr::Block(b)) => {
            // Labels compare strictly (a labeled block is a different
            // control construct); bodies match structurally.
            let labels_eq = match (&a.label, &b.label) {
                (None, None) => true,
                (Some(x), Some(y)) => x.name.ident == y.name.ident,
                _ => false,
            };
            labels_eq && crate::matcher_general_ctrl::match_block(&a.block, &b.block, cx)
        }
        (syn::Expr::Match(a), syn::Expr::Match(b)) => {
            crate::matcher_general_ctrl::match_match(a, b, cx)
        }
        (syn::Expr::Loop(a), syn::Expr::Loop(b)) => {
            crate::matcher_general_ctrl::match_loop(a, b, cx)
        }
        (syn::Expr::While(a), syn::Expr::While(b)) => {
            crate::matcher_general_ctrl::match_while(a, b, cx)
        }
        (syn::Expr::ForLoop(a), syn::Expr::ForLoop(b)) => {
            crate::matcher_general_ctrl::match_for(a, b, cx)
        }
        (syn::Expr::Closure(a), syn::Expr::Closure(b)) => {
            crate::matcher_general_ctrl::match_closure(a, b, cx)
        }
        (syn::Expr::Let(a), syn::Expr::Let(b)) => {
            crate::matcher_general_ctrl::match_let_expr(a, b, cx)
        }
        // WI-5b: `break` (optional label + value) and `continue`
        // (optional label) — loop bodies almost always contain one.
        (syn::Expr::Break(a), syn::Expr::Break(b)) => {
            render(&a.label) == render(&b.label) && opt_expr_eq(&a.expr, &b.expr, cx)
        }
        (syn::Expr::Continue(a), syn::Expr::Continue(b)) => render(&a.label) == render(&b.label),
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

pub(crate) fn match_type(pat: &syn::Type, tgt: &syn::Type, cx: &mut MatchCx) -> bool {
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
        (syn::Type::Ptr(a), syn::Type::Ptr(b)) => {
            // `*const T` never matches `*mut T`; the pointee recurses so a
            // `# type T` placeholder binds.
            render(&a.const_token) == render(&b.const_token)
                && render(&a.mutability) == render(&b.mutability)
                && match_type(&a.elem, &b.elem, cx)
        }
        (syn::Type::BareFn(a), syn::Type::BareFn(b)) => {
            // `for<'a>` binders are ignored (lifetime-wildcard spirit);
            // unsafety/ABI compare strictly; variadics never match (C1
            // rejects `...` in patterns, so a variadic target fails safe).
            a.variadic.is_none()
                && b.variadic.is_none()
                && render(&a.unsafety) == render(&b.unsafety)
                && render(&a.abi) == render(&b.abi)
                && a.inputs.len() == b.inputs.len()
                && a.inputs
                    .iter()
                    .zip(b.inputs.iter())
                    .all(|(x, y)| match_type(&x.ty, &y.ty, cx))
                && match_return_type(&a.output, &b.output, cx)
        }
        (syn::Type::ImplTrait(a), syn::Type::ImplTrait(b)) => {
            match_bounds(&a.bounds, &b.bounds, cx)
        }
        (syn::Type::TraitObject(a), syn::Type::TraitObject(b)) => {
            // `dyn` presence compares strictly; a bare path is a different
            // variant (`Type::Path`) and never reaches this arm.
            a.dyn_token.is_some() == b.dyn_token.is_some() && match_bounds(&a.bounds, &b.bounds, cx)
        }
        // `_` matches any type.
        (syn::Type::Infer(_), _) => true,
        (syn::Type::Never(_), syn::Type::Never(_)) => true,
        _ => false,
    }
}

/// Match `impl Trait` / `dyn Trait` bound lists structurally (T149): same
/// length, each bound matches; plain lifetime bounds are wildcards per the
/// SPEC deliberate-wildcards rule, `for<'a>` binders and `?` modifiers
/// compare strictly.
fn match_bounds(
    pat: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    tgt: &syn::punctuated::Punctuated<syn::TypeParamBound, syn::Token![+]>,
    cx: &mut MatchCx,
) -> bool {
    pat.len() == tgt.len()
        && pat
            .iter()
            .zip(tgt.iter())
            .all(|(a, b)| match_type_param_bound(a, b, cx))
}

fn match_type_param_bound(
    pat: &syn::TypeParamBound,
    tgt: &syn::TypeParamBound,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        // Lifetimes are wildcards in v1 (documented).
        (syn::TypeParamBound::Lifetime(_), syn::TypeParamBound::Lifetime(_)) => true,
        (syn::TypeParamBound::Trait(a), syn::TypeParamBound::Trait(b)) => {
            render(&a.modifier) == render(&b.modifier)
                && render(&a.lifetimes) == render(&b.lifetimes)
                && match_path(&a.path, &b.path, cx)
        }
        _ => render_eq(pat, tgt),
    }
}

/// Match `fn` return types structurally (T149): absent/absent matches, and
/// a whole-type placeholder (`T`) binds the target span like elsewhere.
/// `pub(crate)` for `matcher_general_ctrl` closure arms (WI-5b).
pub(crate) fn match_return_type(
    pat: &syn::ReturnType,
    tgt: &syn::ReturnType,
    cx: &mut MatchCx,
) -> bool {
    match (pat, tgt) {
        (syn::ReturnType::Default, syn::ReturnType::Default) => true,
        (syn::ReturnType::Type(_, a), syn::ReturnType::Type(_, b)) => {
            if let Some(name) = single_placeholder_type(a, cx) {
                return cx.bind_span(&name, b.span());
            }
            match_type(a, b, cx)
        }
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
        Scope::File => true,
    }
}

/// Whitespace-insensitive attribute comparison: strip ALL whitespace so raw
/// pattern text (`#[allow(x)]`) compares equal to `quote!`-rendered targets
/// (`# [ allow ( x ) ]`). Matches the fn/item `check_attrs` normalization.
fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<String>()
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
    let mut out = substitute_plus(&masked, bindings, &[], sigiled);
    for (sent, lit) in &sentinels {
        out = out.replace(sent, lit);
    }
    Ok(out)
}

/// Backstop for `# repeat` nesting (T126: same bound as the call-matcher
/// fold — bindings are strict sub-slices of the matched candidate, so
/// structural descent terminates on its own; this only guards
/// pathological spans).
const MAX_GENERAL_REPEAT_DEPTH: usize = 64;

/// T126 `# repeat`: rewrite every non-symbol binding with the same
/// general-expression rule (bottom-up fold). Symbol seeds are pre-bound
/// paths, not matchable fragments, so they are skipped — like the
/// call-matcher fold in `matcher_expr`.
fn fold_general_bindings(
    bindings: &mut HashMap<String, String>,
    compiled: &CompiledPattern,
    pat_expr: &syn::Expr,
    plus_raw: &str,
    reverse: &HashMap<String, String>,
    depth: usize,
    rule: &str,
) -> Result<(), StructuredError> {
    if depth > MAX_GENERAL_REPEAT_DEPTH {
        return Err(StructuredError::new(
            ErrorCode::MatchConflict,
            "repeat nesting exceeds 64 levels (pathological nesting?)",
        )
        .with_rule(rule)
        .with_suggestion("remove `# repeat` or simplify the nesting"));
    }
    // Snapshot names: rewriting mutates the map.
    let names: Vec<String> = bindings.keys().cloned().collect();
    for name in names {
        if compiled.table.symbols.contains_key(&name) {
            continue;
        }
        if let Some(current) = bindings.get(&name).cloned() {
            let rewritten = rewrite_nested_general(
                &current, compiled, pat_expr, plus_raw, reverse, depth, rule,
            )?;
            bindings.insert(name, rewritten);
        }
    }
    Ok(())
}

/// Re-apply the same general-expression rule inside one binding fragment:
/// collect sub-expressions over the fragment, rewrite outermost-first with
/// recursive descent into sub-bindings, and splice the result. The produced
/// output is never re-scanned, so self-reproducing rules apply exactly one
/// level and terminate by construction. Unparseable fragments are kept
/// verbatim (conservative, like the call-matcher fold).
fn rewrite_nested_general(
    text: &str,
    compiled: &CompiledPattern,
    pat_expr: &syn::Expr,
    plus_raw: &str,
    reverse: &HashMap<String, String>,
    depth: usize,
    rule: &str,
) -> Result<String, StructuredError> {
    let expr: syn::Expr = match syn::parse_str(text) {
        Ok(e) => e,
        Err(_) => return Ok(text.to_string()),
    };
    let index = LineIndex::new(text);
    let mut collector = Collector {
        index: &index,
        stmts: Vec::new(),
        exprs: Vec::new(),
        types: Vec::new(),
    };
    collector.visit_expr(&expr);
    let mut cands = collector.exprs;
    // Longest-first on shared start (T143): left-nested `a + 1 + 1`
    // parses as `(a + 1) + 1` with inner/outer sharing a start byte;
    // shortest-first accepted the inner and the outer tripped the
    // partial-overlap guard. Longest-first restores outermost-wins.
    cands.sort_by_key(|(_, n)| (n.start, Reverse(n.end)));
    // Nested fragments are sub-expressions: strip one trailing `;` so a
    // `;`-terminated file-level pattern never injects semicolons
    // mid-expression. File-level `had_semi` handling is untouched.
    let trimmed = plus_raw.trim_end();
    let nested_plus = trimmed.strip_suffix(';').unwrap_or(trimmed);
    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut edits: Vec<TextEdit> = Vec::new();
    for (tgt, node) in &cands {
        // Outermost-wins, like the file-level pass (SPEC 17.6).
        if accepted
            .iter()
            .any(|&(s, e)| s <= node.start && node.end <= e)
        {
            continue;
        }
        let mut cx = MatchCx {
            original: text,
            index: &index,
            reverse: reverse.clone(),
            bindings: HashMap::new(),
        };
        if !match_expr(pat_expr, tgt, &mut cx) {
            continue;
        }
        for &(s, e) in &accepted {
            if node.start < e && s < node.end && !(s <= node.start && node.end <= e) {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "partially overlapping matches cannot be merged",
                )
                .with_rule(rule));
            }
        }
        fold_general_bindings(
            &mut cx.bindings,
            compiled,
            pat_expr,
            plus_raw,
            reverse,
            depth + 1,
            rule,
        )?;
        let replacement =
            substitute_general_plus(nested_plus, &cx.bindings, compiled.table.sigiled)?;
        edits.push(TextEdit::new(node.start, node.end, replacement));
        accepted.push((node.start, node.end));
    }
    if edits.is_empty() {
        return Ok(text.to_string());
    }
    crate::edits::apply_edits(text, edits)
}

/// Apply a generalized Stmt/Expr/Type hunk. Returns the match count.
pub fn apply_general_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (shape, minus_sub, plus_raw, plus_body, minus_attrs, plus_attrs, had_semi) =
        match &compiled.kind {
            PatternKind::General {
                shape,
                minus_sub,
                plus_raw,
                plus_body,
                minus_attrs,
                plus_attrs,
                had_semi,
                ..
            } => (
                *shape,
                minus_sub.clone(),
                plus_raw.clone(),
                plus_body.clone(),
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
    // `# attrs=strip` skips the mirror gate: mirrored minus attrs are
    // eligibility-ignored (like the fn/item `check_attrs` Strip arm) and
    // the Stmt replacement path drops the envelope. The plus-writes-attrs
    // gate above stays: adding attributes while stripping is ambiguous.
    if hunk.attrs_policy() != AttrsPolicy::Strip
        && !minus_attrs.is_empty()
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
    // T143: exprs sort longest-first on shared start so left-nested
    // `(a + 1) + 1`-shaped spans see the outer first (outermost-wins);
    // stmts/types have no shared-start nesting, keep source order.
    collector
        .exprs
        .sort_by_key(|(_, n)| (n.start, Reverse(n.end)));
    collector.types.sort_by_key(|(_, n)| (n.start, n.end));
    collector.stmts.sort_by_key(|(_, n)| (n.start, n.end));

    let mut accepted: Vec<(usize, usize)> = Vec::new();
    let mut matched = 0;
    let mut all_bindings: Vec<HashMap<String, String>> = Vec::new();

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
                if !crate::matcher_general_ctrl::match_local(pat_local, tgt, &mut cx) {
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
                    // Mirrored attributes (T153): the plus attr lines equal the
                    // target envelope modulo whitespace, so reuse the original
                    // envelope slice (attrs + `let` indent, exactly like the
                    // preserve path) and substitute only the attr-stripped
                    // plus body. Splicing raw plus text would replace the
                    // target's `let` indent with the pattern's one-space
                    // diff-prefix artifact. Scoped to Preserve/Explicit (the
                    // policies that keep attributes); Strip keeps its path.
                    let mirrored = !minus_attrs.is_empty()
                        && matches!(
                            hunk.attrs_policy(),
                            AttrsPolicy::Preserve | AttrsPolicy::Explicit
                        );
                    let mut replacement = if mirrored {
                        let mut with_prefix = prefix.to_string();
                        with_prefix.push_str(&substitute_general_plus(
                            &plus_body,
                            &cx.bindings,
                            compiled.table.sigiled,
                        )?);
                        with_prefix
                    } else {
                        substitute_general_plus(&plus_raw, &cx.bindings, compiled.table.sigiled)?
                    };
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
                all_bindings.push(cx.bindings.clone());
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
                // T126 `# repeat`: fold nested matches by rewriting each
                // bound fragment with the same rule before substitution
                // (bottom-up, like the call-matcher fold).
                if hunk.repeats() {
                    fold_general_bindings(
                        &mut cx.bindings,
                        compiled,
                        &pat_expr,
                        &plus_raw,
                        &reverse,
                        0,
                        &rule,
                    )?;
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
                    let mut end = node.end;
                    if had_semi {
                        // The candidate span excludes the statement's `;`:
                        // consume the original semicolon (mod whitespace) so
                        // the replacement ends with exactly one `;`
                        // (dogfood: `E + 1;` rewrites gave `;;`).
                        let rest = &original[node.end..];
                        let ws = rest.len() - rest.trim_start().len();
                        if rest.get(ws..).is_some_and(|t| t.starts_with(';')) {
                            end = node.end + ws + 1;
                        }
                        if !replacement.trim_end().ends_with(';') {
                            replacement.push(';');
                        }
                    }
                    edits.push(TextEdit::new(node.start, end, replacement));
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
                all_bindings.push(cx.bindings.clone());
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
                all_bindings.push(cx.bindings.clone());
                matched += 1;
                if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                    break;
                }
            }
        }
    }
    Ok((matched, all_bindings))
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
    fn mirrored_attrs_preserve_indent() {
        // T153: mirrored `#[...]` lines compare whitespace-insensitively
        // (raw pattern text vs `quote!` rendering) and the replacement reuses
        // the original envelope slice, so the target's `let` indent survives
        // instead of being replaced by the one-space diff-prefix artifact.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- #[allow(x)]\n- let x = E;\n+ #[allow(x)]\n+ let y = E;\n";
        let out = apply(patch, "fn f() {\n    #[allow(x)]\n    let x = 1;\n}\n").unwrap();
        assert_eq!(
            out, "fn f() {\n    #[allow(x)]\n    let y = 1;\n}\n",
            "got: {out:?}"
        );
        // `# attrs=explicit` with mirrored attrs applies the same way.
        let explicit = patch.replace("# stmt S", "# stmt S\n# attrs=explicit");
        let out2 = apply(&explicit, "fn f() {\n    #[allow(x)]\n    let x = 1;\n}\n").unwrap();
        assert_eq!(
            out2, "fn f() {\n    #[allow(x)]\n    let y = 1;\n}\n",
            "got: {out2:?}"
        );
        // Wrong attribute still fails instead of silently matching.
        let wrong = patch.replace("- #[allow(x)]", "- #[allow(y)]");
        assert!(apply(&wrong, "fn f() {\n    #[allow(x)]\n    let x = 1;\n}\n").is_err());
    }

    #[test]
    fn strip_with_mirrored_minus_drops_attrs() {
        // T154: `# attrs=strip` skips the mirror gate — mirrored minus
        // attrs are eligibility-ignored and the envelope is dropped at
        // rewrite, not rejected with a self-contradicting "use strip"
        // suggestion. Target indent survives like the preserve path.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# attrs=strip\n# stmt S\n# expr E\n- #[allow(x)]\n- let x = E;\n+ let y = E;\n";
        let out = apply(patch, "fn f() {\n    #[allow(x)]\n    let x = 1;\n}\n").unwrap();
        assert_eq!(out, "fn f() {\n    let y = 1;\n}\n", "got: {out:?}");
        // Eligibility ignores pattern attrs under strip (fn parity), so an
        // unattributed target still rewrites.
        let bare_file = patch.replace("@@ -1,4 +1,4 @@", "@@ -1,3 +1,3 @@");
        let out2 = apply(&bare_file, "fn f() {\n    let x = 1;\n}\n").unwrap();
        assert_eq!(out2, "fn f() {\n    let y = 1;\n}\n", "got: {out2:?}");
        // Writing attributes the minus side omitted stays rejected under
        // strip: adding attrs while stripping is ambiguous.
        let plus_only = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# attrs=strip\n# stmt S\n# expr E\n- let x = E;\n+ #[allow(x)]\n+ let y = E;\n";
        assert!(apply(plus_only, "fn f() {\n    let x = 1;\n}\n").is_err());
    }

    #[test]
    fn explicit_doc_comment_mirror_applies() {
        // T155: `///` doc lines normalize to `#[doc = "..."]` in
        // `split_leading_attrs`, so explicit doc-comment mirroring on `let`
        // compares like `#[...]` attrs and the original envelope survives.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# attrs=explicit\n# stmt S\n# expr E\n- /// Doc.\n- let x = E;\n+ /// Doc.\n+ let y = E;\n";
        let out = apply(patch, "fn f() {\n    /// Doc.\n    let x = 1;\n}\n").unwrap();
        assert_eq!(
            out, "fn f() {\n    /// Doc.\n    let y = 1;\n}\n",
            "got: {out:?}"
        );
        // Wrong doc text still fails instead of silently matching.
        let wrong = patch.replace("- /// Doc.", "- /// Other.");
        assert!(apply(&wrong, "fn f() {\n    /// Doc.\n    let x = 1;\n}\n").is_err());
    }

    #[test]
    fn explicit_single_line_block_doc_mirror_applies() {
        // T156: single-line `/** */` normalizes like `///`, so explicit
        // block-doc mirroring on `let` applies byte-exact.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# attrs=explicit\n# stmt S\n# expr E\n- /** Block doc. */\n- let x = E;\n+ /** Block doc. */\n+ let y = E;\n";
        let out = apply(
            patch,
            "fn f() {\n    /** Block doc. */\n    let x = 1;\n}\n",
        )
        .unwrap();
        assert_eq!(
            out, "fn f() {\n    /** Block doc. */\n    let y = 1;\n}\n",
            "got: {out:?}"
        );
        // Multi-line block docs fail fast with the needle (T156), not the
        // wrong-policy "requires none" message.
        let multi = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# attrs=explicit\n# stmt S\n# expr E\n- /** Multi\n- line. */\n- let x = E;\n+ /** Multi\n+ line. */\n+ let y = E;\n";
        let err = apply(multi, "fn f() {\n    let x = 1;\n}\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("block comments"), "unexpected: {err}");
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
    fn impl_trait_multisegment_bound_rewrites() {
        // T149: the realistic `impl std::fmt::Display` spelling (which
        // trips the item-rename bare-name gate) rewrites end to end.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=boxed\n# lang=rust\n- impl std::fmt::Display\n+ Box<dyn std::fmt::Display>\n";
        let out = apply(patch, "fn a(x: impl std::fmt::Display) {}\n").unwrap();
        assert!(out.contains("Box<dyn std::fmt::Display>"), "got: {out}");
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
    fn rewrites_return_expr() {
        // T418: `return ...` is an `Expr::Return` general-expr pattern
        // (b9c00d5 threads a param through two `return` call sites).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,6 +1,6 @@ rule=retthread\n# lang=rust\n# allow=many\n- return compress(section, used);\n+ return compress(section, extra, used);\n";
        let src = "fn a() {\n    if x {\n        return compress(section, used);\n    }\n    return compress(section, used);\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(
            out.matches("return compress(section, extra, used);")
                .count(),
            2,
            "got: {out}"
        );
        assert!(
            !out.contains("return compress(section, used);"),
            "got: {out}"
        );
    }

    #[test]
    fn rewrites_method_with_dotted_string_arg() {
        // T419: a `.` inside a string-literal arg is not a
        // receiver/method split (router fix in `classify_call_pattern`) —
        // `R.contains("a.b")` misrouted to a dotted-callee `Call` before.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=dotstr\n# lang=rust\n# allow=many\n# expr Receiver\n- Receiver.contains(\"a.b\")\n+ Receiver.found(\"a.b\")\n";
        let src = "fn m() {\n    let a = agents.contains(\"a.b\");\n    let b = readme.contains(\"a.b\");\n    let c = other.contains(\"zzz\");\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches(".found(\"a.b\")").count(), 2, "got: {out}");
        assert!(out.contains("other.contains(\"zzz\");"), "got: {out}");
    }

    #[test]
    fn rewrites_hetero_method_with_dotted_arg() {
        // WI-2 follow-up: `- E.push(F.clone())` (hetero receiver + dotted
        // arg) must bind both holes and substitute both sides. The call
        // router stays out (composite `__RSMPL_F.clone()` arg is concrete,
        // bypassing to the general matcher); the general matcher binds
        // `E->order`, `F->rel` and emits `rel.clone()`, never literal `F`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=hetpush\n# lang=rust\n# allow=many\n# expr E\n# expr F\n- E.push(F.clone())\n+ E.push2(F.clone())\n";
        let src = "fn m() {\n    order.push(rel.clone());\n    other.push(rel.clone());\n    third.push(other.clone());\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches(".push2(").count(), 3, "got: {out}");
        assert!(out.contains("order.push2(rel.clone());"), "got: {out}");
        assert!(out.contains("third.push2(other.clone());"), "got: {out}");
        assert!(!out.contains("F.clone()"), "unsubstituted metavar: {out}");
        // Same shape diet (inferred, no `# expr` lines) — the WI-2
        // follow-up's actual failure (`no match found` before the
        // `infer_method_dot` fix).
        let diet = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=hetpush\n# allow=many\n- E.push(F.clone())\n+ E.push2(F.clone())\n";
        let out = apply(diet, src).unwrap();
        assert_eq!(out.matches(".push2(").count(), 3, "diet got: {out}");
        assert!(!out.contains("F.clone()"), "diet unsubstituted: {out}");
    }

    #[test]
    fn rewrites_unary_not() {
        // T415: leading-bang patterns compile as general unary exprs
        // (router bypass in classify_call_pattern plus the macro
        // pre-route carve-out) - two sites rewrite, other untouched.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=notcheck\n# lang=rust\n# allow=many\n# expr E\n- !check(E)\n+ !checked(E)\n";
        let src = "fn a(x: bool) -> bool {\n    !check(x)\n}\nfn b(y: bool) -> bool {\n    !check(y)\n}\nfn c(z: bool) -> bool {\n    !other(z)\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches("!checked(").count(), 2, "got: {out}");
        assert!(out.contains("!other(z)"), "got: {out}");
    }

    #[test]
    fn rewrites_call_with_unary_arg() {
        // T428: a call-pattern metavar under `!` bypasses the call router
        // to the general matcher (a concrete `!E` arg could never bind) —
        // two sites rewrite, other callee/arity untouched.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@ rule=notarg\n# lang=rust\n# allow=many\n# expr E\n- foo(!E)\n+ bar(!E)\n";
        let src = "fn m() {\n    foo(!ready);\n    foo(!done);\n    baz(!ready);\n    foo(!ready, 1);\n}\n";
        let out = apply(patch, src).unwrap();
        assert_eq!(out.matches("bar(!").count(), 2, "got: {out}");
        assert!(out.contains("baz(!ready);"), "got: {out}");
        assert!(out.contains("foo(!ready, 1);"), "got: {out}");
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
    fn repeat_folds_nested_binary() {
        // T126: `# repeat` folds the nested match into the outer edit for
        // general-expression shapes. Without repeat this hunk would yield
        // `(a + 1) + 2`; the fold rewrites the `E = (a + 1)` binding first.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=bump\n# lang=rust\n# repeat\n# allow=many\n# expr E\n- E + 1\n+ E + 2\n";
        let src = "fn a() {\n    let v = (a + 1) + 1;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("(a + 2) + 2;"), "got: {out}");
    }

    #[test]
    fn repeat_without_nesting_is_single_rewrite() {
        // T126: no nesting means the fold is a no-op over the bindings.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=bump\n# lang=rust\n# repeat\n# expr E\n- E + 1\n+ E + 2\n";
        let src = "fn a() {\n    let v = a + 1;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let v = a + 2;"), "got: {out}");
    }

    #[test]
    fn repeat_many_folds_each_site() {
        // T126: `allow=many` sites each fold their own nesting.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,2 +2,2 @@ rule=bump\n# lang=rust\n# repeat\n# allow=many\n# expr E\n- E + 1\n+ E + 2\n";
        let src = "fn a() {\n    let x = (a + 1) + 1;\n    let y = b + 1;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let x = (a + 2) + 2;"), "got: {out}");
        assert!(out.contains("let y = b + 2;"), "got: {out}");
    }

    #[test]
    fn repeat_self_reproducing_terminates() {
        // T126: the produced output is never re-scanned, so
        // `E + 1` -> `(E + 1) + 1` applies exactly one level.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=dup\n# lang=rust\n# repeat\n# expr E\n- E + 1\n+ (E + 1) + 1\n";
        let src = "fn a() {\n    let v = a + 1;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let v = (a + 1) + 1;"), "got: {out}");
    }

    #[test]
    fn semi_terminated_expr_keeps_single_semicolon() {
        // T126 dogfood: `;`-terminated expression patterns doubled the
        // semicolon (`E + 1;` gave `...;;`) because the candidate span
        // excludes the statement's `;` while `had_semi` appended another.
        // The edit now consumes the original `;`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,2 +2,2 @@ rule=bump\n# lang=rust\n# repeat\n# allow=many\n# expr E\n- E + 1;\n+ E.wrapping_add(1);\n";
        let src = "fn a() {\n    let v = (a + 1) + 1;\n    let w = b + 1;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(
            out.contains("let v = (a.wrapping_add(1)).wrapping_add(1);"),
            "got: {out}"
        );
        assert!(out.contains("let w = b.wrapping_add(1);"), "got: {out}");
        assert!(!out.contains(";;"), "doubled semicolon: {out}");
    }

    #[test]
    fn unparenthesized_left_nesting_rewrites_outer_only() {
        // T143: `a + 1 + 1` parses as `(a + 1) + 1` with inner/outer
        // sharing a start byte; shortest-first ordering accepted the
        // inner and the outer tripped the partial-overlap guard.
        // Longest-first restores outermost-wins: one hunk, outer only.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=bump\n# lang=rust\n# allow=many\n# expr E\n- E + 1\n+ E + 2\n";
        let src = "fn a() {\n    let v = a + 1 + 1;\n    let _ = v;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let v = a + 1 + 2;"), "got: {out}");
    }

    #[test]
    fn unparenthesized_left_nesting_repeat_folds() {
        // T143: `# repeat` on the flat form folds like the parenthesized
        // form (`(a + 2) + 2` equivalent without parens).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=bump\n# lang=rust\n# repeat\n# allow=many\n# expr E\n- E + 1\n+ E + 2\n";
        let src = "fn a() {\n    let v = a + 1 + 1;\n    let _ = v;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("let v = a + 2 + 2;"), "got: {out}");
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
        const NO_MATCH: Option<&str> = Some("read `src/lib.rs` to list valid ranges");
        const ATTR_SUGGESTION: Option<&str> =
            Some("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`");
        let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        // WI-5a fires (SPEC 18.6 R1-block/if, R2, R4): each row applies
        // and the output contains the expected text.
        struct Fire {
            patch: &'static str,
            file: &'static str,
            contains: &'static str,
        }
        let fires = [
            // R1-block: the hole binds the block statement.
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- { E }\n+ { E + 1 }\n",
                file: "fn a() {\n    let v = { 1 };\n    let _ = v;\n}\n",
                contains: "{ 1 + 1 }",
            },
            // R1-if: the condition hole binds; the else branch rewrites.
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- if E { 1 } else { 2 }\n+ if E { 1 } else { 3 }\n",
                file: "fn a(x: bool) {\n    let v = if x { 1 } else { 2 };\n    let _ = v;\n}\n",
                contains: "else { 3 }",
            },
            // R2: `let-else` fires when both sides carry the diverge.
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E else { return 0 };\n+ let y = E else { return 0 };\n",
                file: "fn a(o: Option<i32>) -> i32 {\n    let x = o else { return 0 };\n    x\n}\n",
                contains: "let y = o else { return 0 };",
            },
            // R4: `P @ sub` binds `P` to the whole target pattern; the
            // plus side reuses `P` alone (fixed-point output).
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# pat P\n# expr E\n- let P @ Some(x) = E;\n+ let P = E;\n",
                file: "fn a(o: Option<i32>) {\n    let v @ Some(x) = o;\n    let _ = (v, x);\n}\n",
                contains: "let v @ Some(x) = o;",
            },
            // WI-5b R1-match: the scrutinee hole binds; one arm rewrites.
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- match E { Some(x) => 1, _ => 2 }\n+ match E { Some(x) => 1, _ => 3 }\n",
                file: "fn a(o: Option<i32>) {\n    let v = match o { Some(x) => 1, _ => 2 };\n    let _ = v;\n}\n",
                contains: "_ => 3",
            },
            // WI-5b R1-loop: body rewrites structurally.
            Fire {
                patch: "@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# root=expr\n- loop { break; }\n+ loop { continue; }\n",
                file: "fn a() {\n    loop { break; }\n}\n",
                contains: "continue;",
            },
            // WI-5b R1-while: the condition hole binds; body rewrites.
            Fire {
                patch: "@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- while E { break; }\n+ while E { continue; }\n",
                file: "fn a(x: bool) {\n    while x { break; }\n}\n",
                contains: "continue;",
            },
            // WI-5b R1-for: the bound pattern binds; the iterator hole binds.
            Fire {
                patch: "@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# root=expr\n# pat P\n# expr E\n- for P in E { break; }\n+ for P in E { continue; }\n",
                file: "fn a() {\n    for x in y { break; }\n}\n",
                contains: "continue;",
            },
            // WI-5b R1-closure: param and body holes bind.
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# pat P\n# expr E\n- |P| E\n+ |P| E + 1\n",
                file: "fn a() {\n    let f = |x| x;\n    let _ = f;\n}\n",
                contains: "|x| x + 1",
            },
            // WI-5b R1-let: `if let` scrutinee hole binds via the Let arm.
            Fire {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# root=expr\n# expr E\n- if let Some(x) = E { 1 } else { 2 }\n+ if let Some(x) = E { 1 } else { 3 }\n",
                file: "fn a(y: Option<i32>) {\n    let v = if let Some(x) = y { 1 } else { 2 };\n    let _ = v;\n}\n",
                contains: "else { 3 }",
            },
        ];
        for row in fires {
            let patch = format!("{head}{}", row.patch);
            let out = apply(&patch, row.file).unwrap();
            assert!(
                out.contains(row.contains),
                "SPEC 18.6 drift: want {:?} in:\n{out}",
                row.contains
            );
        }
        let rows = [
            // R2: mixed `let-else` (pattern without diverge vs target
            // with) still rejects — both sides must carry the diverge.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n",
                file: "fn a(o: Option<i32>) -> i32 {\n    let x = o else { return 0 };\n    x\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R4: `P @ sub` still rejects when the *initializer* fails
            // (here `E` binds the pattern `q` but must also equal `1`).
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# stmt S\n# pat P\n# expr E\n- let P @ E = E;\n+ let P @ E = E;\n",
                file: "fn a() {\n    let q = 1;\n    let _ = q;\n}\n",
                code: "RSMPL_MATCH_CONFLICT",
                reason: "no match found",
                suggestion: NO_MATCH,
            },
            // R5 remainder (T149 implements pointers/fn/impl/dyn): a
            // single-segment placeholder bound never matches a
            // multi-segment path (`dyn T` vs `dyn std::fmt::Display`).
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
            // C13: `# symbol` without `# fallback=syntax` (call-matcher gate; general shapes never reach it — C14 rejects them at compile).
            Row {
                patch: "@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# symbol Old=crate::old_api\n# expr E\n- Old(E)\n+ crate::new_api(E)\n",
                file: "fn a() {\n    crate::old_api(1);\n}\n",
                code: "RSMPL_SYMBOL_CONFLICT",
                reason: "symbol `Old` could not be resolved (no semantic environment in v0.1)",
                suggestion: Some("add `# fallback=syntax` to allow syntactic matching"),
            },
            // C14: `# symbol` in a general shape fails even with fallback
            // (call-shape-only, SPEC 18.6) — single symbol keeps the pin
            // deterministic regardless of gate iteration order.
            Row {
                patch: "@@ -1,4 +1,4 @@ rule=r\n# lang=rust\n# expr E\n# fallback=syntax\n# symbol Old=crate::old_api\n- E + 1\n+ E + 2\n",
                file: "fn a() {\n    let v = y + 1;\n    let _ = v;\n}\n",
                code: "RSMPL_PATTERN_CONFLICT",
                reason: "only supported in call shapes",
                suggestion: Some("use a call-shape hunk or a concrete path pattern"),
            },
        ];
        assert_eq!(
            rows.len(),
            8,
            "SPEC 18.6 lists R2-mixed,R4-veto,R5 + C10-C14 runtime rows (R3 closed by T144; WI-5a moved R1-block/if,R2-fire,R4-fire and WI-5b moved R1-match/loops/closures/let to pins above)"
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
        // Concrete generic arguments compare structurally (SPEC 18.6).
        // A fully concrete `Vec<u8>` rewrite must succeed.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n- Vec<u8>\n+ Box<u8>\n";
        let out = apply(patch, "fn a(x: Vec<u8>) {}\n").unwrap();
        assert!(out.contains("Box<u8>"), "got: {out}");
        assert!(!out.contains("Vec<u8>"), "got: {out}");
    }

    #[test]
    fn placeholder_segment_with_args_matches_structurally() {
        // T144 (closes SPEC 18.6 R3): `P<u8>` binds `P=Vec`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# type P\n- P<u8>\n+ Box<u8>\n";
        let out = apply(patch, "fn a(x: Vec<u8>) {}\n").unwrap();
        assert!(out.contains("Box<u8>"), "got: {out}");
    }

    #[test]
    fn placeholder_generic_arg_matches_concrete() {
        // T144: `Vec<T>` with `# type T` matches `Vec<u8>`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# type T\n- Vec<T>\n+ Box<T>\n";
        let out = apply(patch, "fn a(x: Vec<u8>) {}\n").unwrap();
        assert!(out.contains("Box<u8>"), "got: {out}");
    }

    #[test]
    fn method_turbofish_placeholder_matches() {
        // T144: `collect::<T>` binds `T=u8` structurally. The binary
        // wrapper keeps the shape in the general matcher (a bare
        // `E.collect::<T>()` routes to the call matcher, which stays
        // text-based in v0.1).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# lang=rust\n# expr E\n# type T\n- E.collect::<T>() + 1\n+ E.collect::<Vec<T>>() + 1\n";
        let file =
            "fn a(v: Vec<u8>) {\n    let x = v.iter().collect::<u8>() + 1;\n    let _ = x;\n}\n";
        let out = apply(patch, file).unwrap();
        assert!(out.contains("collect::<Vec<u8>>() + 1"), "got: {out}");
    }

    #[test]
    fn turbofish_mismatch_still_rejects() {
        // Arity / `::` mismatches still fail safe (no guessing).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# type T\n- Vec<T, T>\n+ Box<T>\n";
        let err = apply(patch, "fn a(x: Vec<u8>) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        // Parenthesized path args still reject via rendered-text comparison
        // (SPEC 18.6 R3 exception): `P(T)` never equals angle-bracketed `Vec<u8>`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=r\n# lang=rust\n# type P\n# type T\n- P(T)\n+ Box<T>\n";
        let err = apply(patch, "fn a(x: Vec<u8>) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn raw_pointer_constness_is_structural() {
        // T149: `*const T` binds `T=u8` and substitutes; `*mut` stays.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=ptr\n# lang=rust\n# type T\n- *const T\n+ *mut T\n";
        let out = apply(patch, "fn a(x: *const u8) {}\n").unwrap();
        assert!(out.contains("*mut u8"), "got: {out}");
        assert!(!out.contains("*const u8"), "got: {out}");
    }

    #[test]
    fn raw_pointer_constness_mismatch_rejects() {
        // T149: `*const u8` never matches a `*mut u8` target (fail safe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=ptr\n# lang=rust\n- *const u8\n+ *mut u8\n";
        let err = apply(patch, "fn a(x: *mut u8) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn impl_trait_rewrites_structurally() {
        // T149: `impl Display` matches and rewrites; a longer bound list
        // on the target rejects (fail safe).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=boxed\n# lang=rust\n- impl Display\n+ Box<dyn Display>\n";
        let out = apply(patch, "fn a(x: impl Display) {}\n").unwrap();
        assert!(out.contains("Box<dyn Display>"), "got: {out}");
        let err = apply(patch, "fn a(x: impl Display + Send) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn dyn_trait_binds_through_placeholder() {
        // T149: `Box<dyn T>` binds `T=Display`; the `+ Send` tail is
        // literal and preserved.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=dynsend\n# lang=rust\n# type T\n- Box<dyn T>\n+ Box<dyn T + Send>\n";
        let out = apply(patch, "fn a(x: Box<dyn Display>) {}\n").unwrap();
        assert!(out.contains("Box<dyn Display + Send>"), "got: {out}");
    }

    #[test]
    fn dyn_trait_requires_dyn_on_both_sides() {
        // T149: `dyn Display` never matches a bare `Display` (different
        // variants), and `impl Display` never matches `dyn Display`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=d\n# lang=rust\n- dyn Display\n+ dyn Send\n";
        let err = apply(patch, "fn a(x: Display) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=d\n# lang=rust\n- impl Display\n+ Box<dyn Display>\n";
        let err = apply(patch, "fn a(x: dyn Display) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn fn_pointer_matches_and_binds_repeated_metavar() {
        // T149: `fn(T) -> T` binds `T=u8` twice with equality; a
        // heterogeneous `fn(u8) -> u16` target rejects.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=cb\n# lang=rust\n# type T\n- fn(T) -> T\n+ fn(T) -> Box<T>\n";
        let out = apply(patch, "fn a(cb: fn(u8) -> u8) {}\n").unwrap();
        assert!(out.contains("fn(u8) -> Box<u8>"), "got: {out}");
        let err = apply(patch, "fn a(cb: fn(u8) -> u16) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn fn_pointer_unsafety_mismatch_rejects() {
        // T149: a plain `fn(i32)` pattern never matches `unsafe fn(i32)`.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=cb\n# lang=rust\n- fn(i32)\n+ fn(u8)\n";
        let err = apply(patch, "fn a(cb: unsafe fn(i32)) {}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }
}
