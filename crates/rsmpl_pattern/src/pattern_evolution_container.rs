//! WI-19 slice 4: container (`struct`/`enum`/`union`) body-evolving
//! `as` pastes.
//!
//! A `struct`/`enum`/`union` move whose body delta is exactly the
//! container rename (`Old` → `New` inside field types / variant
//! payloads) fires as one intent with the `fn` `as` spelling
//! generalised per kind (`+ pub struct Old as New { ... }`). The three
//! parsers below own that spelling; the abstract
//! `parse_struct/enum/union_evolution_template` parsers decline it via
//! the shared `follows_as_rename` ownership guard. The single shared
//! [`item_body_evolution_veto`] classifier is the WI-2 boundary for all
//! three kinds: field/variant idents, macro names, and attribute
//! interiors never rewrite (loud veto); type-path self-references do.
//!
//! D5: this lives in a sibling module because `pattern_evolution.rs`
//! (1552 lines) must not grow.

use super::pattern_evolution::{
    follows_as_rename, valid_enum_evolution_prefix, valid_struct_evolution_prefix,
    valid_union_evolution_prefix, word_bounded_contains,
};

/// Shared `struct`/`enum`/`union` body-evolution template parser:
/// `<keyword> <old> as <new>` plus the `...` body marker, single-line
/// only (like the abstract container templates). Returns
/// `(prefix, old, new)`.
fn parse_container_body_template(
    text: &str,
    keyword: &str,
    valid_prefix: fn(&str) -> bool,
) -> Option<(String, String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let kw = keyword.as_bytes();
    let mut kw_pos = None;
    let mut i = 0;
    while i + kw.len() <= bytes.len() {
        if bytes[i..].starts_with(kw)
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes
                .get(i + kw.len())
                .is_some_and(|c| c.is_ascii_whitespace())
        {
            kw_pos = Some(i);
            break;
        }
        i += 1;
    }
    let kw_pos = kw_pos?;
    let prefix = &line[..kw_pos];
    if !valid_prefix(prefix) {
        return None;
    }
    let rest = line[kw_pos + kw.len()..].trim_start();
    let mut old_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            old_len += c.len_utf8();
        } else {
            break;
        }
    }
    if old_len == 0 {
        return None;
    }
    let old = &rest[..old_len];
    let after_old = &rest[old_len..];
    let mut j = 0;
    while after_old
        .as_bytes()
        .get(j)
        .is_some_and(|c| c.is_ascii_whitespace())
    {
        j += 1;
    }
    if after_old.get(j..j + 2) != Some("as") {
        return None;
    }
    let k = j + 2;
    if after_old.as_bytes().get(k).is_some_and(|c| is_ident(*c)) {
        return None;
    }
    if j > 0 && after_old.as_bytes()[j - 1].is_ascii_alphanumeric() {
        return None;
    }
    let after_as = after_old[k..].trim_start();
    let mut new_len = 0;
    for c in after_as.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            new_len += c.len_utf8();
        } else {
            break;
        }
    }
    if new_len == 0 || old == &after_as[..new_len] {
        return None;
    }
    let tail = &after_as[new_len..];
    if !tail.contains("...") {
        return None;
    }
    // One rename per template: a second `as` belongs to no shape.
    if tail.contains(" as ") || follows_as_rename(tail) {
        return None;
    }
    Some((
        prefix.to_string(),
        old.to_string(),
        after_as[..new_len].to_string(),
    ))
}

/// WI-19 slice 4 `struct` body-evolving evolution-paste template:
/// `struct <old> as <new> { ... }` (prefix rides verbatim:
/// `pub struct Old as New { ... }`). Returns `(prefix, old, new)`.
pub fn parse_struct_body_evolution_template(text: &str) -> Option<(String, String, String)> {
    parse_container_body_template(text, "struct", valid_struct_evolution_prefix)
}

/// WI-19 slice 4 `enum` body-evolving evolution-paste template:
/// `enum <old> as <new> { ... }`. Returns `(prefix, old, new)`.
pub fn parse_enum_body_evolution_template(text: &str) -> Option<(String, String, String)> {
    parse_container_body_template(text, "enum", valid_enum_evolution_prefix)
}

/// WI-19 slice 4 `union` body-evolving evolution-paste template:
/// `union <old> as <new> { ... }`. Returns `(prefix, old, new)`.
pub fn parse_union_body_evolution_template(text: &str) -> Option<(String, String, String)> {
    parse_container_body_template(text, "union", valid_union_evolution_prefix)
}

/// Single-line container body-evolution paste template: the plus-side
/// prefix verbatim, `<old> as <new>`, `{ ... }` body marker. `prefix`
/// is trimmed; an empty prefix yields a bare header.
pub fn container_body_evolution_template(kind: &str, prefix: &str, old: &str, new: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("{kind} {old} as {new} {{ ... }}")
    } else {
        format!("{pre} {kind} {old} as {new} {{ ... }}")
    }
}

/// WI-19 slice 4 / WI-2: why a captured container body cannot take a
/// binding-blind rename.
///
/// A `syn` occurrence classifier shared by the authored splice and the
/// raiser pairing: an occurrence rewrites iff it is the header ident
/// itself or a type-path self-reference naming the container. Every
/// other namespace — a field ident or variant ident, a macro name, an
/// attribute interior — is left byte-verbatim and vetoes. Lifetimes,
/// strings, chars, and comments ride verbatim with no veto (the
/// [`crate::pattern_evolution::rename_ident_in_code`] scanner already
/// skips them).
///
/// `None` means every code occurrence is a self-reference the splice
/// may rename. Unparsable text yields `None` — the caller's parse gate
/// owns it.
pub fn item_body_evolution_veto(
    minus_text: &str,
    old: &str,
) -> Option<crate::pattern_evolution::BodyVeto> {
    use quote::ToTokens;
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    if old.is_empty() || !minus_text.contains(old) {
        return None;
    }
    struct Probe<'a> {
        old: &'a str,
        field_line: Option<usize>,
        variant_line: Option<usize>,
        macro_line: Option<usize>,
        attr_line: Option<usize>,
    }
    impl<'ast> Visit<'ast> for Probe<'_> {
        fn visit_field(&mut self, node: &'ast syn::Field) {
            if node.ident.as_ref().is_some_and(|id| id == self.old) {
                let line = node.ident.span().start().line;
                if self.field_line.is_none_or(|l| line < l) {
                    self.field_line = Some(line);
                }
            }
            syn::visit::visit_field(self, node);
        }
        fn visit_variant(&mut self, node: &'ast syn::Variant) {
            if node.ident == self.old {
                let line = node.ident.span().start().line;
                if self.variant_line.is_none_or(|l| line < l) {
                    self.variant_line = Some(line);
                }
            }
            syn::visit::visit_variant(self, node);
        }
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == self.old)
            {
                let line = node.path.span().start().line;
                if self.macro_line.is_none_or(|l| line < l) {
                    self.macro_line = Some(line);
                }
            }
            // Do not descend into macro token trees: every ident inside
            // is a distinct namespace the rename must not touch.
        }
        fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
            if word_bounded_contains(&node.to_token_stream().to_string(), self.old) {
                let line = node.span().start().line;
                if self.attr_line.is_none_or(|l| line < l) {
                    self.attr_line = Some(line);
                }
            }
        }
    }
    // The text must parse as one of the three container kinds; any
    // other shape yields no veto (the caller's kind gate owns it).
    let mut probe = Probe {
        old,
        field_line: None,
        variant_line: None,
        macro_line: None,
        attr_line: None,
    };
    if let Ok(item) = syn::parse_str::<syn::ItemStruct>(minus_text) {
        probe.visit_item_struct(&item);
    } else if let Ok(item) = syn::parse_str::<syn::ItemEnum>(minus_text) {
        probe.visit_item_enum(&item);
    } else if let Ok(item) = syn::parse_str::<syn::ItemUnion>(minus_text) {
        probe.visit_item_union(&item);
    } else {
        return None;
    }
    // Earliest veto wins so the repair names one actionable line.
    let mut best: Option<crate::pattern_evolution::BodyVeto> = None;
    let mut consider = |kind: &'static str, line: Option<usize>| {
        if let Some(l) = line {
            if best.as_ref().is_none_or(|b| l < b.line) {
                best = Some(crate::pattern_evolution::BodyVeto { kind, line: l });
            }
        }
    };
    consider("attribute", probe.attr_line);
    consider("field", probe.field_line);
    consider("variant", probe.variant_line);
    consider("macro", probe.macro_line);
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_templates_accept() {
        assert_eq!(
            parse_struct_body_evolution_template("struct Old as New { ... }"),
            Some(("".to_string(), "Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_struct_body_evolution_template("pub struct Old as New { ... }"),
            Some(("pub ".to_string(), "Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_enum_body_evolution_template("pub enum Old as New { ... }"),
            Some(("pub ".to_string(), "Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_union_body_evolution_template("union Old as New { ... }"),
            Some(("".to_string(), "Old".to_string(), "New".to_string()))
        );
    }

    #[test]
    fn container_templates_reject() {
        // Plain abstract form belongs to its own parser.
        assert_eq!(
            parse_struct_body_evolution_template("struct New { ... }"),
            None
        );
        // Same name on both sides is vacuous.
        assert_eq!(
            parse_struct_body_evolution_template("struct Old as Old { ... }"),
            None
        );
        // Marker-less lines stay loud downstream.
        assert_eq!(
            parse_struct_body_evolution_template("struct Old as New {}"),
            None
        );
        // Multi-line text is never a template.
        assert_eq!(
            parse_struct_body_evolution_template("struct Old as New {\n ... \n}"),
            None
        );
        // Bad prefixes decline.
        assert_eq!(
            parse_struct_body_evolution_template("foo( struct Old as New { ... }"),
            None
        );
        // Double rename belongs to no shape.
        assert_eq!(
            parse_struct_body_evolution_template("struct A as B as C { ... }"),
            None
        );
        // Wrong-kind keywords never cross-claim.
        assert_eq!(
            parse_enum_body_evolution_template("struct Old as New { ... }"),
            None
        );
        assert_eq!(
            parse_union_body_evolution_template("enum Old as New { ... }"),
            None
        );
    }

    #[test]
    fn abstract_container_parsers_reject_as_form() {
        // Ownership: the abstract parsers must not mis-name the `as` form.
        assert_eq!(
            crate::pattern_evolution::parse_struct_evolution_template("struct Old as New { ... }"),
            None
        );
        assert_eq!(
            crate::pattern_evolution::parse_enum_evolution_template("enum Old as New { ... }"),
            None
        );
        assert_eq!(
            crate::pattern_evolution::parse_union_evolution_template("union Old as New { ... }"),
            None
        );
        // Untouched shapes still parse.
        assert_eq!(
            crate::pattern_evolution::parse_struct_evolution_template("struct New { ... }"),
            Some(("".to_string(), "New".to_string()))
        );
        assert!(crate::pattern_evolution::is_evolution_template(
            "pub struct Old as New { ... }"
        ));
        assert!(crate::pattern_evolution::is_evolution_template(
            "enum Old as New { ... }"
        ));
        assert!(crate::pattern_evolution::is_evolution_template(
            "union Old as New { ... }"
        ));
    }

    #[test]
    fn container_template_shapes() {
        assert_eq!(
            container_body_evolution_template("struct", "pub ", "Old", "New"),
            "pub struct Old as New { ... }"
        );
        assert_eq!(
            container_body_evolution_template("enum", "", "Old", "New"),
            "enum Old as New { ... }"
        );
    }

    #[test]
    fn container_veto_matrix() {
        // Genuine self-reference in a field type: no veto.
        let firing = "struct Old {\n    next: Option<Box<Old>>,\n}";
        assert_eq!(item_body_evolution_veto(firing, "Old"), None);
        // Enum payload self-reference: no veto.
        let firing_e = "enum Old {\n    A(Box<Old>),\n    B,\n}";
        assert_eq!(item_body_evolution_veto(firing_e, "Old"), None);
        // Union field self-reference: no veto.
        let firing_u = "union Old {\n    x: Old,\n}";
        assert_eq!(item_body_evolution_veto(firing_u, "Old"), None);
        // A field *named* like the container lives in another namespace.
        let field = "struct Old {\n    Old: u32,\n}";
        let veto = item_body_evolution_veto(field, "Old").expect("field vetoes");
        assert_eq!((veto.kind, veto.line), ("field", 2));
        // A variant *named* like the container: veto at the variant.
        let variant = "enum Old {\n    Old,\n    B,\n}";
        let veto = item_body_evolution_veto(variant, "Old").expect("variant vetoes");
        assert_eq!((veto.kind, veto.line), ("variant", 2));
        // Macro invocation names the macro, never the item.
        let mac = "struct Old {\n    next: Box<Old>,\n    tag: Old!(),\n}";
        let veto = item_body_evolution_veto(mac, "Old").expect("macro vetoes");
        assert_eq!(veto.kind, "macro");
        // Attribute interiors are opaque to the rename.
        let attr = "#[derive(Old)]\nstruct Old {\n    x: u32,\n}";
        let veto = item_body_evolution_veto(attr, "Old").expect("attr vetoes");
        assert_eq!(veto.kind, "attribute");
        // Strings need no veto (the scanner skips them).
        let lit = "struct Old {\n    x: u32,\n}\n// Old\n";
        assert_eq!(item_body_evolution_veto(lit, "Old"), None);
    }

    #[test]
    fn container_veto_macro_name() {
        // A macro named like the container vetoes even when the
        // container body itself also self-references through types.
        let text = "enum Old {\n    A(Box<Old>),\n    B(Old!()),\n}";
        let veto = item_body_evolution_veto(text, "Old").expect("macro vetoes");
        assert_eq!(veto.kind, "macro");
    }
}
