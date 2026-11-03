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

/// Needle for an unterminated fn signature (T174): the return/where
/// accumulator ran to end-of-tokens, so it swallowed trailing body lines
/// (`...`, `}`) into the slot text. The real fault is the missing `{`
/// (or `;`), not the slot content — say so before the `...`-content
/// gates (T161) can misfire on the swallowed tokens.
fn unterminated_sig(slot: &str) -> String {
    format!(
        "fn pattern {slot} is missing its opening `{{` (or `;` for declarations): the signature swallowed the body lines — put the brace on the signature line (SPEC 14.5)"
    )
}
/// Parse a single function pattern string like
/// `pub async fn foo<T>(...) -> T where T: Clone { ... }`.
///
/// Join one `#[...]` attribute's tokens back into its authored spacing
/// (WI-14 slice-5 residue): a blanket single-space join re-rendered
/// `#[inline(always)]` as `# [ inline ( always ) ]`, and the engine's
/// `compact_attr` restores every gap except ident-before-`(` — so every
/// paren/bracket/brace attr on a fn vetoed in the raiser probe and
/// mis-emitted when authored. String literals are single tokens from
/// `tokenize_pattern`, so punctuation-gated elision never touches
/// string interiors (unlike a post-hoc string `replace`, which would
/// corrupt `#[doc = "a (b)"]`).
fn join_attr_tokens(acc: &[String]) -> String {
    /// No space before these (closing/opening punctuation and infixes
    /// that bind left).
    fn no_space_before(t: &str) -> bool {
        matches!(
            t,
            "(" | "[" | "]" | ")" | "," | ";" | "::" | "->" | "..." | ".."
        )
    }
    let mut s = String::new();
    for (k, t) in acc.iter().enumerate() {
        if k == 0 {
            s.push_str(t);
            continue;
        }
        let prev = acc[k - 1].as_str();
        // `=>` arrives as two tokens (`=` then `>`); anything else
        // ending in `>` (`->`, `=>` already fused, `>`) binds left.
        let fat_arrow = t == ">" && prev == "=";
        if no_space_before(t) || fat_arrow || prev == "(" || prev == "[" || prev == "#" {
            s.push_str(t);
        } else {
            s.push(' ');
            s.push_str(t);
        }
    }
    s
}

/// Parse a single function pattern string like
/// `pub async fn foo<T>(...) -> T where T: Clone { ... }`.
///
/// Metavariable names for `V/Q/G/W` slots are supplied by the caller from
/// directives (visibility/qualifier_list/generic_params/where_clause kinds).
/// WI-5: `ident_metas` carries `# ident` names plus the inferred `N` hole;
/// a bare `N` in name position is always a hole (diet form).
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)]
pub fn parse_fn_pattern(
    text: &str,
    vis_metas: &[String],
    qual_metas: &[String],
    generic_metas: &[String],
    where_metas: &[String],
    type_metas: &[String],
    ident_metas: &[String],
    returns_none: bool,
) -> Result<FnPattern, StructuredError> {
    // Every gate keys a concrete `fix_pattern` template (code
    // `fn-pattern`) so `write_instead` never echoes the reason (SPEC D.2):
    // the first arg is the diagnostic, the second what to write instead.
    let fail = |msg: String, write_instead: &str| {
        StructuredError::new(ErrorCode::PatternConflict, msg).with_repair(
            StructuredError::fix_pattern_repair("fn-pattern", write_instead),
        )
    };
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
            "write `#[doc = \"...\"]` for doc comments, or use an ordinary hunk",
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
                return Err(fail(
                    "unbalanced attribute in fn pattern".to_string(),
                    "close the `#[...]` attribute with `]` before the `fn` line",
                ));
            }
            attrs_written.push(join_attr_tokens(&acc));
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
                return Err(fail(
                    "unbalanced pub(...) in fn pattern".to_string(),
                    "close the visibility with `)` (e.g. `pub(crate) fn foo(...) { ... })`",
                ));
            }
            // Token-concat strips the `pub(in …)` space (`pub(incrate)`,
            // unparseable on emit) — restore the canonical spelling so
            // the engine writes valid Rust verbatim.
            vis = crate::item_rename::canonical_vis_prefix(&vis);
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
        return Err(fail(
            format!("fn pattern must contain `fn` keyword, got `{text}`"),
            "start the pattern with the `fn` keyword (e.g. `fn foo(...) { ... })`",
        ));
    }
    i += 1;

    // Name.
    if i >= toks.len() {
        return Err(fail(
            "fn pattern missing function name".to_string(),
            "write `fn <name>(...) { ... }` with the function name after `fn`",
        ));
    }
    // Name. WI-5: `N` is the diet name hole (binds any ident, replays on
    // plus); `# ident X` declares any other name as a hole in this position.
    // Sigiled `$X` strips to `X` for the comparison and the stored name.
    let bare_name = toks[i].trim_start_matches('$').to_string();
    let name = if generic_metas.iter().any(|m| m == &bare_name)
        || vis_metas.iter().any(|m| m == &bare_name)
        || qual_metas.iter().any(|m| m == &bare_name)
        || where_metas.iter().any(|m| m == &bare_name)
        || ident_metas.iter().any(|m| m == &bare_name)
        || bare_name == "N"
    {
        NamePat::Meta(bare_name)
    } else {
        NamePat::Concrete(toks[i].clone())
    };
    i += 1;

    // WI-1b terse rename (`- fn foo`: no tokens remain after the name):
    // every value slot stays unconstrained, bodies preserved. Any slot
    // text (`(...)`, `<...>`, `-> T`, `where`, `{`, `;`) falls through to
    // the long-form path below.
    if i >= toks.len() {
        return Ok(crate::terse_fn::terse_pattern(
            attrs_written,
            visibility,
            qualifiers,
            qualifier_meta,
            name,
            returns_none,
        ));
    }

    // WI-1b: the fn line itself is bare (`fn foo`) but body/context lines
    // followed — neither terse (bare pair only) nor long (needs `(...)`).
    // Name the fix instead of the misleading missing-parens error below.
    if crate::terse_fn::is_stray_body_tail(&toks[i..]) {
        return Err(fail(
            "terse fn rename takes no body lines: write just `- fn foo` / `+ fn bar` (no `(...)`, `{`, `...`), or spell both sides long (`fn foo(...) { ... }`, SPEC 14.1)"
                .to_string(),
            "write just `- fn foo` / `+ fn bar`, or spell both sides long as `fn foo(...) { ... }`",
        ));
    }
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
            return Err(fail(
                "unbalanced generics in fn pattern".to_string(),
                "close the `<...>` generic list with `>` (e.g. `fn foo<T>(...) { ... })`",
            ));
        }
        // T161: `...` inside `<...>` can never equal a real target slice
        // (engine compares generics via normalized equality), so every
        // such hunk fails safe with a bare no-match. Fail fast with a
        // repair needle instead (mirrors the T160 params/body gates).
        if acc.iter().any(|t| t == "...") {
            return Err(fail(
                "`...` is not supported in generic parameter lists in v0.1: write concrete `<T>` or abstract with `# generic_params G` + `fn foo G (...)` (SPEC 14.7)".to_string(),
                "write `fn foo<T>(...) { ... }` with a concrete generic list, or add `# generic_params G` and write `fn foo G (...) { ... }`",
            ));
        }
        generics = GenericPat::Concrete(acc.join(" "));
    } else if i < toks.len() && generic_metas.contains(&toks[i]) {
        generics = GenericPat::Meta(toks[i].clone());
        i += 1;
    }

    // Parameter list.
    if i >= toks.len() || toks[i] != "(" {
        return Err(fail(
            "fn pattern missing parameter list".to_string(),
            "write `fn <name>(...)` with a parameter list after the name",
        ));
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
        return Err(fail(
            "unbalanced parameter list in fn pattern".to_string(),
            "close the parameter list with `)` (e.g. `fn foo(...) { ... })`",
        ));
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
            "write `(...)` for any parameters, or spell every parameter concretely",
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
        // WI-1c: a single-token return hole may be declared as
        // `# generic_params G` (historic) or inferred/declared as
        // `# type T`; both bind the return type as a metavariable.
        if i < toks.len() && (generic_metas.contains(&toks[i]) || type_metas.contains(&toks[i])) {
            ret = RetPat::Meta(toks[i].clone());
            i += 1;
        } else {
            let mut acc = Vec::new();
            while i < toks.len() && toks[i] != "where" && toks[i] != "{" && toks[i] != ";" {
                acc.push(toks[i].clone());
                i += 1;
            }
            if i == toks.len() && acc.iter().any(|t| t == "..." || t == "}") {
                return Err(fail(
                    unterminated_sig("`->` return type"),
                    "put the `{ ... }` body brace (or `;` for declarations) on the signature line",
                ));
            }
            if acc.is_empty() {
                return Err(fail(
                    "`->` without return type in fn pattern".to_string(),
                    "write a concrete type after `->` (e.g. `fn foo(...) -> T { ... })`, or omit `->` to match any return type",
                ));
            }
            // T161: `-> ...` can never equal a real return type (engine
            // compares via normalized equality) — fail fast instead of a
            // bare no-match. Omit `->` to match any return type.
            if acc.iter().any(|t| t == "...") {
                return Err(fail(
                    "`...` is not supported as a return type in v0.1: omit `->` to match any return type or write a concrete type (SPEC 14.7)".to_string(),
                    "write `fn foo(...)` with no `->` to match any return type, or `fn foo(...) -> T { ... }` with a concrete type",
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
        if i == toks.len() && acc.iter().any(|t| t == "..." || t == "}") {
            return Err(fail(
                unterminated_sig("`where` clause"),
                "put the `{ ... }` body brace (or `;` for declarations) on the signature line",
            ));
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
                    "write `where T: Clone` with concrete predicates, or add `# where_clause W` and write `... W ...`",
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
            return Err(fail(
                "unbalanced body block in fn pattern".to_string(),
                "close the body block with `}` (e.g. `fn foo(...) { ... })`",
            ));
        }
        // T160: `...` must stand alone in a body block too. The old
        // `any(...)" rule let mixed shapes like `{ ..., foo(); }`
        // through the must-be-`...` gates, silently preserving the body
        // without ever verifying the written statements.
        if inner.iter().any(|t| t == "...") && inner.iter().any(|t| t != "...") {
            return Err(fail(
                "function pattern bodies must be exactly `...` in v0.1 (body evolution is not supported; the original body is always preserved)".to_string(),
                "write the body as exactly `{ ... }`",
            ));
        }
        body_ellipsis = inner.iter().any(|t| t == "...");
    } else if i < toks.len() && toks[i] == ";" {
        body_ellipsis = true; // declaration without body matches any body
        i += 1;
    }
    if i < toks.len() {
        return Err(fail(
            format!(
                "unexpected trailing tokens in fn pattern: `{}`",
                toks[i..].join(" ")
            ),
            "end the pattern after the `{ ... }` body block (or `;` for declarations)",
        ));
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

/// WI-20 slice 1 + WI-13 slices 1–2: rename+body-pair joint shape
/// (SPEC 14.7).
///
/// A one-hunk joint carries the fn header pair in the proven single-line
/// `{ ... }` form plus one (WI-20 / WI-13 slice 1) or two (WI-13 slice
/// 2) extra `-`/`+` body line pairs (each a whole-line general
/// expression, e.g. `- x + 1` / `+ x + 2`, or a plain same-binder `let`
/// statement with an init). Each body pair applies only inside the
/// renamed definition's body (engine span-scoped `body` edits); a second
/// in-body hit or any out-of-body hit is a hard match conflict, never a
/// sweep.
///
/// `minus_raw`/`plus_raw` are the removal-only / addition-only texts (no
/// context lines in the canonical shape). Returns `None` for every other
/// shape (including attr-carrying headers and slot-evolving headers —
/// those stay on the existing error/suppress paths), so callers fall back
/// to the single-shape parse and its established errors.
pub struct FnBodyJoint {
    pub minus: FnPattern,
    pub plus: FnPattern,
    /// One or two whole-line body pairs (each `body_pair_holds`).
    /// Single-pair joints carry exactly one entry (WI-20 / WI-13 slice
    /// 1); two-pair joints carry two (WI-13 slice 2). Three or more
    /// stay on the existing error/suppress paths.
    pub bodies: Vec<(String, String)>,
}

/// Whitespace-insensitive concrete-slot equality for the joint header
/// gate (the engine compares slots the same way against target text).
fn joint_slot_eq(a: &str, b: &str) -> bool {
    a.split_whitespace().collect::<Vec<_>>().join(" ")
        == b.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn joint_slots_equal_except_name(minus: &FnPattern, plus: &FnPattern) -> bool {
    let vis_eq = match (&minus.visibility, &plus.visibility) {
        (VisPat::Absent, VisPat::Absent) => true,
        (VisPat::Concrete(a), VisPat::Concrete(b)) => joint_slot_eq(a, b),
        (VisPat::Meta(a), VisPat::Meta(b)) => a == b,
        _ => false,
    };
    let gen_eq = match (&minus.generics, &plus.generics) {
        (GenericPat::Absent, GenericPat::Absent) => true,
        (GenericPat::Concrete(a), GenericPat::Concrete(b)) => joint_slot_eq(a, b),
        (GenericPat::Meta(a), GenericPat::Meta(b)) => a == b,
        (GenericPat::Any, GenericPat::Any) => true,
        _ => false,
    };
    let ret_eq = match (&minus.ret, &plus.ret) {
        (RetPat::Any, RetPat::Any) => true,
        (RetPat::Absent, RetPat::Absent) => true,
        (RetPat::Concrete(a), RetPat::Concrete(b)) => joint_slot_eq(a, b),
        (RetPat::Meta(a), RetPat::Meta(b)) => a == b,
        _ => false,
    };
    let wh_eq = match (&minus.where_clause, &plus.where_clause) {
        (WherePat::Absent, WherePat::Absent) => true,
        (WherePat::Concrete(a), WherePat::Concrete(b)) => joint_slot_eq(a, b),
        (WherePat::Meta(a), WherePat::Meta(b)) => a == b,
        (WherePat::Any, WherePat::Any) => true,
        _ => false,
    };
    let params_eq = minus.params_ellipsis == plus.params_ellipsis
        && match (&minus.params_text, &plus.params_text) {
            (None, None) => true,
            (Some(a), Some(b)) => joint_slot_eq(a, b),
            _ => false,
        };
    vis_eq
        && minus.qualifiers == plus.qualifiers
        && minus.qualifier_meta == plus.qualifier_meta
        && gen_eq
        && params_eq
        && ret_eq
        && wh_eq
}

#[allow(clippy::too_many_arguments)]
pub fn try_parse_fn_body_joint(
    minus_raw: &str,
    plus_raw: &str,
    vis: &[String],
    qual: &[String],
    gen: &[String],
    wh: &[String],
    ty: &[String],
    ident: &[String],
    returns_none: bool,
) -> Option<FnBodyJoint> {
    let minus_lines: Vec<&str> = minus_raw.lines().collect();
    let plus_lines: Vec<&str> = plus_raw.lines().collect();
    // Header first, body pairs after (the canonical emission order).
    // One pair (WI-20 / WI-13 slice 1) or two pairs (WI-13 slice 2);
    // three or more stay on the existing error/suppress paths.
    if minus_lines.len() != plus_lines.len() || (minus_lines.len() != 2 && minus_lines.len() != 3) {
        return None;
    }
    let (hm, hp) = (minus_lines[0], plus_lines[0]);
    if !hm.contains("fn ") || !hp.contains("fn ") {
        return None;
    }
    for line in minus_lines[1..].iter().chain(plus_lines[1..].iter()) {
        if line.contains("fn ") {
            return None;
        }
    }
    let minus = parse_fn_pattern(hm, vis, qual, gen, wh, ty, ident, returns_none).ok()?;
    let plus = parse_fn_pattern(hp, vis, qual, gen, wh, ty, ident, false).ok()?;
    // Rename-only headers: `{ ... }` bodies, no written attributes, every
    // other slot equal.
    if !minus.body_ellipsis || !plus.body_ellipsis {
        return None;
    }
    if !minus.attrs_written.is_empty() || !plus.attrs_written.is_empty() {
        return None;
    }
    if !joint_slots_equal_except_name(&minus, &plus) {
        return None;
    }
    let names_differ = match (&minus.name, &plus.name) {
        (NamePat::Concrete(a), NamePat::Concrete(b)) => a != b,
        _ => false,
    };
    if !names_differ {
        return None;
    }
    // Whole-line body pairs, each differing text: a general expression
    // (WI-20) or a plain same-binder `let` statement with an init
    // (WI-13 slice 1), checked per pair (WI-13 slice 2 admits two).
    // Binder-changing `let` pairs decline (`None`) — binder renames
    // belong to D11/D16 capture, out of scope here.
    let mut bodies: Vec<(String, String)> = Vec::new();
    for (bm, bp) in minus_lines[1..].iter().zip(plus_lines[1..].iter()) {
        let (bm_t, bp_t) = (bm.trim(), bp.trim());
        if bm_t.is_empty() || bp_t.is_empty() || bm_t == bp_t {
            return None;
        }
        if !body_pair_holds(bm_t, bp_t) {
            return None;
        }
        bodies.push((bm_t.to_string(), bp_t.to_string()));
    }
    Some(FnBodyJoint {
        minus,
        plus,
        bodies,
    })
}

/// WI-13 slices 1–2: the per-pair half of the joint gate (shared law
/// with the raiser's `body_pair_ok`). Accepts one `syn::Expr` each side,
/// or one plain same-binder `let` (`syn::Stmt::Local`, both with `init`,
/// no attributes, binder `pat` token-equal whitespace-insensitively).
fn body_pair_holds(bm_t: &str, bp_t: &str) -> bool {
    if syn::parse_str::<syn::Expr>(bm_t).is_ok() && syn::parse_str::<syn::Expr>(bp_t).is_ok() {
        return true;
    }
    let (lm, lp) = match (
        syn::parse_str::<syn::Stmt>(bm_t),
        syn::parse_str::<syn::Stmt>(bp_t),
    ) {
        (Ok(syn::Stmt::Local(m)), Ok(syn::Stmt::Local(p))) => (m, p),
        _ => return false,
    };
    if !lm.attrs.is_empty() || !lp.attrs.is_empty() {
        return false;
    }
    if lm.init.is_none() || lp.init.is_none() {
        return false;
    }
    let norm = |pat: &syn::Pat| {
        quote::quote!(#pat)
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    norm(&lm.pat) == norm(&lp.pat)
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
        parse_fn_pattern(t, &[], &[], &[], &[], &[], &[], false).unwrap()
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
    fn diet_n_is_name_hole() {
        // WI-5: bare `N` in name position parses as a hole even with no
        // declaration; any other name stays concrete.
        assert_eq!(parse("fn N(...) { ... }").name, NamePat::Meta("N".into()));
        assert_eq!(
            parse("fn foo(...) { ... }").name,
            NamePat::Concrete("foo".into())
        );
        // Terse diet lines share the rule.
        assert_eq!(parse("fn N").name, NamePat::Meta("N".into()));
    }

    #[test]
    fn explicit_ident_declares_name_hole() {
        // WI-5 explicit: `# ident Foo` makes `Foo` a hole in name position.
        let p = parse_fn_pattern(
            "fn Foo(...) { ... }",
            &[],
            &[],
            &[],
            &[],
            &[],
            &["Foo".to_string()],
            false,
        )
        .unwrap();
        assert_eq!(p.name, NamePat::Meta("Foo".into()));
        // Without the declaration the same spelling is concrete.
        assert_eq!(
            parse("fn Foo(...) { ... }").name,
            NamePat::Concrete("Foo".into())
        );
    }

    #[test]
    fn strictness_rejects_pub() {
        let p = parse("fn foo(...) { ... }");
        let item: syn::ItemFn = syn::parse_str("pub fn foo() {}").unwrap();
        assert!(!p.matches_item_fn(&item));
    }

    #[test]
    fn pub_in_vis_keeps_its_space() {
        // The vis token-join strips whitespace (`pub(incrate)`); the
        // canonical spelling is what the engine emits verbatim, so the
        // join restores it — otherwise every `pub(in …)` fn rewrite is
        // unparseable on apply.
        assert_eq!(
            parse("pub(in crate) fn foo(...) { ... }").visibility,
            VisPat::Concrete("pub(in crate)".into())
        );
        assert_eq!(
            parse("pub(crate) fn foo(...) { ... }").visibility,
            VisPat::Concrete("pub(crate)".into())
        );
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
    fn attr_join_replays_authored_spacing() {
        // WI-14 slice-5 residue: token-join + `compact_attr` re-rendered
        // `#[inline(always)]` as `#[inline (always)]`, so every
        // paren/bracket/brace attr on a fn vetoed in the raiser probe
        // and mis-emitted when authored. Punctuation-gated elision
        // replays the authored bytes; string interiors (single tokens)
        // are never touched.
        let j = |s: &str| join_attr_tokens(&tokenize_pattern(s));
        assert_eq!(j("#[inline(always)]"), "#[inline(always)]");
        assert_eq!(j("#[inline]"), "#[inline]");
        assert_eq!(j("#[derive(Debug, Clone)]"), "#[derive(Debug, Clone)]");
        assert_eq!(
            j("#[cfg_attr(feature = \"x\", derive(Debug))]"),
            "#[cfg_attr(feature = \"x\", derive(Debug))]"
        );
        assert_eq!(j("#[serde(rename = \"y\")]"), "#[serde(rename = \"y\")]");
        assert_eq!(j("#[doc = \"hello\"]"), "#[doc = \"hello\"]");
        // String-interior spacing survives (the old blanket `replace`
        // posture would have fused it).
        assert_eq!(j("#[doc = \"a (b)\"]"), "#[doc = \"a (b)\"]");
        assert_eq!(j("#[doc = \"a ,b\"]"), "#[doc = \"a ,b\"]");
        // Same-line `#[a] fn` groups keep their text; the fn tail is
        // not swallowed.
        let p = parse("#[inline(always)]\nfn foo(...) { ... }");
        assert_eq!(p.attrs_written, vec!["#[inline(always)]"]);
        let p2 = parse("#[must_use] #[inline(never)]\nfn foo(...) { ... }");
        assert_eq!(p2.attrs_written.len(), 2, "got: {:?}", p2.attrs_written);
        assert_eq!(p2.attrs_written[1], "#[inline(never)]");
        // Multi-attr plus-side order is preserved verbatim.
        let p3 = parse("#[derive(Debug, Clone)]\nfn foo(...) { ... }");
        assert_eq!(p3.attrs_written, vec!["#[derive(Debug, Clone)]"]);
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
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], &[], &[], false)
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
        let err = parse_fn_pattern(
            "fn foo(...) { ..., foo(); }",
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            false,
        )
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
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("generic parameter"), "unexpected: {err}");
        }
        for shape in [
            "fn bar(...) -> ... { ... }",
            "fn bar(...) -> ... where T: Clone { ... }",
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("return type"), "unexpected: {err}");
        }
        for shape in [
            "fn foo<T>(...) where ... { ... }",
            "fn foo<T>(...) where T: Clone, ... { ... }",
            "fn foo<T>(...) where ..., T: Clone { ... }",
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], &[], &[], false)
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
            &[],
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
            &[],
            &[],
            false,
        )
        .unwrap();
        assert_eq!(w.where_clause, WherePat::Meta("W".into()));
    }

    #[test]
    fn unterminated_signature_fails_fast_with_needle() {
        // T174: without a `{`/`;` terminator the return/where accumulator
        // swallows the trailing body lines, tripping the T161 `...`-content
        // gate with a needle that blames the slot content. Name the real fault.
        for (shape, needle) in [
            ("fn rtest(...) -> i32\n...\n}", "`->` return type"),
            ("fn wtest<T>(...)\nwhere T: Clone\n...\n}", "`where` clause"),
        ] {
            let err = parse_fn_pattern(shape, &[], &[], &[], &[], &[], &[], false)
                .unwrap_err()
                .to_string();
            assert!(err.contains("missing its opening"), "unexpected: {err}");
            assert!(err.contains(needle), "unexpected: {err}");
        }
        // Terminated `...` slots still hit the T161 content gates; `;`
        // declarations and bare `W`-meta tails still parse.
        let err = parse_fn_pattern(
            "fn bar(...) -> ... { ... }",
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("return type"), "unexpected: {err}");
        assert!(parse("fn foo(...) -> i32;").body_ellipsis);
        let w = parse_fn_pattern(
            "fn foo G (...) W",
            &[],
            &[],
            &["G".into()],
            &["W".into()],
            &[],
            &[],
            false,
        )
        .unwrap();
        assert_eq!(w.where_clause, WherePat::Meta("W".into()));
    }

    #[test]
    fn wi20_body_joint_fires_on_header_plus_expr() {
        let joint = try_parse_fn_body_joint(
            "fn foo(x: i32) -> i32 { ... }\nx + 1",
            "fn bar(x: i32) -> i32 { ... }\nx + 2",
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .expect("header+expr joint must parse");
        assert_eq!(
            joint.bodies,
            vec![("x + 1".to_string(), "x + 2".to_string())]
        );
        assert!(joint.minus.body_ellipsis && joint.plus.body_ellipsis);
    }

    #[test]
    fn wi13_body_joint_fires_on_same_binder_let() {
        let joint = try_parse_fn_body_joint(
            "fn foo(x: i32) -> i32 { ... }\nlet y = 1;",
            "fn bar(x: i32) -> i32 { ... }\nlet y = 2;",
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .expect("header+same-binder-let joint must parse");
        assert_eq!(
            joint.bodies,
            vec![("let y = 1;".to_string(), "let y = 2;".to_string())]
        );
    }

    #[test]
    fn wi13_body_joint_fires_on_two_pairs() {
        // WI-13 slice 2: header + two body pairs (mixed expr + `let`).
        let joint = try_parse_fn_body_joint(
            "fn foo(x: i32) -> i32 { ... }\nlet y = 1;\ny + 1",
            "fn bar(x: i32) -> i32 { ... }\nlet y = 2;\ny + 2",
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
            false,
        )
        .expect("header+two-pair joint must parse");
        assert_eq!(
            joint.bodies,
            vec![
                ("let y = 1;".to_string(), "let y = 2;".to_string()),
                ("y + 1".to_string(), "y + 2".to_string()),
            ]
        );
    }

    #[test]
    fn wi20_body_joint_declines() {
        let no = |m: &str, p: &str| {
            try_parse_fn_body_joint(m, p, &[], &[], &[], &[], &[], &[], false).is_none()
        };
        // Single-shape headers (no body pair) are not joints.
        assert!(no(
            "fn foo(x: i32) -> i32 { ... }",
            "fn bar(x: i32) -> i32 { ... }"
        ));
        // Slot evolution is not the rename-only slice.
        assert!(no(
            "fn foo(x: i32) -> i32 { ... }\nx + 1",
            "fn bar(x: u32) -> i32 { ... }\nx + 2"
        ));
        // Three body pairs stay on the existing paths (WI-13 slice 2
        // admits exactly two).
        assert!(no(
            "fn foo(x: i32) { ... }\nlet y = x + 1;\ny * 2\nfoo(y)",
            "fn bar(x: i32) { ... }\nlet y = x + 2;\ny * 3\nbar(y)"
        ));
        // `let` without an init carries no init delta.
        assert!(no(
            "fn foo(x: i32) -> i32 { ... }\nlet y;",
            "fn bar(x: i32) -> i32 { ... }\nlet y;"
        ));
        // Binder-changing `let` pairs belong to D11/D16 capture.
        assert!(no(
            "fn foo(x: i32) -> i32 { ... }\nlet y = 1;",
            "fn bar(x: i32) -> i32 { ... }\nlet z = 1;"
        ));
        // Per-pair veto: one binder-changing pair poisons a two-pair
        // hunk even when the other pair is clean.
        assert!(no(
            "fn foo(x: i32) -> i32 { ... }\nlet y = 1;\ny + 1",
            "fn bar(x: i32) -> i32 { ... }\nlet z = 1;\ny + 2"
        ));
        // Identical body text is no delta.
        assert!(no(
            "fn foo(x: i32) -> i32 { ... }\nx + 1",
            "fn bar(x: i32) -> i32 { ... }\nx + 1"
        ));
        // Attribute headers stay on the existing paths.
        assert!(no(
            "#[inline]\nfn foo(x: i32) -> i32 { ... }\nx + 1",
            "#[inline]\nfn bar(x: i32) -> i32 { ... }\nx + 2"
        ));
    }
}
