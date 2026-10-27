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

/// Normalize a leading doc-comment line (`/// text`, `//! text`, or
/// single-line `/** text */`) to its canonical attribute form
/// (`#[doc = " text"]`) so whitespace-insensitive attribute comparison
/// meets `quote!`-rendered targets (T155; `/** */` arm T156).
///
/// Returns `None` for non-doc lines — including `////...` (an ordinary
/// comment, never a doc attribute) and multi-line block docs (no
/// leading-line structure, so they stay fail-fast). The body is kept
/// verbatim (leading space included: `/// X` maps to `" X"`, matching
/// `syn`), with `\` and `"` escaped so the canonical form stays
/// well-formed.
pub(crate) fn doc_comment_to_attr(trimmed_line: &str) -> Option<String> {
    let body = if let Some(rest) = trimmed_line.strip_prefix("///") {
        // `////` or more is an ordinary comment, never a doc attribute.
        if rest.starts_with('/') {
            return None;
        }
        rest
    } else if let Some(rest) = trimmed_line.strip_prefix("/**") {
        // Single-line block docs only (`/** text */`); `/***...` and
        // `/**/` are ordinary comments, multi-line stays `None`.
        if rest.starts_with('/') || rest.starts_with('*') {
            return None;
        }
        let inner = rest.strip_suffix("*/")?;
        inner
    } else {
        trimmed_line.strip_prefix("//!")?
    };
    let escaped = body.replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!("#[doc = \"{escaped}\"]"))
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
    // Leading doc-comment lines (`///`, `//!`, single-line `/** */`):
    // peel at the string level — the whitespace tokenizer below cannot
    // preserve a doc line's trailing text — and normalize to
    // `#[doc = "..."]` (T155; block arm T156).
    let mut attrs_written = Vec::new();
    let all_lines: Vec<&str> = text.lines().collect();
    let mut peeled = 0;
    while peeled < all_lines.len() {
        match doc_comment_to_attr(all_lines[peeled].trim()) {
            Some(attr) => {
                attrs_written.push(attr);
                peeled += 1;
            }
            None => break,
        }
    }
    // A leading `/*` the helper cannot normalize (multi-line block doc,
    // ordinary block comment) is never a valid fn-pattern start — fail
    // fast instead of the misleading missing-`fn`-keyword error (T156).
    if peeled < all_lines.len() && all_lines[peeled].trim_start().starts_with("/*") {
        return Err(fail(
            "block comments (`/* ... */`) are not supported in patterns in v1 (mirror doc comments with `#[doc = \"...\"]` or use an ordinary hunk)"
                .to_string(),
        ));
    }
    let toks = tokenize_pattern(&all_lines[peeled..].join("\n"));
    let mut i = 0;

    // Optional attributes on their own lines: `#[...]` or doc `///`.
    // Note: the tokenizer emits `#` and `[` as separate tokens, so an
    // attribute opens with the two-token pair `#` + `[`, never `"#["`.
    // (`///`/`//!` lines are already peeled above; the token arms below
    // stay as fallback for bare markers with no trailing text.)
    while i < toks.len()
        && ((toks[i] == "#" && toks.get(i + 1).is_some_and(|n| n == "["))
            || toks[i] == "///"
            || toks[i] == "//!")
    {
        if toks[i] == "#" {
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
        // T161: `...` inside `<...>` can never equal a real target slice
        // (engine compares generics via normalized equality), so every
        // such hunk fails safe with a bare no-match. Fail fast with a
        // repair needle instead (mirrors the T160 params/body gates).
        if acc.iter().any(|t| t == "...") {
            return Err(fail(
                "`...` is not supported in generic parameter lists in v0.1: write concrete `<T>` or abstract with `# generic_params G` + `fn foo G (...)` (SPEC 14.7)".to_string(),
            ));
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
    // T160: `...` must stand alone in a parameter list. The old
    // `any(...)" rule skipped matching wholesale for mixed shapes like
    // `(..., c: T)`, so metavars were never bound yet textually "used" —
    // the plus side then emitted them verbatim (silent corruption) or
    // the written suffix was silently never verified. Lone commas are
    // tolerated (`(...)` ≡ `(...,)`); anything else mixed with `...`
    // fails fast.
    if params_inner.iter().any(|t| t == "...")
        && params_inner.iter().any(|t| t != "..." && t != ",")
    {
        return Err(fail(
            "`...` in a function parameter list must stand alone: write `(...)` to match any parameters or fully-concrete parameters (SPEC 14.7)"
                .to_string(),
        ));
    }
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
            // T161: `-> ...` can never equal a real return type (engine
            // compares via normalized equality) — fail fast instead of a
            // bare no-match. Omit `->` to match any return type.
            if acc.iter().any(|t| t == "...") {
                return Err(fail(
                    "`...` is not supported as a return type in v0.1: omit `->` to match any return type or write a concrete type (SPEC 14.7)".to_string(),
                ));
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
            // T161: `...` inside `where` predicates can never equal real
            // predicates (engine compares via normalized equality) — fail
            // fast instead of a bare no-match.
            if acc.iter().any(|t| t == "...") {
                return Err(fail(
                    "`...` is not supported in where clauses in v0.1: write concrete predicates or abstract with `# where_clause W` + `... W ...` (SPEC 14.7)".to_string(),
                ));
            }
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
        // T160: `...` must stand alone in a body block too. The old
        // `any(...)" rule let mixed shapes like `{ ..., foo(); }`
        // through the must-be-`...` gates, silently preserving the body
        // without ever verifying the written statements.
        if inner.iter().any(|t| t == "...") && inner.iter().any(|t| t != "...") {
            return Err(fail(
                "function pattern bodies must be exactly `...` in v0.1 (body evolution is not supported; the original body is always preserved)".to_string(),
            ));
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

    #[test]
    fn leading_attribute_lines_recorded() {
        // T153: the tokenizer emits `#` + `[` as separate tokens, so the
        // attribute loop must detect the pair (the old `"#["` literal never
        // matched and explicit-attribute fn patterns failed to parse).
        let p = parse("#[inline]\nfn foo(...) { ... }");
        assert_eq!(p.attrs_written.len(), 1, "got: {:?}", p.attrs_written);
        let p2 = parse("#[inline]\n#[must_use]\nfn foo(...) { ... }");
        assert_eq!(p2.attrs_written.len(), 2, "got: {:?}", p2.attrs_written);
        // T155: `/// text` doc lines peel at the string level into
        // `#[doc = " text"]`, so doc-comment mirroring parses like any
        // other explicit attribute.
        let p3 = parse("/// Doc comment.\nfn foo(...) { ... }");
        assert_eq!(p3.attrs_written.len(), 1, "got: {:?}", p3.attrs_written);
        let p4 = parse("//! Inner doc.\nfn foo(...) { ... }");
        assert_eq!(p4.attrs_written.len(), 1, "got: {:?}", p4.attrs_written);
    }

    #[test]
    fn doc_comment_to_attr_cases() {
        // Leading space kept verbatim (`/// X` is `" X"`, matching `syn`).
        assert_eq!(
            doc_comment_to_attr("/// Doc."),
            Some("#[doc = \" Doc.\"]".to_string())
        );
        assert_eq!(
            doc_comment_to_attr("//! Inner."),
            Some("#[doc = \" Inner.\"]".to_string())
        );
        // No space, no problem.
        assert_eq!(
            doc_comment_to_attr("///Doc."),
            Some("#[doc = \"Doc.\"]".to_string())
        );
        // `////` is an ordinary comment, never a doc attribute.
        assert_eq!(doc_comment_to_attr("//// Not doc."), None);
        assert_eq!(doc_comment_to_attr("// ordinary"), None);
        // Quotes and backslashes escape so the canonical form stays
        // well-formed.
        assert_eq!(
            doc_comment_to_attr("/// Say \"hi\" \\ bye."),
            Some("#[doc = \" Say \\\"hi\\\" \\\\ bye.\"]".to_string())
        );
        // T156: single-line block docs normalize with the verbatim inner
        // body (leading/trailing space kept, matching `syn`).
        assert_eq!(
            doc_comment_to_attr("/** Block doc. */"),
            Some("#[doc = \" Block doc. \"]".to_string())
        );
        assert_eq!(
            doc_comment_to_attr("/**No spaces*/"),
            Some("#[doc = \"No spaces\"]".to_string())
        );
        assert_eq!(
            doc_comment_to_attr("/** Say \"hi\" \\ bye. */"),
            Some("#[doc = \" Say \\\"hi\\\" \\\\ bye. \"]".to_string())
        );
        // Not doc comments: `/**/` (empty block), `/***...` (extra star),
        // unclosed `/**` (multi-line opener), ordinary block/line comments.
        assert_eq!(doc_comment_to_attr("/**/"), None);
        assert_eq!(doc_comment_to_attr("/***/"), None);
        assert_eq!(doc_comment_to_attr("/*** Doc. */"), None);
        assert_eq!(doc_comment_to_attr("/** Unclosed."), None);
        assert_eq!(doc_comment_to_attr("/* plain */"), None);
        assert_eq!(doc_comment_to_attr("/* plain"), None);
    }

    #[test]
    fn mixed_param_ellipsis_fails_fast() {
        // T160: `...` mixed with other param tokens never matched
        // structurally (any-rule skipped the whole list) yet kept the
        // plus side "bound", leaking raw metavars into the output.
        for shape in [
            "fn foo(..., c: T) { ... }",
            "fn foo(T, ..., c: i32) { ... }",
            "fn foo(c: i32, ...) { ... }",
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("must stand alone"), "unexpected: {err}");
        }
        // Pure shapes still parse: lone `...`, comma variant, empty,
        // and fully concrete.
        assert!(parse("fn foo(...) { ... }").params_ellipsis);
        assert!(parse("fn foo(...,) { ... }").params_ellipsis);
        assert!(!parse("fn foo() { ... }").params_ellipsis);
        assert!(!parse("fn foo(a: i32, b: i32) { ... }").params_ellipsis);
    }

    #[test]
    fn mixed_body_ellipsis_fails_fast() {
        // T160: `{ ..., foo(); }` slipped through the must-be-`...`
        // gates (any-rule) and the written statements were silently
        // never verified; bodies are always preserved.
        let err = parse_fn_pattern("fn foo(...) { ..., foo(); }", &[], &[], &[], &[], false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("exactly `...`"), "unexpected: {err}");
        assert!(parse("fn foo(...) { ... }").body_ellipsis);
        assert!(parse("fn foo(...) {...}").body_ellipsis);
    }

    #[test]
    fn leading_block_comment_fails_fast_with_needle() {
        // T156: a leading `/*` the helper cannot normalize fails with the
        // targeted needle, not the misleading missing-`fn`-keyword error.
        let err = parse_fn_pattern(
            "/* plain */\nfn foo(...) { ... }",
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("block comments"), "unexpected: {err}");
        let err = parse_fn_pattern(
            "/** Multi\nline. */\nfn foo(...) { ... }",
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("block comments"), "unexpected: {err}");
        // Single-line block doc normalizes and parses (T156).
        let pat = parse_fn_pattern(
            "/** Block doc. */\nfn foo(...) { ... }",
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .unwrap();
        assert_eq!(pat.attrs_written, vec!["#[doc = \" Block doc. \"]"]);
    }

    #[test]
    fn ellipsis_in_generics_where_return_fails_fast() {
        // T161: `...` inside `<...>`, `where ...`, or `-> ...` can never
        // equal a real target slice (normalized equality) — fail fast
        // with a repair needle instead of a bare no-match.
        for shape in [
            "fn foo<...>(...) { ... }",
            "fn foo<T, ...>(...) { ... }",
            "fn foo<..., T>(...) { ... }",
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("generic parameter"), "unexpected: {err}");
        }
        for shape in [
            "fn bar(...) -> ... { ... }",
            "fn bar(...) -> ... where T: Clone { ... }",
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("return type"), "unexpected: {err}");
        }
        for shape in [
            "fn foo<T>(...) where ... { ... }",
            "fn foo<T>(...) where T: Clone, ... { ... }",
            "fn foo<T>(...) where ..., T: Clone { ... }",
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("where clause"), "unexpected: {err}");
        }
        // Pure shapes still parse: concrete generics/where/return and
        // omitted slots (plus `G`/`W` metas, which wildcard those slots).
        assert!(parse("fn foo<T>(...) { ... }").generics != GenericPat::Absent);
        assert!(parse("fn foo<T>(...) where T: Clone { ... }").where_clause != WherePat::Absent);
        assert!(!matches!(
            parse("fn foo(...) { ... }").ret,
            RetPat::Concrete(_)
        ));
        assert!(parse("fn foo(...) -> i32 { ... }").ret != RetPat::Any);
        let g = parse_fn_pattern(
            "fn foo G (...) { ... }",
            &[],
            &[],
            &["G".into()],
            &[],
            false,
        )
        .unwrap();
        assert_eq!(g.generics, GenericPat::Meta("G".into()));
        let w = parse_fn_pattern(
            "fn foo<T>(...) W { ... }",
            &[],
            &[],
            &[],
            &["W".into()],
            false,
        )
        .unwrap();
        assert_eq!(w.where_clause, WherePat::Meta("W".into()));
    }
}
