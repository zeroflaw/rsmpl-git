//! WI-18 slice 8 call/general inference pass (split from
//! `infer.rs` per D5 — the parent file sits over the 1600-line gate).

use std::collections::{HashMap, HashSet};

use rsmpl_patch::{MetavarKind, StructuredError};

use crate::metavar::MetavarTable;

use super::infer::{
    add_kind, fn_like, is_candidate, names_in, names_in_paren_groups, names_in_type_args,
    occurs_in, rfind_dot,
};
use super::infer_control::{
    assign_control_line, assign_expr_ast, assign_type_ast, is_control_expr, is_control_line,
};

/// Call/method/macro argument + receiver + callee positions → `expr`.
///
/// A callee explicitly declared as `# type` (`P(T)` tuple-struct type
/// patterns) is type-owned: the whole line is left to the type fallback
/// instead of forcing an `expr` disagreement.
pub(crate) fn assign_call_slots(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
    base: &MetavarTable,
) -> Result<(), StructuredError> {
    // WI-11b: member-context pair lines infer in the member pass, not
    // as calls — but only when a struct/enum container header is present
    // (else `foo(E)` stays a call; the WI-5 pins).
    let member_mode = super::infer_def::has_member_container(masked_minus)
        || super::infer_def::has_member_container(masked_plus);
    for (side, text) in [masked_minus, masked_plus].into_iter().enumerate() {
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with("let ") || t.starts_with("fn ") {
                continue;
            }
            if member_mode && super::infer_def::is_member_pair_line(t) {
                continue;
            }
            // Control-flow lines (`match`/`for`/closures/`if`/`while`)
            // are AST-owned: the control pass assigns `pat` slots inside
            // patterns and `expr` elsewhere. Claiming paren-group names
            // here as `expr` would tag pattern holes (e.g. `P` in
            // `match E { Some(P) => .. }`) as `expr`, conflicting with
            // the `pat` kind (spurious incompatible-positions instead
            // of the C8 needle for `Or`-with-hole).
            if is_control_line(t) {
                continue;
            }
            // Method split at the receiver/method dot: depth-aware (same
            // family as the WI-2 router fix). `rfind_dot` ignored paren
            // depth, so `E.push(F.clone())` split at the inner `F.clone`
            // dot (recv `E.push(F`, empty args) and `E` never inferred.
            // Only dots at paren-depth 0 (outside strings) split; the
            // last such dot with a `(` after it wins (WI-2 follow-up).
            if let Some(dot) = infer_method_dot(t) {
                let recv = t[..dot].trim();
                let after = t[dot + 1..].trim();
                if let Some(paren) = after.find('(') {
                    let method = after[..paren].trim();
                    // Strip turbofish `<...>` from method for the check.
                    let method_base = method.split('<').next().unwrap_or(method).trim();
                    if is_candidate(method_base) && kinds.contains_key(method_base) {
                        add_kind(kinds, method_base, MetavarKind::Expr);
                    }
                    // Turbofish args inside `method::<T>`: type holes.
                    // Only the inner args infer (`Vec` in `::<Vec<T>>`
                    // stays literal — same constructor rule as the
                    // general fallback).
                    if let Some((_, after)) = method.split_once('<') {
                        for n in names_in_type_args(after, kinds) {
                            add_kind(kinds, &n, MetavarKind::Type);
                        }
                    }
                    if recv == recv.trim() && !recv.is_empty() {
                        let r = recv.trim_start_matches('&').trim();
                        if is_candidate(r)
                            && kinds.contains_key(r)
                            && !r.contains(char::is_whitespace)
                        {
                            add_kind(kinds, r, MetavarKind::Expr);
                        }
                        for n in names_in(recv, kinds) {
                            // Composite receivers (e.g. `E.foo`) still bind
                            // whole-slot `E` as `expr`; deeper handling is
                            // the general pass's job (same kind).
                            let _ = n;
                        }
                    }
                    // Args inside `(...)`: each bare candidate arg is an
                    // `expr` hole (`...` never matches `is_candidate`).
                    if let Some(args) = balanced_args(&format!("({}", &after[paren + 1..])) {
                        for n in names_in(&args, kinds) {
                            add_kind(kinds, &n, MetavarKind::Expr);
                        }
                    }
                    continue;
                }
            }
            // Bracket/brace macro invocations (`name![args]`,
            // `name!{args}`): same shared-arg `expr` rule as paren
            // macros (WI-7a; the paren path below never sees these
            // lines since there is no `(` to split on).
            if let Some(args) = crate::macro_bracket_args(t) {
                let other = if side == 0 { masked_plus } else { masked_minus };
                for n in names_in(&args, kinds) {
                    // Declared non-`expr` names keep their kind (same
                    // agreement-error posture as the paren macro arm).
                    if base.kind_of(&n).is_some_and(|k| k != MetavarKind::Expr) {
                        continue;
                    }
                    if occurs_in(other, &n) {
                        add_kind(kinds, &n, MetavarKind::Expr);
                    }
                }
                continue;
            }
            // Plain call `callee(args)` / macro `name!(args)`.
            if let Some(paren) = t.find('(') {
                let callee = t[..paren].trim();
                // Skip `if`/`match`/`for`/`while`/`return` keywords.
                let base_callee = callee.trim_end_matches('!').trim();
                if [
                    "if", "match", "for", "while", "loop", "return", "else", "let",
                ]
                .contains(&base_callee)
                {
                    continue;
                }
                // Type-owned lines (`P(T)` tuple-struct types with `# type P`):
                // the callee is a type constructor, not a call — leave the
                // whole line to explicit declarations (no `expr` inference).
                {
                    let last = base_callee
                        .rsplit("::")
                        .next()
                        .unwrap_or(base_callee)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if matches!(
                        base.kind_of(last),
                        Some(
                            MetavarKind::Type
                                | MetavarKind::Pat
                                | MetavarKind::Item
                                | MetavarKind::GenericParams
                                | MetavarKind::WhereClause
                                | MetavarKind::Visibility
                                | MetavarKind::Qualifier
                                | MetavarKind::QualifierList
                        )
                    ) {
                        continue;
                    }
                }
                // Callee last segment candidate → `expr` (covers `E(...)`
                // and `crate::E(...)`; concrete paths yield no candidates).
                // A turbofish callee (`id::<T>`) keeps its type args as
                // `type` holes. A line that parses as a `syn::Type`
                // (`P(T)` tuple-struct type) leaves its callee to the type
                // pass — it is a type constructor, not a call.
                if syn::parse_str::<syn::Type>(t).is_ok() {
                    // Type-owned: skip callee `expr` (the type fallback
                    // assigns `type` to preserved names only).
                } else if let Some((callee_base, turbo_inner)) = split_turbofish(base_callee) {
                    for n in names_in_type_args(turbo_inner, kinds) {
                        add_kind(kinds, &n, MetavarKind::Type);
                    }
                    let last = callee_base
                        .rsplit("::")
                        .next()
                        .unwrap_or(callee_base)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if is_candidate(last) && kinds.contains_key(last) {
                        add_kind(kinds, last, MetavarKind::Expr);
                    }
                } else {
                    let last = base_callee
                        .rsplit("::")
                        .next()
                        .unwrap_or(base_callee)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if is_candidate(last) && kinds.contains_key(last) {
                        add_kind(kinds, last, MetavarKind::Expr);
                    }
                }
                // Macro invocations (`foo!(...)`) carry token-tree bodies,
                // so only shared (both-sides) arg names infer as `expr`
                // holes (WI-9b1): single-side uppercase idents (e.g.
                // `TEXT_SECTION` vs `TEXT_STRIPPED`) stay literal deltas
                // and plus-only holes stay unbound-rejected. Strings are
                // already blanked in the inference view and macro names
                // stay literal (explicit `# macro M` binds via
                // placeholders; WI-7-upper names-never-infer).
                if callee.ends_with('!') {
                    let other = if side == 0 { masked_plus } else { masked_minus };
                    if let Some(args) = balanced_args(&t[paren..]) {
                        for n in names_in(&args, kinds) {
                            // Declared non-`expr` names (e.g. `# macro M`)
                            // keep their declared kind — inferring `expr`
                            // here would mask the shape's own agreement
                            // error with a spurious conflict.
                            if base.kind_of(&n).is_some_and(|k| k != MetavarKind::Expr) {
                                continue;
                            }
                            if occurs_in(other, &n) {
                                add_kind(kinds, &n, MetavarKind::Expr);
                            }
                        }
                    }
                    continue;
                }
                // Type-parseable lines (`Box<dyn Fn(T) -> T>`, `fn(T) -> T`)
                // are type-owned: the type fallback assigns `type` to
                // preserved names, so the call pass must not claim the
                // paren-group names as `expr` (WI-6c-diet). The callee
                // guard above already skips the callee; this skips the
                // args. Non-type lines still infer `expr` holes.
                if syn::parse_str::<syn::Type>(t).is_err() {
                    if let Some(args) = balanced_args(&t[paren..]) {
                        for n in names_in(&args, kinds) {
                            add_kind(kinds, &n, MetavarKind::Expr);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Split `id::<T, U>` into (`id`, `T, U`). Returns `None` when there is
/// no `::<...>` turbofish.
fn split_turbofish(callee: &str) -> Option<(&str, &str)> {
    let pos = callee.find("::<")?;
    let base = callee[..pos].trim();
    let rest = &callee[pos + 3..];
    let end = rest.find('>')?;
    Some((base, rest[..end].trim()))
}

/// Text inside the first balanced `(...)` group (without parens).
fn balanced_args(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut depth = 0;
    let mut in_str = false;
    let mut esc = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
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
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(s[1..i].to_string());
            }
        }
        i += 1;
    }
    None
}

/// Receiver/method split dot for inference: the last `.` outside strings
/// at paren-depth 0 with a `(` after it. Mirrors `method_split_dots` in
/// `expr_pattern` (WI-2 router fix): dots inside the arg list
/// (`E.push(F.clone())` inner dot) or inside strings never split.
pub(crate) fn infer_method_dot(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut dots = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
    while i < bytes.len() {
        let b = bytes[i];
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
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if b == b'.' && depth == 0 {
            dots.push(i);
        }
        i += 1;
    }
    dots.into_iter().rev().find(|&d| s[d + 1..].contains('('))
}

/// Fallback for names with no kind yet: `# root=` wins; `let`-led uses
/// the `let` split kinds; paren/bang/dot shapes → `expr`; `syn::Type`
/// parse → `type`; else `expr` (mirrors `try_general` order).
///
/// Per-line: a patch may mix a call line (`foo(E)` → `expr`) with a
/// type line (`Vec<E>` → `type`); whole-text heuristics would collapse
/// both to one kind and miss the ambiguity. Each line assigns only the
/// names it contains, so cross-line conflicts surface as incompatible
/// positions.
pub(crate) fn assign_general_fallback(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
    root: Option<&str>,
    base: &MetavarTable,
) -> Result<(), StructuredError> {
    for text in [masked_minus, masked_plus] {
        // Forced roots are whole-text (the root governs the shape) —
        // but a control-flow expression gets AST-aware kinds first: a
        // blind whole-text pass would claim `Some` in
        // `match E { Some(x) => 1 }` (or `P` in `|P| E`) as `expr`,
        // contradicting the arm/param positions (WI-5b).
        match root {
            Some("type") => {
                // Whole-text single type: AST walk assigns by position;
                // unparsable text keeps the legacy whole-text fallback.
                if let Ok(t) = syn::parse_str::<syn::Type>(text.trim()) {
                    assign_type_ast(&t, kinds);
                    continue;
                }
                for n in names_in(text, kinds) {
                    add_kind(kinds, &n, MetavarKind::Type);
                }
                continue;
            }
            Some("expr") => {
                let t = text.trim();
                // Single-expression control shapes (match/loops/closures
                // `if`/`let`): walk the AST so arm constructors stay
                // literal and pat slots infer `pat`.
                let mut handled = false;
                if let Ok(syn::Stmt::Expr(e, _)) = syn::parse_str::<syn::Stmt>(t) {
                    if is_control_expr(&e) {
                        assign_expr_ast(&e, kinds);
                        handled = true;
                    }
                }
                if !handled {
                    if let Ok(e) = syn::parse_str::<syn::Expr>(t) {
                        if is_control_expr(&e) {
                            assign_expr_ast(&e, kinds);
                            handled = true;
                        }
                    }
                }
                if handled {
                    continue;
                }
                for n in names_in(text, kinds) {
                    add_kind(kinds, &n, MetavarKind::Expr);
                }
                continue;
            }
            Some("item") => {
                for n in names_in(text, kinds) {
                    add_kind(kinds, &n, MetavarKind::Item);
                }
                continue;
            }
            Some("stmt") => continue, // handled by let pass
            _ => {}
        }
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            // Single-token item capture (`- I`) owns its token; the
            // general fallback must not also claim it as `type`.
            {
                let stripped = t.trim_start_matches('$');
                if !t.contains(char::is_whitespace)
                    && is_candidate(stripped)
                    && kinds.contains_key(stripped)
                {
                    continue;
                }
            }
            // WI-11a: definition RHS positions infer before the
            // item-header skip below (which would leave them literal).
            // (`type NAME = RHS` assigns `type`, `const`/`static` value
            // assigns `expr`; NAME never infers. Sibling module per D5.)
            if super::infer_def::assign_def_rhs_line(t, kinds, masked_minus, masked_plus) {
                continue;
            }
            // WI-11b: member-context pair lines (`name: Type`,
            // `Name(...)`, `Name { ... }`, `Name = EXPR`) assign before
            // the generic fallback (which would misclaim payload types
            // as `expr`). Container headers return false here.
            if super::infer_def::assign_member_evo_line(t, kinds, masked_minus, masked_plus) {
                continue;
            }
            // `use`-path and item-rename headers never infer: leave pending
            // (literal) so their classifiers emit their own needles.
            if t.starts_with("use ") || t.starts_with("use(") {
                continue;
            }
            if [
                "struct", "enum", "trait", "union", "type", "const", "static", "mod", "impl",
            ]
            .iter()
            .any(|k| t.starts_with(&format!("{k} ")))
            {
                continue;
            }
            if fn_like(line) {
                continue; // fn slots handled above; leftovers are concrete
            }
            // Turbofish call lines (`id::<T>(E)`) are owned by the call
            // pass (type args → `type`, value args → `expr`); the generic
            // fallback would otherwise claim the type args as `expr`.
            if line.contains("::<") {
                continue;
            }
            // `let`-led but unmatched by the split (composite patterns):
            // leave literal.
            if t.starts_with("let") {
                continue;
            }
            // WI-5b: control-flow lines get AST-aware kinds (a text
            // fallback would claim `P` in `for P in E` / `|P|` as
            // `expr`, contradicting `# pat P`).
            if assign_control_line(t, kinds) {
                continue;
            }
            // Macro invocations (`foo!(...)`, `foo![...]`,
            // `foo!{...}`) carry token-tree bodies, not expression
            // args (same rule as the call pass). The opener after the
            // bang decides — `!=` / `!cond` never skip (WI-7a: bracket
            // and brace macros without any paren must also skip, else
            // `syn::Type` (macro-in-type-position) misclaims holes).
            if let Some(bang) = t.find('!') {
                let after = t[bang + 1..].trim_start();
                if after.starts_with('(') || after.starts_with('[') || after.starts_with('{') {
                    continue;
                }
            }
            let has_call_shape = t.contains('(') || t.contains('!') || rfind_dot(t).is_some();
            // Types first: `fn(T) -> T` (fn-pointer), `Box<dyn T>`,
            // `Vec<T>` all parse as `syn::Type` despite containing
            // parens/dots — the call branch would misclaim their holes
            // as `expr`. Only preserved names infer (`T` on both sides);
            // one-sided concrete additions (`+ Send`) stay literal.
            if syn::parse_str::<syn::Type>(t).is_ok() {
                for n in names_in_type_args(t, kinds) {
                    if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                        add_kind(kinds, &n, MetavarKind::Type);
                    }
                }
                continue;
            }
            if has_call_shape {
                // Type-owned lines (`P(T)` with `# type P`): the callee is
                // a type constructor — skip expression inference entirely.
                if let Some(paren) = t.find('(') {
                    let callee = t[..paren].trim();
                    let base_callee = callee.trim_end_matches('!').trim();
                    let last = base_callee
                        .rsplit("::")
                        .next()
                        .unwrap_or(base_callee)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if matches!(
                        base.kind_of(last),
                        Some(
                            MetavarKind::Type
                                | MetavarKind::Pat
                                | MetavarKind::Item
                                | MetavarKind::GenericParams
                                | MetavarKind::WhereClause
                                | MetavarKind::Visibility
                                | MetavarKind::Qualifier
                                | MetavarKind::QualifierList
                        )
                    ) {
                        continue;
                    }
                }
                // WI-23: dotted expression lines (`E0.timeout`,
                // `E0.a + 1`, `&E0.f`) carry no paren groups, so the
                // group scan below yields nothing and the base hole
                // stays literal. The sibling `infer_control` module
                // owns the AST-walk fallback (D5: this file sits over
                // the 1600-line gate); qself lines keep the legacy
                // scan (v1 has no holes inside `qself`).
                if super::infer_control::try_assign_dotted_expr(t, kinds) {
                    continue;
                }
                // Holes live inside the argument groups; the callee prefix
                // (`Box` in `Box::new(E)`, `foo` in `foo(E)`) stays literal.
                // The call pass already bound true call args; this covers
                // general-expression tails (`foo(E) + 1`) it bypassed.
                for n in names_in_paren_groups(t, kinds) {
                    add_kind(kinds, &n, MetavarKind::Expr);
                }
                continue;
            }
            for n in names_in(t, kinds) {
                add_kind(kinds, &n, MetavarKind::Expr);
            }
        }
    }
    Ok(())
}
