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
    valid_trait_evolution_prefix, valid_union_evolution_prefix, word_bounded_contains,
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

/// E2 slice 2a `trait` body-evolving evolution-paste template:
/// `trait <old> as <new> { ... }` (prefix rides verbatim:
/// `pub trait Old as New { ... }`). Returns `(prefix, old, new)`.
/// The engine renames the captured suffix/body through the shared
/// scanner; the raiser proves the pairing through the same scanner,
/// so pairing and splice agree by construction.
pub fn parse_trait_body_evolution_template(text: &str) -> Option<(String, String, String)> {
    parse_container_body_template(text, "trait", valid_trait_evolution_prefix)
}

/// E2 slice 2b `trait` body-joint evolution-paste template:
/// `trait <old> as <new> { fn <m_old> as <m_new>; ... }` (prefix rides
/// verbatim: `pub trait Old as New { fn m_old as m_new; ... }`).
/// Returns `(prefix, old, new, m_old, m_new)`. Single-line only. The
/// self-ref `parse_trait_body_evolution_template` declines this form
/// (second ` as ` veto) and the abstract parser declines the first, so
/// the engine dispatches the joint first — disjoint by construction.
/// All four idents must be pairwise distinct (the splice double-scans
/// `Old→New` then `m_old→m_new`, so any cross-talk would corrupt).
pub fn parse_trait_body_joint_template(
    text: &str,
) -> Option<(String, String, String, String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 || !line.contains("...") {
        return None;
    }
    let brace = line.find('{')?;
    let (head, body) = line.split_at(brace);
    let (prefix, old, new) = parse_trait_as_head(head)?;
    // Body: `fn <m_old> as <m_new>; ... }`, nothing else.
    let mut rest = body[1..].trim_start();
    rest = rest.strip_prefix("fn")?;
    if rest.starts_with(|c: char| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    let (m_old, tail) = take_ident(rest.trim_start())?;
    let tail = tail.trim_start().strip_prefix("as")?;
    if tail.starts_with(|c: char| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    let (m_new, tail) = take_ident(tail.trim_start())?;
    let tail = tail.trim_start().strip_prefix(';')?;
    let tail = tail.trim_start().strip_prefix("...")?;
    let tail = tail.trim();
    let tail = tail.strip_suffix('}').unwrap_or(tail).trim();
    if !tail.is_empty() {
        return None;
    }
    if m_old == m_new || old == m_old || old == m_new || new == m_old || new == m_new {
        return None;
    }
    Some((prefix, old, new, m_old, m_new))
}

/// `<prefix> trait <old> as <new>` with nothing trailing (E2 slice 2b
/// joint head). Mirrors the head half of `parse_container_body_template`
/// with the `trait` keyword fixed and an empty-tail requirement.
fn parse_trait_as_head(head: &str) -> Option<(String, String, String)> {
    let bytes = head.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut kw_pos = None;
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"trait")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            kw_pos = Some(i);
            break;
        }
        i += 1;
    }
    let kw_pos = kw_pos?;
    let prefix = &head[..kw_pos];
    if !valid_trait_evolution_prefix(prefix) {
        return None;
    }
    let (old, tail) = take_ident(head[kw_pos + 5..].trim_start())?;
    let tail = tail.trim_start().strip_prefix("as")?;
    if tail.starts_with(|c: char| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    let (new, tail) = take_ident(tail.trim_start())?;
    if old == new || !tail.trim().is_empty() {
        return None;
    }
    Some((prefix.to_string(), old, new))
}

/// Word-bounded leading ident of `s` plus the remainder (`None` when `s`
/// leads with no ident). Shared by the joint head and body parsers.
fn take_ident(s: &str) -> Option<(String, &str)> {
    let mut len = 0;
    for c in s.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            len += c.len_utf8();
        } else {
            break;
        }
    }
    if len == 0 {
        return None;
    }
    Some((s[..len].to_string(), &s[len..]))
}

/// E2 slice 2b joint paste template: the plus-side prefix verbatim,
/// `<old> as <new>`, plus the single method rename `fn <m_old> as
/// <m_new>;` ahead of the `...` body marker. `prefix` is trimmed; an
/// empty prefix yields a bare header.
pub fn trait_body_joint_template(
    prefix: &str,
    old: &str,
    new: &str,
    m_old: &str,
    m_new: &str,
) -> String {
    let pre = prefix.trim_end();
    let head = if pre.is_empty() {
        format!("trait {old} as {new}")
    } else {
        format!("{pre} trait {old} as {new}")
    };
    format!("{head} {{ fn {m_old} as {m_new}; ... }}")
}

/// E2 slice 2b: why a captured trait body cannot take a binding-blind
/// method rename. A `syn` occurrence classifier shared by the joint
/// splice and the joint pairing: the renamed method's own ident is
/// expected (its definition plus word-bounded self-calls rename along),
/// but a macro name or an attribute interior spelling `m_old` is a
/// distinct namespace and vetoes. Returns the earliest veto. Unparsable
/// text yields `None` — the caller's parse gate owns it.
pub fn trait_joint_method_veto(
    minus_text: &str,
    m_old: &str,
) -> Option<crate::pattern_evolution::BodyVeto> {
    use quote::ToTokens;
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    if m_old.is_empty() || !minus_text.contains(m_old) {
        return None;
    }
    struct Probe<'a> {
        m_old: &'a str,
        macro_line: Option<usize>,
        attr_line: Option<usize>,
    }
    impl<'ast> Visit<'ast> for Probe<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == self.m_old)
            {
                let line = node.path.span().start().line;
                if self.macro_line.is_none_or(|l| line < l) {
                    self.macro_line = Some(line);
                }
            }
            // Do not descend into macro token trees: every ident inside
            // is a distinct namespace (mirrors `item_body_evolution_veto`).
        }
        fn visit_attribute(&mut self, node: &'ast syn::Attribute) {
            if word_bounded_contains(&node.to_token_stream().to_string(), self.m_old) {
                let line = node.span().start().line;
                if self.attr_line.is_none_or(|l| line < l) {
                    self.attr_line = Some(line);
                }
            }
        }
    }
    let Ok(item) = syn::parse_str::<syn::ItemTrait>(minus_text) else {
        return None;
    };
    let mut probe = Probe {
        m_old,
        macro_line: None,
        attr_line: None,
    };
    probe.visit_item_trait(&item);
    match (probe.attr_line, probe.macro_line) {
        (Some(a), Some(m)) => Some(if m < a {
            crate::pattern_evolution::BodyVeto {
                kind: "macro",
                line: m,
            }
        } else {
            crate::pattern_evolution::BodyVeto {
                kind: "attribute",
                line: a,
            }
        }),
        (Some(a), None) => Some(crate::pattern_evolution::BodyVeto {
            kind: "attribute",
            line: a,
        }),
        (None, Some(m)) => Some(crate::pattern_evolution::BodyVeto {
            kind: "macro",
            line: m,
        }),
        (None, None) => None,
    }
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
        method_line: Option<usize>,
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
        fn visit_trait_item_fn(&mut self, node: &'ast syn::TraitItemFn) {
            // E2 slice 2a: a trait method *named* like the trait lives
            // in another namespace — renaming it would change the
            // method, not the self-ty, so it vetoes the `as` splice.
            if node.sig.ident == self.old {
                let line = node.sig.ident.span().start().line;
                if self.method_line.is_none_or(|l| line < l) {
                    self.method_line = Some(line);
                }
            }
            syn::visit::visit_trait_item_fn(self, node);
        }
    }
    // The text must parse as one of the three container kinds or a
    // trait; any other shape yields no veto (the caller's kind gate
    // owns it).
    let mut probe = Probe {
        old,
        field_line: None,
        variant_line: None,
        macro_line: None,
        attr_line: None,
        method_line: None,
    };
    if let Ok(item) = syn::parse_str::<syn::ItemStruct>(minus_text) {
        probe.visit_item_struct(&item);
    } else if let Ok(item) = syn::parse_str::<syn::ItemEnum>(minus_text) {
        probe.visit_item_enum(&item);
    } else if let Ok(item) = syn::parse_str::<syn::ItemUnion>(minus_text) {
        probe.visit_item_union(&item);
    } else if let Ok(item) = syn::parse_str::<syn::ItemTrait>(minus_text) {
        probe.visit_item_trait(&item);
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
    consider("method", probe.method_line);
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
        // E2 slice 2a: trait `as` template.
        assert_eq!(
            parse_trait_body_evolution_template("trait Old as New { ... }"),
            Some(("".to_string(), "Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_trait_body_evolution_template("pub trait Old as New { ... }"),
            Some(("pub ".to_string(), "Old".to_string(), "New".to_string()))
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
        // E2 slice 2a: the abstract trait parser declines the `as` form.
        assert_eq!(
            crate::pattern_evolution::parse_trait_evolution_template("trait Old as New { ... }"),
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
        assert!(crate::pattern_evolution::is_evolution_template(
            "trait Old as New { ... }"
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

    #[test]
    fn trait_veto_matrix() {
        // E2 slice 2a: self-reference in a trait method signature —
        // no veto.
        let firing = "trait Old {\n    fn make() -> Old;\n    fn use_it(&self);\n}";
        assert_eq!(item_body_evolution_veto(firing, "Old"), None);
        // A trait method *named* like the trait: veto at the method.
        let method = "trait Old {\n    fn Old(&self);\n}";
        let veto = item_body_evolution_veto(method, "Old").expect("method vetoes");
        assert_eq!((veto.kind, veto.line), ("method", 2));
        // Macro invocation names the macro, never the item.
        let mac = "trait Old {\n    fn make() -> Old;\n    fn tag() -> Old!();\n}";
        let veto = item_body_evolution_veto(mac, "Old").expect("macro vetoes");
        assert_eq!(veto.kind, "macro");
        // Attribute interiors are opaque to the rename.
        let attr = "#[derive(Old)]\ntrait Old {\n    fn f(&self);\n}";
        let veto = item_body_evolution_veto(attr, "Old").expect("attr vetoes");
        assert_eq!(veto.kind, "attribute");
    }

    #[test]
    fn trait_joint_templates_accept() {
        // E2 slice 2b: the one-method-rename joint form.
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn m_old as m_new; ... }"),
            Some((
                "".to_string(),
                "Old".to_string(),
                "New".to_string(),
                "m_old".to_string(),
                "m_new".to_string()
            ))
        );
        assert_eq!(
            parse_trait_body_joint_template("pub trait Old as New { fn m_old as m_new; ... }"),
            Some((
                "pub ".to_string(),
                "Old".to_string(),
                "New".to_string(),
                "m_old".to_string(),
                "m_new".to_string()
            ))
        );
        // Builder round-trips through the parser.
        let built = trait_body_joint_template("pub ", "Old", "New", "m_old", "m_new");
        assert_eq!(built, "pub trait Old as New { fn m_old as m_new; ... }");
        assert_eq!(
            parse_trait_body_joint_template(&built).map(|t| t.3),
            Some("m_old".to_string())
        );
        assert_eq!(
            trait_body_joint_template("", "Old", "New", "m_old", "m_new"),
            "trait Old as New { fn m_old as m_new; ... }"
        );
    }

    #[test]
    fn trait_joint_templates_reject() {
        // The self-ref form belongs to its own parser.
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { ... }"),
            None
        );
        // Same-name joints are vacuous.
        assert_eq!(
            parse_trait_body_joint_template("trait Old as Old { fn m as m; ... }"),
            None
        );
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn m as m; ... }"),
            None
        );
        // Cross-talk between the two renames is unsound (the splice
        // double-scans, so any shared ident would corrupt).
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn New as m_new; ... }"),
            None
        );
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn m_old as Old; ... }"),
            None
        );
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn Old as m_new; ... }"),
            None
        );
        // Marker-less and multi-line stay loud downstream.
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn m_old as m_new; }"),
            None
        );
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New {\n fn m_old as m_new; ... \n}"),
            None
        );
        // A second method joint belongs to no shape.
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn a as b; fn c as d; ... }"),
            None
        );
        // Missing `;` between the method rename and the marker.
        assert_eq!(
            parse_trait_body_joint_template("trait Old as New { fn m_old as m_new ... }"),
            None
        );
        // Wrong-kind keywords never cross-claim.
        assert_eq!(
            parse_struct_body_evolution_template("trait Old as New { fn m_old as m_new; ... }"),
            None
        );
    }

    #[test]
    fn trait_joint_ownership_disjoint() {
        // The joint dispatches first: neither sibling parser claims it.
        let joint = "trait Old as New { fn m_old as m_new; ... }";
        assert_eq!(parse_trait_body_evolution_template(joint), None);
        assert_eq!(
            crate::pattern_evolution::parse_trait_evolution_template(joint),
            None
        );
        assert!(crate::pattern_evolution::is_evolution_template(joint));
        // Untouched shapes still parse where they belong.
        assert!(parse_trait_body_evolution_template("trait Old as New { ... }").is_some());
    }

    #[test]
    fn trait_joint_method_veto_matrix() {
        // The renamed method's own ident never vetoes.
        let firing = "trait Old {\n    fn m_old(&self);\n}";
        assert_eq!(trait_joint_method_veto(firing, "m_old"), None);
        // Absent ident never vetoes.
        assert_eq!(trait_joint_method_veto(firing, "other"), None);
        // A macro named like the method is a distinct namespace.
        let mac = "trait Old {\n    fn m_old(&self);\n    m_old!();\n}";
        let veto = trait_joint_method_veto(mac, "m_old").expect("macro vetoes");
        assert_eq!((veto.kind, veto.line), ("macro", 3));
        // Attribute interiors are opaque to the rename.
        let attr = "trait Old {\n    #[doc = \"m_old\"]\n    fn m_old(&self);\n}";
        let veto = trait_joint_method_veto(attr, "m_old").expect("attr vetoes");
        assert_eq!(veto.kind, "attribute");
    }
}
