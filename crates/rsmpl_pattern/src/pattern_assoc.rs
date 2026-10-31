//! Associated-item rename pairs (`type`/`const` in `trait`/`impl` contexts).
//!
//! WI-13 slice 1: rename authoring only (definition ident-span edit).
//! Evolution, `# follow=uses`, and raising land in WI-26/27/28.

use rsmpl_patch::{ErrorCode, StructuredError};

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
                    "assoc renames are ident-only in v1: bounds, generics, types and values must match on both sides (write the same suffix, or use an ordinary hunk; evolution lands in WI-26)",
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
}
