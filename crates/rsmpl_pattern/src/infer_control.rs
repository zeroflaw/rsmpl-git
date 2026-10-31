//! WI-5b control-flow kind assignment for diet inference.
//!
//! Split from `infer` per D5 (the parent file sits near the 2000-line
//! gate). Owns the AST-aware position assignment for control-flow
//! shapes (`for`/`match`/`while`/`loop`/closure/`let`): pattern slots
//! infer `pat`, type slots `type`, everything else `expr`. Whole
//! placeholders and WI-3 structural composites infer by position;
//! `Or`/macro interiors stay literal, mirroring `check_let_pat`.

use std::collections::{HashMap, HashSet};

use rsmpl_patch::MetavarKind;

use super::infer::{add_kind, is_candidate, names_in, split_top_colon};

/// WI-23: diet fallback for dotted expression lines on the
/// `has_call_shape` path (`E0.timeout`, `E0.a + 1`, `&E0.f`). Those
/// lines carry no paren groups, so the group scan yields nothing and
/// the base hole stays literal. Parse the line as an expression and
/// walk it positionally (field bases → `expr`, lowercase members
/// ignored as non-candidates) — the same discipline as the call
/// pass, covering every WI-6 variant. `$`-stripped for sigiled mode
/// (`$E0.timeout` never parses as Rust). Returns true when the line
/// was handled (the caller skips the group scan); qself-qualified
/// lines (`<T as Tr>::f(x)`) return false and keep the legacy scan —
/// v1 has no holes inside `qself` (the engine compares the written
/// type token-equal), so inferring `T` there would break the
/// previously-firing qualified call sweep (fixture 94).
pub(crate) fn try_assign_dotted_expr(
    line: &str,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
) -> bool {
    let stripped: String = line.split('$').collect();
    let Ok(e) = syn::parse_str::<syn::Expr>(stripped.trim()) else {
        return false;
    };
    if expr_has_qself(&e) {
        return false;
    }
    assign_expr_ast(&e, kinds);
    true
}

/// True when the expression carries a `QSelf` qualifier anywhere
/// (`<T as Tr>::f(x)`). v1 has no holes inside `qself`, so the
/// WI-23 walk must not infer there.
fn expr_has_qself(e: &syn::Expr) -> bool {
    struct Probe(bool);
    impl<'ast> syn::visit::Visit<'ast> for Probe {
        fn visit_qself(&mut self, _: &'ast syn::QSelf) {
            self.0 = true;
        }
    }
    let mut p = Probe(false);
    syn::visit::visit_expr(&mut p, e);
    p.0
}

/// WI-5b: AST-aware kind assignment for control-flow shapes.
///
/// Text-level inference assigns every name on a `for P in E` or `|P| E`
/// line as `expr`, contradicting an explicit `# pat P`. These helpers
/// parse the fragment with `syn` and assign by position: pattern slots
/// (match arms, for-loop pat, closure params, `let` pat, `@` heads/subs,
/// WI-3 composite leaves) → `pat`; type slots → `type`; everything else
/// → `expr`. `Or` arms and macro interiors stay literal, mirroring
/// `check_let_pat`.
pub(crate) fn assign_expr_text(text: &str, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    match syn::parse_str::<syn::Expr>(text.trim()) {
        Ok(e) => assign_expr_ast(&e, kinds),
        Err(_) => {
            for n in names_in(text, kinds) {
                add_kind(kinds, &n, MetavarKind::Expr);
            }
        }
    }
}

fn assign_pat_text(text: &str, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    // `syn::Pat` has no `Parse` impl — probe through a `let` statement
    // (WI-3 composites included) before the textual shapes below.
    let t = text.trim();
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(&format!("let {t} = _;")) {
        assign_pat_ast(&local.pat, kinds);
        return;
    }
    // Textual fallback for the v1-admitted shapes (`P`, `P @ sub`,
    // `P: T`), mirroring `check_let_pat`.
    let (pat_part, ty_part) = split_top_colon(t);
    if let Some(ty) = ty_part {
        let p = pat_part.trim();
        if is_single_ident(p) {
            add_kind(kinds, p.trim_start_matches('$'), MetavarKind::Pat);
        } else if let Some((head, sub)) = split_at_pat(p) {
            if is_single_ident(head.trim()) {
                add_kind(kinds, head.trim().trim_start_matches('$'), MetavarKind::Pat);
                assign_pat_text(sub, kinds);
            }
        }
        assign_type_text(&ty, kinds);
        return;
    }
    if let Some((head, sub)) = split_at_pat(t) {
        if is_single_ident(head.trim()) {
            add_kind(kinds, head.trim().trim_start_matches('$'), MetavarKind::Pat);
            assign_pat_text(sub, kinds);
        }
        return;
    }
    if is_single_ident(t) {
        add_kind(kinds, t.trim_start_matches('$'), MetavarKind::Pat);
    }
}

fn is_single_ident(t: &str) -> bool {
    let s = t.trim().trim_start_matches('$');
    !s.is_empty()
        && !s.contains(char::is_whitespace)
        && !s.contains([
            '(', ')', '{', '}', '[', ']', ',', '|', '@', ':', ';', '!', '"',
        ])
        && is_candidate(s)
}

/// Split `P @ sub` at the `@` (first one outside strings).
fn split_at_pat(t: &str) -> Option<(&str, &str)> {
    let mut in_str = false;
    let mut esc = false;
    for (i, b) in t.bytes().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'@' {
            return Some((t[..i].trim(), t[i + 1..].trim()));
        }
    }
    None
}

fn assign_type_text(text: &str, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    if let Ok(t) = syn::parse_str::<syn::Type>(text.trim()) {
        assign_type_ast(&t, kinds);
    }
}

/// True for the WI-5 shapes (plus `if`/`block`, which the text fallback
/// already gets right but the AST walk covers identically).
pub(crate) fn is_control_expr(e: &syn::Expr) -> bool {
    matches!(
        e,
        syn::Expr::If(_)
            | syn::Expr::Block(_)
            | syn::Expr::Match(_)
            | syn::Expr::Loop(_)
            | syn::Expr::While(_)
            | syn::Expr::ForLoop(_)
            | syn::Expr::Closure(_)
            | syn::Expr::Let(_)
            // WI-6 slice 1: block/value shapes the engine matches
            // structurally now (holes inside infer by position).
            | syn::Expr::Async(_)
            | syn::Expr::TryBlock(_)
            | syn::Expr::Unsafe(_)
            | syn::Expr::Yield(_)
    )
}

/// Non-mutating predicate for passes that must defer to the control
/// reader (e.g. the call-slot pass skips these lines so paren-group
/// names inside patterns are never claimed as `expr`).
pub(crate) fn is_control_line(t: &str) -> bool {
    let u = t.trim().trim_end_matches(';').trim();
    if u.is_empty() {
        return false;
    }
    if let Ok(syn::Stmt::Expr(e, _)) = syn::parse_str::<syn::Stmt>(u) {
        if is_control_expr(&e) {
            return true;
        }
    }
    if let Ok(e) = syn::parse_str::<syn::Expr>(u) {
        if is_control_expr(&e) {
            return true;
        }
    }
    if u.starts_with("for ") || u.starts_with("for(") {
        return true;
    }
    // Closure header present (`|params|` with a body after).
    if let Some(start) = u.find('|') {
        if u[start + 1..].find('|').is_some() {
            return true;
        }
    }
    false
}

/// Try a control-flow reading of one fallback line. Returns true when
/// the line was handled (AST walk or `for`/closure text split), so the
/// caller skips the text fallback that would claim `P` as `expr`.
pub(crate) fn assign_control_line(
    line: &str,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
) -> bool {
    let t = line.trim().trim_end_matches(';').trim();
    if t.is_empty() {
        return false;
    }
    if let Ok(syn::Stmt::Expr(e, _)) = syn::parse_str::<syn::Stmt>(t) {
        if is_control_expr(&e) {
            assign_expr_ast(&e, kinds);
            return true;
        }
    }
    if let Ok(e) = syn::parse_str::<syn::Expr>(t) {
        if is_control_expr(&e) {
            assign_expr_ast(&e, kinds);
            return true;
        }
    }
    // Backup text splits when the full parse fails (multi-arm lines the
    // one-line `syn` parse rejects still carry a recognizable header).
    if let Some(rest) = t.strip_prefix("for ").or_else(|| t.strip_prefix("for(")) {
        let rest = rest.strip_prefix('(').unwrap_or(rest);
        if let Some(pos) = rest.find(" in ") {
            let (pat, after) = rest.split_at(pos);
            let after = &after[" in ".len()..];
            let iter = after.split('{').next().unwrap_or(after);
            assign_pat_text(pat, kinds);
            assign_expr_text(iter, kinds);
            if let Some(body) = after.find('{').map(|i| &after[i..]) {
                assign_expr_text(body, kinds);
            }
            return true;
        }
    }
    if t.contains('|') {
        // Closure `|params| body` (possibly `async`/`move`-led): params
        // split by `,`; `P: T` parts infer `pat` + `type`.
        let u = t
            .trim_start_matches("async ")
            .trim_start_matches("move ")
            .trim();
        if let Some(start) = u.find('|') {
            if let Some(end) = u[start + 1..].find('|').map(|i| i + start + 1) {
                let params = &u[start + 1..end];
                for p in params.split(',') {
                    let p = p.trim();
                    if p.is_empty() {
                        continue;
                    }
                    if let Some((l, r)) = p.split_once(':') {
                        if l.trim().contains("::") || r.trim().contains("::") {
                            continue;
                        }
                        assign_pat_text(l.trim(), kinds);
                        assign_type_text(r.trim(), kinds);
                    } else {
                        assign_pat_text(p, kinds);
                    }
                }
                assign_expr_text(u[end + 1..].trim(), kinds);
                return true;
            }
        }
    }
    false
}

pub(crate) fn assign_pat_ast(pat: &syn::Pat, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    match pat {
        syn::Pat::Ident(pid) => {
            add_kind(
                kinds,
                pid.ident.to_string().trim_start_matches('$'),
                MetavarKind::Pat,
            );
            if let Some((_, sub)) = &pid.subpat {
                assign_pat_ast(sub, kinds);
            }
        }
        syn::Pat::Type(pt) => {
            assign_pat_ast(&pt.pat, kinds);
            assign_type_ast(&pt.ty, kinds);
        }
        // WI-3 structural composites: bare idents in bindable leaf
        // positions infer `pat` (mirrors the engine's `match_pat`
        // arms + the C8 `pat_holes_ok` gate). `Or` stays literal
        // (see below).
        syn::Pat::TupleStruct(ps) => {
            for e in &ps.elems {
                assign_pat_ast(e, kinds);
            }
        }
        syn::Pat::Tuple(pt) => {
            for e in &pt.elems {
                assign_pat_ast(e, kinds);
            }
        }
        syn::Pat::Struct(ps) => {
            for f in &ps.fields {
                assign_pat_ast(&f.pat, kinds);
            }
        }
        syn::Pat::Paren(pp) => assign_pat_ast(&pp.pat, kinds),
        syn::Pat::Reference(pr) => assign_pat_ast(&pr.pat, kinds),
        syn::Pat::Slice(ps) => {
            for e in &ps.elems {
                assign_pat_ast(e, kinds);
            }
        }
        // `Or` stays literal: the engine treats `Or` as concrete and
        // the C8 gate rejects holes inside it. Walking arms would
        // claim concrete variants (`None` in `Some(P) | None`) as
        // `pat` while the call pass claims them as `expr`,
        // producing a spurious incompatible-positions conflict
        // instead of the C8 needle.
        syn::Pat::Or(_) => {}
        // Remaining composites (`Const`, `Macro`, ...) stay literal:
        // v1 admits no holes there (C8/C19).
        _ => {}
    }
}

pub(crate) fn assign_type_ast(ty: &syn::Type, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    match ty {
        syn::Type::Path(tp) if tp.qself.is_none() && tp.path.segments.len() == 1 => {
            let seg = &tp.path.segments[0];
            let id = seg.ident.to_string();
            // Bare `T` infers `type`; `T<...>` infers the head too (the
            // engine binds single-segment heads with args structurally).
            if kinds.contains_key(id.trim_start_matches('$')) {
                add_kind(kinds, id.trim_start_matches('$'), MetavarKind::Type);
            }
            if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                assign_angle_args(ab, kinds);
            }
            // WI-6c-diet: parenthesized `Fn(T) -> U` inputs/output infer
            // `type` too (the engine binds them structurally); without
            // this the call pass claims `T` as `expr` and diet fails
            // with an incompatible-positions conflict.
            if let syn::PathArguments::Parenthesized(p) = &seg.arguments {
                assign_paren_args(p, kinds);
            }
        }
        syn::Type::Path(tp) => {
            for seg in &tp.path.segments {
                if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                    assign_angle_args(ab, kinds);
                }
                if let syn::PathArguments::Parenthesized(p) = &seg.arguments {
                    assign_paren_args(p, kinds);
                }
            }
        }
        syn::Type::Reference(tr) => assign_type_ast(&tr.elem, kinds),
        syn::Type::Slice(ts) => assign_type_ast(&ts.elem, kinds),
        syn::Type::Array(ta) => assign_type_ast(&ta.elem, kinds),
        syn::Type::Tuple(tt) => {
            for t in &tt.elems {
                assign_type_ast(t, kinds);
            }
        }
        syn::Type::Paren(tp) => assign_type_ast(&tp.elem, kinds),
        syn::Type::Group(tg) => assign_type_ast(&tg.elem, kinds),
        syn::Type::Ptr(tp) => assign_type_ast(&tp.elem, kinds),
        syn::Type::BareFn(bf) => {
            for i in &bf.inputs {
                assign_type_ast(&i.ty, kinds);
            }
            if let syn::ReturnType::Type(_, t) = &bf.output {
                assign_type_ast(t, kinds);
            }
        }
        syn::Type::ImplTrait(ti) => {
            for b in &ti.bounds {
                assign_type_param_bound(b, kinds);
            }
        }
        syn::Type::TraitObject(to) => {
            for b in &to.bounds {
                assign_type_param_bound(b, kinds);
            }
        }
        _ => {}
    }
}

fn assign_type_param_bound(
    b: &syn::TypeParamBound,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
) {
    if let syn::TypeParamBound::Trait(tb) = b {
        if tb.path.segments.len() == 1 && tb.path.segments[0].arguments.is_none() {
            let id = tb.path.segments[0].ident.to_string();
            if kinds.contains_key(id.trim_start_matches('$')) {
                add_kind(kinds, id.trim_start_matches('$'), MetavarKind::Type);
            }
        } else {
            for seg in &tb.path.segments {
                if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                    assign_angle_args(ab, kinds);
                }
                if let syn::PathArguments::Parenthesized(p) = &seg.arguments {
                    assign_paren_args(p, kinds);
                }
            }
        }
    }
}

/// Parenthesized `Fn(T) -> U` arguments: inputs and output infer `type`
/// (WI-6c-diet). Mirrors the engine's `match_path_args` structural arm.
fn assign_paren_args(
    p: &syn::ParenthesizedGenericArguments,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
) {
    for t in &p.inputs {
        assign_type_ast(t, kinds);
    }
    if let syn::ReturnType::Type(_, t) = &p.output {
        assign_type_ast(t, kinds);
    }
}

fn assign_angle_args(
    ab: &syn::AngleBracketedGenericArguments,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
) {
    for arg in &ab.args {
        match arg {
            syn::GenericArgument::Type(t) => assign_type_ast(t, kinds),
            syn::GenericArgument::Const(e) => assign_expr_ast(e, kinds),
            _ => {}
        }
    }
}

fn assign_path_expr(path: &syn::Path, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    if path.segments.len() == 1 {
        let seg = &path.segments[0];
        add_kind(
            kinds,
            seg.ident.to_string().trim_start_matches('$'),
            MetavarKind::Expr,
        );
        if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
            assign_angle_args(ab, kinds);
        }
    } else {
        for seg in &path.segments {
            if let syn::PathArguments::AngleBracketed(ab) = &seg.arguments {
                assign_angle_args(ab, kinds);
            }
        }
    }
}

fn assign_block(block: &syn::Block, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    for stmt in &block.stmts {
        match stmt {
            syn::Stmt::Local(l) => {
                assign_pat_ast(&l.pat, kinds);
                if let Some(init) = &l.init {
                    assign_expr_ast(&init.expr, kinds);
                    if let Some((_, div)) = &init.diverge {
                        assign_expr_ast(div, kinds);
                    }
                }
            }
            syn::Stmt::Expr(e, _) => assign_expr_ast(e, kinds),
            // WI-6b: holes may live inside an inner `fn` body (the
            // engine matches it structurally); signatures stay strict
            // and macros stay opaque, so neither assigns kinds here.
            syn::Stmt::Item(it) => {
                if let syn::Item::Fn(f) = it {
                    assign_block(&f.block, kinds);
                }
            }
            syn::Stmt::Macro(_) => {}
        }
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn assign_expr_ast(e: &syn::Expr, kinds: &mut HashMap<String, HashSet<MetavarKind>>) {
    match e {
        syn::Expr::Path(ep) => {
            if let Some(q) = &ep.qself {
                assign_type_ast(&q.ty, kinds);
            }
            if ep.path.segments.len() == 1
                && matches!(ep.path.segments[0].arguments, syn::PathArguments::None)
            {
                add_kind(
                    kinds,
                    ep.path.segments[0]
                        .ident
                        .to_string()
                        .trim_start_matches('$'),
                    MetavarKind::Expr,
                );
            } else {
                assign_path_expr(&ep.path, kinds);
            }
        }
        syn::Expr::Lit(_) => {}
        syn::Expr::Binary(eb) => {
            assign_expr_ast(&eb.left, kinds);
            assign_expr_ast(&eb.right, kinds);
        }
        syn::Expr::Unary(eu) => assign_expr_ast(&eu.expr, kinds),
        syn::Expr::Cast(ec) => {
            assign_expr_ast(&ec.expr, kinds);
            assign_type_ast(&ec.ty, kinds);
        }
        syn::Expr::Reference(er) => assign_expr_ast(&er.expr, kinds),
        syn::Expr::Paren(ep) => assign_expr_ast(&ep.expr, kinds),
        syn::Expr::Group(eg) => assign_expr_ast(&eg.expr, kinds),
        syn::Expr::Call(ec) => {
            assign_expr_ast(&ec.func, kinds);
            for a in &ec.args {
                assign_expr_ast(a, kinds);
            }
        }
        syn::Expr::MethodCall(em) => {
            assign_expr_ast(&em.receiver, kinds);
            let m = em.method.to_string();
            add_kind(kinds, m.trim_start_matches('$'), MetavarKind::Expr);
            if let Some(ab) = em.turbofish.as_ref() {
                assign_angle_args(ab, kinds);
            }
            for a in &em.args {
                assign_expr_ast(a, kinds);
            }
        }
        syn::Expr::Assign(ea) => {
            assign_expr_ast(&ea.left, kinds);
            assign_expr_ast(&ea.right, kinds);
        }
        syn::Expr::Field(ef) => {
            assign_expr_ast(&ef.base, kinds);
            if let syn::Member::Named(id) = &ef.member {
                add_kind(
                    kinds,
                    id.to_string().trim_start_matches('$'),
                    MetavarKind::Expr,
                );
            }
        }
        syn::Expr::Index(ei) => {
            assign_expr_ast(&ei.expr, kinds);
            assign_expr_ast(&ei.index, kinds);
        }
        syn::Expr::Array(ea) => {
            for x in &ea.elems {
                assign_expr_ast(x, kinds);
            }
        }
        syn::Expr::Tuple(et) => {
            for x in &et.elems {
                assign_expr_ast(x, kinds);
            }
        }
        syn::Expr::Struct(es) => {
            assign_path_expr(&es.path, kinds);
            for f in &es.fields {
                assign_expr_ast(&f.expr, kinds);
            }
        }
        syn::Expr::Range(er) => {
            if let Some(s) = &er.start {
                assign_expr_ast(s, kinds);
            }
            if let Some(x) = &er.end {
                assign_expr_ast(x, kinds);
            }
        }
        syn::Expr::Await(ea) => assign_expr_ast(&ea.base, kinds),
        syn::Expr::Try(et) => assign_expr_ast(&et.expr, kinds),
        syn::Expr::Return(er) => {
            if let Some(x) = &er.expr {
                assign_expr_ast(x, kinds);
            }
        }
        syn::Expr::If(ei) => {
            assign_expr_ast(&ei.cond, kinds);
            assign_block(&ei.then_branch, kinds);
            if let Some((_, b)) = &ei.else_branch {
                assign_expr_ast(b, kinds);
            }
        }
        syn::Expr::Block(eb) => {
            assign_block(&eb.block, kinds);
        }
        syn::Expr::Match(em) => {
            assign_expr_ast(&em.expr, kinds);
            for arm in &em.arms {
                assign_pat_ast(&arm.pat, kinds);
                if let Some((_, g)) = &arm.guard {
                    assign_expr_ast(g, kinds);
                }
                assign_expr_ast(&arm.body, kinds);
            }
        }
        syn::Expr::Loop(el) => assign_block(&el.body, kinds),
        syn::Expr::While(ew) => {
            assign_expr_ast(&ew.cond, kinds);
            assign_block(&ew.body, kinds);
        }
        syn::Expr::ForLoop(ef) => {
            assign_pat_ast(&ef.pat, kinds);
            assign_expr_ast(&ef.expr, kinds);
            assign_block(&ef.body, kinds);
        }
        syn::Expr::Closure(ec) => {
            for p in &ec.inputs {
                assign_pat_ast(p, kinds);
            }
            if let syn::ReturnType::Type(_, t) = &ec.output {
                assign_type_ast(t, kinds);
            }
            assign_expr_ast(&ec.body, kinds);
        }
        syn::Expr::Let(el) => {
            assign_pat_ast(&el.pat, kinds);
            assign_expr_ast(&el.expr, kinds);
        }
        // WI-6 slice 1: bodies/values recurse like any other block shape.
        syn::Expr::Async(ea) => assign_block(&ea.block, kinds),
        syn::Expr::TryBlock(et) => assign_block(&et.block, kinds),
        syn::Expr::Unsafe(eu) => assign_block(&eu.block, kinds),
        syn::Expr::Yield(ey) => {
            if let Some(x) = &ey.expr {
                assign_expr_ast(x, kinds);
            }
        }
        // Macro bodies and const blocks stay literal: v1 has no
        // interior abstraction there.
        _ => {}
    }
}
