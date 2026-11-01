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
use crate::pattern::PatternKind;

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
}
