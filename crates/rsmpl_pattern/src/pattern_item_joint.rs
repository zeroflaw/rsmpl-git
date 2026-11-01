//! WI-5 slice 2b: struct rename + field-type evolution joint.
//!
//! Spelling (bare-def + member, no container header — the attr-joint
//! precedent: the def pair names the container, the member pair rides
//! it, file order def-above-member canonical, either order accepted):
//!
//! ```diff
//! - struct Old
//! + struct New
//! - x: u32
//! + x: u64
//! ```
//!
//! Struct-only, field-type-only, one evo pair, same-container. Variant
//! lines, two evos, another-container evos, alias/const lines, and
//! insertions are not this shape (`Ok(None)` — never a loud error, so
//! today's ordinary stays ordinary). Single pairs fall through as well.

use rsmpl_patch::StructuredError;

use crate::metavar::MetavarTable;
use crate::pattern::{PatternKind, VariantPayload};

/// Classify the joint. `Ok(None)` for every non-joint shape.
pub(crate) fn classify_struct_rename_field_evo(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    if !context_raw.trim().is_empty() {
        return Ok(None);
    }
    let mut minuses: Vec<&str> = minus_raw.lines().collect();
    let mut pluses: Vec<&str> = plus_raw.lines().collect();
    // Drop empties (hunk join artifacts never carry them, defensive).
    minuses.retain(|l| !l.trim().is_empty());
    pluses.retain(|l| !l.trim().is_empty());
    if minuses.len() != 2 || pluses.len() != 2 {
        return Ok(None);
    }
    // Find the rename pair (struct-only) and the field pair by shape,
    // order-insensitive (emit is def-first, accept both).
    // Try both assignments: (0=rename,1=field) or (1=rename,0=field).
    for (ri, fi) in [(0usize, 1usize), (1usize, 0usize)] {
        let (Some(rename), Some(field)) = (
            parse_struct_rename(minuses[ri], pluses[ri]),
            parse_field_evo(minuses[fi], pluses[fi], table)?,
        ) else {
            continue;
        };
        let (old_container, new_container) = rename;
        // Same-container law: the field's carrier is the renamed struct.
        // The classifier cannot consult the pre-image (no file here),
        // so it pins the only check it can: nothing else names a
        // container. The raiser (with pre-image) and the engine (with
        // AST) enforce the carrier == old_container law; a joint whose
        // field belongs elsewhere fails there, never here.
        let _ = old_container.clone();
        return Ok(Some(PatternKind::StructRenameFieldEvo {
            old_container,
            new_container,
            field: field.name,
            vis: field.vis,
            minus_sub: field.minus_sub,
            plus_raw: field.plus_raw,
            placeholders: field.placeholders,
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
        }));
    }
    Ok(None)
}

/// A bare `struct Old` → `struct New` pair (no brace, no generics —
/// the terse rename the raiser emits). Returns (old, new).
fn parse_struct_rename(minus: &str, plus: &str) -> Option<(String, String)> {
    let (old_kind, old_name) = split_bare_struct(minus)?;
    let (new_kind, new_name) = split_bare_struct(plus)?;
    if old_kind != "struct" || new_kind != "struct" {
        return None;
    }
    if old_name.is_empty() || new_name.is_empty() || old_name == new_name {
        return None;
    }
    Some((old_name, new_name))
}

/// Classify the enum joint. `Ok(None)` for every non-joint shape.
///
/// Spelling: bare `enum Old` → `enum New` plus one same-identity
/// variant-evo member (tuple payload, struct payload, or discriminant),
/// order-insensitive, context-free. Concrete-only v1 (no `$` holes).
/// Bare renames (`A` → `B`), field lines (`x: u32`), insertions, and
/// hetero shapes fall through as `Ok(None)`.
pub(crate) fn classify_enum_rename_variant_evo(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    if !context_raw.trim().is_empty() {
        return Ok(None);
    }
    let mut minuses: Vec<&str> = minus_raw.lines().collect();
    let mut pluses: Vec<&str> = plus_raw.lines().collect();
    minuses.retain(|l| !l.trim().is_empty());
    pluses.retain(|l| !l.trim().is_empty());
    if minuses.len() != 2 || pluses.len() != 2 {
        return Ok(None);
    }
    for (ri, mi) in [(0usize, 1usize), (1usize, 0usize)] {
        let Some(rename) = parse_bare_enum(minuses[ri], pluses[ri]) else {
            continue;
        };
        let Ok(Some((variant, payload))) = parse_variant_member(minuses[mi], pluses[mi], table)
        else {
            continue;
        };
        let (old_container, new_container) = rename;
        let _ = old_container.clone();
        return Ok(Some(PatternKind::EnumRenameVariantEvo {
            old_container,
            new_container,
            variant,
            payload,
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
        }));
    }
    Ok(None)
}

/// A bare `enum Old` → `enum New` pair. Returns (old, new).
fn parse_bare_enum(minus: &str, plus: &str) -> Option<(String, String)> {
    let (old_kind, old_name) = split_bare_enum(minus)?;
    let (new_kind, new_name) = split_bare_enum(plus)?;
    if old_kind != "enum" || new_kind != "enum" {
        return None;
    }
    if old_name.is_empty() || new_name.is_empty() || old_name == new_name {
        return None;
    }
    Some((old_name, new_name))
}

/// Bare `enum Name` (optional `pub…` vis, optional trailing `;`).
fn split_bare_enum(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let t = t.strip_suffix(';').map(str::trim).unwrap_or(t);
    if t.contains(['{', '}', '(', ')', '<', '=', ':']) {
        return None;
    }
    let mut parts: Vec<&str> = t.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let name = parts.pop()?.to_string();
    if !is_ident(&name) {
        return None;
    }
    if parts == ["enum"] {
        return Some(("enum".to_string(), name));
    }
    if parts.len() == 2 && parts[1] == "enum" && is_vis(parts[0]) {
        return Some(("enum".to_string(), name));
    }
    if parts.len() > 2 && parts[parts.len() - 1] == "enum" {
        let vis = parts[..parts.len() - 1].join(" ");
        if is_vis(&vis) || vis.starts_with("pub(") {
            return Some(("enum".to_string(), name));
        }
        return None;
    }
    None
}

/// Same-identity variant evolution (concrete-only): tuple payload
/// (arity-equal, ≥1 type differs), struct payload (field-names-equal,
/// ≥1 type differs), or discriminant (`=` both sides, exprs differ).
/// Returns (variant, payload). `Err` on loud malformed types (never
/// reached on the fallthrough shapes below — they return `Ok(None)`).
fn parse_variant_member(
    minus: &str,
    plus: &str,
    table: &MetavarTable,
) -> Result<Option<(String, VariantPayload)>, StructuredError> {
    let _ = table;
    let mut m = minus.trim();
    let mut p = plus.trim();
    m = m.strip_suffix(',').map(str::trim).unwrap_or(m);
    p = p.strip_suffix(',').map(str::trim).unwrap_or(p);
    m = m.strip_suffix(';').map(str::trim).unwrap_or(m);
    p = p.strip_suffix(';').map(str::trim).unwrap_or(p);
    if m.is_empty() || p.is_empty() || m.starts_with("#[") || p.starts_with("#[") {
        return Ok(None);
    }
    // Field lines are the struct joint's domain, never this shape.
    if m.contains(':') || p.contains(':') {
        // Discriminant exprs never carry a top-level `:` outside `::`;
        // keep it simple: any `:` declines (variant payloads here are
        // tuple payloads; struct payloads use `:` inside braces and are
        // handled below via brace parse, not this gate).
        // Struct-payload members contain both `{` and `:`; tuple/disc
        // members contain neither bare `:` — so gate only non-brace
        // lines here.
        if !(m.contains('{') && p.contains('{')) {
            return Ok(None);
        }
    }
    if m.contains('$') || p.contains('$') {
        return Ok(None);
    }
    let (m_ident, m_tail) = match split_variant(m) {
        Some(v) => v,
        None => return Ok(None),
    };
    let (p_ident, p_tail) = match split_variant(p) {
        Some(v) => v,
        None => return Ok(None),
    };
    if m_ident != p_ident || m_ident.is_empty() || !is_ident(&m_ident) {
        return Ok(None);
    }
    // Discriminant: `= expr` both sides, exprs differ.
    if m_tail.trim_start().starts_with('=') && p_tail.trim_start().starts_with('=') {
        let m_expr = m_tail.trim_start().strip_prefix('=').unwrap_or("").trim();
        let p_expr = p_tail.trim_start().strip_prefix('=').unwrap_or("").trim();
        if m_expr.is_empty() || p_expr.is_empty() || m_expr == p_expr {
            return Ok(None);
        }
        let _: syn::Expr = syn::parse_str(m_expr).map_err(|_| {
            joint_err("joint discriminant type does not parse as a Rust expression")
        })?;
        let _: syn::Expr = syn::parse_str(p_expr).map_err(|_| {
            joint_err("joint discriminant type does not parse as a Rust expression")
        })?;
        return Ok(Some((
            m_ident,
            VariantPayload::Discriminant {
                minus_sub: m_expr.to_string(),
                plus_raw: p_expr.to_string(),
                placeholders: Vec::new(),
            },
        )));
    }
    // Tuple payload: `(T, …)` both sides, arity-equal, ≥1 differs.
    if m_tail.trim_start().starts_with('(') && p_tail.trim_start().starts_with('(') {
        let m_inner = m_tail
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'));
        let p_inner = p_tail
            .trim()
            .strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'));
        let (Some(mi), Some(pi)) = (m_inner, p_inner) else {
            return Ok(None);
        };
        let split_tys = |s: &str| -> Option<Vec<String>> {
            if s.trim().is_empty() {
                return Some(Vec::new());
            }
            // Top-level comma split (types here are concrete; generics
            // nest `<…>` — track depth).
            let mut out = Vec::new();
            let mut depth = 0i32;
            let mut cur = String::new();
            for c in s.chars() {
                match c {
                    '<' | '(' | '[' => {
                        depth += 1;
                        cur.push(c);
                    }
                    '>' | ')' | ']' => {
                        depth -= 1;
                        cur.push(c);
                    }
                    ',' if depth == 0 => {
                        out.push(cur.trim().to_string());
                        cur.clear();
                    }
                    _ => cur.push(c),
                }
            }
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            Some(out)
        };
        let m_tys = match split_tys(mi) {
            Some(v) => v,
            None => return Ok(None),
        };
        let p_tys = match split_tys(pi) {
            Some(v) => v,
            None => return Ok(None),
        };
        if m_tys.len() != p_tys.len() || m_tys.is_empty() {
            return Ok(None);
        }
        for (a, b) in m_tys.iter().zip(p_tys.iter()) {
            let _: syn::Type = syn::parse_str(a).map_err(|_| {
                joint_err("joint variant payload type does not parse as a Rust type")
            })?;
            let _: syn::Type = syn::parse_str(b).map_err(|_| {
                joint_err("joint variant payload type does not parse as a Rust type")
            })?;
        }
        if m_tys == p_tys {
            return Ok(None);
        }
        return Ok(Some((
            m_ident,
            VariantPayload::Tuple {
                minus_subs: m_tys,
                plus_raws: p_tys,
                placeholders: Vec::new(),
            },
        )));
    }
    // Struct payload: `{ f: T, … }` both sides, field-names-equal,
    // ≥1 type differs.
    if m_tail.trim_start().starts_with('{') && p_tail.trim_start().starts_with('{') {
        let m_inner = m_tail
            .trim()
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'));
        let p_inner = p_tail
            .trim()
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'));
        let (Some(mi), Some(pi)) = (m_inner, p_inner) else {
            return Ok(None);
        };
        let split_fields = |s: &str| -> Option<Vec<(String, String)>> {
            let mut out = Vec::new();
            let mut depth = 0i32;
            let mut cur = String::new();
            let push = |seg: &str, out: &mut Vec<(String, String)>| -> Option<()> {
                let seg = seg.trim();
                if seg.is_empty() {
                    return Some(());
                }
                let colon = seg.find(':')?;
                let (n, t) = seg.split_at(colon);
                let t = t.strip_prefix(':')?.trim();
                if !is_ident(n.trim()) || t.is_empty() {
                    return None;
                }
                out.push((n.trim().to_string(), t.to_string()));
                Some(())
            };
            for c in s.chars() {
                match c {
                    '<' | '(' | '[' | '{' => {
                        depth += 1;
                        cur.push(c);
                    }
                    '>' | ')' | ']' | '}' => {
                        depth -= 1;
                        cur.push(c);
                    }
                    ',' if depth == 0 => {
                        push(&cur.clone(), &mut out)?;
                        cur.clear();
                    }
                    _ => cur.push(c),
                }
            }
            if !cur.trim().is_empty() {
                push(&cur.clone(), &mut out)?;
            }
            Some(out)
        };
        let m_fs = match split_fields(mi) {
            Some(v) => v,
            None => return Ok(None),
        };
        let p_fs = match split_fields(pi) {
            Some(v) => v,
            None => return Ok(None),
        };
        if m_fs.len() != p_fs.len() || m_fs.is_empty() {
            return Ok(None);
        }
        let m_names: Vec<&str> = m_fs.iter().map(|(n, _)| n.as_str()).collect();
        let p_names: Vec<&str> = p_fs.iter().map(|(n, _)| n.as_str()).collect();
        if m_names != p_names {
            return Ok(None);
        }
        for ((_, a), (_, b)) in m_fs.iter().zip(p_fs.iter()) {
            let _: syn::Type = syn::parse_str(a).map_err(|_| {
                joint_err("joint variant payload type does not parse as a Rust type")
            })?;
            let _: syn::Type = syn::parse_str(b).map_err(|_| {
                joint_err("joint variant payload type does not parse as a Rust type")
            })?;
        }
        let m_tys: Vec<String> = m_fs.iter().map(|(_, t)| t.clone()).collect();
        let p_tys: Vec<String> = p_fs.iter().map(|(_, t)| t.clone()).collect();
        if m_tys == p_tys {
            return Ok(None);
        }
        return Ok(Some((
            m_ident,
            VariantPayload::Struct {
                fields: m_names.into_iter().map(str::to_string).collect(),
                minus_subs: m_tys,
                plus_raws: p_tys,
                placeholders: Vec::new(),
            },
        )));
    }
    Ok(None)
}

/// Split `Variant` + tail (`(…)`, `{ … }`, `= …`, or bare).
fn split_variant(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    let len = t
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    if len == 0 {
        return None;
    }
    let (ident, tail) = t.split_at(len);
    Some((ident.to_string(), tail.trim().to_string()))
}

fn joint_err(msg: impl Into<String>) -> StructuredError {
    StructuredError::new(rsmpl_patch::ErrorCode::PatternConflict, msg.into())
}

/// Bare `struct Name` (optional `pub…` vis, optional trailing `;`).
/// Anything with a brace, paren, `<`, `=`, or `:` is not bare.
fn split_bare_struct(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let t = t.strip_suffix(';').map(str::trim).unwrap_or(t);
    if t.contains(['{', '}', '(', ')', '<', '=', ':']) {
        return None;
    }
    let mut parts: Vec<&str> = t.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let name = parts.pop()?.to_string();
    if !is_ident(&name) {
        return None;
    }
    // Optional vis prefix (bare `pub`, `pub(...)`) + `struct`.
    if parts == ["struct"] {
        return Some(("struct".to_string(), name));
    }
    if parts.len() == 2 && parts[1] == "struct" && is_vis(parts[0]) {
        return Some(("struct".to_string(), name));
    }
    if parts.len() > 2 && parts[parts.len() - 1] == "struct" {
        // `pub(crate) struct` lexes as [`pub(crate)`, `struct`].
        let vis = parts[..parts.len() - 1].join(" ");
        if is_vis(&vis) || vis.starts_with("pub(") {
            return Some(("struct".to_string(), name));
        }
        return None;
    }
    None
}

fn is_vis(s: &str) -> bool {
    s == "pub" || s.starts_with("pub(")
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().enumerate().all(|(i, c)| {
            if i == 0 {
                c.is_ascii_alphabetic() || c == '_'
            } else {
                c.is_ascii_alphanumeric() || c == '_'
            }
        })
}

struct FieldEvo {
    name: String,
    vis: String,
    minus_sub: String,
    plus_raw: String,
    placeholders: Vec<(String, String)>,
}

/// Same-name `name: Ty` pair with differing concrete types (v1:
/// concrete only — no metavariables, the diet emission never mints
/// them). Attr lines decline (the attr joint owns them).
fn parse_field_evo(
    minus: &str,
    plus: &str,
    table: &MetavarTable,
) -> Result<Option<FieldEvo>, StructuredError> {
    let _ = table;
    let (Some((m_vis, m_name, m_ty)), Some((p_vis, p_name, p_ty))) =
        (split_field(minus), split_field(plus))
    else {
        return Ok(None);
    };
    if m_name != p_name || m_vis != p_vis || m_ty == p_ty {
        return Ok(None);
    }
    if m_name.is_empty() || !is_ident(&m_name) {
        return Ok(None);
    }
    // Reject metavariable spellings (v1 concrete only).
    if m_ty.contains('$') || p_ty.contains('$') {
        return Ok(None);
    }
    let m_parsed: syn::Type = syn::parse_str(&m_ty)
        .map_err(|_| field_err("joint field type does not parse as a Rust type"))?;
    let p_parsed: syn::Type = syn::parse_str(&p_ty)
        .map_err(|_| field_err("joint field type does not parse as a Rust type"))?;
    let norm = |t: &syn::Type| {
        quote::quote!(#t)
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let m_norm = norm(&m_parsed);
    let p_norm = norm(&p_parsed);
    if m_norm == p_norm {
        return Ok(None);
    }
    Ok(Some(FieldEvo {
        name: m_name,
        vis: m_vis,
        minus_sub: m_norm,
        plus_raw: p_ty,
        placeholders: Vec::new(),
    }))
}

/// Split `vis name: Ty` on the single top-level colon. Returns
/// (vis, name, ty_raw). Rejects variant-shaped lines (top-level `(`/`{`/`=`).
fn split_field(line: &str) -> Option<(String, String, String)> {
    let mut t = line.trim();
    t = t.strip_suffix(',').map(str::trim).unwrap_or(t);
    t = t.strip_suffix(';').map(str::trim).unwrap_or(t);
    if t.is_empty() || t.starts_with("#[") {
        return None;
    }
    // Variant-shaped suffix veto (payload parens/braces, discriminant).
    if t.contains('(') || t.contains('{') || t.contains('=') {
        // `=` inside a type (`Option<T = ()>`)? Field types never carry a
        // top-level `=` outside generics; keep the veto simple: any `=`
        // declines (the discriminant detector owns it).
        return None;
    }
    let colon = find_field_colon(t)?;
    let (left, right) = t.split_at(colon);
    let ty = right.strip_prefix(':')?.trim();
    if ty.is_empty() {
        return None;
    }
    let left = left.trim();
    if left.is_empty() || left.contains(['{', '}', '(', ')', ';', '=']) {
        return None;
    }
    let mut parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let name = parts.pop()?.to_string();
    let vis = if parts.is_empty() {
        String::new()
    } else {
        let v = parts.join(" ");
        if v == "pub" || v.starts_with("pub(") || v.starts_with("pub ") {
            v
        } else {
            return None;
        }
    };
    Some((vis, name, ty.to_string()))
}

/// Single top-level colon (no `::` adjacency), bracket-aware.
fn find_field_colon(t: &str) -> Option<usize> {
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let bytes = t.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] as char {
            '(' => dp += 1,
            ')' => dp -= 1,
            '[' => db += 1,
            ']' => db -= 1,
            '{' => dbr += 1,
            '}' => dbr -= 1,
            '<' => da += 1,
            '>' => {
                if da > 0 {
                    da -= 1;
                }
            }
            ':' if dp == 0 && db == 0 && dbr == 0 && da == 0 => {
                let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
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

fn field_err(msg: impl Into<String>) -> StructuredError {
    StructuredError::new(rsmpl_patch::ErrorCode::PatternConflict, msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metavar::MetavarTable;

    fn table() -> MetavarTable {
        MetavarTable::default()
    }

    fn classify(m: &str, p: &str) -> Option<PatternKind> {
        classify_struct_rename_field_evo(m, p, "", &table()).expect("never loud on these shapes")
    }

    #[test]
    fn joint_fires_def_first() {
        let m = "struct Old\nx: u32";
        let p = "struct New\nx: u64";
        match classify(m, p).expect("must fire") {
            PatternKind::StructRenameFieldEvo {
                old_container,
                new_container,
                field,
                ..
            } => {
                assert_eq!(old_container, "Old");
                assert_eq!(new_container, "New");
                assert_eq!(field, "x");
            }
            other => panic!("wrong kind: {other:?}"),
        }
    }

    #[test]
    fn joint_fires_member_first() {
        let m = "x: u32\nstruct Old";
        let p = "x: u64\nstruct New";
        assert!(matches!(
            classify(m, p),
            Some(PatternKind::StructRenameFieldEvo { .. })
        ));
    }

    #[test]
    fn non_joint_shapes_decline() {
        // Single pairs fall through.
        assert!(classify("struct Old", "struct New").is_none());
        assert!(classify("x: u32", "x: u64").is_none());
        // Variant-shaped member declines.
        assert!(classify("struct Old\nOld(u32)", "struct New\nOld(u64)").is_none());
        // Two renames decline.
        assert!(classify("struct Old\nstruct A", "struct New\nstruct B").is_none());
        // Two evos decline.
        assert!(classify("x: u32\ny: u32", "x: u64\ny: u64").is_none());
        // Same-name pair declines.
        assert!(classify("struct Old\nx: u32", "struct Old\nx: u64").is_none());
        // Insertion shape declines (unbalanced).
        assert!(classify("struct Old", "struct New\nb: bool,").is_none());
        // Context header declines.
        assert!(classify_struct_rename_field_evo(
            "struct Old\nx: u32",
            "struct New\nx: u64",
            "struct Old {",
            &table()
        )
        .unwrap()
        .is_none());
    }

    fn eclassify(m: &str, p: &str) -> Option<PatternKind> {
        classify_enum_rename_variant_evo(m, p, "", &table()).expect("never loud on these shapes")
    }

    #[test]
    fn enum_joint_fires_all_shapes() {
        // Tuple payload.
        assert!(matches!(
            eclassify("enum Old\nA(u32)", "enum New\nA(u64)"),
            Some(PatternKind::EnumRenameVariantEvo { .. })
        ));
        // Struct payload.
        assert!(matches!(
            eclassify("enum Old\nA { x: u32 }", "enum New\nA { x: u64 }"),
            Some(PatternKind::EnumRenameVariantEvo { .. })
        ));
        // Discriminant.
        assert!(matches!(
            eclassify("enum Old\nA = 1", "enum New\nA = 2"),
            Some(PatternKind::EnumRenameVariantEvo { .. })
        ));
        // Member-first order.
        assert!(matches!(
            eclassify("A(u32)\nenum Old", "A(u64)\nenum New"),
            Some(PatternKind::EnumRenameVariantEvo { .. })
        ));
    }

    #[test]
    fn enum_joint_non_shapes_decline() {
        // Single pairs fall through.
        assert!(eclassify("enum Old", "enum New").is_none());
        assert!(eclassify("A(u32)", "A(u64)").is_none());
        // Bare variant rename declines (not an evolution).
        assert!(eclassify("enum Old\nA", "enum New\nB").is_none());
        // Field line + enum def declines (cross-kind).
        assert!(eclassify("enum Old\nx: u32", "enum New\nx: u64").is_none());
        // Struct def + variant member declines.
        assert!(eclassify("struct Old\nA(u32)", "struct New\nA(u64)").is_none());
        // Same-name def declines.
        assert!(eclassify("enum Old\nA(u32)", "enum Old\nA(u64)").is_none());
        // Insertion declines (unbalanced).
        assert!(eclassify("enum Old", "enum New\nB,").is_none());
        // Two evos decline.
        assert!(eclassify("enum Old\nA(u32)", "enum New\nA(u32)").is_none());
        // Context header declines.
        assert!(classify_enum_rename_variant_evo(
            "enum Old\nA(u32)",
            "enum New\nA(u64)",
            "enum Old {",
            &table()
        )
        .unwrap()
        .is_none());
    }
}
