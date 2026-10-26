//! Function-item pattern parsing with strict slot semantics (SPEC 14).
//!
//! `fn foo(...) { ... }` requires: no visibility, no qualifiers, name `foo`,
//! no generics, any params, any return type, no where clause, any body.
//! Attributes are governed by `# attrs=...` (handled by the engine), so the
//! pattern parser only records explicitly written attributes.

use rsmpl_patch::{ErrorCode, StructuredError};

/// How the pattern constrains visibility.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisPat {
    Absent,
    Concrete(String),
    Meta(String),
}

/// How the pattern constrains the function name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamePat {
    Concrete(String),
    Meta(String),
}

/// Generic parameters constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenericPat {
    Absent,
    Concrete(String),
    Meta(String),
    Any,
}

/// Return-type constraint. `Any` is the `fn foo(...)` shorthand (matches
/// with or without `-> T`); `Absent` requires no return type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetPat {
    Any,
    Absent,
    Concrete(String),
    Meta(String),
}

/// Where-clause constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WherePat {
    Absent,
    Concrete(String),
    Meta(String),
    Any,
}

/// Structured function pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnPattern {
    pub attrs_written: Vec<String>,
    pub visibility: VisPat,
    pub qualifiers: Vec<String>,
    pub qualifier_meta: Option<String>,
    pub name: NamePat,
    pub generics: GenericPat,
    pub params_ellipsis: bool,
    pub params_text: Option<String>,
    pub ret: RetPat,
    pub where_clause: WherePat,
    pub body_ellipsis: bool,
    pub returns_none_directive: bool,
}

impl FnPattern {
    /// Check a `syn::ItemFn` against this pattern (syntax-level; modifiers
    /// strict, params/body ellipsis, return shorthand). Attributes and span
    /// checks are the engine's job.
    pub fn matches_item_fn(&self, item: &syn::ItemFn) -> bool {
        use syn::Visibility as V;
        // Visibility.
        match &self.visibility {
            VisPat::Absent => {
                if !matches!(item.vis, V::Inherited) {
                    return false;
                }
            }
            VisPat::Concrete(_) => {
                // Concrete visibility text must equal rendered vis.
                let rendered = quote::quote!(#item)
                    .to_string()
                    .split("fn")
                    .next()
                    .unwrap_or("")
                    .to_string();
                let _ = rendered;
                // Simplified: any non-inherited vis matches a concrete vis
                // pattern; exact equality is checked by the engine via text.
                if matches!(item.vis, V::Inherited) {
                    return false;
                }
            }
            VisPat::Meta(_) => {}
        }
        // Qualifiers.
        if let Some(_q) = &self.qualifier_meta {
            // Abstracted: always matches at syntax level.
        } else if !self.qualifiers.is_empty() {
            let have: Vec<String> = fn_qualifiers(item).collect();
            if have != self.qualifiers {
                return false;
            }
        } else if item.sig.constness.is_some()
            || item.sig.asyncness.is_some()
            || item.sig.unsafety.is_some()
            || item.sig.abi.is_some()
        {
            return false;
        }
        // Name.
        match &self.name {
            NamePat::Concrete(n) => {
                if item.sig.ident != n {
                    return false;
                }
            }
            NamePat::Meta(_) => {}
        }
        // Generics.
        match &self.generics {
            GenericPat::Absent => {
                if !item.sig.generics.params.is_empty() {
                    return false;
                }
            }
            GenericPat::Meta(_) | GenericPat::Any => {}
            GenericPat::Concrete(_) => {
                if item.sig.generics.params.is_empty() {
                    return false;
                }
            }
        }
        // Return type.
        match &self.ret {
            RetPat::Any => {}
            RetPat::Absent => {
                if !matches!(item.sig.output, syn::ReturnType::Default) {
                    return false;
                }
            }
            RetPat::Concrete(_) | RetPat::Meta(_) => {
                // Presence required; exact type equality is engine-checked.
                if matches!(item.sig.output, syn::ReturnType::Default) {
                    return false;
                }
            }
        }
        // Where clause.
        match &self.where_clause {
            WherePat::Absent => {
                if item.sig.generics.where_clause.is_some() {
                    return false;
                }
            }
            WherePat::Meta(_) | WherePat::Any => {}
            WherePat::Concrete(_) => {
                if item.sig.generics.where_clause.is_none() {
                    return false;
                }
            }
        }
        true
    }
}

/// Rendered qualifier list in pattern-token form (`extern "C"` keeps its
/// ABI literal, matching what `parse_fn_pattern` stores).
fn fn_qualifiers(item: &syn::ItemFn) -> impl Iterator<Item = String> {
    let mut v = Vec::new();
    if item.sig.constness.is_some() {
        v.push("const".to_string());
    }
    if item.sig.asyncness.is_some() {
        v.push("async".to_string());
    }
    if item.sig.unsafety.is_some() {
        v.push("unsafe".to_string());
    }
    if let Some(abi) = &item.sig.abi {
        match &abi.name {
            Some(lit) => v.push(format!("extern {}", lit.token())),
            None => v.push("extern".to_string()),
        }
    }
    v.into_iter()
}

/// Parse a single function pattern string like
/// `pub async fn foo<T>(...) -> T where T: Clone { ... }`.
///
/// Metavariable names for `V/Q/G/W` slots are supplied by the caller from
/// directives (visibility/qualifier_list/generic_params/where_clause kinds).
#[allow(clippy::too_many_lines)]
pub fn parse_fn_pattern(
    text: &str,
    vis_metas: &[String],
    qual_metas: &[String],
    generic_metas: &[String],
    where_metas: &[String],
    returns_none: bool,
) -> Result<FnPattern, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg);
    let toks = tokenize_pattern(text);
    let mut i = 0;

    // Optional attributes on their own lines: `#[...]` or doc `///`.
    let mut attrs_written = Vec::new();
    while i < toks.len() && (toks[i] == "#[" || toks[i] == "///" || toks[i] == "//!") {
        if toks[i] == "#[" {
            let mut depth = 0;
            let mut acc = Vec::new();
            while i < toks.len() {
                if toks[i] == "[" {
                    depth += 1;
                }
                if toks[i] == "]" {
                    depth -= 1;
                    acc.push(toks[i].clone());
                    i += 1;
                    break;
                }
                acc.push(toks[i].clone());
                i += 1;
            }
            if depth != 0 {
                return Err(fail("unbalanced attribute in fn pattern".to_string()));
            }
            attrs_written.push(acc.join(" "));
        } else {
            attrs_written.push(toks[i].clone());
            i += 1;
        }
    }

    // Visibility: `pub`, `pub(...)`, or visibility metavariable.
    let mut visibility = VisPat::Absent;
    if i < toks.len() && toks[i] == "pub" {
        let mut vis = String::from("pub");
        i += 1;
        if i < toks.len() && toks[i] == "(" {
            let mut depth = 0;
            while i < toks.len() {
                vis.push_str(&toks[i]);
                if toks[i] == "(" {
                    depth += 1;
                }
                if toks[i] == ")" {
                    depth -= 1;
                    i += 1;
                    break;
                }
                i += 1;
            }
            if depth != 0 {
                return Err(fail("unbalanced pub(...) in fn pattern".to_string()));
            }
        }
        visibility = VisPat::Concrete(vis);
    } else if i < toks.len() && vis_metas.contains(&toks[i]) {
        visibility = VisPat::Meta(toks[i].clone());
        i += 1;
    }

    // Qualifiers in any order: const async unsafe extern ["ABI"].
    let mut qualifiers = Vec::new();
    let mut qualifier_meta = None;
    loop {
        if i < toks.len() && qual_metas.contains(&toks[i]) {
            qualifier_meta = Some(toks[i].clone());
            i += 1;
            continue;
        }
        if i < toks.len() && ["const", "async", "unsafe", "extern"].contains(&toks[i].as_str()) {
            let q = toks[i].clone();
            i += 1;
            // `extern "ABI"`.
            if q == "extern" && i < toks.len() && toks[i].starts_with('"') {
                qualifiers.push(format!("extern {}", toks[i]));
                i += 1;
            } else {
                qualifiers.push(q);
            }
            continue;
        }
        break;
    }

    // `fn` keyword required.
    if i >= toks.len() || toks[i] != "fn" {
        return Err(fail(format!(
            "fn pattern must contain `fn` keyword, got `{text}`"
        )));
    }
    i += 1;

    // Name.
    if i >= toks.len() {
        return Err(fail("fn pattern missing function name".to_string()));
    }
    let name = if generic_metas.contains(&toks[i])
        || vis_metas.contains(&toks[i])
        || qual_metas.contains(&toks[i])
        || where_metas.contains(&toks[i])
    {
        NamePat::Meta(toks[i].clone())
    } else {
        NamePat::Concrete(toks[i].clone())
    };
    i += 1;

    // Generics: `<...>` or generic metavariable or absent.
    let mut generics = GenericPat::Absent;
    if i < toks.len() && toks[i] == "<" {
        let mut depth = 0;
        let mut acc = Vec::new();
        while i < toks.len() {
            if toks[i] == "<" {
                depth += 1;
            }
            if toks[i] == ">" {
                depth -= 1;
                acc.push(toks[i].clone());
                i += 1;
                if depth == 0 {
                    break;
                }
                continue;
            }
            acc.push(toks[i].clone());
            i += 1;
        }
        if depth != 0 {
            return Err(fail("unbalanced generics in fn pattern".to_string()));
        }
        generics = GenericPat::Concrete(acc.join(" "));
    } else if i < toks.len() && generic_metas.contains(&toks[i]) {
        generics = GenericPat::Meta(toks[i].clone());
        i += 1;
    }

    // Parameter list.
    if i >= toks.len() || toks[i] != "(" {
        return Err(fail("fn pattern missing parameter list".to_string()));
    }
    let mut depth = 0;
    let mut params_inner = Vec::new();
    while i < toks.len() {
        if toks[i] == "(" {
            depth += 1;
            if depth > 1 {
                params_inner.push(toks[i].clone());
            }
            i += 1;
            continue;
        }
        if toks[i] == ")" {
            depth -= 1;
            if depth == 0 {
                i += 1;
                break;
            }
            params_inner.push(toks[i].clone());
            i += 1;
            continue;
        }
        params_inner.push(toks[i].clone());
        i += 1;
    }
    if depth != 0 {
        return Err(fail("unbalanced parameter list in fn pattern".to_string()));
    }
    let params_joined = params_inner.join(" ");
    let params_ellipsis = params_inner.iter().any(|t| t == "...");
    let params_text = if params_joined.trim().is_empty() {
        None
    } else {
        Some(params_joined)
    };

    // Return type.
    let mut ret = RetPat::Any;
    if i < toks.len() && toks[i] == "->" {
        i += 1;
        if i < toks.len() && generic_metas.contains(&toks[i]) {
            ret = RetPat::Meta(toks[i].clone());
            i += 1;
        } else {
            let mut acc = Vec::new();
            while i < toks.len() && toks[i] != "where" && toks[i] != "{" && toks[i] != ";" {
                acc.push(toks[i].clone());
                i += 1;
            }
            if acc.is_empty() {
                return Err(fail("`->` without return type in fn pattern".to_string()));
            }
            ret = RetPat::Concrete(acc.join(" "));
        }
    } else if returns_none {
        ret = RetPat::Absent;
    }

    // Where clause.
    let mut where_clause = WherePat::Absent;
    if i < toks.len() && toks[i] == "where" {
        i += 1;
        let mut acc = Vec::new();
        while i < toks.len() && toks[i] != "{" && toks[i] != ";" {
            acc.push(toks[i].clone());
            i += 1;
        }
        if acc.len() == 1 && where_metas.contains(&acc[0]) {
            where_clause = WherePat::Meta(acc[0].clone());
        } else {
            where_clause = WherePat::Concrete(acc.join(" "));
        }
    } else if i < toks.len() && where_metas.contains(&toks[i]) {
        where_clause = WherePat::Meta(toks[i].clone());
        i += 1;
    }

    // Body: `{ ... }` / `{...}` / concrete / `;` (declaration).
    let mut body_ellipsis = false;
    if i < toks.len() && toks[i] == "{" {
        let mut inner = Vec::new();
        let mut depth = 0;
        while i < toks.len() {
            if toks[i] == "{" {
                depth += 1;
                if depth > 1 {
                    inner.push(toks[i].clone());
                }
                i += 1;
                continue;
            }
            if toks[i] == "}" {
                depth -= 1;
                if depth == 0 {
                    i += 1;
                    break;
                }
                inner.push(toks[i].clone());
                i += 1;
                continue;
            }
            inner.push(toks[i].clone());
            i += 1;
        }
        if depth != 0 {
            return Err(fail("unbalanced body block in fn pattern".to_string()));
        }
        body_ellipsis = inner.iter().any(|t| t == "...");
    } else if i < toks.len() && toks[i] == ";" {
        body_ellipsis = true; // declaration without body matches any body
        i += 1;
    }
    if i < toks.len() {
        return Err(fail(format!(
            "unexpected trailing tokens in fn pattern: `{}`",
            toks[i..].join(" ")
        )));
    }

    Ok(FnPattern {
        attrs_written,
        visibility,
        qualifiers,
        qualifier_meta,
        name,
        generics,
        params_ellipsis,
        params_text,
        ret,
        where_clause,
        body_ellipsis,
        returns_none_directive: returns_none,
    })
}

/// Minimal tokenizer keeping multi-char tokens (`...`, `->`, `::`) intact.
fn tokenize_pattern(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(cur.clone());
            cur.clear();
        }
    };
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            flush(&mut cur, &mut out);
        } else if c == '.' && chars.peek() == Some(&'.') {
            flush(&mut cur, &mut out);
            chars.next();
            if chars.peek() == Some(&'.') {
                chars.next();
                out.push("...".to_string());
            } else {
                out.push("..".to_string());
            }
        } else if c == ':' && chars.peek() == Some(&':') {
            flush(&mut cur, &mut out);
            chars.next();
            out.push("::".to_string());
        } else if c == '-' && chars.peek() == Some(&'>') {
            flush(&mut cur, &mut out);
            chars.next();
            out.push("->".to_string());
        } else if "(){}<>[],;#".contains(c) {
            flush(&mut cur, &mut out);
            out.push(c.to_string());
        } else if c == '"' {
            flush(&mut cur, &mut out);
            let mut s = String::from("\"");
            for nc in chars.by_ref() {
                s.push(nc);
                if nc == '"' {
                    break;
                }
            }
            out.push(s);
        } else {
            cur.push(c);
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(t: &str) -> FnPattern {
        parse_fn_pattern(t, &[], &[], &[], &[], false).unwrap()
    }

    #[test]
    fn plain_fn_slots() {
        let p = parse("fn foo(...) { ... }");
        assert_eq!(p.visibility, VisPat::Absent);
        assert!(p.qualifiers.is_empty());
        assert_eq!(p.name, NamePat::Concrete("foo".into()));
        assert_eq!(p.generics, GenericPat::Absent);
        assert!(p.params_ellipsis);
        assert!(p.body_ellipsis);
    }

    #[test]
    fn strictness_rejects_pub() {
        let p = parse("fn foo(...) { ... }");
        let item: syn::ItemFn = syn::parse_str("pub fn foo() {}").unwrap();
        assert!(!p.matches_item_fn(&item));
    }

    #[test]
    fn strictness_rejects_generics() {
        let p = parse("fn foo(...) { ... }");
        let item: syn::ItemFn = syn::parse_str("fn foo<T>(x: T) {}").unwrap();
        assert!(!p.matches_item_fn(&item));
    }

    #[test]
    fn return_shorthand_matches_both() {
        let p = parse("fn foo(...) { ... }");
        let a: syn::ItemFn = syn::parse_str("fn foo() {}").unwrap();
        let b: syn::ItemFn = syn::parse_str("fn foo() -> i32 { 1 }").unwrap();
        assert!(p.matches_item_fn(&a));
        assert!(p.matches_item_fn(&b));
    }

    #[test]
    fn pub_pattern_matches_pub() {
        let p = parse("pub fn foo(...) { ... }");
        let item: syn::ItemFn = syn::parse_str("pub fn foo() {}").unwrap();
        assert!(p.matches_item_fn(&item));
    }
}
