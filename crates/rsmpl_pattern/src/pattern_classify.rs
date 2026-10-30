//! Hunk routing: from minus/plus/context text to a `PatternKind` (SPEC 13.4).
//!
//! Explicit `# root=` wins; otherwise fn/item/call/macro/insert heuristics.
//! Generalized shapes delegate to `pattern_general`.

use rsmpl_patch::{ErrorCode, MetavarKind, RsmplHunk, StructuredError};

use crate::expr_pattern::{classify_call_pattern, find_macro_bang, ExprPatternKind};
use crate::fn_pattern::parse_fn_pattern;
use crate::metavar::MetavarTable;
use crate::pattern::{GeneralShape, PatternKind};
use crate::pattern_evolution::{
    parse_const_evolution_template, parse_enum_evolution_template, parse_evolution_template,
    parse_static_evolution_template, parse_struct_evolution_template,
    parse_type_evolution_template, parse_union_evolution_template,
};
use crate::pattern_general::try_general;
use crate::quote_utils::qualifier_leads_to_fn;

/// Word-bounded `fn` keyword followed by whitespace (a line that looks
/// like an `fn` header, valid template or not).
fn has_fn_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..].starts_with(b"fn")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Word-bounded `struct` keyword followed by whitespace (a line that
/// looks like a `struct` header, valid template or not).
fn has_struct_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"struct")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Word-bounded `enum` keyword followed by whitespace (a line that
/// looks like an `enum` header, valid template or not).
fn has_enum_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"enum")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Word-bounded `union` keyword followed by whitespace (a line that
/// looks like a `union` header, valid template or not).
fn has_union_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"union")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Word-bounded `type` keyword followed by whitespace (a line that
/// looks like a `type` alias header, valid template or not).
fn has_type_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"type")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Word-bounded `const` keyword followed by whitespace (a line that
/// looks like a `const` item header, valid template or not).
fn has_const_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"const")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Word-bounded `static` keyword followed by whitespace (a line that
/// looks like a `static` item header, valid template or not).
fn has_static_keyword(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"static")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            return true;
        }
        i += 1;
    }
    false
}
#[allow(clippy::too_many_lines)]
pub(crate) fn classify(
    minus_raw: &str,
    plus_raw: &str,
    context_raw: &str,
    table: &MetavarTable,
    hunk: &RsmplHunk,
    root: Option<&str>,
) -> Result<PatternKind, StructuredError> {
    let m = minus_raw.trim();
    let p = plus_raw.trim();
    let c = context_raw.trim();

    // Explicit root wins.
    if let Some(r) = root {
        match r {
            "item" => {
                if is_single_meta(m, table) {
                    return Ok(PatternKind::ItemCapture {
                        meta: m.to_string(),
                    });
                }
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "`# root=item` requires a single-metavar item-capture pattern (write `- I` for a declared `# item I` metavariable)",
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "C16",
                    "write `- I` for a declared `# item I` metavariable",
                )));
            }
            "expr" => {
                let (minus, maps, lits) = classify_call_pattern(m, table)?;
                if matches!(minus, ExprPatternKind::Raw { .. }) {
                    if let Some(g) = try_general(m, p, table, Some(GeneralShape::Expr))? {
                        return Ok(g);
                    }
                }
                return Ok(PatternKind::Expr {
                    minus,
                    plus_raw: p.to_string(),
                    placeholders: maps
                        .expr_placeholders
                        .into_iter()
                        .map(|(ph, n)| (n, ph))
                        .collect(),
                    literals: lits,
                    has_path_ellipsis: has_path_ellipsis(m),
                });
            }
            "stmt" => {
                return match try_general(m, p, table, Some(GeneralShape::Stmt))? {
                    Some(g) => Ok(g),
                    None => Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` with or without `let-else`)",
                    )
                    .with_repair(StructuredError::fix_pattern_repair(
                        "C3",
                        "write a `let` statement pattern",
                    ))),
                };
            }
            "type" => {
                return match try_general(m, p, table, Some(GeneralShape::Type))? {
                    Some(g) => Ok(g),
                    None => Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=type` requires a Rust type pattern",
                    )
                    .with_repair(StructuredError::fix_pattern_repair(
                        "C5",
                        "write a Rust type pattern",
                    ))),
                };
            }
            _ => {}
        }
    }

    // Single-metavar minus => item/stmt capture (cut).
    if m.lines().count() == 1 && is_single_meta(m, table) {
        return Ok(PatternKind::ItemCapture {
            meta: m.to_string(),
        });
    }

    // Insertion: no minus, plus present.
    if m.is_empty() && !p.is_empty() {
        let uses = hunk.uses();
        // WI-8a + WI-8 slices 1-6: `# use=N` with a single-line `fn`
        // header template (`+ pub fn New(...) { ... }`), `struct`
        // header template (`+ pub struct New { ... }`), `enum`
        // header template (`+ pub enum New { ... }`), `union`
        // header template (`+ pub union New { ... }`), `type`
        // header template (`+ pub type New = ...;`), `const`
        // header template (`+ pub const New = ...;`), or `static`
        // header template (`+ pub static [mut ]New = ...;`) is an
        // evolution paste — the engine splices the captured body under
        // the template header. Anything else with `# use=` stays a literal
        // insertion.
        let use_name = if uses.len() == 1
            && p.lines().count() == 1
            && (p.trim() == uses[0]
                || parse_evolution_template(p.trim()).is_some()
                || parse_struct_evolution_template(p.trim()).is_some()
                || parse_enum_evolution_template(p.trim()).is_some()
                || parse_union_evolution_template(p.trim()).is_some()
                || parse_type_evolution_template(p.trim()).is_some()
                || parse_const_evolution_template(p.trim()).is_some()
                || parse_static_evolution_template(p.trim()).is_some())
        {
            Some(uses[0].to_string())
        } else if uses.len() == 1 && p.lines().count() == 1 && has_fn_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `fn` header with `(...)` + `...` (got `{}`); write `+ <vis>fn <New>(...) {{ ... }}` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else if uses.len() == 1 && p.lines().count() == 1 && has_struct_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `struct` header with `...` (got `{}`); write `+ <vis>struct <New> {{ ... }}` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else if uses.len() == 1 && p.lines().count() == 1 && has_enum_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `enum` header with `...` (got `{}`); write `+ <vis>enum <New> {{ ... }}` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else if uses.len() == 1 && p.lines().count() == 1 && has_union_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `union` header with `...` (got `{}`); write `+ <vis>union <New> {{ ... }}` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else if uses.len() == 1 && p.lines().count() == 1 && has_type_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `type` header with `= ...;` (got `{}`); write `+ <vis>type <New> = ...;` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else if uses.len() == 1 && p.lines().count() == 1 && has_const_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `const` header with `= ...;` (got `{}`); write `+ <vis>const <New> = ...;` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else if uses.len() == 1 && p.lines().count() == 1 && has_static_keyword(p.trim()) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "evolution paste template must be a single-line `static` header with `= ...;` (got `{}`); write `+ <vis>static [mut ]<New> = ...;` or paste the bare capture name for a verbatim move",
                    p.trim()
                ),
            ));
        } else {
            None
        };
        return Ok(PatternKind::Insertion {
            text: p.to_string(),
            use_name,
        });
    }

    // Method-context: context lines open an `impl` or `trait` block and
    // the minus side is fn-like. The trait half (T182) routes
    // `TraitItem::Fn` (`;` declarations and default bodies) through the
    // shared slot planner; `impl` is checked first so a pathological
    // both-match stays deterministic.
    if (c.contains("impl ") || is_trait_context(c)) && looks_like_fn(m, table) {
        let impl_header = c.lines().next().unwrap_or("").to_string();
        return Ok(PatternKind::ImplContext { impl_header });
    }

    // Non-function item renames (`struct Old` -> `struct New`). Checked
    // before fn shapes: `const Old`/`static Old` also satisfy
    // `looks_like_fn`, and every fn shape falls through here as `None`.
    if let Some(rename) = classify_item_rename(m, p)? {
        return Ok(rename);
    }

    // `use`-path prefix renames (`use crate::old` -> `use crate::new`).
    // Disjoint from item/fn/call shapes; falls through as `None`.
    if let Some((minus_segs, plus_segs)) = crate::use_rename::classify_use_rename(m, p, table)? {
        return Ok(PatternKind::UseRename {
            minus_segs,
            plus_path: plus_segs.join("::"),
        });
    }

    // Function patterns (minus starts with fn-ish, possibly with vis/attrs).
    if looks_like_fn(m, table) && looks_like_fn(p, table) {
        let (vis_metas, qual_metas, gen_metas, where_metas, type_metas) = slot_metas(table);
        let minus_fn = parse_fn_pattern(
            &first_fn_block(m),
            &vis_metas,
            &qual_metas,
            &gen_metas,
            &where_metas,
            &type_metas,
            hunk.returns_none(),
        )?;
        let plus_fn = parse_fn_pattern(
            &first_fn_block(p),
            &vis_metas,
            &qual_metas,
            &gen_metas,
            &where_metas,
            &type_metas,
            false,
        )?;
        // WI-1b: terse rename is both-sides-or-neither. A terse minus with
        // a long plus (or vice versa) would silently insert/delete slots
        // in `plan_sig_edits` — fail fast instead (SPEC 2.6).
        if crate::terse_fn::is_terse(&minus_fn) != crate::terse_fn::is_terse(&plus_fn) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "terse fn rename requires both sides terse (`- fn foo` / `+ fn bar`); any slot text (`(...)`, `<...>`, `-> T`, `where`, `{`) needs the long form on both sides (SPEC 14.1)",
            ));
        };
        let minus_name = match &minus_fn.name {
            crate::fn_pattern::NamePat::Concrete(n) => n.clone(),
            crate::fn_pattern::NamePat::Meta(n) => format!("${n}"),
        };
        let plus_name = match &plus_fn.name {
            crate::fn_pattern::NamePat::Concrete(n) => n.clone(),
            crate::fn_pattern::NamePat::Meta(n) => format!("${n}"),
        };
        let vis_meta = match &minus_fn.visibility {
            crate::fn_pattern::VisPat::Meta(v) => Some(v.clone()),
            _ => None,
        };
        return Ok(PatternKind::Fn {
            minus_name,
            plus_name,
            vis_meta,
        });
    }

    // Macro modes: explicit token_tree or `!` in pattern.
    let is_macro_mode = hunk.macro_mode() != rsmpl_patch::MacroMode::Invocation;
    // A leading `!` is unary (T415: `!f(...)` parses as `Expr::Unary`,
    // never `NAME!(...)`) so it falls through to the expression router.
    let leading_bang = m.trim_start().starts_with('!');
    // T428: only a genuine macro bang (`name!` + body opener, strings
    // skipped) routes to macro — `foo(!E)`, `a != b` fall through to the
    // call/general routers. WI-6b: the bang must open the whole pattern
    // (`name!(...)` with no whitespace before the name) — a `!` nested
    // inside a block (`if E { log!("hit"); F }`) is a macro *statement*,
    // not a macro pattern, and falls through to the general router
    // (mirrors the `classify_call_pattern` whole-name guard).
    let whole_is_invocation = !leading_bang
        && find_macro_bang(m).is_some_and(|bang| {
            let before = m[..bang].trim();
            !before.is_empty() && !before.contains(char::is_whitespace)
        });
    if is_macro_mode || whole_is_invocation {
        return Ok(PatternKind::Macro {
            minus_raw: m.to_string(),
            plus_raw: p.to_string(),
        });
    }

    // Default: expression/call rewrite.
    if !m.is_empty() {
        let (minus, maps, lits) = classify_call_pattern(m, table)?;
        // Non-call shapes (`Raw`) get a second chance as generalized
        // stmt/expr/type patterns (T10). Call/method/macro routing is
        // untouched, including `foo(E);` (which classifies as `Call`).
        if matches!(minus, ExprPatternKind::Raw { .. }) {
            if let Some(g) = try_general(m, p, table, None)? {
                return Ok(g);
            }
        }
        return Ok(PatternKind::Expr {
            minus,
            plus_raw: p.to_string(),
            placeholders: maps
                .expr_placeholders
                .into_iter()
                .map(|(ph, n)| (n, ph))
                .collect(),
            literals: lits,
            has_path_ellipsis: has_path_ellipsis(m),
        });
    }

    Ok(PatternKind::Raw {
        minus: m.to_string(),
        plus: p.to_string(),
    })
}

/// True when the pattern uses `::...::` path ellipsis (SPEC 16.5), as opposed
/// to argument/body ellipsis `foo(...)`.
fn has_path_ellipsis(text: &str) -> bool {
    text.contains("::...") || text.contains("...::")
}

fn is_single_meta(text: &str, table: &MetavarTable) -> bool {
    let t = text.trim().trim_start_matches('$');
    let single_line = text.trim().lines().count() == 1;
    single_line && !t.contains(char::is_whitespace) && *t != *"..." && table.is_meta(t)
}

/// Visibility / qualifier / generic / where / type slot metas for fn parsing.
pub(crate) type SlotMetas = (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
);

fn slot_metas(table: &MetavarTable) -> SlotMetas {
    let mut vis = Vec::new();
    let mut qual = Vec::new();
    let mut gen = Vec::new();
    let mut wh = Vec::new();
    let mut ty = Vec::new();
    for (kind, name) in &table.declared {
        match kind {
            MetavarKind::Visibility => vis.push(name.clone()),
            MetavarKind::Qualifier | MetavarKind::QualifierList => qual.push(name.clone()),
            MetavarKind::GenericParams => gen.push(name.clone()),
            MetavarKind::WhereClause => wh.push(name.clone()),
            MetavarKind::Type => ty.push(name.clone()),
            _ => {}
        }
    }
    (vis, qual, gen, wh, ty)
}

fn looks_like_fn(text: &str, table: &MetavarTable) -> bool {
    // Leading slot metavariables (`V fn foo`, `Q fn foo`): the first token
    // names a declared visibility/qualifier(-list) metavariable and `fn`
    // follows. This keeps SPEC 14.3/14.4 abstracted patterns routable.
    let slot_meta_lead = text.lines().any(|l| {
        let t = l.trim().trim_start_matches("#[");
        let mut toks = t
            .split(|c: char| c.is_whitespace() || c == '(')
            .filter(|s| !s.is_empty());
        // Only a leading slot metavariable followed by `fn` counts, so an
        // expression pattern that merely starts with `V`/`Q` is not
        // misrouted into the function classifier.
        match toks.next() {
            Some(head) => {
                let bare = head.trim_start_matches('$');
                t.contains("fn ")
                    && matches!(
                        table.kind_of(bare),
                        Some(
                            MetavarKind::Visibility
                                | MetavarKind::Qualifier
                                | MetavarKind::QualifierList
                        )
                    )
            }
            None => false,
        }
    });
    text.lines().any(|l| {
        let t = l.trim().trim_start_matches("#[");
        t.starts_with("fn ")
            || t.starts_with("pub ")
            // WI-6 slice 1: `async {` / `unsafe {` open block expressions,
            // not fn headers — only route when a fn/trait/impl follows.
            || (t.starts_with("async ") && qualifier_leads_to_fn(t))
            || (t.starts_with("unsafe ") && qualifier_leads_to_fn(t))
            || t.starts_with("const ")
            || t.starts_with("extern ")
            || (t.starts_with("pub("))
    }) || text.contains("\nfn ")
        || text.trim_start().starts_with("fn ")
        || slot_meta_lead
}

fn first_fn_block(text: &str) -> String {
    // The fn pattern may span multiple lines (`fn foo(...) {` / `...` / `}`);
    // join lines with newlines preserved for the fn parser.
    text.to_string()
}

/// Single header-line check (T182): the trimmed line opens a `trait` block
/// (`trait T ...` / `unsafe trait T ...`). Used for the C14 label and by the
/// engine to pick the trait half of method-context matching.
pub fn is_trait_header(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("trait ") || t.starts_with("unsafe trait ")
}

/// Multi-line context check for the router (T182): any context line opens a
/// trait block. Line-anchored so `// trait` comments or `dyn Trait` bounds
/// never route a hunk into method-context matching.
fn is_trait_context(context_raw: &str) -> bool {
    context_raw.lines().any(is_trait_header)
}

/// Ident a `trait` header line declares (`trait T<U>: B ...` → `T`; T182).
/// `None` when the line is not a trait header (attribute lines, garbage) —
/// callers fall back to range-overlap behavior instead of guessing.
pub fn trait_header_ident(header: &str) -> Option<String> {
    let t = header.trim_start();
    let rest = t
        .strip_prefix("trait ")
        .or_else(|| t.strip_prefix("unsafe trait "))?;
    let name = rest.split_whitespace().next()?;
    let name = name.split(['<', ':']).next().unwrap_or(name);
    if name.is_empty() || name == "{" {
        None
    } else {
        Some(name.to_string())
    }
}

/// Classify a non-function item rename (`struct Old` -> `struct New`).
///
/// Returns `Ok(None)` when neither side is an item-rename shape (the caller
/// falls through to fn/macro/call classification). Either side being an
/// item shape while the other is not — or a kind mismatch — is a hard
/// `PatternConflict`, never a silent fallthrough.
///
/// Escape hatch (T149): when `parse_side` fails on a side but both raw
/// sides parse as Rust types (e.g. `- impl std::fmt::Display`, where the
/// multi-segment bound trips the bare-name gate), the hunk falls through
/// to the general type matcher instead of failing as a malformed rename.
/// Genuine item typos (`struct Old<T>`, `$Meta`, `auto trait`) never parse
/// as types, so they still fail here.
fn classify_item_rename(
    minus_raw: &str,
    plus_raw: &str,
) -> Result<Option<PatternKind>, StructuredError> {
    let minus = match crate::item_rename::parse_side(minus_raw) {
        Ok(m) => m,
        Err(e) => {
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            return Err(e);
        }
    };
    let plus = match crate::item_rename::parse_side(plus_raw) {
        Ok(p) => p,
        Err(e) => {
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            return Err(e);
        }
    };
    match (minus, plus) {
        (Some(mside), Some(pside)) => {
            let mk = mside.kind;
            let mn = mside.name;
            let mattrs = mside.attrs;
            let mprefix = mside.prefix;
            let pk = pside.kind;
            let pn = pside.name;
            let pattrs = pside.attrs;
            let pprefix = pside.prefix;
            if mk != pk {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "item rename kind mismatch: minus is `{}` but plus is `{}` (kinds must match)",
                        mk.as_str(),
                        pk.as_str()
                    ),
                ));
            }
            if mn == pn {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "item rename minus and plus names are identical (`{mn}`); write the new name on the `+` side"
                    ),
                ));
            }
            if mprefix != pprefix {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "item rename prefixes differ (`{mprefix}` vs `{pprefix}`); item renames are rename-only — write the same prefix on both sides or neither (visibility/generics/bodies are preserved)"
                    ),
                ));
            }
            Ok(Some(PatternKind::ItemRename {
                kind: mk,
                minus_name: mn,
                plus_name: pn,
                minus_attrs: mattrs,
                plus_attrs: pattrs,
            }))
        }
        (Some(_), None) => {
            // `impl Trait` type patterns (T149): a bare `impl Name` minus
            // with a non-item plus (e.g. `Box<dyn Display>`) is a general
            // type rewrite, not a malformed item rename — fall through
            // when both sides parse as Rust types. Anything else stays a
            // hard authoring error (fail over guessing).
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "item rename plus side must be a bare `<kind> <Name>` pattern matching the minus kind",
            ))
        }
        (None, Some(_)) => {
            // Symmetric with the above: a non-item minus with a bare-item
            // plus falls through only when both sides are Rust types.
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "item rename minus side must be a bare `<kind> <Name>` pattern matching the plus kind",
            ))
        }
        (None, None) => Ok(None),
    }
}

/// True when both pattern sides parse as Rust types (T149): lets `impl
/// Trait` type rewrites (`- impl Display` / `+ Box<dyn Display>`) fall
/// through the item-rename classifier to the general type matcher instead
/// of failing as malformed renames. Raw text (declared metavars are plain
/// idents here); backticked literals stay hard errors — fail safe.
fn is_type_pair(minus_raw: &str, plus_raw: &str) -> bool {
    syn::parse_str::<syn::Type>(minus_raw.trim()).is_ok()
        && syn::parse_str::<syn::Type>(plus_raw.trim()).is_ok()
}
