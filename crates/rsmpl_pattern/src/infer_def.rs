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

/// WI-11b: diet inference for member-context evolution lines.
///
/// - `name: RHS` field pairs assign `type` from the RHS (both-sides gate);
/// - `Name(...)` tuple payloads assign `type` per position;
/// - `Name { f: T, ... }` struct payloads assign `type` per field type;
/// - `Name = EXPR` discriminants assign `expr` (paren-group discipline).
///
/// Container headers (`struct S {`, `enum E {`) return false (legacy skip);
/// bare renames (`old: T` with no template need is decided by the generic
/// pass — we still claim the line so the generic `expr` fallback does not
/// misclaim `T`). Returns true when the line carries a member shape.
pub(crate) fn assign_member_evo_line(
    t: &str,
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
) -> bool {
    // Member lines only infer under a struct/enum container header
    // elsewhere in the texts (else `foo(E)` call args would misinfer as
    // payload types — the WI-5 `foo(N)`-stays-literal pin). The header
    // rides in the full minus/plus texts (context echoes into both).
    if !has_member_container(masked_minus) && !has_member_container(masked_plus) {
        return false;
    }
    let line = t.trim().trim_end_matches([',', ';']).trim();
    if line.is_empty() {
        return false;
    }
    // Container headers never infer here.
    if line.ends_with('{') {
        return false;
    }
    // Field pair: `[pub...] name : Type` (depth-0 colon only — struct
    // payload colons live inside braces and belong to the arm below).
    if let Some(colon) = top_colon_depth0(line) {
        let (head, rhs) = (line[..colon].trim(), line[colon + 1..].trim());
        let head = strip_pub(head);
        if is_bare(head) && !rhs.is_empty() && syn::parse_str::<syn::Type>(rhs).is_ok() {
            for n in names_in_type_args(rhs, kinds) {
                if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                    add_kind(kinds, &n, MetavarKind::Type);
                }
            }
            return true;
        }
        // Not a `[pub] ident : Type` head: another shape owns this text.
        if is_bare(head) {
            return true; // claim so the generic fallback stays out
        }
        return false;
    }
    // Variant tuple payload: `Name(...)`.
    if let Some(open) = line.find('(') {
        let name = line[..open].trim();
        if is_bare(name) && line.trim_end().ends_with(')') {
            let inner = line[open + 1..line.trim_end().len() - 1].trim();
            for part in split_top(inner) {
                let part = part.trim();
                if syn::parse_str::<syn::Type>(part).is_ok() {
                    for n in names_in_type_args(part, kinds) {
                        if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                            add_kind(kinds, &n, MetavarKind::Type);
                        }
                    }
                }
            }
            return true;
        }
        return false;
    }
    // Variant struct payload: `Name { f: T, ... }`.
    if let Some(open) = line.find('{') {
        let name = line[..open].trim();
        if is_bare(name) && line.trim_end().ends_with('}') {
            let inner = line[open + 1..line.trim_end().len() - 1].trim();
            for part in split_top(inner) {
                let part = part.trim();
                if let Some(ci) = top_colon(part) {
                    let fty = part[ci + 1..].trim();
                    if syn::parse_str::<syn::Type>(fty).is_ok() {
                        for n in names_in_type_args(fty, kinds) {
                            if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                                add_kind(kinds, &n, MetavarKind::Type);
                            }
                        }
                    }
                }
            }
            return true;
        }
        return false;
    }
    // Discriminant: `Name = EXPR`.
    if let Some((head, expr)) = split_top_eq(line) {
        if is_bare(head.trim()) && !expr.trim().is_empty() {
            let expr = expr.trim();
            if expr.contains('(') || expr.contains('!') || rfind_dot(expr).is_some() {
                for n in names_in_paren_groups(expr, kinds) {
                    add_kind(kinds, &n, MetavarKind::Expr);
                }
            } else {
                for n in names_in(expr, kinds) {
                    add_kind(kinds, &n, MetavarKind::Expr);
                }
            }
            return true;
        }
    }
    false
}

fn is_bare(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
}

/// True when the text holds a `struct S {` / `enum E {` container header
/// (the member-evolution context). Lines need not be valid items beyond
/// the header shape — the classifier owns precision; here we only gate
/// inference so call-shaped lines without a container never claim.
pub(crate) fn has_member_container(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        if !t.ends_with('{') {
            return false;
        }
        syn::parse_str::<syn::Item>(&format!("{t}}}"))
            .is_ok_and(|it| matches!(it, syn::Item::Struct(_) | syn::Item::Enum(_)))
    })
}

fn strip_pub(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("pub") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return rest.trim_start();
        }
        if rest.trim_start().starts_with('(') {
            // `pub(...)`: skip to the closing paren.
            let mut depth = 0i32;
            let chars: Vec<char> = rest.chars().collect();
            for (i, &c) in chars.iter().enumerate() {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        return rest[i + 1..].trim_start();
                    }
                }
            }
        }
    }
    t
}

/// Depth-0 field-separator colon (path `::` never counts; brackets and
/// strings opaque). Struct-payload colons sit at brace depth ≥ 1 and
/// never count here.
fn top_colon_depth0(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let mut ins = false;
    let mut esc = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if ins {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                ins = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => ins = true,
            '(' => dp += 1,
            ')' => dp -= 1,
            '[' => db += 1,
            ']' => db -= 1,
            '{' => dbr += 1,
            '}' => dbr -= 1,
            '<' => da += 1,
            '>' if da > 0 => da -= 1,
            '>' => {}
            ':' if dp == 0 && db == 0 && dbr == 0 && da == 0 => {
                let prev = i.checked_sub(1).map(|j| bytes[j]);
                let next = bytes.get(i + 1).copied();
                if prev != Some(b':') && next != Some(b':') {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn top_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            let prev = i.checked_sub(1).map(|j| bytes[j]);
            let next = bytes.get(i + 1).copied();
            if prev != Some(b':') && next != Some(b':') {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn split_top(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let mut ins = false;
    let mut esc = false;
    let mut cur = String::new();
    for c in text.chars() {
        if ins {
            cur.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                ins = false;
            }
            continue;
        }
        match c {
            '"' => {
                ins = true;
                cur.push(c);
            }
            '(' => {
                dp += 1;
                cur.push(c);
            }
            ')' => {
                dp -= 1;
                cur.push(c);
            }
            '[' => {
                db += 1;
                cur.push(c);
            }
            ']' => {
                db -= 1;
                cur.push(c);
            }
            '{' => {
                dbr += 1;
                cur.push(c);
            }
            '}' => {
                dbr -= 1;
                cur.push(c);
            }
            '<' => {
                da += 1;
                cur.push(c);
            }
            '>' => {
                if da > 0 {
                    da -= 1;
                }
                cur.push(c);
            }
            ',' if dp == 0 && db == 0 && dbr == 0 && da == 0 => {
                out.push(cur);
                cur = String::new();
            }
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// True when the line has a member-pair shape (WI-11b): `name: Type`,
/// `Name(...)`, `Name { ... }`, or `Name = EXPR`. The classifier owns
/// precision (container + full parse); here the shape only steers the
/// call pass aside when a container header is present, so `foo(E)` stays
/// a call without one.
pub(crate) fn is_member_pair_line(t: &str) -> bool {
    let line = t.trim().trim_end_matches([',', ';']).trim();
    if line.is_empty() || line.ends_with('{') {
        return false;
    }
    // Tuple payload `Name(...)` (a field type's parens never leave a
    // bare-`Name` head, e.g. `old: fn(u8)` → `old: fn`).
    if let Some(open) = line.find('(') {
        let name = line[..open].trim();
        if is_bare(name) && line.trim_end().ends_with(')') {
            return true;
        }
    }
    // Struct payload `Name { ... }`.
    if let Some(open) = line.find('{') {
        let name = line[..open].trim();
        if is_bare(name) && line.trim_end().ends_with('}') {
            return true;
        }
    }
    // Discriminant `Name = EXPR`.
    if let Some((head, expr)) = split_top_eq(line) {
        if is_bare(head.trim()) && !expr.trim().is_empty() {
            return true;
        }
    }
    // Field pair (depth-0 colon only — payload colons sit in brackets).
    if let Some(colon) = top_colon_depth0(line) {
        if is_bare(strip_pub(line[..colon].trim())) {
            return true;
        }
    }
    false
}

fn split_top_eq(text: &str) -> Option<(String, String)> {
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let mut ins = false;
    let mut esc = false;
    let bytes: Vec<char> = text.chars().collect();
    for (i, &c) in bytes.iter().enumerate() {
        if ins {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                ins = false;
            }
            continue;
        }
        let at_top = dp == 0 && db == 0 && dbr == 0 && da == 0;
        // A top-level `=` splits (expression spellings `==`/`!=`/`<=`/
        // `>=`/`=>` excluded below); anything nested only tracks depth.
        if c == '=' && at_top {
            let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
            let next = bytes.get(i + 1).copied();
            if prev == Some('=')
                || prev == Some('!')
                || prev == Some('<')
                || prev == Some('>')
                || next == Some('=')
                || next == Some('>')
            {
                continue;
            }
            return Some((bytes[..i].iter().collect(), bytes[i + 1..].iter().collect()));
        }
        match c {
            '"' => ins = true,
            '(' => dp += 1,
            ')' => dp -= 1,
            '[' => db += 1,
            ']' => db -= 1,
            '{' => dbr += 1,
            '}' => dbr -= 1,
            '<' => da += 1,
            '>' if da > 0 => da -= 1,
            '>' => {}
            _ => {}
        }
    }
    None
}
