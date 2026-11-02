//! WI-19 residue: inherent-`impl`-block body-evolving `as` move pairing.
//!
//! An `impl Old { ... }` move whose body delta is exactly the self-ty
//! rename (`Old` → `New` in type paths inside the block) pairs like the
//! container body-evolving shapes: inherent-only, generics/attrs
//! token-equal, the full-text agreement (the
//! [`rsmpl_pattern::rename_ident_in_code`] scanner-renamed minus item
//! == the plus run, raw string equality) plus the WI-2
//! [`rsmpl_pattern::impl_body_evolution_veto`] boundary proves the
//! pairing. Trait-impls never pair (WI-3 owns that header).
//!
//! D5: sibling module because `move_evolve.rs` (1441 lines) must not grow.

use super::move_evolve::tok;
use syn::spanned::Spanned;

/// Body-evolving `impl` pair check: same shape modulo the self-ty ident,
/// with a token-different item list the full-text agreement in
/// [`find_impl_body_evolved_hits`] proves is exactly the old→new rename.
/// Inherent-only, generics/attrs token-equal. Returns the plus-side
/// self-ty ident when the pair evolves.
pub(crate) fn impl_body_evolved_match(
    minus: &syn::ItemImpl,
    plus: &syn::ItemImpl,
) -> Option<String> {
    if minus.trait_.is_some() || plus.trait_.is_some() {
        return None;
    }
    if minus.unsafety.is_some() != plus.unsafety.is_some() {
        return None; // `unsafe` delta belongs to no arm (name-only scope)
    }
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    let old = self_ty_last_ident(&minus.self_ty)?;
    let new = self_ty_last_ident(&plus.self_ty)?;
    if old == new {
        return None; // vacuous — no self-ty delta, no arm owns it
    }
    let same_body = minus.items.len() == plus.items.len()
        && !minus
            .items
            .iter()
            .zip(plus.items.iter())
            .any(|(a, b)| tok(a) != tok(b));
    if same_body {
        return None; // no body delta — byte-identical impls stay ordinary
    }
    Some(new)
}

/// Last segment of a plain-path self-ty. Anything else is not a
/// moveable self-ty.
fn self_ty_last_ident(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

/// All body-evolving `impl` hits of one minus item text in a pure-plus
/// hunk body. Mirrors
/// [`super::move_evolve::find_fn_body_evolved_hits`]; spans coincide
/// with parsed `impl` item spans by construction. Returns
/// `(plus_first, plus_last, prefix, old_name, new_name)`: the paste
/// spells `+ impl <old> as <new> { ... }` and the engine renames the
/// captured suffix/body through the shared scanner — the same scanner
/// proves the pairing here, so pairing and splice agree by construction.
pub(crate) fn find_impl_body_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemImpl>(minus_text) else {
        return Vec::new();
    };
    if minus.trait_.is_some() {
        return Vec::new();
    }
    let Some(old) = self_ty_last_ident(&minus.self_ty) else {
        return Vec::new();
    };
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Impl(plus) = it else {
            continue;
        };
        let Some(new) = impl_body_evolved_match(&minus, plus) else {
            continue;
        };
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        // The template prefix rides verbatim (`unsafe impl` stays
        // `unsafe`); the splice honors it, so the pairing carries it.
        let first_line = plus_body[s - 1].get(1..).unwrap_or("");
        let prefix = if first_line.trim_start().starts_with("unsafe ")
            || first_line.trim_start() == "unsafe"
        {
            "unsafe ".to_string()
        } else {
            String::new()
        };
        let plus_text: String = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        if rsmpl_pattern::rename_ident_in_code(minus_text, &old, &new) != plus_text {
            continue;
        }
        if rsmpl_pattern::impl_body_evolution_veto(minus_text, &old).is_some() {
            continue;
        }
        out.push((s - 1, e - 1, prefix, old.clone(), new));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plus_body(text: &str) -> Vec<String> {
        text.lines().map(|l| format!("+{l}")).collect()
    }

    #[test]
    fn impl_body_evolved_fires() {
        let minus = "impl Old {\n    fn make() -> Old {\n        Old\n    }\n}";
        let plus = "impl New {\n    fn make() -> New {\n        New\n    }\n}";
        let hits = find_impl_body_evolved_hits(minus, &plus_body(plus));
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!(hits[0].3, "Old");
        assert_eq!(hits[0].4, "New");
    }

    #[test]
    fn impl_body_evolved_trait_declines() {
        let minus = "impl Tr for Old {\n    fn m(&self) {}\n}";
        let plus = "impl Tr for New {\n    fn m(&self) {}\n}";
        assert!(find_impl_body_evolved_hits(minus, &plus_body(plus)).is_empty());
    }

    #[test]
    fn impl_body_evolved_no_delta_declines() {
        let minus = "impl Old {\n    fn m(&self) {}\n}";
        assert!(find_impl_body_evolved_hits(minus, &plus_body(minus)).is_empty());
    }

    #[test]
    fn impl_body_evolved_unsafe_prefix_fires() {
        let minus = "unsafe impl Old {\n    fn m(&self) {}\n}";
        let plus = "unsafe impl New {\n    fn m(&self) {}\n}";
        // No self-path in the body, so no delta — stays ordinary even
        // with the `unsafe` prefix (name-only scope needs a body delta).
        assert!(find_impl_body_evolved_hits(minus, &plus_body(plus)).is_empty());
        let minus_b = "unsafe impl Old {\n    fn make() -> Old {\n        Old\n    }\n}";
        let plus_b = "unsafe impl New {\n    fn make() -> New {\n        New\n    }\n}";
        let hits = find_impl_body_evolved_hits(minus_b, &plus_body(plus_b));
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!(hits[0].2, "unsafe ");
        assert_eq!((hits[0].3.as_str(), hits[0].4.as_str()), ("Old", "New"));
    }

    #[test]
    fn impl_body_evolved_unsafety_delta_declines() {
        let minus = "unsafe impl Old {\n    fn make() -> Old {\n        Old\n    }\n}";
        let plus = "impl New {\n    fn make() -> New {\n        New\n    }\n}";
        assert!(find_impl_body_evolved_hits(minus, &plus_body(plus)).is_empty());
    }

    #[test]
    fn impl_body_evolved_assoc_veto() {
        let minus = "impl Old {\n    fn Old(&self) {}\n}\n";
        let plus = "impl New {\n    fn New(&self) {}\n}\n";
        // Full-text agreement holds, but the assoc-name veto strands it.
        assert!(find_impl_body_evolved_hits(minus, &plus_body(plus)).is_empty());
    }
}
