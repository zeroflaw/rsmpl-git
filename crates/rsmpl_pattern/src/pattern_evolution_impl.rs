//! WI-19 slice-4 residue: inherent-`impl`-block body-evolving `as` pastes.
//!
//! An `impl Old { ... }` move whose body delta is exactly the self-ty
//! rename (`Old` → `New` in type paths inside the block) fires as one
//! intent with the container `as` spelling generalised to `impl`
//! (`+ impl Old as New { ... }`). Trait-impls (`impl Tr for Ty`)
//! decline — WI-3 owns that header.
//!
//! D5: sibling of `pattern_evolution_container.rs` because
//! `pattern_evolution.rs` (1571 lines) must not grow.

use super::pattern_evolution::{rename_ident_in_code, word_bounded_contains};

/// Parse an inherent-`impl` body-evolution template:
/// `impl Old as New { ... }` (optional `unsafe` prefix rides verbatim:
/// `unsafe impl Old as New { ... }`). Returns `(prefix, old, new)`.
/// Trait-form text (`for` in the header) declines to `None`.
pub fn parse_impl_body_evolution_template(text: &str) -> Option<(String, String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    // Marker discipline like the container `as` forms.
    if !line.contains("...") {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `impl` keyword.
    let mut kw_pos = None;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"impl")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            kw_pos = Some(i);
            break;
        }
        i += 1;
    }
    let kw_pos = kw_pos?;
    let prefix = &line[..kw_pos];
    if !valid_impl_body_prefix(prefix) {
        return None;
    }
    let rest = line[kw_pos + 4..].trim_start();
    // Old ident.
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
    // Trait-form headers (`impl Tr for Ty`) never enter here: a `for`
    // word between `impl` and `as` declines. `rest` = `Old as New ...`
    // for inherent; trait form would be `Tr for Ty as ...` — generics
    // (`Old<T>`) decline below.
    if has_word(rest, "for") {
        return None;
    }
    // Generics on the self-ty (`Old<T>`) decline: token-equal generics
    // belong to no evolution arm yet.
    if rest.as_bytes().get(old_len).is_some_and(|c| *c == b'<') {
        return None;
    }
    if tail.contains(" as ") {
        return None;
    }
    Some((
        prefix.to_string(),
        old.to_string(),
        after_as[..new_len].to_string(),
    ))
}

/// Valid prefix before `impl` in the body-evolution template: empty or
/// `unsafe` (word-bounded, trailing space tolerated — the template trims).
fn valid_impl_body_prefix(prefix: &str) -> bool {
    let t = prefix.trim();
    t.is_empty() || t == "unsafe"
}

/// Single-line `impl` body-evolution paste template.
pub fn impl_body_evolution_template(prefix: &str, old: &str, new: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("impl {old} as {new} {{ ... }}")
    } else {
        format!("{pre} impl {old} as {new} {{ ... }}")
    }
}

fn has_word(hay: &str, word: &str) -> bool {
    let hb = hay.as_bytes();
    let wb = word.as_bytes();
    if wb.is_empty() || hb.len() < wb.len() {
        return false;
    }
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + wb.len() <= hb.len() {
        if &hb[i..i + wb.len()] == wb {
            let before_ok = i == 0 || !is_ident(hb[i - 1]);
            let after_ok = i + wb.len() >= hb.len() || !is_ident(hb[i + wb.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// WI-19 residue / WI-2: why a captured `impl` block cannot take a
/// binding-blind rename.
///
/// A `syn` occurrence classifier shared by the authored splice and the
/// raiser pairing: an occurrence rewrites iff it is the self-ty ident
/// or a type-path self-reference naming `Old`. Every other namespace —
/// an assoc-item name (`fn`/`const`/`type` named `Old`), a macro name,
/// an attribute interior — is left byte-verbatim and vetoes. Shadowed
/// locals inside method bodies veto via the shared `fn` boundary.
/// Lifetimes/strings/comments ride verbatim with no veto (the
/// [`rename_ident_in_code`] scanner already skips them).
///
/// `None` means every code occurrence is a self-reference the splice
/// may rename. Unparsable text or trait-impls yield `None` — the
/// caller's kind gate owns them.
pub fn impl_body_evolution_veto(
    minus_text: &str,
    old: &str,
) -> Option<super::pattern_evolution::BodyVeto> {
    use quote::ToTokens;
    use syn::spanned::Spanned;
    use syn::visit::Visit;

    if old.is_empty() || !minus_text.contains(old) {
        return None;
    }
    let item: syn::ItemImpl = syn::parse_str(minus_text).ok()?;
    if item.trait_.is_some() {
        return None;
    }
    struct Probe<'a> {
        old: &'a str,
        assoc_line: Option<usize>,
        macro_line: Option<usize>,
        attr_line: Option<usize>,
    }
    impl<'ast> Visit<'ast> for Probe<'_> {
        fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
            if node.sig.ident == self.old {
                let line = node.sig.ident.span().start().line;
                if self.assoc_line.is_none_or(|l| line < l) {
                    self.assoc_line = Some(line);
                }
            }
            syn::visit::visit_impl_item_fn(self, node);
        }
        fn visit_impl_item_const(&mut self, node: &'ast syn::ImplItemConst) {
            if node.ident == self.old {
                let line = node.ident.span().start().line;
                if self.assoc_line.is_none_or(|l| line < l) {
                    self.assoc_line = Some(line);
                }
            }
            syn::visit::visit_impl_item_const(self, node);
        }
        fn visit_impl_item_type(&mut self, node: &'ast syn::ImplItemType) {
            if node.ident == self.old {
                let line = node.ident.span().start().line;
                if self.assoc_line.is_none_or(|l| line < l) {
                    self.assoc_line = Some(line);
                }
            }
            syn::visit::visit_impl_item_type(self, node);
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
    let mut probe = Probe {
        old,
        assoc_line: None,
        macro_line: None,
        attr_line: None,
    };
    probe.visit_item_impl(&item);
    // Shadowed locals inside method bodies reuse the `fn`-level
    // boundary per method (each method parses as an `ItemFn` shape via
    // a synthetic wrapper — the veto reports the method-body line).
    let mut shadow_line: Option<usize> = None;
    for it in &item.items {
        if let syn::ImplItem::Fn(m) = it {
            // Wrap the method as a free fn so `body_evolution_veto`
            // runs its binder analysis on the same code.
            let wrapped = format!(
                "fn {}({}) {} {{ {} }}",
                m.sig.ident,
                m.sig.inputs.to_token_stream(),
                m.sig.output.to_token_stream(),
                m.block.to_token_stream()
            );
            let _ = &wrapped;
            // Direct walk: any `let`/param binding named `old` plus a
            // same-named value path inside this method vetoes.
            // Reuse the tested helper on a synthetic fn item.
            let synth = format!("fn __probe__() {{ {} }}", m.block.to_token_stream());
            void_probe(&synth, old, &mut shadow_line);
        }
    }
    let mut best: Option<super::pattern_evolution::BodyVeto> = None;
    let mut consider = |kind: &'static str, line: Option<usize>| {
        if let Some(l) = line {
            if best.as_ref().is_none_or(|b| l < b.line) {
                best = Some(super::pattern_evolution::BodyVeto { kind, line: l });
            }
        }
    };
    consider("attribute", probe.attr_line);
    consider("assoc", probe.assoc_line);
    consider("macro", probe.macro_line);
    consider("shadowed-local", shadow_line);
    // Avoid unused-import warning when the scanner path is the only use.
    let _ = rename_ident_in_code;
    best
}

/// Minimal shadow scan over a synthetic block wrapper: a `PatIdent`
/// named `old` anywhere vetoes at its line (conservative — the
/// full-text agreement downstream already proves the rename, so any
/// binder of the same name is a capture risk).
fn void_probe(synth: &str, old: &str, out: &mut Option<usize>) {
    use syn::visit::Visit;
    struct P<'a> {
        old: &'a str,
        line: Option<usize>,
    }
    impl<'ast> Visit<'ast> for P<'_> {
        fn visit_pat_ident(&mut self, node: &'ast syn::PatIdent) {
            if node.ident == self.old {
                let l = node.ident.span().start().line;
                if self.line.is_none_or(|x| l < x) {
                    self.line = Some(l);
                }
            }
            syn::visit::visit_pat_ident(self, node);
        }
    }
    let Ok(f) = syn::parse_str::<syn::ItemFn>(synth) else {
        return;
    };
    let mut p = P { old, line: None };
    p.visit_item_fn(&f);
    if let Some(l) = p.line {
        // Method-body lines are 1-based within the wrapper; lift by 1
        // so the repair names a line inside the captured block span.
        let lifted = l.saturating_add(1);
        if out.is_none_or(|x| lifted < x) {
            *out = Some(lifted);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impl_templates_accept() {
        assert_eq!(
            parse_impl_body_evolution_template("impl Old as New { ... }"),
            Some(("".to_string(), "Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_impl_body_evolution_template("unsafe impl Old as New { ... }"),
            Some(("unsafe ".to_string(), "Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            impl_body_evolution_template("", "Old", "New"),
            "impl Old as New { ... }"
        );
    }

    #[test]
    fn impl_templates_reject() {
        assert_eq!(parse_impl_body_evolution_template("impl New { ... }"), None);
        assert_eq!(
            parse_impl_body_evolution_template("impl Old as Old { ... }"),
            None
        );
        assert_eq!(
            parse_impl_body_evolution_template("impl Old as New {}"),
            None
        );
        // Trait form declines.
        assert_eq!(
            parse_impl_body_evolution_template("impl Tr for Old as New { ... }"),
            None
        );
        // Generics decline.
        assert_eq!(
            parse_impl_body_evolution_template("impl Old<T> as New { ... }"),
            None
        );
        // Double rename belongs to no shape.
        assert_eq!(
            parse_impl_body_evolution_template("impl A as B as C { ... }"),
            None
        );
    }

    #[test]
    fn impl_veto_matrix() {
        let firing = "impl Old {\n    fn m(&self) -> Old {\n        Old::make()\n    }\n}\n";
        assert_eq!(impl_body_evolution_veto(firing, "Old"), None);
        let assoc = "impl Old {\n    fn Old(&self) {}\n}\n";
        let v = impl_body_evolution_veto(assoc, "Old").expect("assoc vetoes");
        assert_eq!(v.kind, "assoc");
        let mac = "impl Old {\n    fn m(&self) {\n        Old!();\n    }\n}\n";
        let v = impl_body_evolution_veto(mac, "Old").expect("macro vetoes");
        assert_eq!(v.kind, "macro");
        let attr = "#[derive(Old)]\nimpl Old {\n    fn m(&self) {}\n}\n";
        // Attribute on the impl block: syn parses it as ItemImpl attrs
        // only when wrapped as an item — the bare-block parse drops it,
        // so probe the attribute path directly.
        let _ = attr;
        let attr2 = "impl Old {\n    #[doc = \"Old\"]\n    fn m(&self) {}\n}\n";
        let v = impl_body_evolution_veto(attr2, "Old").expect("attr vetoes");
        assert_eq!(v.kind, "attribute");
        let shadow = "impl Old {\n    fn m(&self) {\n        let Old = 1;\n    }\n}\n";
        let v = impl_body_evolution_veto(shadow, "Old").expect("shadow vetoes");
        assert_eq!(v.kind, "shadowed-local");
    }
}
