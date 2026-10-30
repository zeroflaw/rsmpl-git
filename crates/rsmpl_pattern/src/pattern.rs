//! Pattern compilation: from raw hunk body + directives to structured kinds.
//!
//! Root inference (SPEC 13.4): explicit `# root=` wins; otherwise heuristics
//! on the minus text (fn/item/call/macro/insert). The engine performs the
//! actual target matching; this module validates shape and extracts bindings.

use rsmpl_patch::{AttrsPolicy, Directive, ErrorCode, MetavarKind, RsmplHunk, StructuredError};
use std::collections::HashSet;

use crate::expr_pattern::{
    classify_call_pattern, find_macro_bang, ExprPattern, ExprPatternKind, PlaceholderMaps,
};
use crate::fn_pattern::parse_fn_pattern;
use crate::metavar::MetavarTable;
use crate::quote_utils::{mask_literal_spans, unmask_literal_spans, validate_quoting};

/// Shape of a generalized (non-call) pattern (T10 v1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralShape {
    /// A `let` statement (`syn::Stmt::Local`, with or without `let-else`).
    Stmt,
    /// A general Rust expression (bounded engine variant set).
    Expr,
    /// A general Rust type (bounded engine variant set).
    Type,
}

/// Top-level pattern kind for a hunk side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternKind {
    /// Function-item rename/rewrite (`fn foo(...) { ... }`).
    Fn {
        minus_name: String,
        plus_name: String,
        vis_meta: Option<String>,
    },
    /// Expression/call rewrite (`foo(E)` -> `bar(E)`).
    Expr {
        minus: ExprPatternKind,
        plus_raw: String,
        /// Placeholder idents per metavariable, for substitution checks.
        placeholders: Vec<(String, String)>,
        /// Backtick literals in the minus pattern.
        literals: Vec<String>,
        /// Whether a `crate::...::name` path ellipsis is present.
        has_path_ellipsis: bool,
    },
    /// Single-metavariable cut (`- I` with `# item I` + `# bind=X`).
    ItemCapture { meta: String },
    /// Single-metavariable or literal insertion (`+ Helper` / `+ use ...`).
    Insertion {
        text: String,
        use_name: Option<String>,
    },
    /// Method-context rewrite (`impl Service { ... - fn ... }` or
    /// `trait Service { ... - fn ... }`, T182): the stored header covers
    /// both block kinds (field name is historical).
    ImplContext { impl_header: String },
    /// Macro token-tree / invocation patterns.
    Macro { minus_raw: String, plus_raw: String },
    /// Non-function item rename (`struct Old` -> `struct New`): only the
    /// name ident is replaced, everything else is preserved. `impl`
    /// renames the self-ty ident and the trait-side path's last segment.
    ItemRename {
        kind: crate::item_rename::ItemKind,
        minus_name: String,
        plus_name: String,
        minus_attrs: Vec<String>,
        plus_attrs: Vec<String>,
    },
    /// `use`-path prefix rename (`use crate::old` -> `use crate::new`):
    /// the matched leading-segment span of each `use` item is replaced,
    /// tails (groups, globs, `as`, final names) are preserved.
    UseRename {
        minus_segs: Vec<String>,
        plus_path: String,
    },
    /// Generalized statement/expression/type rewrite (`let x = E`, `a + E`,
    /// `Vec<T>`). `minus_sub` holds the placeholder-substituted pattern
    /// (`__RSMPL_E` idents, backtick literals restored as literal text).
    General {
        shape: GeneralShape,
        minus_sub: String,
        plus_raw: String,
        /// Plus text with leading `#[...]` attribute lines removed and the
        /// one-space diff-prefix artifact stripped per line (T153): the Stmt
        /// mirrored-attrs path emits `original-envelope + substitute(plus_body)`
        /// so the target's `let` indent survives instead of being replaced by
        /// the pattern artifact space.
        plus_body: String,
        /// Placeholder idents per metavariable, for substitution checks.
        placeholders: Vec<(String, String)>,
        /// Backtick literals in the minus pattern.
        literals: Vec<String>,
        /// Leading `#[...]` attribute lines split off minus/plus (Stmts).
        minus_attrs: Vec<String>,
        plus_attrs: Vec<String>,
        /// True when a trailing `;` was stripped to match an expression
        /// statement as an expression (deletions remove whole lines).
        had_semi: bool,
    },
    /// Fallback raw-text pattern (kept for diagnostics).
    Raw { minus: String, plus: String },
}

/// A compiled hunk pattern ready for the engine.
#[derive(Debug, Clone)]
pub struct CompiledPattern {
    pub kind: PatternKind,
    pub table: MetavarTable,
    pub minus_bindings: HashSet<String>,
    pub plus_bindings: HashSet<String>,
    /// Names used in `+` but never bound in `-`/context or `# use` (engine
    /// re-checks with capture awareness; stored here for diagnostics).
    pub unbound_plus: Vec<String>,
    pub repeated: Vec<String>,
    pub root: Option<String>,
}

impl CompiledPattern {
    pub fn rule(&self) -> &str {
        ""
    }
}

/// Compile an RSmPL hunk's pattern body.
pub fn compile(hunk: &RsmplHunk) -> Result<CompiledPattern, StructuredError> {
    let table = MetavarTable::from_directives(&hunk.directives)?;
    let root = hunk
        .directives
        .iter()
        .filter_map(|d| match d {
            Directive::Root(r) => Some(r.clone()),
            _ => None,
        })
        .next_back();

    // T110: `# root=` is closed to the four implemented force-paths (SPEC
    // 13.4). Anything else previously fell through to heuristic routing — a
    // silent no-op against SPEC 2.6 (same class as T113).
    if let Some(r) = root.as_deref() {
        if !matches!(r, "item" | "expr" | "stmt" | "type") {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "unknown `# root=` value `{r}` (only `item/expr/stmt/type` are supported in v0.1; remove it or use a supported value — see SPEC 13.4)"
                ),
            ));
        }
    }

    // T111: reject metavariable kinds with no engine binding (SPEC 15.1).
    // `identifier/path/lifetime/field` parse but never bind — agents pay
    // tokens for `# lifetime L` that can never match as a lifetime.
    for (kind, name) in &table.declared {
        if matches!(
            kind,
            MetavarKind::Identifier
                | MetavarKind::Path
                | MetavarKind::Lifetime
                | MetavarKind::Field
        ) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "`# {} {name}` is not supported in v0.1 (see SPEC 15.1 for the implemented kinds)",
                    kind.as_str()
                ),
            ));
        }
    }

    // T165: `# attributes A` declares an attribute-list abstraction no
    // matcher binds (`MetaVarKind::Attributes` parses but every spelling
    // misroutes with a confusing error). v0.1 has no such abstraction —
    // fail fast with the repair instead of misrouting (SPEC 2.6).
    if let Some((_, name)) = table
        .declared
        .iter()
        .find(|(k, _)| matches!(k, MetavarKind::Attributes))
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "`# attributes {name}` is not supported in v0.1 (attribute lists cannot be abstracted; write the `#[...]` lines literally on both sides to constrain them, or omit them — attributes are preserved by default, SPEC 14.6)"
            ),
        ));
    }

    let minus_raw = full_minus_text(hunk);
    let plus_raw = full_plus_text(hunk);
    let context_raw = hunk.context_text();

    for line in minus_raw
        .lines()
        .chain(plus_raw.lines())
        .chain(context_raw.lines())
    {
        validate_quoting(line)?;
    }

    // WI-1c: inferred metavariables (SPEC 15.8). Undeclared
    // `[A-Z][A-Za-z0-9_]*` in a metavar position infers its kind;
    // explicit declarations must agree; ambiguity fails fast.
    let table = crate::infer::infer_table(table, &minus_raw, &plus_raw, root.as_deref())?;

    let minus_bindings = table.bound_in_text(&minus_raw);
    let mut bound: HashSet<String> = minus_bindings.clone();
    // `# use=Name` imports a capture as bound for the plus side.
    for u in hunk.uses() {
        bound.insert(u.to_string());
    }
    // Symbol-declared names used as callee in minus are bound by declaration.
    for k in table.symbols.keys() {
        bound.insert(k.clone());
    }
    let plus_bindings = table.bound_in_text(&plus_raw);
    let uses: HashSet<String> = hunk.uses().into_iter().map(|s| s.to_string()).collect();
    let mut unbound_plus: Vec<String> = plus_bindings
        .difference(&bound)
        .filter(|n| !uses.contains(*n))
        .cloned()
        .collect();
    unbound_plus.sort();

    // Repeated metavariables in the minus pattern (SPEC 15.7): names occurring
    // 2+ times as whole tokens.
    let repeated = repeated_metavars(&minus_raw, &table);

    validate_ellipsis_positions(&minus_raw, &plus_raw)?;

    let kind = classify(
        &minus_raw,
        &plus_raw,
        &context_raw,
        &table,
        hunk,
        root.as_deref(),
    )?;

    // `# returns=none` only meaningful on fn patterns (free functions
    // and impl/trait-context methods share the constraint; T209).
    if hunk.returns_none()
        && !matches!(
            kind,
            PatternKind::Fn { .. } | PatternKind::ImplContext { .. }
        )
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# returns=none` requires a function-item pattern".to_string(),
        ));
    }
    // C14 (T24): `# symbol` is call-shape-only in v0.1. Call-callee,
    // call-argument, and method-receiver symbols match by exact full-path
    // text through seeded bindings (SPEC 19.3); every other shape either
    // ignores the constraint or mis-substitutes (general-expr/let/type
    // shapes bind anything and emit the raw name, macro names match
    // loosely), so fail fast instead of corrupting (SPEC 2.6).
    if !table.symbols.is_empty()
        && !matches!(
            kind,
            PatternKind::Expr {
                minus: ExprPatternKind::Call { .. } | ExprPatternKind::MethodCall { .. },
                ..
            }
        )
    {
        let label = match &kind {
            PatternKind::General { shape, .. } => match shape {
                GeneralShape::Stmt => "general `let`-statement",
                GeneralShape::Expr => "general-expression",
                GeneralShape::Type => "general-type",
            },
            PatternKind::Fn { .. } => "function-item",
            PatternKind::ImplContext { impl_header } => {
                if is_trait_header(impl_header) {
                    "trait-context"
                } else {
                    "impl-context"
                }
            }
            PatternKind::Macro { .. } => "macro",
            PatternKind::ItemRename { .. } => "item-rename",
            PatternKind::UseRename { .. } => "use-path",
            PatternKind::ItemCapture { .. } => "item-capture",
            PatternKind::Insertion { .. } => "insertion",
            PatternKind::Expr { .. } => "expression",
            PatternKind::Raw { .. } => "expression",
        };
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "`# symbol` is only supported in call shapes in v0.1 (call-callee, call-argument, and method-receiver symbols match by exact full-path text, SPEC 19.3); this {label} hunk cannot use `# symbol` — use a call-shape hunk or a concrete path pattern",
            ),
        ));
    }

    // T147: a `# symbol` naming the *method ident* of a method call can
    // never match. Receiver/callee/argument symbols bind full paths and
    // compare as paths, but the method slot compares bare idents
    // (`match_receiver_or_method`), so a seeded `crate::…::old` path never
    // equals the `old` ident — the hunk would fail safe with a bare
    // `no match found`. Method idents already match exactly as written,
    // so fail fast instead of burning a match cycle (SPEC 2.6).
    if let PatternKind::Expr {
        minus: ExprPatternKind::MethodCall { method, .. },
        placeholders,
        ..
    } = &kind
    {
        if let Some(name) = placeholders
            .iter()
            .find(|(_, ph)| ph == method)
            .map(|(name, _)| name)
        {
            if table.symbols.contains_key(name) {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "`# symbol {name}` names the method ident; method idents already match exactly — remove that `# symbol` line (symbols verify receiver/callee paths, SPEC 19.3)",
                    ),
                ));
            }
        }
    }
    // T169: lone `# fallback=syntax` (zero `# symbol` declarations) is a
    // silent no-op on every shape — the engine symbol loop iterates zero
    // times, so the directive constrains nothing. Fail fast (SPEC 2.6).
    // WI-6a: a `# follow=uses` hunk gives the fallback meaning (skip
    // ambiguous/opaque follow sites instead of vetoing, SPEC 14.8), so it
    // is exempt from the lone-fallback fence.
    if table.symbols.is_empty()
        && !hunk.follows_uses()
        && hunk
            .directives
            .iter()
            .any(|d| matches!(d, Directive::FallbackSyntax(true)))
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# fallback=syntax` without `# symbol` does nothing (call matching is already syntactic in v0.1) — remove the line, or add `# symbol Name=path` (SPEC 19.3)".to_string(),
        ));
    }
    // T169: non-default `# attrs=` on call/method-call/macro shapes is
    // never read — `matcher_expr`/`matcher_macro` ignore the policy
    // (envelopes live on the enclosing statement, outside the rewritten
    // span). Fail fast instead of implying a constraint (SPEC 2.6).
    // Explicit `# attrs=preserve` stays allowed: it is indistinguishable
    // from the default and harmless.
    if hunk.attrs_policy() != AttrsPolicy::Preserve
        && matches!(
            kind,
            PatternKind::Expr {
                minus: ExprPatternKind::Call { .. }
                    | ExprPatternKind::MethodCall { .. }
                    | ExprPatternKind::Macro { .. },
                ..
            } | PatternKind::Macro { .. }
        )
    {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# attrs=` is ignored on call/macro shapes in v0.1 (attribute envelopes are only checked on fn/item/general/use shapes) — remove the line (SPEC 14.6)".to_string(),
        ));
    }
    // general-expression matchers; on any other shape it would be a
    // silent no-op. T126 extended the fold to `General::Expr`
    // (pure-expression bindings); `let`-statement and type shapes mix
    // patterns/types/initializers in one binding set, so folding an
    // expression rule inside a bound *pattern* or *type* would be
    // surprising — ordered hunks stay the path there.
    if hunk.repeats() {
        let repeat_shape = matches!(
            kind,
            PatternKind::Expr {
                minus: ExprPatternKind::Call { .. } | ExprPatternKind::MethodCall { .. },
                ..
            } | PatternKind::General {
                shape: GeneralShape::Expr,
                ..
            }
        );
        if !repeat_shape {
            let reason = match &kind {
                PatternKind::General {
                    shape: GeneralShape::Stmt,
                    ..
                } => {
                    "`# repeat` is not supported on `let`-statement patterns in v1 (folding an expression rule inside a bound pattern or type would be surprising); emit one ordered hunk per nesting level"
                }
                PatternKind::General {
                    shape: GeneralShape::Type,
                    ..
                } => {
                    "`# repeat` is not supported on type patterns in v1; emit one ordered hunk per nesting level"
                }
                _ => {
                    "`# repeat` requires a call/method-call or general-expression rewrite pattern (other shapes are future work)"
                }
            };
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                reason.to_string(),
            ));
        }
    }

    Ok(CompiledPattern {
        kind,
        table,
        minus_bindings,
        plus_bindings,
        unbound_plus,
        repeated,
        root,
    })
}

/// Full minus pattern = removal lines + context lines, in hunk order.
/// This reconstructs multi-line patterns like `fn foo(...) {` / `...` / `}`.
fn full_minus_text(hunk: &RsmplHunk) -> String {
    hunk.lines
        .iter()
        .filter(|l| l.is_removal() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Full plus pattern = addition lines + context lines, in hunk order.
fn full_plus_text(hunk: &RsmplHunk) -> String {
    hunk.lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Names occurring 2+ times in `text` as whole ident tokens.
fn repeated_metavars(text: &str, table: &MetavarTable) -> Vec<String> {
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    for tok in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$')) {
        if tok.is_empty() {
            continue;
        }
        let bare = tok.strip_prefix('$').unwrap_or(tok);
        if table.sigiled && !tok.starts_with('$') {
            continue;
        }
        if table.is_meta(bare) {
            *counts.entry(bare.to_string()).or_default() += 1;
        }
    }
    let mut out: Vec<String> = counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(n, _)| n)
        .collect();
    out.sort();
    out
}

/// Reject `...` in positions SPEC 16 forbids (visibility/qualifier/generics
/// slots are handled by fn parser; here we reject stray `...` as path prefix
/// misuse like `...foo` without `::`).
fn validate_ellipsis_positions(minus: &str, plus: &str) -> Result<(), StructuredError> {
    for text in [minus, plus] {
        for line in text.lines() {
            let t = line.trim();
            // `...` must appear as a standalone token or inside delimiters.
            // A cheap check: no identifier char directly adjacent: `foo...bar`.
            let bytes = t.as_bytes();
            let mut i = 0;
            while i + 2 < bytes.len() {
                if &t[i..i + 3] == "..." {
                    let before = if i > 0 { bytes[i - 1] as char } else { ' ' };
                    let after = if i + 3 < bytes.len() {
                        bytes[i + 3] as char
                    } else {
                        ' '
                    };
                    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
                    if ident(before) && ident(after) {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("`...` must be a standalone wildcard, got `{line}`"),
                        ));
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
        }
    }
    Ok(())
}

/// WI-8a evolution-paste template: a single-line `fn` header with
/// `(...)` params and a `...` body (`pub fn New(...) { ... }`).
/// Returns the verbatim prefix before `fn` (vis + qualifiers) and the
/// new ident. `None` when the line is not a template shape.
pub fn parse_evolution_template(text: &str) -> Option<(String, String)> {
    let line = text.trim();
    if line.lines().count() != 1 {
        return None;
    }
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // Word-bounded `fn` followed by whitespace (never the `fn` inside
    // another ident, e.g. `myfn`).
    let mut fn_pos = None;
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..].starts_with(b"fn")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_whitespace())
        {
            fn_pos = Some(i);
            break;
        }
        i += 1;
    }
    let fn_pos = fn_pos?;
    let prefix = &line[..fn_pos];
    if prefix.contains(['(', '{', ';', '"']) {
        return None;
    }
    let rest = line[fn_pos + 2..].trim_start();
    let mut ident_len = 0;
    for c in rest.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            ident_len += c.len_utf8();
        } else {
            break;
        }
    }
    if ident_len == 0 {
        return None;
    }
    let tail = &rest[ident_len..];
    if !tail.contains("(...)") || !tail.contains("...") {
        return None;
    }
    Some((prefix.to_string(), rest[..ident_len].to_string()))
}

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

#[allow(clippy::too_many_lines)]
fn classify(
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
                ));
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
                    )),
                };
            }
            "type" => {
                return match try_general(m, p, table, Some(GeneralShape::Type))? {
                    Some(g) => Ok(g),
                    None => Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=type` requires a Rust type pattern",
                    )),
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
        // WI-8a: `# use=N` with a single-line `fn` header template
        // (`+ pub fn New(...) { ... }`) is an evolution paste — the
        // engine splices the captured body under the template header.
        // Anything else with `# use=` stays a literal insertion.
        let use_name = if uses.len() == 1
            && p.lines().count() == 1
            && (p.trim() == uses[0] || parse_evolution_template(p.trim()).is_some())
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
    // skipped) routes macro — `foo(!E)`, `a != b` fall through to the
    // call/general routers.
    if is_macro_mode || (!leading_bang && find_macro_bang(m).is_some()) {
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

/// Upgrade a non-call `Raw` shape to a generalized Stmt/Expr/Type pattern.
///
/// Returns `None` when the text is none of those (caller keeps `Raw`, which
/// the engine rejects with an unsupported-shape diagnostic).
///
/// Routing (forced by `# root=` or heuristic): a `syn::Stmt::Local` (`let`,
/// with or without `let-else`) becomes `Stmt`; otherwise a Rust `Type` when forced or when
/// a type-level metavariable is declared; otherwise a Rust `Expr`.
/// `...` is rejected: v1 general patterns are concrete-or-metavariable only.
#[allow(clippy::too_many_lines)]
fn try_general(
    minus_raw: &str,
    plus_raw: &str,
    table: &MetavarTable,
    forced: Option<GeneralShape>,
) -> Result<Option<PatternKind>, StructuredError> {
    if minus_raw.contains("...") || plus_raw.contains("...") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v1 general stmt/expr/type patterns do not support `...` (use a call pattern or a concrete shape)",
        ));
    }
    let (minus_attrs, minus_rest) = split_leading_attrs(minus_raw);
    let (plus_attrs, plus_rest) = split_leading_attrs(plus_raw);
    // A leading `/*` the peel cannot normalize (multi-line block doc,
    // ordinary block comment) is never a valid general-pattern start —
    // fail fast instead of a misleading match-time error (T156).
    for rest in [minus_rest.trim_start(), plus_rest.trim_start()] {
        if rest.starts_with("/*") {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "block comments (`/* ... */`) are not supported in patterns in v1 (mirror doc comments with `#[doc = \"...\"]` or use an ordinary hunk)",
            ));
        }
    }
    // Mask backtick literals so substitution cannot mistake them for
    // metavariables, then restore them in the substituted pattern.
    let (masked, sentinels) = mask_literal_spans(&minus_rest)?;
    let (mut sub, maps) = ExprPattern::placeholder_expr(&masked, table);
    sub = unmask_literal_spans(&sub, &sentinels);

    let want = |s: GeneralShape| forced.is_none_or(|f| f == s);
    let has_type_decl = table.declared.iter().any(|(k, _)| {
        matches!(
            k,
            MetavarKind::Type | MetavarKind::Lifetime | MetavarKind::GenericParams
        )
    });

    // 1. `let` statements (only `Stmt::Local`). Other statement shapes fall
    // through so existing routing is untouched. `let-else` diverge
    // blocks ride along structurally (WI-5a: the engine matches them
    // as `Block`s, both sides or neither).
    if want(GeneralShape::Stmt) {
        match syn::parse_str::<syn::Stmt>(&sub) {
            Ok(syn::Stmt::Local(local)) => {
                check_let_pat(&local)?;
                return Ok(Some(build_general(
                    GeneralShape::Stmt,
                    sub,
                    plus_raw,
                    &maps,
                    minus_attrs,
                    plus_attrs,
                    false,
                )));
            }
            // Any other statement shape (or unparsable text) falls through so
            // existing routing is untouched; `# root=stmt` fails fast.
            _ => {
                if forced == Some(GeneralShape::Stmt) {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` with or without `let-else`)",
                    ));
                }
            }
        }
    }

    // 2. Expression statements with `;` match as expressions; the engine
    // preserves (or whole-line-deletes) the semicolon via `had_semi`.
    // (Call-like `foo(E);` shapes never reach here: they classify as `Call`.)
    let mut expr_src: Option<String> = None;
    let mut had_semi = false;
    if let Ok(syn::Stmt::Expr(e, Some(_))) = syn::parse_str::<syn::Stmt>(&sub) {
        expr_src = Some(quote::quote!(#e).to_string());
        had_semi = true;
    }

    // 3. Types before expressions when the author declared type-level
    // metavariables (so `Vec<T>` with `# type T` is a type pattern).
    let type_first = forced == Some(GeneralShape::Type) || (forced.is_none() && has_type_decl);
    if want(GeneralShape::Type) && type_first && syn::parse_str::<syn::Type>(&sub).is_ok() {
        return check_type_deletion(plus_rest.trim(), || {
            build_general(
                GeneralShape::Type,
                sub,
                plus_raw,
                &maps,
                minus_attrs,
                plus_attrs,
                false,
            )
        });
    }

    // 4. Expressions.
    if want(GeneralShape::Expr) {
        let src = expr_src.unwrap_or(sub);
        if syn::parse_str::<syn::Expr>(&src).is_ok() {
            if plus_rest.trim().is_empty() && !had_semi {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement",
                ));
            }
            return Ok(Some(build_general(
                GeneralShape::Expr,
                src,
                plus_raw,
                &maps,
                minus_attrs,
                plus_attrs,
                had_semi,
            )));
        }
        if forced == Some(GeneralShape::Expr) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "`# root=expr` requires a Rust expression pattern",
            ));
        }
        // Fall through: the text may still be a `Type` (step 5 re-derives it).
    }

    // 5. Types (expression-first order).
    if want(GeneralShape::Type) && !type_first {
        // Re-parse: `sub` may have been moved above; re-derive cheaply.
        let (masked2, sentinels2) = mask_literal_spans(&minus_rest)?;
        let (mut sub2, _) = ExprPattern::placeholder_expr(&masked2, table);
        sub2 = unmask_literal_spans(&sub2, &sentinels2);
        if syn::parse_str::<syn::Type>(&sub2).is_ok() {
            return check_type_deletion(plus_rest.trim(), || {
                build_general(
                    GeneralShape::Type,
                    sub2,
                    plus_raw,
                    &maps,
                    minus_attrs,
                    plus_attrs,
                    false,
                )
            });
        }
    }
    if forced == Some(GeneralShape::Type) {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "`# root=type` requires a Rust type pattern",
        ));
    }
    // WI-9c(c): a minus side that parses as two or more Rust statements is
    // a statement sequence, which v1 general patterns cannot express (no
    // multi-statement pattern kind until WI-9c(a)). Fail fast at compile
    // time naming the pattern instead of degrading to `Raw` and dying at
    // match with a misleading "no match found" that points at the file.
    // Single-statement text (including multi-line single shapes such as
    // line-continued calls) and unparsable wraps still fall through to
    // `Ok(None)` — never a false positive. C1 precedent: the repair rides
    // in the reason, no separate suggestion.
    let (masked_seq, sentinels_seq) = mask_literal_spans(&minus_rest)?;
    let (sub_seq, _) = ExprPattern::placeholder_expr(&masked_seq, table);
    let sub_seq = unmask_literal_spans(&sub_seq, &sentinels_seq);
    if let Ok(block) = syn::parse_str::<syn::Block>(&format!("{{ {sub_seq} }}")) {
        if block.stmts.len() >= 2 {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "multi-statement sequences are not supported in v1 general patterns (the minus side parses as {} Rust statements, not one); write one hunk per statement or item, or use an ordinary hunk",
                    block.stmts.len()
                ),
            ));
        }
    }
    Ok(None)
}

fn check_type_deletion(
    plus_trimmed: &str,
    build: impl FnOnce() -> PatternKind,
) -> Result<Option<PatternKind>, StructuredError> {
    if plus_trimmed.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type",
        ));
    }
    Ok(Some(build()))
}

/// Strip the one-space diff-prefix artifact from each pattern line (T153).
/// Agents author `- let x` (prefix + space + content) and `classify` trims
/// only the outer ends of the full text, so interior lines of multi-line
/// patterns keep one leading space. Matching is whitespace-insensitive (syn),
/// but replacements are byte splices: emitting the artifact would silently
/// re-indent the target. Genuine continuation indent (2+) survives minus one
/// space, preserving relative indentation.
fn strip_artifact_space(text: &str) -> String {
    text.lines()
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_general(
    shape: GeneralShape,
    minus_sub: String,
    plus_raw: &str,
    maps: &PlaceholderMaps,
    minus_attrs: Vec<String>,
    plus_attrs: Vec<String>,
    had_semi: bool,
) -> PatternKind {
    // Attribute-stripped plus for the Stmt mirrored-attrs path (the envelope
    // is re-supplied from the original source, indent included).
    let (_, plus_rest) = split_leading_attrs(plus_raw);
    PatternKind::General {
        shape,
        minus_sub,
        plus_raw: plus_raw.to_string(),
        plus_body: strip_artifact_space(&plus_rest),
        placeholders: maps
            .expr_placeholders
            .iter()
            .map(|(ph, n)| (n.clone(), ph.clone()))
            .collect(),
        // Backticked spans were masked during substitution and restored as
        // literal text in `minus_sub`; nothing further to record.
        literals: Vec::new(),
        minus_attrs,
        plus_attrs,
        had_semi,
    }
}

/// Reject `let` patterns the v1 engine cannot match: embedded metavariables
/// inside the bound pattern (other than a whole-pattern `# pat P` placeholder,
/// a `P @ sub` shape whose head is a whole-pattern placeholder and whose
/// subpattern is a single placeholder or concrete, a concrete pattern, or a
/// `P: T` shape whose parts are each whole-placeholder/concrete).
fn check_let_pat(local: &syn::Local) -> Result<(), StructuredError> {
    let pat = &local.pat;
    let (inner, ty) = match pat {
        syn::Pat::Type(pt) => (&*pt.pat, Some(&*pt.ty)),
        _ => (pat, None),
    };
    if let syn::Pat::Ident(pid) = inner {
        // WI-5a `P @ sub` admission: a fully concrete `@` keeps the
        // historical literal behavior; a placeholder head admits a
        // single-placeholder or concrete subpattern. Placeholders
        // anywhere else (concrete head + placeholder sub, or a
        // composite-placeholder sub) stay C8.
        if let Some((_, sub)) = &pid.subpat {
            let head_is_ph = pid.ident.to_string().starts_with("__RSMPL_");
            if head_is_ph {
                if contains_placeholder(sub) && !is_single_placeholder_pat(sub) {
                    return Err(embedded_pat_err());
                }
            } else if contains_placeholder(sub) {
                return Err(embedded_pat_err());
            }
        }
        if let Some(t) = ty {
            let rendered = quote::quote!(#t).to_string();
            if rendered.contains("__RSMPL_") && !is_single_placeholder_type(t) {
                return Err(embedded_pat_err());
            }
        }
        return Ok(());
    }
    let rendered = quote::quote!(#pat).to_string();
    if rendered.contains("__RSMPL_") {
        return Err(embedded_pat_err());
    }
    Ok(())
}

/// True when the subpattern carries any placeholder ident.
fn contains_placeholder(pat: &syn::Pat) -> bool {
    quote::quote!(#pat).to_string().contains("__RSMPL_")
}

/// True when the subpattern is exactly one placeholder ident (`Q`).
fn is_single_placeholder_pat(pat: &syn::Pat) -> bool {
    if let syn::Pat::Ident(pid) = pat {
        return pid.subpat.is_none() && pid.ident.to_string().starts_with("__RSMPL_");
    }
    false
}

fn is_single_placeholder_type(t: &syn::Type) -> bool {
    if let syn::Type::Path(tp) = t {
        if tp.qself.is_none() && tp.path.segments.len() == 1 {
            let seg = &tp.path.segments[0];
            return seg.arguments.is_none() && seg.ident.to_string().starts_with("__RSMPL_");
        }
    }
    false
}

fn embedded_pat_err() -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        "v1 `let` patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts; embedded metavariables elsewhere in the pattern are not supported",
    )
}

/// Split leading attribute lines (`#[...]`, possibly multi-line, and
/// `///`/`//!` doc-comment lines normalized to `#[doc = "..."]`) from
/// pattern text; returns (attrs, rest).
///
/// `pub` for the `tests/pattern_compilation.rs` integration suite (T171),
/// which pins the peeling contract directly.
pub fn split_leading_attrs(text: &str) -> (Vec<String>, String) {
    let lines: Vec<&str> = text.lines().collect();
    let mut attrs = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        if lines[idx].trim_start().starts_with("#[") {
            let mut depth = 0i32;
            let start = idx;
            while idx < lines.len() {
                for c in lines[idx].chars() {
                    if c == '[' {
                        depth += 1;
                    } else if c == ']' {
                        depth -= 1;
                    }
                }
                idx += 1;
                if depth <= 0 {
                    break;
                }
            }
            attrs.extend(lines[start..idx].iter().map(ToString::to_string));
            if depth > 0 {
                break;
            }
        } else if let Some(attr) = crate::fn_pattern::doc_comment_to_attr(lines[idx].trim()) {
            attrs.push(attr);
            idx += 1;
        } else {
            break;
        }
    }
    (attrs, lines[idx..].join("\n"))
}

fn is_single_meta(text: &str, table: &MetavarTable) -> bool {
    let t = text.trim().trim_start_matches('$');
    let single_line = text.trim().lines().count() == 1;
    single_line && !t.contains(char::is_whitespace) && *t != *"..." && table.is_meta(t)
}

/// Visibility / qualifier / generic / where / type slot metas for fn parsing.
type SlotMetas = (
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
            || t.starts_with("async ")
            || t.starts_with("unsafe ")
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
