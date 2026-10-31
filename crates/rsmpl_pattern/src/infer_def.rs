//! WI-11a diet inference for definition RHS positions (split from
//! `infer.rs` per D5 — the parent file sits over the 1600-line gate).

use std::collections::{HashMap, HashSet};

use rsmpl_patch::MetavarKind;

use super::infer::{
    add_kind, names_in, names_in_paren_groups, names_in_type_args, occurs_in, rfind_dot,
};

/// WI-11a: diet inference for definition RHS positions
/// (`type NAME = RHS;`, `const`/`static [mut] NAME: TY = EXPR;`).
///
/// The item-header skip would leave every name on these lines literal,
/// so diet `E`/`T` in the value/type regions never bound. Only the
/// regions after the concrete NAME are claimed: the alias RHS and the
/// ascribed type assign `type` (same both-sides gate as the type-first
/// fallback), the const/static value assigns `expr` (same paren-group
/// discipline as expression lines). Returns true when the line carries
/// the shape (caller skips the legacy fallthrough); bare renames and
/// non-definition lines return false untouched.
pub(crate) fn assign_def_rhs_line(
    t: &str,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
) -> bool {
    let mut rest = t.trim_start();
    if let Some(after_pub) = rest.strip_prefix("pub") {
        if after_pub.is_empty() || after_pub.starts_with(char::is_whitespace) {
            rest = after_pub.trim_start();
        } else if let Some(after) = after_pub.trim_start().strip_prefix('(') {
            let mut depth = 1i32;
            let mut end = None;
            for (i, c) in after.char_indices() {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i + 1);
                        break;
                    }
                }
            }
            if let Some(e) = end {
                rest = after[e..].trim_start();
            } else {
                return false;
            }
        } else {
            return false;
        }
    }
    fn eat_kw<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
        let rest = s.strip_prefix(kw)?;
        if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            Some(rest.trim_start())
        } else {
            None
        }
    }
    fn is_bare(name: &str) -> bool {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    }
    // `type NAME = RHS;`
    if let Some(after) = eat_kw(rest, "type") {
        let Some(eq) = after.find('=') else {
            return false; // bare rename: keep the legacy skip
        };
        let (name, rhs) = (after[..eq].trim(), after[eq + 1..].trim());
        if !is_bare(name) {
            return false;
        }
        let rhs = rhs.strip_suffix(';').unwrap_or(rhs).trim();
        if rhs.is_empty() {
            return false;
        }
        if syn::parse_str::<syn::Type>(rhs).is_ok() {
            for n in names_in_type_args(rhs, kinds) {
                if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                    add_kind(kinds, &n, MetavarKind::Type);
                }
            }
        }
        return true;
    }
    // `const NAME: TY = EXPR;` / `static [mut] NAME: TY = EXPR;`
    let mut after = match eat_kw(rest, "const").or_else(|| eat_kw(rest, "static")) {
        Some(a) => a,
        None => return false,
    };
    let is_static = !rest.starts_with("const");
    if is_static {
        if let Some(next) = eat_kw(after, "mut") {
            after = next;
        }
    }
    let name_end = after
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let (name, tail) = (after[..name_end].trim(), &after[name_end..]);
    if !is_bare(name) {
        return false;
    }
    // Top-level `:` (path `::` never counts).
    let mut colon = None;
    let bytes = tail.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            let prev = i.checked_sub(1).map(|j| bytes[j]);
            let next = bytes.get(i + 1).copied();
            if prev != Some(b':') && next != Some(b':') {
                colon = Some(i);
                break;
            }
        }
        i += 1;
    }
    let Some(ci) = colon else {
        return false; // no ascription: keep the legacy skip
    };
    let after_ty = tail[ci + 1..].trim();
    let Some(eq) = after_ty.find('=') else {
        return false;
    };
    let (ty_text, expr_text) = (after_ty[..eq].trim(), after_ty[eq + 1..].trim());
    if ty_text.is_empty() || expr_text.is_empty() {
        return false;
    }
    if syn::parse_str::<syn::Type>(ty_text).is_ok() {
        for n in names_in_type_args(ty_text, kinds) {
            if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                add_kind(kinds, &n, MetavarKind::Type);
            }
        }
    }
    let expr_text = expr_text.strip_suffix(';').unwrap_or(expr_text).trim();
    if expr_text.is_empty() {
        return true;
    }
    // Expression discipline mirrors the expression-line fallback: holes
    // live inside argument groups, else every name is a value hole.
    if expr_text.contains('(') || expr_text.contains('!') || rfind_dot(expr_text).is_some() {
        for n in names_in_paren_groups(expr_text, kinds) {
            add_kind(kinds, &n, MetavarKind::Expr);
        }
    } else {
        for n in names_in(expr_text, kinds) {
            add_kind(kinds, &n, MetavarKind::Expr);
        }
    }
    true
}
