//! Associated-item rename pairs (`type`/`const` in `trait`/`impl` contexts).
//!
//! WI-13 slice 1: rename authoring only (definition ident-span edit).
//! Evolution, `# follow=uses`, and raising land in WI-26/27/28.

use rsmpl_patch::{ErrorCode, RsmplHunk, StructuredError};

use crate::metavar::MetavarTable;
use crate::pattern::PatternKind;

/// `type` or `const` associated item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssocKind {
    Type,
    Const,
}

impl AssocKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Const => "const",
        }
    }
}

/// Token-normalized rendering (authored spacing never matters).
fn norm_tokens(t: &impl quote::ToTokens) -> String {
    quote::quote!(#t)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when `text` has an `impl ... {` header line or a `trait ...` line.
fn has_impl_or_trait_header(context_raw: &str) -> bool {
    context_raw.lines().any(|l| {
        let t = l.trim();
        (t.starts_with("impl ") || t.starts_with("impl<") || t.starts_with("unsafe impl"))
            && t.ends_with('{')
            || {
                let s = t.strip_prefix("unsafe ").unwrap_or(t);
                s.starts_with("trait ") && t.ends_with('{')
            }
    })
}

/// First context line that parses as an `impl`/`trait` block header.
fn assoc_context_header(context_raw: &str) -> Option<String> {
    for line in context_raw.lines() {
        let t = line.trim();
        if !t.ends_with('{') {
            continue;
        }
        if syn::parse_str::<syn::Item>(&format!("{t}}}")).is_ok() {
            let item: syn::Item = syn::parse_str(&format!("{t}}}")).ok()?;
            if matches!(item, syn::Item::Impl(_) | syn::Item::Trait(_)) {
                return Some(line.trim().to_string());
            }
        }
    }
    None
}

/// Strip container-header lines (`impl ... {` / `trait ... {`) from pair text.
/// A pair line never ends with `{`, so the strip cannot eat the pair.
fn strip_assoc_headers(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let t = l.trim();
            if !t.ends_with('{') {
                return true;
            }
            !syn::parse_str::<syn::Item>(&format!("{t}}}"))
                .is_ok_and(|it| matches!(it, syn::Item::Impl(_) | syn::Item::Trait(_)))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One parsed assoc side: kind, name, normalized remainder (everything
/// except the name ident, token-normalized).
struct AssocSide {
    kind: AssocKind,
    name: String,
    rest: String,
}

fn parse_assoc_side(text: &str) -> Result<Option<AssocSide>, StructuredError> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let err = |msg: String| {
        StructuredError::new(ErrorCode::PatternConflict, msg).with_repair(
            StructuredError::fix_pattern_repair(
                "assoc",
                "write an assoc pair (`- type Item;` / `+ type Output;`, `- const MAX: i32;` / `+ const RENAMED: i32;`) under a `trait T {` or `impl ... {` context line, or use an ordinary hunk",
            ),
        )
    };
    // `type` sides: try trait-form then impl-form.
    if t.starts_with("type ") || t.starts_with("type\t") {
        if let Ok(it) = syn::parse_str::<syn::TraitItemType>(t) {
            let rest = format!(
                "{}|{}|{}",
                norm_tokens(&it.generics),
                it.colon_token
                    .as_ref()
                    .map(|_| norm_tokens(&it.bounds))
                    .unwrap_or_default(),
                it.default
                    .as_ref()
                    .map(|(_, ty)| norm_tokens(ty))
                    .unwrap_or_default(),
            );
            return Ok(Some(AssocSide {
                kind: AssocKind::Type,
                name: it.ident.to_string(),
                rest,
            }));
        }
        if let Ok(it) = syn::parse_str::<syn::ImplItemType>(t) {
            let rest = format!("{}|{}", norm_tokens(&it.generics), norm_tokens(&it.ty),);
            return Ok(Some(AssocSide {
                kind: AssocKind::Type,
                name: it.ident.to_string(),
                rest,
            }));
        }
        return Err(err(format!(
            "assoc `type` side `{t}` does not parse as a trait or impl associated type"
        )));
    }
    // `const` sides: try trait-form then impl-form. A `const fn` (or
    // qualified `const unsafe/async/extern fn`) method header is not an
    // assoc pair — fall through to the method-context router instead of
    // failing as a malformed const.
    if let Some(stripped) = t.strip_prefix("const ") {
        let after = stripped.trim_start();
        let head = after
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        if matches!(head, "fn" | "unsafe" | "async" | "extern") {
            return Ok(None);
        }
        if let Ok(it) = syn::parse_str::<syn::TraitItemConst>(t) {
            let rest = format!(
                "{}|{}",
                norm_tokens(&it.ty),
                it.default
                    .as_ref()
                    .map(|(_, e)| norm_tokens(e))
                    .unwrap_or_default(),
            );
            return Ok(Some(AssocSide {
                kind: AssocKind::Const,
                name: it.ident.to_string(),
                rest,
            }));
        }
        if let Ok(it) = syn::parse_str::<syn::ImplItemConst>(t) {
            let rest = format!("{}|{}", norm_tokens(&it.ty), norm_tokens(&it.expr),);
            return Ok(Some(AssocSide {
                kind: AssocKind::Const,
                name: it.ident.to_string(),
                rest,
            }));
        }
        return Err(err(format!(
            "assoc `const` side `{t}` does not parse as a trait or impl associated const"
        )));
    }
    Ok(None)
}

/// One parsed assoc evolution side: kind, name, preserved prefix
/// (generics|bounds for `type`, ascribed type for `const`), and the
/// optional RHS/default (absent for `type Item;` / `const MAX: i32;`).
struct AssocEvoSide {
    kind: AssocKind,
    name: String,
    prefix: String,
    rhs: Option<String>,
}

/// Parse one `type` evolution side into (name, prefix, rhs).
/// `None` when the side is not a `type` assoc shape at all.
fn parse_type_evo_side(
    text: &str,
) -> Result<Option<(String, String, Option<String>)>, StructuredError> {
    let t = text.trim();
    if !(t.starts_with("type ") || t.starts_with("type\t")) {
        return Ok(None);
    }
    if let Ok(it) = syn::parse_str::<syn::TraitItemType>(t) {
        let prefix = format!(
            "{}|{}",
            norm_tokens(&it.generics),
            it.colon_token
                .as_ref()
                .map(|_| norm_tokens(&it.bounds))
                .unwrap_or_default(),
        );
        let rhs = it.default.as_ref().and(raw_rhs(t));
        return Ok(Some((it.ident.to_string(), prefix, rhs)));
    }
    if let Ok(it) = syn::parse_str::<syn::ImplItemType>(t) {
        return Ok(Some((
            it.ident.to_string(),
            norm_tokens(&it.generics),
            raw_rhs(t),
        )));
    }
    Err(StructuredError::new(
        ErrorCode::PatternConflict,
        format!("assoc `type` side `{t}` does not parse as a trait or impl associated type"),
    )
    .with_repair(StructuredError::fix_pattern_repair(
        "assoc",
        "write an assoc evolution pair (`- type Item = Vec<T>;` / `+ type Item = Box<[T]>;`) under a `trait T {` or `impl ... {` context line, or use an ordinary hunk",
    )))
}

/// Parse one `const` evolution side into (name, ascribed type, rhs).
/// `None` when the side is not a `const` assoc shape at all.
fn parse_const_evo_side(
    text: &str,
) -> Result<Option<(String, String, Option<String>)>, StructuredError> {
    let t = text.trim();
    let Some(stripped) = t.strip_prefix("const ") else {
        return Ok(None);
    };
    let after = stripped.trim_start();
    let head = after
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    if matches!(head, "fn" | "unsafe" | "async" | "extern") {
        return Ok(None);
    }
    if let Ok(it) = syn::parse_str::<syn::TraitItemConst>(t) {
        let ty = norm_tokens(&it.ty);
        let rhs = it.default.as_ref().and(raw_rhs(t));
        return Ok(Some((it.ident.to_string(), ty, rhs)));
    }
    if let Ok(it) = syn::parse_str::<syn::ImplItemConst>(t) {
        return Ok(Some((
            it.ident.to_string(),
            norm_tokens(&it.ty),
            raw_rhs(t),
        )));
    }
    Err(StructuredError::new(
        ErrorCode::PatternConflict,
        format!("assoc `const` side `{t}` does not parse as a trait or impl associated const"),
    )
    .with_repair(StructuredError::fix_pattern_repair(
        "assoc",
        "write an assoc evolution pair (`- const MAX: i32 = E;` / `+ const MAX: i32 = E + 1;`) under a `trait T {` or `impl ... {` context line, or use an ordinary hunk",
    )))
}

/// Compile one assoc RHS through the placeholder pipeline; returns the
/// substituted minus text plus the (name → placeholder) map. The plus
/// side is validated the same way but stored raw for match-time
/// substitution. `...` is rejected (v1 templates carry no ellipsis).
fn compile_assoc_rhs(
    rhs: &str,
    table: &MetavarTable,
    is_expr: bool,
) -> Result<(String, Vec<(String, String)>), StructuredError> {
    let (masked, sentinels) = crate::quote_utils::mask_literal_spans(rhs)?;
    if masked.contains("...") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "ellipsis is not supported in assoc evolution RHS templates in v1 (write the full RHS)",
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "assoc",
            "write the full RHS without `...`",
        )));
    }
    let (mut sub, maps) = crate::expr_pattern::ExprPattern::placeholder_expr(&masked, table);
    sub = crate::quote_utils::unmask_literal_spans(&sub, &sentinels);
    if is_expr {
        syn::parse_str::<syn::Expr>(&sub).map_err(|e| {
            StructuredError::new(
                ErrorCode::PatternConflict,
                format!("assoc const value `{rhs}` does not parse as a Rust expression: {e}"),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "assoc",
                "write a Rust expression template for the value",
            ))
        })?;
    } else {
        syn::parse_str::<syn::Type>(&sub).map_err(|e| {
            StructuredError::new(
                ErrorCode::PatternConflict,
                format!("assoc type RHS `{rhs}` does not parse as a Rust type: {e}"),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "assoc",
                "write a Rust type template for the RHS",
            ))
        })?;
    }
    Ok((
        sub,
        maps.expr_placeholders
            .into_iter()
            .map(|(ph, n)| (n, ph))
            .collect(),
    ))
}

fn norm_attr(s: &str) -> String {
    s.split_whitespace().collect()
}

/// Raw RHS text after the last `=` (trailing `;` stripped, trimmed).
/// Author spacing is preserved for emission; parsing validates it.
fn raw_rhs(text: &str) -> Option<String> {
    let t = text.trim();
    let eq = t.rfind('=')?;
    let mut rhs = t[eq + 1..].trim();
    if let Some(stripped) = rhs.strip_suffix(';') {
        rhs = stripped.trim();
    }
    if rhs.is_empty() {
        None
    } else {
        Some(rhs.to_string())
    }
}

fn assoc_evo_err(msg: impl Into<String>, write_instead: &str) -> StructuredError {
    StructuredError::new(ErrorCode::PatternConflict, msg.into())
        .with_repair(StructuredError::fix_pattern_repair("assoc", write_instead))
}

/// Classify an associated-item evolution (SPEC 13.4 / 14.8, WI-26).
///
/// Same header + pair spelling as renames, but the name is concrete
/// and preserved while the RHS/default is a template: `type Item =
/// Vec<T>;` → `type Item = Box<[T]>;`, `const MAX: i32 = E;` →
/// `const MAX: i32 = E + 1;`. Returns `Ok(None)` when neither side
/// carries the evolution shape (renames, bare decls, and foreign
/// shapes fall through untouched). A claimed shape with a malformed
/// half is a loud `PatternConflict` (SPEC 2.6).
pub(crate) fn classify_assoc_evolution(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    table: &MetavarTable,
    hunk: &RsmplHunk,
) -> Result<Option<PatternKind>, StructuredError> {
    if !has_impl_or_trait_header(context_raw) {
        return Ok(None);
    }
    let minus_stripped = strip_assoc_headers(minus_raw);
    let plus_stripped = strip_assoc_headers(plus_raw);
    if minus_stripped.trim().lines().count() != 1 || plus_stripped.trim().lines().count() != 1 {
        return Ok(None);
    }
    let (minus_attrs, minus_pair) = crate::pattern_general::split_leading_attrs(&minus_stripped);
    let (plus_attrs, plus_pair) = crate::pattern_general::split_leading_attrs(&plus_stripped);
    let minus_pair = minus_pair.trim().to_string();
    let plus_pair = plus_pair.trim().to_string();
    // Shape probe: at least one side must name a `type`/`const` assoc
    // shape with an `=` (a bare `type Item;` decl carries no RHS to
    // evolve and stays with the rename classifier).
    let minus_is_assoc = minus_pair.starts_with("type ")
        || minus_pair.starts_with("type\t")
        || minus_pair.starts_with("const ");
    let plus_is_assoc = plus_pair.starts_with("type ")
        || plus_pair.starts_with("type\t")
        || plus_pair.starts_with("const ");
    if !minus_is_assoc && !plus_is_assoc {
        return Ok(None);
    }
    let m_ty = parse_type_evo_side(&minus_pair)?;
    let p_ty = parse_type_evo_side(&plus_pair)?;
    let m_c = parse_const_evo_side(&minus_pair)?;
    let p_c = parse_const_evo_side(&plus_pair)?;
    let m_side: Option<AssocEvoSide> = match (m_ty, m_c) {
        (Some((name, prefix, rhs)), None) => Some(AssocEvoSide {
            kind: AssocKind::Type,
            name,
            prefix,
            rhs,
        }),
        (None, Some((name, ty, rhs))) => Some(AssocEvoSide {
            kind: AssocKind::Const,
            name,
            prefix: ty,
            rhs,
        }),
        (None, None) => None,
        _ => {
            return Err(assoc_evo_err(
                "assoc evolution side parses as both `type` and `const` (write one assoc pair)",
                "write one assoc pair",
            ));
        }
    };
    let p_side: Option<AssocEvoSide> = match (p_ty, p_c) {
        (Some((name, prefix, rhs)), None) => Some(AssocEvoSide {
            kind: AssocKind::Type,
            name,
            prefix,
            rhs,
        }),
        (None, Some((name, ty, rhs))) => Some(AssocEvoSide {
            kind: AssocKind::Const,
            name,
            prefix: ty,
            rhs,
        }),
        (None, None) => None,
        _ => {
            return Err(assoc_evo_err(
                "assoc evolution side parses as both `type` and `const` (write one assoc pair)",
                "write one assoc pair",
            ));
        }
    };
    match (m_side, p_side) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Ok(None),
        (Some(m), Some(p)) => {
            if m.kind != p.kind {
                return Ok(None);
            }
            if m.name != p.name {
                return Ok(None);
            }
            if m.prefix != p.prefix {
                let what = if m.kind == AssocKind::Type {
                    "generics/bounds"
                } else {
                    "ascribed type"
                };
                return Err(assoc_evo_err(
                    format!(
                        "assoc evolution preserves {what} (`{}` vs `{}`); write the same {what} on both sides, or use an ordinary hunk",
                        m.prefix, p.prefix
                    ),
                    "write the same generics/bounds (type) or ascribed type (const) on both sides",
                ));
            }
            match (m.rhs, p.rhs) {
                (None, None) => Ok(None),
                (Some(_), None) | (None, Some(_)) => Err(assoc_evo_err(
                    "assoc evolution needs an RHS/default on both sides (a bare decl carries nothing to evolve; renames use the bare pair with a new name)",
                    "write `type NAME = RHS;` / `const NAME: TY = EXPR;` on both sides, or the bare rename pair",
                )),
                (Some(mrhs), Some(prhs)) => {
                    if !minus_attrs.is_empty() || !plus_attrs.is_empty() {
                        if !plus_attrs.is_empty() && minus_attrs.is_empty() {
                            return Err(assoc_evo_err(
                                "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither",
                                "write the attributes on both sides or neither",
                            ));
                        }
                        if hunk.attrs_policy() != rsmpl_patch::AttrsPolicy::Strip
                            && minus_attrs.iter().map(|a| norm_attr(a)).collect::<Vec<_>>()
                                != plus_attrs.iter().map(|a| norm_attr(a)).collect::<Vec<_>>()
                        {
                            return Err(assoc_evo_err(
                                "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
                                "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
                            ));
                        }
                    }
                    let Some(header) = assoc_context_header(context_raw) else {
                        return Err(assoc_evo_err(
                            "assoc evolution pair needs a `trait T {` or `impl ... {` context line naming the block (add the header as context, or use an ordinary hunk)",
                            "add a `trait T {` or `impl ... {` context line naming the block, or use an ordinary hunk",
                        ));
                    };
                    let want_trait = {
                        let t = header.trim_start();
                        t.starts_with("trait ") || t.starts_with("unsafe trait ")
                    };
                    let is_type = m.kind == AssocKind::Type;
                    let (minus_sub, placeholders) = compile_assoc_rhs(&mrhs, table, !is_type)?;
                    compile_assoc_rhs(&prhs, table, !is_type)?;
                    Ok(Some(PatternKind::AssocEvolution {
                        impl_header: header,
                        want_trait,
                        is_type,
                        name: m.name,
                        ty: if is_type { String::new() } else { m.prefix },
                        minus_sub,
                        plus_raw: prhs,
                        placeholders,
                        minus_attrs,
                        plus_attrs,
                    }))
                }
            }
        }
    }
}

/// Classify an associated-item rename (SPEC 13.4 / 14.8).
///
/// Shape-first, context-second: both sides must parse as the same
/// assoc-pair shape before any context line is consulted, so ordinary
/// hunks merely sitting inside a trait/impl body never route here.
pub(crate) fn classify_assoc_rename(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
) -> Result<Option<PatternKind>, StructuredError> {
    if !has_impl_or_trait_header(context_raw) {
        return Ok(None);
    }
    let minus_stripped = strip_assoc_headers(minus_raw);
    let plus_stripped = strip_assoc_headers(plus_raw);
    let (minus_attrs, minus_pair) = crate::pattern_general::split_leading_attrs(&minus_stripped);
    let (plus_attrs, plus_pair) = crate::pattern_general::split_leading_attrs(&plus_stripped);
    let minus = parse_assoc_side(&minus_pair)?;
    let plus = parse_assoc_side(&plus_pair)?;
    match (minus, plus) {
        (Some(m), Some(p)) => {
            if m.kind != p.kind {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "assoc rename kind mismatch: minus is `{}` but plus is `{}` (kinds must match)",
                        m.kind.as_str(),
                        p.kind.as_str()
                    ),
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "assoc",
                    "write the same assoc kind on both sides",
                )));
            }
            if m.name == p.name {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "assoc rename minus and plus names are identical (`{}`); write the new name on the `+` side",
                        m.name
                    ),
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "assoc",
                    "write the new name on the `+` side",
                )));
            }
            if m.rest != p.rest {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "assoc renames are ident-only in v1: bounds, generics, types and values must match on both sides (write the same suffix, evolve the RHS with an assoc evolution hunk, or use an ordinary hunk)",
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "assoc",
                    "write the same suffix on both sides, or use an ordinary hunk",
                )));
            }
            let Some(header) = assoc_context_header(context_raw) else {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "assoc rename pair needs a `trait T {` or `impl ... {` context line naming the block (add the header as context, or use an ordinary hunk)",
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "assoc",
                    "add a `trait T {` or `impl ... {` context line naming the block, or use an ordinary hunk",
                )));
            };
            let want_trait = {
                let t = header.trim_start();
                t.starts_with("trait ") || t.starts_with("unsafe trait ")
            };
            Ok(Some(PatternKind::AssocRename {
                impl_header: header,
                want_trait,
                is_type: m.kind == AssocKind::Type,
                minus_name: m.name,
                plus_name: p.name,
                minus_attrs,
                plus_attrs,
            }))
        }
        (Some(_), None) | (None, Some(_)) => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "assoc rename sides must both be `type` or both be `const` pairs with the same suffix (or use an ordinary hunk)",
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "assoc",
            "write both sides as `type` or both as `const` pairs with the same suffix, or use an ordinary hunk",
        ))),
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cls(m: &str, p: &str, c: &str) -> Result<Option<PatternKind>, String> {
        classify_assoc_rename(m, p, c).map_err(|e| e.to_string())
    }

    #[test]
    fn fires_trait_type() {
        let k = cls(
            "trait Tr {\ntype Item;",
            "trait Tr {\ntype Output;",
            "trait Tr {",
        )
        .unwrap()
        .unwrap();
        assert!(matches!(k, PatternKind::AssocRename { .. }));
    }

    #[test]
    fn fires_impl_const_with_value() {
        let k = cls(
            "impl S {\nconst MAX: i32 = 10;",
            "impl S {\nconst LIMIT: i32 = 10;",
            "impl S {",
        )
        .unwrap()
        .unwrap();
        assert!(matches!(k, PatternKind::AssocRename { .. }));
    }

    #[test]
    fn identical_names_fail() {
        assert!(cls("type Item;", "type Item;", "trait Tr {").is_err());
    }

    #[test]
    fn suffix_mismatch_fails() {
        assert!(cls(
            "impl S {\ntype Item = i32;",
            "impl S {\ntype Output = u64;",
            "impl S {",
        )
        .is_err());
    }

    #[test]
    fn no_context_declines() {
        assert!(cls("type Item;", "type Output;", "").unwrap().is_none());
    }

    #[test]
    fn fn_pair_declines() {
        assert!(cls("fn foo() {}", "fn bar() {}", "impl S {")
            .unwrap()
            .is_none());
    }

    fn evo(m: &str, p: &str, c: &str) -> Result<Option<PatternKind>, String> {
        use crate::metavar::MetavarTable;
        let table = MetavarTable::from_directives(&[]).map_err(|e| e.to_string())?;
        let table = crate::infer::infer_table(table, m, p, None).map_err(|e| e.to_string())?;
        let hunk: RsmplHunk = match rsmpl_patch::parse_patch(&format!(
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=r\n  {c}\n- {m}\n+ {p}\n"
        ))
        .map_err(|e| e.to_string())?
        .files
        .remove(0)
        .hunks
        .remove(0)
        {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => return Err("want rsmpl hunk".to_string()),
        };
        classify_assoc_evolution(m, p, c, &table, &hunk).map_err(|e| e.to_string())
    }

    #[test]
    fn evo_fires_trait_type() {
        let k = evo(
            "trait Tr {\ntype Item = Vec<u8>;",
            "trait Tr {\ntype Item = Box<[u8]>;",
            "trait Tr {",
        )
        .unwrap()
        .unwrap();
        assert!(matches!(k, PatternKind::AssocEvolution { .. }));
    }

    #[test]
    fn evo_fires_impl_const_hole() {
        let k = evo(
            "impl S {\nconst MAX: i32 = E;",
            "impl S {\nconst MAX: i32 = E + 1;",
            "impl S {",
        )
        .unwrap()
        .unwrap();
        assert!(matches!(k, PatternKind::AssocEvolution { .. }));
    }

    #[test]
    fn evo_ascribed_mismatch_fails() {
        assert!(evo(
            "impl S {\nconst MAX: u32 = E;",
            "impl S {\nconst MAX: i64 = E;",
            "impl S {",
        )
        .is_err());
    }

    #[test]
    fn evo_one_sided_default_fails() {
        assert!(evo(
            "trait Tr {\ntype Item;",
            "trait Tr {\ntype Item = i32;",
            "trait Tr {",
        )
        .is_err());
    }

    #[test]
    fn evo_bare_decls_decline() {
        assert!(evo("type Item;", "type Item;", "trait Tr {")
            .unwrap()
            .is_none());
    }

    #[test]
    fn evo_name_delta_declines() {
        assert!(evo(
            "impl S {\ntype Item = i32;",
            "impl S {\ntype Output = i32;",
            "impl S {",
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn evo_no_context_declines() {
        assert!(evo("type Item = Vec<u8>;", "type Item = Box<[u8]>;", "",)
            .unwrap()
            .is_none());
    }
}
