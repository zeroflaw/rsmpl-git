//! Pattern compilation: from raw hunk body + directives to structured kinds.
//!
//! Root inference (SPEC 13.4): explicit `# root=` wins; otherwise heuristics
//! on the minus text (fn/item/call/macro/insert). The engine performs the
//! actual target matching; this module validates shape and extracts bindings.

use rsmpl_patch::{Directive, ErrorCode, MetavarKind, RsmplHunk, StructuredError};
use std::collections::HashSet;

use crate::expr_pattern::{classify_call_pattern, ExprPattern, ExprPatternKind, PlaceholderMaps};
use crate::fn_pattern::parse_fn_pattern;
use crate::metavar::MetavarTable;
use crate::quote_utils::{mask_literal_spans, unmask_literal_spans, validate_quoting};

/// Shape of a generalized (non-call) pattern (T10 v1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralShape {
    /// A `let` statement (`syn::Stmt::Local`, no `let-else`).
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
    /// Impl-context rewrite (`impl Service { ... - fn ... }`).
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

    // `# returns=none` only meaningful on fn patterns.
    if hunk.returns_none() && !matches!(kind, PatternKind::Fn { .. }) {
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
            PatternKind::ImplContext { .. } => "impl-context",
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
            "item" if is_single_meta(m, table) => {
                return Ok(PatternKind::ItemCapture {
                    meta: m.to_string(),
                });
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
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` only, no `let-else`)",
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
        let use_name = if uses.len() == 1 && p.lines().count() == 1 && p.trim() == uses[0] {
            Some(uses[0].to_string())
        } else {
            None
        };
        return Ok(PatternKind::Insertion {
            text: p.to_string(),
            use_name,
        });
    }

    // Impl-context: context lines mention `impl`.
    if c.contains("impl ") && looks_like_fn(m, table) {
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
        let (vis_metas, qual_metas, gen_metas, where_metas) = slot_metas(table);
        let minus_fn = parse_fn_pattern(
            &first_fn_block(m),
            &vis_metas,
            &qual_metas,
            &gen_metas,
            &where_metas,
            hunk.returns_none(),
        )?;
        let plus_fn = parse_fn_pattern(
            &first_fn_block(p),
            &vis_metas,
            &qual_metas,
            &gen_metas,
            &where_metas,
            false,
        )?;
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
    if is_macro_mode || (m.contains('!') && m.contains('(')) {
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
/// no `let-else`) becomes `Stmt`; otherwise a Rust `Type` when forced or when
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
    // through so existing routing is untouched.
    if want(GeneralShape::Stmt) {
        match syn::parse_str::<syn::Stmt>(&sub) {
            Ok(syn::Stmt::Local(local)) => {
                if local.init.as_ref().is_some_and(|i| i.diverge.is_some()) {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "v1 general patterns do not support `let-else` statements",
                    ));
                }
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
                        "`# root=stmt` requires a `let` statement pattern (v1 supports `let` only, no `let-else`)",
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
/// inside the bound pattern (other than a whole-pattern placeholder, or a
/// `P: T` shape whose parts are each whole-placeholder/concrete).
fn check_let_pat(local: &syn::Local) -> Result<(), StructuredError> {
    let pat = &local.pat;
    let (inner, ty) = match pat {
        syn::Pat::Type(pt) => (&*pt.pat, Some(&*pt.ty)),
        _ => (pat, None),
    };
    if matches!(inner, syn::Pat::Ident(_)) {
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
fn split_leading_attrs(text: &str) -> (Vec<String>, String) {
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

fn slot_metas(table: &MetavarTable) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let mut vis = Vec::new();
    let mut qual = Vec::new();
    let mut gen = Vec::new();
    let mut wh = Vec::new();
    for (kind, name) in &table.declared {
        match kind {
            MetavarKind::Visibility => vis.push(name.clone()),
            MetavarKind::Qualifier | MetavarKind::QualifierList => qual.push(name.clone()),
            MetavarKind::GenericParams => gen.push(name.clone()),
            MetavarKind::WhereClause => wh.push(name.clone()),
            _ => {}
        }
    }
    (vis, qual, gen, wh)
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

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_patch::parse_patch;

    fn hunk_of(patch: &str) -> RsmplHunk {
        let p = parse_patch(patch).unwrap();
        match p.files[0].hunks[0].clone() {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("expected rsmpl"),
        }
    }

    #[test]
    fn compiles_fn_rename() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        );
        let c = compile(&h).unwrap();
        assert!(matches!(c.kind, PatternKind::Fn { .. }));
    }

    #[test]
    fn split_leading_attrs_peels_doc_lines() {
        // T155: `///`/`//!` peel alongside `#[...]` and normalize to the
        // `#[doc = "..."]` canonical form; peeling stops at the first
        // non-attribute line.
        let (attrs, rest) = split_leading_attrs("/// Doc.\n#[inline]\n//! Inner.\nfn foo() {}");
        assert_eq!(
            attrs,
            vec![
                "#[doc = \" Doc.\"]".to_string(),
                "#[inline]".to_string(),
                "#[doc = \" Inner.\"]".to_string(),
            ],
            "got: {attrs:?}"
        );
        assert_eq!(rest, "fn foo() {}");
        let (none, same) = split_leading_attrs("//// Not doc.\nfn foo() {}");
        assert!(none.is_empty(), "got: {none:?}");
        assert_eq!(same, "//// Not doc.\nfn foo() {}");
        // T156: single-line block docs peel to `#[doc]`; multi-line
        // openers and ordinary block comments stop the peel.
        let (attrs, rest) = split_leading_attrs("/** Block doc. */\nlet x = E;");
        assert_eq!(attrs, vec!["#[doc = \" Block doc. \"]".to_string()]);
        assert_eq!(rest, "let x = E;");
        let (none, _) = split_leading_attrs("/** Unclosed.\nlet x = E;");
        assert!(none.is_empty(), "got: {none:?}");
        let (none, _) = split_leading_attrs("/* plain */\nlet x = E;");
        assert!(none.is_empty(), "got: {none:?}");
    }

    #[test]
    fn compiles_visibility_meta_fn() {
        // `V fn foo` routes to a function pattern, not Raw/Expr.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=x\n# lang=rust\n# visibility V\n- V fn foo(...) {\n+ pub fn bar(...) {\n      ...\n  }\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(c.kind, PatternKind::Fn { .. }),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn compiles_call_rewrite() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=rename_calls\n# lang=rust\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n",
        );
        let c = compile(&h).unwrap();
        assert!(matches!(c.kind, PatternKind::Expr { .. }));
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn compiles_use_rename() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@ rule=u\n# lang=rust\n# allow=many\n- use crate::old\n+ use crate::new\n",
        );
        let c = compile(&h).unwrap();
        match &c.kind {
            PatternKind::UseRename {
                minus_segs,
                plus_path,
            } => {
                assert_eq!(minus_segs, &vec!["crate".to_string(), "old".to_string()]);
                assert_eq!(plus_path, "crate::new");
            }
            _ => panic!("expected use rename, got: {:?}", c.kind),
        }
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn use_call_callee_not_stolen() {
        // `use_foo(E)` is a call, not a `use` prefix: word-boundary guard.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- use_foo(E)\n+ use_bar(E)\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(c.kind, PatternKind::Expr { .. }),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn detects_unbound_plus() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=x\n# lang=rust\n# expr E\n- foo(E)\n+ bar(Q)\n",
        );
        let c = compile(&h).unwrap();
        // Q undeclared => not tracked as metavar at all.
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn detects_declared_but_unbound_plus() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,8 +40,8 @@ rule=x\n# lang=rust\n# expr E\n# expr Q\n- foo(E)\n+ bar(Q)\n",
        );
        let c = compile(&h).unwrap();
        assert_eq!(c.unbound_plus, vec!["Q".to_string()]);
    }

    #[test]
    fn expr_carries_placeholders_and_path_ellipsis() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -80,20 +80,20 @@ rule=x\n# lang=rust\n# allow=many\n# symbol Old=crate::old_api\n- Old(...)\n+ crate::new_api(...)\n",
        );
        let c = compile(&h).unwrap();
        match &c.kind {
            PatternKind::Expr {
                placeholders,
                has_path_ellipsis,
                ..
            } => {
                assert!(!placeholders.is_empty() || c.table.symbols.contains_key("Old"));
                assert!(!has_path_ellipsis);
            }
            _ => panic!("expected expr"),
        }
    }

    #[test]
    fn compiles_item_capture() {
        let h = hunk_of(
            "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -20,12 +20,0 @@ rule=cut\n# lang=rust\n# item I\n# bind=Helper\n- I\n",
        );
        let c = compile(&h).unwrap();
        assert!(matches!(c.kind, PatternKind::ItemCapture { .. }));
    }

    #[test]
    fn repeated_metavar_detected() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -40,1 +40,1 @@ rule=x\n# lang=rust\n# expr E\n- assert_eq!(E, E)\n+ assert_equal_once(E)\n",
        );
        let c = compile(&h).unwrap();
        assert!(c.repeated.contains(&"E".to_string()));
    }

    #[test]
    fn let_shape_compiles_to_general_stmt() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let x = E;\n+ let y = E;\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Stmt,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
        assert!(c.unbound_plus.is_empty());
    }

    #[test]
    fn impl_trait_multisegment_pair_routes_to_general_type() {
        // T149: `- impl std::fmt::Display` trips the bare-name gate in
        // `parse_side`, but both sides are Rust types, so the hunk falls
        // through to the general type matcher instead of failing.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- impl std::fmt::Display\n+ Box<dyn std::fmt::Display>\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Type,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn block_comment_gate_single_line_normalizes_multi_line_needles() {
        // T156: single-line `/** */` peels and the `let` compiles;
        // multi-line block docs fail fast with the targeted needle.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,2 +2,2 @@ rule=x\n# lang=rust\n# expr E\n- /** Block doc. */\n- let x = E;\n+ /** Block doc. */\n+ let y = E;\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Stmt,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,3 +2,3 @@ rule=x\n# lang=rust\n# expr E\n- /** Multi\n- line. */\n- let x = E;\n+ /** Multi\n+ line. */\n+ let y = E;\n",
        );
        let err = compile(&h).unwrap_err().to_string();
        assert!(err.contains("block comments"), "unexpected: {err}");
    }

    #[test]
    fn binary_expr_compiles_to_general_expr() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + 1\n+ E.wrapping_add(1)\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Expr,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn type_with_type_metavar_compiles_to_general_type() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n+ Box<[T]>\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(
                &c.kind,
                PatternKind::General {
                    shape: GeneralShape::Type,
                    ..
                }
            ),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn call_with_semi_stays_call() {
        // `foo(E);` must keep call-matcher routing (not Stmt/GeneralExpr).
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n- foo(E);\n+ bar(E);\n",
        );
        let c = compile(&h).unwrap();
        assert!(
            matches!(&c.kind, PatternKind::Expr { .. }),
            "got: {:?}",
            c.kind
        );
    }

    #[test]
    fn root_stmt_forces_let() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n# stmt S\n- let x = 1;\n+ let y = 1;\n",
        );
        assert!(matches!(
            compile(&h).unwrap().kind,
            PatternKind::General {
                shape: GeneralShape::Stmt,
                ..
            }
        ));
        let bad = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n- foo\n+ bar\n",
        );
        assert!(compile(&bad).is_err());
    }

    #[test]
    fn ellipsis_rejected_in_general_shapes() {
        // `...` still works in call routing...
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- foo(..., E)\n+ bar(E)\n",
        );
        assert!(matches!(
            compile(&h).unwrap().kind,
            PatternKind::Expr { .. }
        ));
        // ...but not in general shapes.
        let h2 = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + ...\n+ E\n",
        );
        assert!(compile(&h2).is_err());
    }

    #[test]
    fn type_deletion_rejected() {
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,0 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n",
        );
        assert!(compile(&h).is_err());
    }

    #[test]
    fn item_rename_shapes_compile() {
        for (minus, plus) in [
            ("struct Old", "struct New"),
            ("enum Old", "enum New"),
            ("trait Old", "trait New"),
            ("type Old", "type New"),
            ("const Old", "const New"),
            ("static mut Old", "static mut New"),
            ("mod old", "mod new"),
            ("impl Old", "impl New"),
            ("pub struct Old", "pub struct New"),
            ("struct Old { ... }", "struct New { ... }"),
        ] {
            let text = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- {minus}\n+ {plus}\n"
            );
            let c = compile(&hunk_of(&text)).unwrap();
            assert!(
                matches!(&c.kind, PatternKind::ItemRename { .. }),
                "{minus} got: {:?}",
                c.kind
            );
            assert!(c.unbound_plus.is_empty());
        }
    }

    #[test]
    fn item_rename_mismatch_rejected() {
        // Kind mismatch.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old\n+ enum New\n",
        );
        assert!(compile(&h).is_err());
        // Identical names.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old\n+ struct Old\n",
        );
        assert!(compile(&h).is_err());
        // Concrete body.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct Old<T>\n+ struct New\n",
        );
        assert!(compile(&h).is_err());
        // `$Meta` names unsupported.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- struct $S\n+ struct New\n",
        );
        assert!(compile(&h).is_err());
    }

    #[test]
    fn item_rename_prefix_difference_rejected() {
        // T23a: spelled vis/qualifier differences fail fast (rename-only,
        // never a silent vis change). Bare-vs-spelled both directions.
        for (minus, plus) in [
            ("pub struct Old", "struct New"),
            ("struct Old", "pub struct New"),
            ("pub struct Old", "pub(crate) struct New"),
            ("static Old", "static mut New"),
            ("unsafe trait Old", "trait New"),
        ] {
            let text = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- {minus}\n+ {plus}\n"
            );
            let err = compile(&hunk_of(&text)).unwrap_err();
            assert!(
                err.to_string().contains("prefixes differ"),
                "{minus} / {plus} got: {err}"
            );
        }
        // Same spelled prefix still compiles (`pub (crate)` ≡ `pub(crate)`).
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- pub (crate) struct Old\n+ pub(crate) struct New\n";
        assert!(matches!(
            compile(&hunk_of(text)).unwrap().kind,
            PatternKind::ItemRename { .. }
        ));
    }

    #[test]
    fn const_fn_still_routes_to_fn() {
        // `const fn` must not be captured by the item-rename classifier.
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- const fn foo(...) {\n+ const fn bar(...) {\n      ...\n  }\n",
        );
        assert!(
            matches!(compile(&h).unwrap().kind, PatternKind::Fn { .. }),
            "got const-fn routing"
        );
    }

    #[test]
    fn general_coverage_diagnostics_are_pinned() {
        // SPEC 18.6 compile-time rows (C1-C10): each unsupported shape must
        // fail with the exact table diagnostic, so the table cannot drift
        // from the code without a test failing.
        let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let cases = [
            // C1: `...` in a general shape.
            (
                format!(
                    "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n- E + ...\n+ E\n"
                ),
                "v1 general stmt/expr/type patterns do not support `...` (use a call pattern or a concrete shape)",
            ),
            // C2: `let-else` in the minus pattern.
            (
                format!(
                    "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let x = E else {{ return; }};\n+ let y = E;\n"
                ),
                "v1 general patterns do not support `let-else` statements",
            ),
            // C3: `# root=stmt` with a non-`let` shape.
            (
                format!("{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=stmt\n- foo\n+ bar\n"),
                "`# root=stmt` requires a `let` statement pattern (v1 supports `let` only, no `let-else`)",
            ),
            // C4: `# root=expr` with a non-expression shape.
            (
                format!(
                    "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=expr\n- let x = 1;\n+ let y = 1;\n"
                ),
                "`# root=expr` requires a Rust expression pattern",
            ),
            // C5: `# root=type` with a non-type shape.
            (
                format!(
                    "{head}@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# root=type\n- foo bar\n+ Box<u8>\n"
                ),
                "`# root=type` requires a Rust type pattern",
            ),
            // C6: empty `+` side on an expression.
            (
                format!("{head}@@ -2,1 +2,0 @@ rule=x\n# lang=rust\n# expr E\n- E + 1\n"),
                "v1 expression deletion (empty `+` side) is only supported for `...;` statement shapes; match a `let` statement or write an explicit replacement",
            ),
            // C7: empty `+` side on a type.
            (
                format!("{head}@@ -1,1 +1,0 @@ rule=x\n# lang=rust\n# type T\n- Vec<T>\n"),
                "v1 type patterns cannot be deleted (empty `+` side would produce invalid syntax); write an explicit replacement type",
            ),
            // C8: embedded metavariable inside the `let` bound pattern.
            // (T16a: paren tuples reach the gate too — the `let`-led
            // bypass stops the call router claiming `let(...)`; the
            // slice shape below pins the C8 diagnostic byte-for-byte.)
            (
                format!(
                    "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let [E, x] = E;\n+ let [E, x] = E;\n"
                ),
                "v1 `let` patterns support a whole-pattern `# pat P` placeholder, a concrete pattern, or `P: T` with placeholder parts; embedded metavariables elsewhere in the pattern are not supported",
            ),
            // C9: `# returns=none` on a non-function hunk.
            (
                format!(
                    "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr E\n# returns=none\n- foo(E)\n+ bar(E)\n"
                ),
                "`# returns=none` requires a function-item pattern",
            ),
            // C14: `# symbol` in a general shape (call-shape-only even
            // with `# fallback=syntax`, where the constraint is unenforceable).
            (
                format!(
                    "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# expr E\n# fallback=syntax\n# symbol Old=crate::old_api\n- E + 1\n+ E + 2\n"
                ),
                "`# symbol` is only supported in call shapes in v0.1",
            ),
        ];
        assert_eq!(cases.len(), 10, "SPEC 18.6 lists C1-C10 compile-time rows");
        for (patch, want) in cases {
            let err = compile(&hunk_of(&patch)).unwrap_err();
            let rendered = err.render();
            assert_eq!(
                err.code,
                rsmpl_patch::ErrorCode::PatternConflict,
                "wrong code for {want:?}:\n{rendered}"
            );
            assert!(
                rendered.contains(want),
                "SPEC 18.6 drift: want {want:?} in:\n{rendered}"
            );
        }
    }

    #[test]
    fn paren_tuple_let_reaches_let_gate() {
        // T16a router-precision pin: `let (E, x)` must reach the general
        // `let` gate (C8) instead of classifying as a `let(...)` call.
        let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let patch = format!(
            "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# stmt S\n# expr E\n- let (E, x) = E;\n+ let (E, x) = E;\n"
        );
        let err = compile(&hunk_of(&patch)).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render().contains("embedded metavariables elsewhere"),
            "paren-tuple let missed the C8 gate:\n{}",
            err.render()
        );
        // Concrete paren-tuple lets compile to the Stmt shape.
        let ok = format!(
            "{head}@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n- let (a, b) = y;\n+ let (c, d) = y;\n"
        );
        assert!(
            matches!(
                compile(&hunk_of(&ok)).unwrap().kind,
                PatternKind::General {
                    shape: GeneralShape::Stmt,
                    ..
                }
            ),
            "concrete paren-tuple let should be General::Stmt"
        );
    }

    #[test]
    fn method_ident_symbol_rejected_at_compile() {
        // T147: a `# symbol` naming the method ident can never match (the
        // method slot compares bare idents, never seeded paths), so fail
        // fast instead of a bare match-cycle `no match found`.
        let head = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let patch = format!(
            "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol old=crate::old_api\n# expr R\n# expr E\n- R.old(E)\n+ R.new(E)\n"
        );
        let err = compile(&hunk_of(&patch)).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render().contains("names the method ident"),
            "missing needle:\n{}",
            err.render()
        );
        // Negative control: a receiver symbol is the supported C14 shape
        // and must still compile.
        let ok = format!(
            "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# fallback=syntax\n# symbol R=crate::r::R\n# expr E\n- R.old(E)\n+ R.new(E)\n"
        );
        assert!(
            matches!(
                compile(&hunk_of(&ok)).unwrap().kind,
                PatternKind::Expr {
                    minus: ExprPatternKind::MethodCall { .. },
                    ..
                }
            ),
            "receiver symbols must keep compiling"
        );
        // Second negative control: a plain `# expr` metavar in method
        // position binds any method and must not trip the symbol gate.
        let ok_meta = format!(
            "{head}@@ -2,1 @@ rule=x\n# lang=rust\n# allow=many\n# expr R\n# expr M\n# expr E\n- R.M(E)\n+ R.new(E)\n"
        );
        assert!(
            matches!(
                compile(&hunk_of(&ok_meta)).unwrap().kind,
                PatternKind::Expr {
                    minus: ExprPatternKind::MethodCall { .. },
                    ..
                }
            ),
            "method-position expr metavars must keep compiling"
        );
    }

    #[test]
    fn unknown_root_values_rejected() {
        // T110: only item/expr/stmt/type have a force path; anything else
        // must fail fast instead of silently falling through to heuristic
        // routing (SPEC 2.6).
        for bad in ["pat", "impl_item", "trait_item", "bogus"] {
            let patch = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root={bad}\n- foo\n+ bar\n"
            );
            let err = compile(&hunk_of(&patch)).unwrap_err();
            assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
            assert!(
                err.render().contains("unknown `# root=` value"),
                "missing needle for root={bad}: {}",
                err.render()
            );
        }
        // Supported roots still route.
        let ok = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# root=expr\n# expr E\n- foo(E)\n+ bar(E)\n",
        );
        assert!(compile(&ok).is_ok());
    }

    #[test]
    fn unsupported_metavar_kinds_rejected() {
        // T111: identifier/path/lifetime/field have no engine binding; they
        // must fail fast instead of parsing as generic placeholders (SPEC 2.6).
        for kind in ["identifier", "path", "lifetime", "field"] {
            let patch = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 +2,1 @@ rule=x\n# lang=rust\n# {kind} X\n- foo(X)\n+ bar(X)\n"
            );
            let err = compile(&hunk_of(&patch)).unwrap_err();
            assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
            assert!(
                err.render().contains("is not supported in v0.1"),
                "missing needle for kind={kind}: {}",
                err.render()
            );
        }
        // Supported-kind controls still compile (incl. structural S/M/TT).
        for decl in ["# expr E", "# stmt S", "# macro M", "# token_tree TT"] {
            let patch = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n{decl}\n- foo\n+ bar\n"
            );
            assert!(compile(&hunk_of(&patch)).is_ok(), "decl {decl} broke");
        }
    }

    #[test]
    fn repeat_non_call_shape_rejected() {
        // T20: `# repeat` is a call/method-call feature; T126 extended it
        // to general-expression shapes. On any other shape it would be a
        // silent no-op, so it fails fast (SPEC 2.6, 17.8, 24.4). Pairs
        // differ so the shape needle fires (not an earlier same-name error).
        for (minus, plus, needle) in [
            (
                "struct Old",
                "struct New",
                "`# repeat` requires a call/method-call or general-expression",
            ),
            (
                "let x = E;",
                "let y = E;",
                "`# repeat` is not supported on `let`-statement patterns in v1",
            ),
            (
                "mac!(E)",
                "mac2!(E)",
                "`# repeat` requires a call/method-call or general-expression",
            ),
        ] {
            let patch = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# expr E\n- {minus}\n+ {plus}\n"
            );
            // `mac!(E)` routes to the macro matcher; the rest are item /
            // general shapes. All must reject `# repeat` with a
            // shape-specific needle.
            let err = compile(&hunk_of(&patch)).unwrap_err();
            assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
            assert!(
                err.render().contains(needle),
                "missing needle {needle:?} for {minus:?}: {}",
                err.render()
            );
        }
        // Type shape rejects with its own needle (needs a `# type`
        // declaration to reach the repeat gate).
        let ty = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# type T\n- Vec<T>\n+ Box<T>\n",
        );
        let err = compile(&ty).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render()
                .contains("`# repeat` is not supported on type patterns in v1"),
            "missing type needle: {}",
            err.render()
        );
        // Function shape (shared context lines, like `compiles_fn_rename`).
        let h = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        );
        let err = compile(&h).unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render()
                .contains("`# repeat` requires a call/method-call or general-expression"),
            "missing fn needle: {}",
            err.render()
        );
        let ok = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# expr E\n- foo(E)\n+ bar(E)\n",
        );
        assert!(compile(&ok).is_ok());
        // T126: general-expression shapes accept `# repeat`.
        let ok_expr = hunk_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# repeat\n# expr E\n- E + 1\n+ E + 2\n",
        );
        assert!(compile(&ok_expr).is_ok());
    }
}
