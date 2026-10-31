//! Member-context definition RHS evolution (SPEC 14.8, WI-11b).
//!
//! Split from `pattern_classify` per D5. The router hook in
//! `pattern_classify::classify` stays (two lines); every helper lives here.
//!
//! Shapes (all with a container header as context):
//! - `struct S {` + `- old: Vec<T>` / `+ old: Box<[T]>` (field-type);
//! - `enum E {` + `- Old(Vec<T>)` / `+ Old(Box<[T]>)` (tuple payload);
//! - `enum E {` + `- Old { f: Vec<T> }` / `+ Old { f: Box<[T]> }` (struct);
//! - `enum E {` + `- Old = E` / `+ Old = E + 1` (discriminant).
//!
//! Single-region rule: one hunk evolves exactly one region. Name changes
//! never combine with evolution (rename hunk + evolution hunk, or an
//! ordinary hunk). Returns `Ok(None)` when neither side carries a member
//! evolution shape — bare renames and every other shape fall through
//! untouched. A claimed shape with a malformed half is a loud
//! `PatternConflict` (never a silent fallthrough, SPEC 2.6).

use rsmpl_patch::StructuredError;

use crate::metavar::MetavarTable;
use crate::pattern::{PatternKind, VariantPayload};
use crate::pattern_classify::is_member_ident;

/// Classify member-context RHS evolution. See module docs.
pub(crate) fn classify_member_evolution(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    // No container header, no member claim: call-shaped pairs (`foo(E)`
    // → `bar(E)`) and expression statements (`Old = 1`) belong to the
    // call/general classifiers. The header is the disambiguator (the
    // inference gate in `infer_def` uses the same rule).
    if struct_context(context_raw).is_none() && enum_context(context_raw).is_none() {
        return Ok(None);
    }
    let minus_stripped = strip_container_headers(minus_raw);
    let plus_stripped = strip_container_headers(plus_raw);
    let (minus_attrs, minus_pair) = crate::pattern_general::split_leading_attrs(&minus_stripped);
    let (plus_attrs, plus_pair) = crate::pattern_general::split_leading_attrs(&plus_stripped);
    let minus_pair = minus_pair.trim().to_string();
    let plus_pair = plus_pair.trim().to_string();

    // Field-type shape first: both sides carry a top-level `:`.
    let minus_field = parse_field_side(&minus_pair)?;
    let plus_field = parse_field_side(&plus_pair)?;
    match (minus_field, plus_field) {
        (Some(m), Some(p)) => {
            return field_evolution(&m, &p, context_raw, &minus_attrs, &plus_attrs, table);
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(field_err(
                "field evolution sides must both be `<name>: <Type>` pairs (or use an ordinary hunk)",
            ));
        }
        (None, None) => {}
    }

    // Variant payload/discriminant shapes.
    let minus_v = parse_variant_side(&minus_pair);
    let plus_v = parse_variant_side(&plus_pair);
    match (minus_v, plus_v) {
        (Some(m), Some(p)) => {
            variant_evolution(&m, &p, context_raw, &minus_attrs, &plus_attrs, table)
        }
        (Some(_), None) | (None, Some(_)) => Err(variant_err(
            "variant evolution sides must both carry the same variant shape (`Name`, `Name(...)`, `Name { ... }`, `Name = ...`; or use an ordinary hunk)",
        )),
        (None, None) => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

fn member_context_container(line: &str, want_struct: bool) -> Option<String> {
    let t = line.trim();
    if !t.ends_with('{') {
        return None;
    }
    let item: syn::Item = syn::parse_str(&format!("{t}}}")).ok()?;
    match item {
        syn::Item::Struct(s) if want_struct => Some(s.ident.to_string()),
        syn::Item::Enum(e) if !want_struct => Some(e.ident.to_string()),
        _ => None,
    }
}

fn struct_context(context_raw: &str) -> Option<String> {
    context_raw
        .lines()
        .find_map(|l| member_context_container(l, true))
}

fn enum_context(context_raw: &str) -> Option<String> {
    context_raw
        .lines()
        .find_map(|l| member_context_container(l, false))
}

fn strip_container_headers(text: &str) -> String {
    text.lines()
        .filter(|l| {
            member_context_container(l, true).is_none()
                && member_context_container(l, false).is_none()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// Field-type evolution
// ---------------------------------------------------------------------------

struct FieldSide {
    vis: String,
    name: String,
    ty: String,
    raw_ty: String,
}

fn parse_field_side(side: &str) -> Result<Option<FieldSide>, StructuredError> {
    let t = side.trim().trim_end_matches([',', ';']).trim();
    if t.is_empty() {
        return Ok(None);
    }
    let Some(colon) = crate::pattern_classify::find_field_colon(t) else {
        return Ok(None);
    };
    let (vis, rest) = split_member_vis(t[..colon].trim());
    if !is_member_ident(rest) {
        return Ok(None);
    }
    let ty_raw = t[colon + 1..].trim();
    if ty_raw.is_empty() {
        return Err(field_err("field evolution side names no type"));
    }
    let ty: syn::Type = syn::parse_str(ty_raw).map_err(|_| {
        field_err(format!(
            "field evolution type `{ty_raw}` does not parse as a Rust type"
        ))
    })?;
    Ok(Some(FieldSide {
        vis: vis.to_string(),
        name: rest.to_string(),
        ty: norm_ty(&ty),
        raw_ty: ty_raw.to_string(),
    }))
}

fn field_evolution(
    m: &FieldSide,
    p: &FieldSide,
    context_raw: &str,
    minus_attrs: &[String],
    plus_attrs: &[String],
    table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    check_attr_mirror(minus_attrs, plus_attrs)?;
    // Rename owns "different names, same type" only: fall through before
    // the container check so rename-owned pairs keep their owner. Every
    // same-name pair (even a no-op same-type one — the WI-11a alias
    // precedent, and the WI-1.5 plus-as-minus synth) is evolution.
    if m.name != p.name && m.ty == p.ty {
        return Ok(None);
    }
    let Some(container) = struct_context(context_raw) else {
        return Err(field_err(
            "field-type evolution needs a `struct S {` context line naming the container (add the header as context, or use an ordinary hunk)",
        ));
    };
    if m.name != p.name {
        return Err(rename_instead(&m.name, &p.name, &m.ty, &p.ty));
    }
    if m.vis != p.vis {
        return Err(field_err(format!(
            "field-type evolution visibility differs (`{}` vs `{}`); renames are ident-only — write the same prefix on both sides or neither",
            m.vis, p.vis
        )));
    }
    let (minus_sub, placeholders) = compile_rhs(&m.raw_ty, false, table)?;
    compile_rhs(&p.raw_ty, false, table)?;
    Ok(Some(PatternKind::FieldTypeEvolution {
        struct_name: container,
        field: m.name.clone(),
        vis: m.vis.clone(),
        minus_sub,
        plus_raw: p.raw_ty.clone(),
        placeholders,
        minus_attrs: minus_attrs.to_vec(),
        plus_attrs: plus_attrs.to_vec(),
    }))
}

// ---------------------------------------------------------------------------
// Variant payload / discriminant evolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum PayloadRaw {
    Tuple(Vec<String>),
    Struct(Vec<(String, String)>),
}

#[derive(Debug, Clone)]
struct VariantSide {
    name: String,
    payload: Option<PayloadRaw>,
    discriminant: Option<String>,
}

fn parse_variant_side(side: &str) -> Option<VariantSide> {
    let t = side.trim().trim_end_matches([',', ';']).trim();
    if t.is_empty() {
        return None;
    }
    // A field pair (top-level `:` outside any brackets) is never a
    // variant side. Struct-payload colons (`Old { f: T }`) sit inside
    // braces, so the naive `find_field_colon` (brace-blind) would
    // misreject them — use the nesting-aware probe here.
    if top_colon_nested(t).is_some() {
        return None;
    }
    // Split discriminant on the first top-level `=`.
    let (head, disc) = match split_top_level_eq(t) {
        Some((h, d)) => {
            let d = d.trim();
            if h.trim().is_empty() || d.is_empty() {
                return None;
            }
            // `==` / `=>` / `!=` are expressions, not discriminants.
            let d0 = d.chars().next().unwrap_or(' ');
            if d0 == '=' || d0 == '>' {
                return None;
            }
            (h.trim().to_string(), Some(d.to_string()))
        }
        None => (t.to_string(), None),
    };
    // Tuple payload: `Name(...)`.
    if let Some(open) = head.find('(') {
        let name = head[..open].trim();
        if !is_member_ident(name) {
            return None;
        }
        let rest = head[open..].trim();
        if !rest.ends_with(')') {
            return None;
        }
        let inner = rest[1..rest.len() - 1].trim();
        if inner.is_empty() {
            return None;
        }
        let parts = split_top_level(inner, ',')?;
        let mut tys = Vec::new();
        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            // Must parse as a type, else another shape owns this text.
            if syn::parse_str::<syn::Type>(part).is_err() {
                return None;
            }
            tys.push(part.to_string());
        }
        return Some(VariantSide {
            name: name.to_string(),
            payload: Some(PayloadRaw::Tuple(tys)),
            discriminant: disc,
        });
    }
    // Struct payload: `Name { ... }`.
    if let Some(open) = head.find('{') {
        let name = head[..open].trim();
        if !is_member_ident(name) {
            return None;
        }
        let rest = head[open..].trim();
        if !rest.ends_with('}') {
            return None;
        }
        let inner = rest[1..rest.len() - 1].trim();
        if inner.is_empty() {
            return None;
        }
        let parts = split_top_level(inner, ',')?;
        let mut fields = Vec::new();
        for part in parts {
            let part = part.trim().trim_end_matches(',').trim();
            if part.is_empty() {
                continue;
            }
            let colon = crate::pattern_classify::find_field_colon(part)?;
            let (fname, fty) = (part[..colon].trim(), part[colon + 1..].trim());
            if !is_member_ident(fname) || fty.is_empty() {
                return None;
            }
            if syn::parse_str::<syn::Type>(fty).is_err() {
                return None;
            }
            fields.push((fname.to_string(), fty.to_string()));
        }
        if fields.is_empty() {
            return None;
        }
        return Some(VariantSide {
            name: name.to_string(),
            payload: Some(PayloadRaw::Struct(fields)),
            discriminant: disc,
        });
    }
    // Bare name (+ optional discriminant).
    if !is_member_ident(head.trim()) {
        return None;
    }
    // A bare word with no payload and no discriminant is the rename pair,
    // not evolution — decline so the rename classifier owns it.
    let dexpr = disc?;
    if syn::parse_str::<syn::Expr>(&dexpr).is_err() {
        return None;
    }
    Some(VariantSide {
        name: head.trim().to_string(),
        payload: None,
        discriminant: Some(dexpr),
    })
}

fn variant_evolution(
    m: &VariantSide,
    p: &VariantSide,
    context_raw: &str,
    minus_attrs: &[String],
    plus_attrs: &[String],
    table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    check_attr_mirror(minus_attrs, plus_attrs)?;
    let Some(container) = enum_context(context_raw) else {
        return Err(variant_err(
            "variant evolution needs an `enum E {` context line naming the container (add the header as context, or use an ordinary hunk)",
        ));
    };
    if m.name != p.name {
        let msum = variant_summary(m);
        let psum = variant_summary(p);
        return Err(rename_instead(&m.name, &p.name, &msum, &psum));
    }
    match (&m.payload, &p.payload) {
        (None, None) => {
            // Discriminant evolution (same-name; even a no-op identical
            // discriminant claims evolution — the WI-11a alias precedent,
            // and the WI-1.5 plus-as-minus synth).
            match (&m.discriminant, &p.discriminant) {
                (Some(md), Some(pd)) => {
                    let (minus_sub, placeholders) = compile_rhs(md, true, table)?;
                    compile_rhs(pd, true, table)?;
                    Ok(Some(PatternKind::VariantPayloadEvolution {
                        enum_name: container,
                        variant: m.name.clone(),
                        payload: VariantPayload::Discriminant {
                            minus_sub,
                            plus_raw: pd.clone(),
                            placeholders,
                        },
                        minus_attrs: minus_attrs.to_vec(),
                        plus_attrs: plus_attrs.to_vec(),
                    }))
                }
                _ => Err(variant_err(
                    "variant discriminant evolution needs `Name = EXPR;` on both sides (or use an ordinary hunk)",
                )),
            }
        }
        (Some(mp), Some(pp)) => {
            // The other region is preserved token-equal.
            if norm_opt_expr(&m.discriminant) != norm_opt_expr(&p.discriminant) {
                return Err(variant_err(
                    "variant payload and discriminant cannot evolve together — write a payload hunk plus a discriminant hunk, or use an ordinary hunk",
                ));
            }
            match (mp, pp) {
                (PayloadRaw::Tuple(mtys), PayloadRaw::Tuple(ptys)) => {
                    if mtys.len() != ptys.len() {
                        return Err(variant_err(format!(
                            "tuple variant payload arity differs ({} vs {}); write the same arity on both sides, or use an ordinary hunk",
                            mtys.len(),
                            ptys.len()
                        )));
                    }
                    // Even an identical payload claims evolution (no-op plus
                    // re-match, WI-11a precedent).
                    let mut minus_subs = Vec::new();
                    let mut plus_raws = Vec::new();
                    let mut pmap: std::collections::BTreeMap<String, String> =
                        std::collections::BTreeMap::new();
                    for (a, b) in mtys.iter().zip(ptys.iter()) {
                        let (sub, ph) = compile_rhs(a, false, table)?;
                        for (n, p) in ph {
                            pmap.insert(n, p);
                        }
                        compile_rhs(b, false, table)?;
                        minus_subs.push(sub);
                        plus_raws.push(b.clone());
                    }
                    Ok(Some(PatternKind::VariantPayloadEvolution {
                        enum_name: container,
                        variant: m.name.clone(),
                        payload: VariantPayload::Tuple {
                            minus_subs,
                            plus_raws,
                            placeholders: pmap.into_iter().collect(),
                        },
                        minus_attrs: minus_attrs.to_vec(),
                        plus_attrs: plus_attrs.to_vec(),
                    }))
                }
                (PayloadRaw::Struct(mfs), PayloadRaw::Struct(pfs)) => {
                    let mnames: Vec<&str> = mfs.iter().map(|(n, _)| n.as_str()).collect();
                    let pnames: Vec<&str> = pfs.iter().map(|(n, _)| n.as_str()).collect();
                    if mnames != pnames {
                        return Err(variant_err(
                            "struct variant payload field names differ; evolution preserves field names — write the same fields on both sides, or use an ordinary hunk",
                        ));
                    }
                    // Even an identical payload claims evolution (no-op plus
                    // re-match, WI-11a precedent).
                    let mut minus_subs = Vec::new();
                    let mut plus_raws = Vec::new();
                    let mut pmap: std::collections::BTreeMap<String, String> =
                        std::collections::BTreeMap::new();
                    for ((_, a), (_, b)) in mfs.iter().zip(pfs.iter()) {
                        let (sub, ph) = compile_rhs(a, false, table)?;
                        for (n, p) in ph {
                            pmap.insert(n, p);
                        }
                        compile_rhs(b, false, table)?;
                        minus_subs.push(sub);
                        plus_raws.push(b.clone());
                    }
                    Ok(Some(PatternKind::VariantPayloadEvolution {
                        enum_name: container,
                        variant: m.name.clone(),
                        payload: VariantPayload::Struct {
                            fields: mnames.into_iter().map(|s| s.to_string()).collect(),
                            minus_subs,
                            plus_raws,
                            placeholders: pmap.into_iter().collect(),
                        },
                        minus_attrs: minus_attrs.to_vec(),
                        plus_attrs: plus_attrs.to_vec(),
                    }))
                }
                _ => Err(variant_err(
                    "variant payload kinds differ (tuple vs struct); write the same shape on both sides, or use an ordinary hunk",
                )),
            }
        }
        _ => Err(variant_err(
            "variant evolution sides must carry the same shape (`Name(...)`, `Name { ... }`, or `Name = ...` on both sides; or use an ordinary hunk)",
        )),
    }
}

fn variant_summary(v: &VariantSide) -> String {
    match (&v.payload, &v.discriminant) {
        (Some(PayloadRaw::Tuple(t)), Some(d)) => {
            format!("tuple(..) = {d} [{} types]", t.len())
        }
        (Some(PayloadRaw::Tuple(t)), None) => format!("tuple(..) [{} types]", t.len()),
        (Some(PayloadRaw::Struct(f)), Some(d)) => {
            format!("struct{{..}} = {d} [{} fields]", f.len())
        }
        (Some(PayloadRaw::Struct(f)), None) => format!("struct{{..}} [{} fields]", f.len()),
        (None, Some(d)) => format!("= {d}"),
        (None, None) => "bare".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Shared template + error helpers
// ---------------------------------------------------------------------------

fn compile_rhs(
    rhs: &str,
    is_expr: bool,
    table: &MetavarTable,
) -> Result<(String, Vec<(String, String)>), StructuredError> {
    let err = |msg: String, template: &str| {
        StructuredError::new(rsmpl_patch::ErrorCode::PatternConflict, msg).with_repair(
            StructuredError::fix_pattern_repair("member-evolution", template),
        )
    };
    let (masked, sentinels) = crate::quote_utils::mask_literal_spans(rhs).map_err(|e| {
        err(
            format!("member evolution mask failed: {e}"),
            "write the full RHS",
        )
    })?;
    if masked.contains("...") {
        return Err(err(
            "ellipsis is not supported in member evolution templates in v1 (write the full RHS)"
                .to_string(),
            "write the full RHS without `...`",
        ));
    }
    let (mut sub, maps) = crate::expr_pattern::ExprPattern::placeholder_expr(&masked, table);
    sub = crate::quote_utils::unmask_literal_spans(&sub, &sentinels);
    if is_expr {
        syn::parse_str::<syn::Expr>(&sub).map_err(|e| {
            err(
                format!("member evolution value `{rhs}` does not parse as a Rust expression: {e}"),
                "write a Rust expression template for the value",
            )
        })?;
    } else {
        syn::parse_str::<syn::Type>(&sub).map_err(|e| {
            err(
                format!("member evolution type `{rhs}` does not parse as a Rust type: {e}"),
                "write a Rust type template for the type",
            )
        })?;
    }
    let placeholders: Vec<(String, String)> = maps
        .expr_placeholders
        .into_iter()
        .map(|(ph, n)| (n, ph))
        .collect();
    Ok((sub, placeholders))
}

fn norm_ty(ty: &syn::Type) -> String {
    quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn norm_expr(s: &str) -> String {
    syn::parse_str::<syn::Expr>(s)
        .map(|e| {
            quote::quote!(#e)
                .to_string()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|_| s.split_whitespace().collect())
}

fn norm_opt_expr(s: &Option<String>) -> String {
    s.as_deref().map(norm_expr).unwrap_or_default()
}

fn check_attr_mirror(minus: &[String], plus: &[String]) -> Result<(), StructuredError> {
    if !plus.is_empty() && minus.is_empty() {
        return Err(StructuredError::new(
            rsmpl_patch::ErrorCode::PatternConflict,
            "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither",
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C11",
            "write the attributes on both sides or neither",
        )));
    }
    let norm = |ss: &[String]| {
        ss.iter()
            .map(|a| a.split_whitespace().collect::<String>())
            .collect::<Vec<_>>()
    };
    if !minus.is_empty() && norm(minus) != norm(plus) {
        return Err(StructuredError::new(
            rsmpl_patch::ErrorCode::PatternConflict,
            "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C12",
            "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
        )));
    }
    Ok(())
}

fn rename_instead(mname: &str, pname: &str, mrhs: &str, phs: &str) -> StructuredError {
    if mrhs.trim() == phs.trim() {
        field_err(format!(
            "member names differ (`{mname}` vs `{pname}`) with the same RHS; renames are ident-only — write the bare pair under the same container header"
        ))
    } else {
        field_err(format!(
            "member names differ (`{mname}` vs `{pname}`) and the RHS differs; rename and evolution never combine — write a bare rename hunk plus an evolution hunk, or use an ordinary hunk"
        ))
    }
}

fn field_err(msg: impl Into<String>) -> StructuredError {
    StructuredError::new(
        rsmpl_patch::ErrorCode::PatternConflict,
        msg.into(),
    )
    .with_repair(StructuredError::fix_pattern_repair(
        "member-evolution",
        "write a member pair (`- name: Type` / `+ name: Type`, same name both sides) under a `struct S {` context line, or use an ordinary hunk",
    ))
}

fn variant_err(msg: impl Into<String>) -> StructuredError {
    StructuredError::new(
        rsmpl_patch::ErrorCode::PatternConflict,
        msg.into(),
    )
    .with_repair(StructuredError::fix_pattern_repair(
        "member-evolution",
        "write a variant pair (`- Old(...)` / `+ Old(...)`, same name both sides) under an `enum E {` context line, or use an ordinary hunk",
    ))
}

fn split_member_vis(s: &str) -> (&str, &str) {
    if let Some(rest) = s.strip_prefix("pub") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return ("pub", s["pub".len()..].trim_start());
        }
        if let Some(after) = rest.trim_start().strip_prefix('(') {
            let mut depth = 1i32;
            for (i, c) in after.char_indices() {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        return ("pub(...)", after[i + 1..].trim_start());
                    }
                }
            }
        }
    }
    ("", s)
}

/// Byte index of the field-separator colon at bracket depth 0 (path
/// `::` never counts, `[...]`/quotes opaque). `None` when the side
/// carries no top-level separator — struct-payload colons live inside
/// braces and never count here.
fn top_colon_nested(side: &str) -> Option<usize> {
    let bytes = side.as_bytes();
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let mut ins = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        let c = b as char;
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
        match c {
            '"' => ins = true,
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
                let prev = i.checked_sub(1).map(|j| bytes[j]);
                let next = bytes.get(i + 1).copied();
                if prev != Some(b':') && next != Some(b':') {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split `text` on top-level `sep`, respecting `()[]{}<>` nesting and
/// `"..."` strings. `None` on unbalanced delimiters.
fn split_top_level(text: &str, sep: char) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    let mut depth_angle = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut cur = String::new();
    for c in text.chars() {
        if in_str {
            cur.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '(' => {
                depth_paren += 1;
                cur.push(c);
            }
            ')' => {
                depth_paren -= 1;
                if depth_paren < 0 {
                    return None;
                }
                cur.push(c);
            }
            '[' => {
                depth_brack += 1;
                cur.push(c);
            }
            ']' => {
                depth_brack -= 1;
                if depth_brack < 0 {
                    return None;
                }
                cur.push(c);
            }
            '{' => {
                depth_brace += 1;
                cur.push(c);
            }
            '}' => {
                depth_brace -= 1;
                if depth_brace < 0 {
                    return None;
                }
                cur.push(c);
            }
            '<' => {
                depth_angle += 1;
                cur.push(c);
            }
            '>' => {
                if depth_angle > 0 {
                    depth_angle -= 1;
                }
                cur.push(c);
            }
            _ if c == sep
                && depth_paren == 0
                && depth_brack == 0
                && depth_brace == 0
                && depth_angle == 0 =>
            {
                out.push(cur);
                cur = String::new();
            }
            _ => cur.push(c),
        }
    }
    if in_str || depth_paren != 0 || depth_brack != 0 || depth_brace != 0 {
        return None;
    }
    out.push(cur);
    Some(out)
}

/// Split on the first top-level `=` (path `==`/`=>`/`!=` excluded by the
/// caller). `None` when no top-level `=` exists.
fn split_top_level_eq(text: &str) -> Option<(String, String)> {
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut depth_brace = 0i32;
    let mut depth_angle = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let bytes: Vec<char> = text.chars().collect();
    for (i, &c) in bytes.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        let at_top = depth_paren == 0 && depth_brack == 0 && depth_brace == 0 && depth_angle == 0;
        // A top-level `=` splits (expression spellings `==`/`!=`/`<=`/
        // `>=`/`=>` excluded below); anything nested only tracks depth.
        if c == '=' && at_top && !in_str {
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
            '"' => in_str = true,
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            '[' => depth_brack += 1,
            ']' => depth_brack -= 1,
            '{' => depth_brace += 1,
            '}' => depth_brace -= 1,
            '<' => depth_angle += 1,
            '>' if depth_angle > 0 => depth_angle -= 1,
            '>' => {}
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Member addition (WI-12b slice 1)
// ---------------------------------------------------------------------------

/// Classify member-context addition: minus member set empty, plus member
/// set exactly one field/variant line, container header present.
///
/// Called before `classify_member_evolution` (which rejects minus-empty
/// shapes loud). Returns `Ok(None)` for every non-addition shape —
/// including malformed plus lines inside a member container, which stay
/// ordinary/`Insertion` rather than erroring here (the evolution
/// classifier below still owns malformed *pairs*).
pub(crate) fn classify_member_addition(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    _table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    let struct_c = struct_context(context_raw);
    let enum_c = enum_context(context_raw);
    if struct_c.is_none() && enum_c.is_none() {
        return Ok(None);
    }
    let minus_stripped = strip_container_headers(minus_raw);
    let plus_stripped = strip_container_headers(plus_raw);
    let (minus_attrs, minus_pair) = crate::pattern_general::split_leading_attrs(&minus_stripped);
    let (plus_attrs, plus_pair) = crate::pattern_general::split_leading_attrs(&plus_stripped);
    // Addition owns minus-empty shapes only; anything with a minus
    // member (or minus attrs) belongs to evolution/rename/ordinary.
    if !minus_attrs.is_empty() || !minus_pair.trim().is_empty() {
        return Ok(None);
    }
    let plus_pair = plus_pair.trim().to_string();
    if plus_pair.is_empty() || plus_pair.lines().count() != 1 {
        return Ok(None);
    }
    // Struct container: the plus line must parse as a field side.
    if let Some(container) = struct_c {
        let Some(side) = parse_field_side(&plus_pair)? else {
            return Ok(None);
        };
        return Ok(Some(PatternKind::FieldAddition {
            struct_name: container,
            field: side.name.clone(),
            plus_raw: plus_pair,
            minus_attrs: Vec::new(),
            plus_attrs,
        }));
    }
    // Enum container: the plus line must parse as a variant side.
    // `parse_variant_side` declines bare unit names (rename-owned in
    // the evolution classifier) — for addition a bare `C` is the
    // common case, so accept a validated unit name as fallback.
    // Anything else it declines (malformed payloads, field lines)
    // stays unclaimed here.
    if let Some(container) = enum_c {
        if let Some(side) = parse_variant_side(&plus_pair) {
            return Ok(Some(PatternKind::VariantAddition {
                enum_name: container,
                variant: side.name.clone(),
                plus_raw: plus_pair,
                minus_attrs: Vec::new(),
                plus_attrs,
            }));
        }
        let bare = plus_pair.trim_end_matches([',', ';']).trim();
        if !bare.is_empty() && is_member_ident(bare) {
            return Ok(Some(PatternKind::VariantAddition {
                enum_name: container,
                variant: bare.to_string(),
                plus_raw: plus_pair,
                minus_attrs: Vec::new(),
                plus_attrs,
            }));
        }
        return Ok(None);
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Match-arm addition (WI-12d)
// ---------------------------------------------------------------------------

/// Classify match-arm addition: minus-empty plus-single under a
/// `match <scrut> {` context line, the plus line parsing as a
/// `syn::Arm` whose pattern names a variant (last path segment,
/// optional single-segment enum prefix).
///
/// Called after `classify_member_addition` (which owns minus-empty
/// shapes under struct/enum headers). Returns `Ok(None)` for every
/// non-arm shape — including malformed arms inside a match context,
/// which stay ordinary/`Insertion` rather than erroring here (the
/// fallthrough keeps today's errors byte-identical).
pub(crate) fn classify_match_arm_addition(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    _table: &MetavarTable,
) -> Result<Option<PatternKind>, StructuredError> {
    // Like member containers, the `match <scrut> {` header lives in
    // the minus/plus texts (context lines ride both sides): strip it
    // before the minus-empty gate, mirroring `strip_container_headers`.
    let minus_stripped = strip_match_headers(minus_raw);
    let plus_stripped = strip_match_headers(plus_raw);
    let (minus_attrs, minus_pair) = crate::pattern_general::split_leading_attrs(&minus_stripped);
    let (plus_attrs, plus_pair) = crate::pattern_general::split_leading_attrs(&plus_stripped);
    // Addition owns minus-empty shapes only; anything with a minus
    // arm (or minus attrs) belongs elsewhere.
    if !minus_attrs.is_empty() || !minus_pair.trim().is_empty() {
        return Ok(None);
    }
    let plus_pair = plus_pair.trim().to_string();
    if plus_pair.is_empty() || plus_pair.lines().count() != 1 {
        return Ok(None);
    }
    let Some(scrutinee) = match_context_scrutinee(context_raw) else {
        return Ok(None);
    };
    let arm: syn::Arm = match syn::parse_str(&plus_pair) {
        Ok(a) => a,
        Err(_) => return Ok(None),
    };
    let Some((prefix, variant)) = arm_variant(&arm) else {
        return Ok(None);
    };
    Ok(Some(PatternKind::ArmAddition {
        enum_prefix: prefix,
        variant,
        arm_raw: plus_pair,
        scrutinee,
        minus_attrs: Vec::new(),
        plus_attrs,
    }))
}

/// Drop `match <scrut> {` header lines (the arm-addition context
/// spelling) from a minus/plus text, keeping arm lines and attrs.
fn strip_match_headers(text: &str) -> String {
    text.lines()
        .filter(|l| match_context_scrutinee(l).is_none())
        .collect::<Vec<_>>()
        .join("\n")
}

/// First `match <scrut> {` context line, scrutinee token-normalised.
/// `None` when no context line opens a match.
fn match_context_scrutinee(context_raw: &str) -> Option<String> {
    for line in context_raw.lines() {
        let t = line.trim();
        if !t.starts_with("match") || !t.ends_with('{') {
            continue;
        }
        // `match <scrut> {` closes to `match <scrut> {}` (empty arms
        // parse; only the scrutinee is read).
        let probe = format!("{t}}}");
        if let Ok(syn::Expr::Match(m)) = syn::parse_str::<syn::Expr>(&probe) {
            return Some(norm_arm_expr(&m.expr));
        }
    }
    None
}

/// The variant an arm pattern names: last path segment plus the
/// optional single-segment enum prefix (`E::C` → (`E`, `C`);
/// bare `C` → (`None`, `C`)). Tuple/struct payloads share the path
/// rule. Multi-segment paths (`a::E::C`) need symbol resolution
/// (WI-16) and decline; leading-`::` and `qself` decline likewise.
/// Bare patterns must start uppercase — a lowercase bare ident is a
/// binding (`x => …`), not a variant.
fn arm_variant(arm: &syn::Arm) -> Option<(Option<String>, String)> {
    // A lone ident parses as `Pat::Ident` (binding position), not
    // `Pat::Path`: a bare uppercase arm (`C => …`) is the glob-import
    // spelling of a unit variant; a lowercase one is a catch-all
    // binding and declines.
    if let syn::Pat::Ident(pi) = &arm.pat {
        if pi.by_ref.is_some() || pi.mutability.is_some() || pi.subpat.is_some() {
            return None;
        }
        let name = pi.ident.to_string();
        if !is_member_ident(&name) || !starts_upper(&name) {
            return None;
        }
        return Some((None, name));
    }
    let path = match &arm.pat {
        syn::Pat::Path(pp) => {
            if pp.qself.is_some() {
                return None;
            }
            &pp.path
        }
        syn::Pat::TupleStruct(ps) => &ps.path,
        syn::Pat::Struct(ps) => &ps.path,
        _ => return None,
    };
    if path.leading_colon.is_some() {
        return None;
    }
    let last = path.segments.last()?.ident.to_string();
    if !is_member_ident(&last) || !starts_upper(&last) {
        return None;
    }
    match path.segments.len() {
        1 => Some((None, last)),
        2 => {
            let first = path.segments.first()?.ident.to_string();
            if !is_member_ident(&first) {
                return None;
            }
            Some((Some(first), last))
        }
        _ => None,
    }
}

fn starts_upper(s: &str) -> bool {
    s.strip_prefix("r#")
        .unwrap_or(s)
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
}

fn norm_arm_expr(e: &syn::Expr) -> String {
    quote::quote!(#e)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
