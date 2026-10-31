//! Pattern compilation: from raw hunk body + directives to structured kinds.
//!
//! Root inference (SPEC 13.4): explicit `# root=` wins; otherwise heuristics
//! on the minus text (fn/item/call/macro/insert). The engine performs the
//! actual target matching; this module validates shape and extracts bindings.
//!
//! Routing lives in `pattern_classify`, generalized shapes in
//! `pattern_general`, trailing-`...` validation in `pattern_ellipsis`, and
//! the evolution-paste template in `pattern_evolution`.

use rsmpl_patch::{AttrsPolicy, Directive, ErrorCode, MetavarKind, RsmplHunk, StructuredError};
use std::collections::HashSet;

use crate::expr_pattern::ExprPatternKind;
use crate::metavar::MetavarTable;
use crate::pattern_classify::classify;
use crate::pattern_ellipsis::validate_ellipsis_positions;
use crate::quote_utils::validate_quoting;

pub use crate::pattern_classify::{
    impl_block_matches, is_trait_header, parse_impl_header, trait_header_ident, ImplHeaderFilter,
};
pub use crate::pattern_evolution::{
    is_evolution_template, parse_const_evolution_template, parse_enum_evolution_template,
    parse_evolution_template, parse_fn_full_header_template, parse_mod_evolution_template,
    parse_static_evolution_template, parse_struct_evolution_template,
    parse_trait_evolution_template, parse_type_evolution_template, parse_union_evolution_template,
};
pub use crate::pattern_general::split_leading_attrs;

/// Shape of a generalized (non-call) pattern (T10 v1 subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralShape {
    /// A `let` statement (`syn::Stmt::Local`, with or without `let-else`).
    Stmt,
    /// A general Rust expression (bounded engine variant set).
    Expr,
    /// A general Rust type (bounded engine variant set).
    Type,
    /// WI-5c: a repeated-block statement sequence (2–4 `let` statements,
    /// exact count both sides, no `...`/`# repeat`). Stored as
    /// newline-joined single-statement templates; the engine matches K
    /// consecutive `Block` statements with shared bindings.
    Sequence,
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
    /// Struct-field rename (`struct S {` context + `old: T` -> `new: T`):
    /// only the field ident span is replaced; the container, visibility,
    /// and type are preserved. Named fields only (SPEC 13.4).
    FieldRename {
        struct_name: String,
        old: String,
        new: String,
        /// `quote!`-normalized expected field type (both sides equal).
        ty: String,
        /// Leading `#[...]`/doc attribute lines on each side (SPEC 14.6
        /// mirror parity with item renames).
        minus_attrs: Vec<String>,
        plus_attrs: Vec<String>,
    },
    /// Enum-variant rename (`enum E {` context + `Old` -> `New`):
    /// only the variant ident span is replaced; discriminants and
    /// fields are preserved (SPEC 13.4).
    VariantRename {
        enum_name: String,
        old: String,
        new: String,
        minus_attrs: Vec<String>,
        plus_attrs: Vec<String>,
    },
    /// Type-alias RHS evolution (`- type A = Vec<T>;` /
    /// `+ type A = Box<[T]>;`): the name is concrete and preserved, the
    /// RHS is a type template. Visibility, generics, `where` clauses,
    /// and attributes are preserved. `# follow=uses` is rejected (C17).
    TypeAliasEvolution {
        name: String,
        minus_sub: String,
        plus_raw: String,
        /// Placeholder idents per metavariable, for substitution checks.
        placeholders: Vec<(String, String)>,
        minus_attrs: Vec<String>,
        plus_attrs: Vec<String>,
    },
    /// Const/static value evolution (`- const MAX: i32 = E;` /
    /// `+ const MAX: i32 = E + 1;`): name and ascribed type are preserved
    /// token-equal, the value is a general-expr template. `static [mut]`
    /// shares this kind (`is_static`/`is_mut` from the pattern; the
    /// target must agree). `# follow=uses` is rejected (C17).
    ConstEvolution {
        is_static: bool,
        is_mut: bool,
        name: String,
        ty: String,
        minus_sub: String,
        plus_raw: String,
        /// Placeholder idents per metavariable, for substitution checks.
        placeholders: Vec<(String, String)>,
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
///
/// WI-3b/2b: any `PatternConflict` reaching this boundary without a specific
/// `repair` gains the keyed generic `fix_pattern` payload (`code: "pattern"`,
/// concrete template — never the echoed reason, per SPEC D.2). Specific
/// C-table gates pin exact codes; this covers the long tail
/// (slot/type/macro sub-compilers) so no failure is prose-only.
pub fn compile(hunk: &RsmplHunk) -> Result<CompiledPattern, StructuredError> {
    compile_inner(hunk).map_err(|e| {
        let mut e = if e.code == ErrorCode::PatternConflict && e.repair.is_none() {
            e.with_repair(StructuredError::fix_pattern_repair(
                "pattern",
                "rewrite the pattern to a supported shape (see SPEC 18.6) or use an ordinary hunk",
            ))
        } else {
            e
        };
        // Hint for `rule=`-only text hunks (SPEC 7.1): a header `rule=`
        // with no `#` directives reclassifies an ordinary-shaped edit as
        // semantic, so the pattern parse names the symptom, not the
        // reclassification. Point at the marker, not the symptom.
        if e.code == ErrorCode::PatternConflict
            && hunk.directives.is_empty()
            && hunk.header.rule.is_some()
        {
            const HINT: &str = "remove `rule=` for plain text hunks (or add `#` directives for a semantic pattern)";
            let next = match e.suggestion.take() {
                None => HINT.to_string(),
                Some(s) if s.contains("remove `rule=`") => s,
                Some(s) => format!("{s} — {HINT}"),
            };
            e = e.with_suggestion(next);
        }
        e
    })
}

/// Compile an RSmPL hunk's pattern body (inner; see `compile`).
fn compile_inner(hunk: &RsmplHunk) -> Result<CompiledPattern, StructuredError> {
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
            // WI-3: `fix_directive` over the shared distance-2 table (no
            // second spelling list); far misses repair to a removal.
            let close = rsmpl_patch::suggest_value(r, &["item", "expr", "stmt", "type"]);
            let found = format!("root={r}");
            let (did_you_mean, write_instead) = match close {
                Some(best) => {
                    let line = format!("# root={best}");
                    (Some(line.clone()), line)
                }
                None => (
                    None,
                    "remove the directive or use `# root=item|expr|stmt|type` (see SPEC 13.4)"
                        .to_string(),
                ),
            };
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "unknown `# root=` value `{r}` (only `item/expr/stmt/type` are supported in v0.1; remove it or use a supported value — see SPEC 13.4)"
                ),
            )
            .with_repair(StructuredError::fix_directive_repair(
                found.as_str(),
                did_you_mean.as_deref(),
                write_instead.as_str(),
            )));
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
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "kind",
                "see SPEC 15.1 for the implemented kinds",
            )));
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
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "attributes",
            "write the `#[...]` lines literally on both sides to constrain them, or omit them — attributes are preserved by default, SPEC 14.6",
        )));
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

    // WI-5 C20: a name hole (`N` / `# ident X`) never binds in
    // call/method/macro callee positions (and never a qualified path
    // segment). Those spellings keep their backtick/literal rules; the
    // hole lives in fn-header name position only (SPEC 15.8). Callee
    // text carries `__RSMPL_{name}` placeholders (every declared name
    // is substituted), so the check runs on placeholder tokens —
    // whole-token, so `__RSMPL_N` never matches inside `__RSMPL_N2`.
    {
        let ident_names: Vec<String> = table
            .declared
            .iter()
            .filter(|(k, _)| *k == MetavarKind::Ident)
            .map(|(_, n)| n.clone())
            .collect();
        if !ident_names.is_empty() {
            let ph_tokens = |s: &str| -> Vec<String> {
                s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .map(|t| t.to_string())
                    .collect()
            };
            let hole_in = |s: &str| -> Option<String> {
                let toks = ph_tokens(s);
                ident_names.iter().find_map(|n| {
                    let ph = format!("__RSMPL_{n}");
                    toks.iter().any(|t| t == &ph).then(|| n.clone())
                })
            };
            let misplaced: Option<String> = match &kind {
                PatternKind::Expr {
                    minus:
                        ExprPatternKind::Call { callee, .. }
                        | ExprPatternKind::Macro { name: callee, .. },
                    ..
                } => hole_in(callee),
                PatternKind::Expr {
                    minus: ExprPatternKind::MethodCall { method, .. },
                    ..
                } => hole_in(method),
                _ => None,
            };
            if let Some(bad) = misplaced {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "`{bad}` is a name hole (`# ident` / diet `N`); it only binds in fn-header name position — write the call/method callee literally (or `` `{bad}` `` to match the literal ident, SPEC 15.8)"
                    ),
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "C20",
                    "write the call/method callee literally, or use a fn-header name-hole pattern",
                )));
            }
        }
    }
    // WI-8 slice 9: an evolution paste is verbatim plus-side text
    // spliced under a capture, never a match pattern — its idents
    // (`T`, `Vec`, ...) are not metavariable holes, so the unbound-plus
    // gate never fires for it (the abstract `(...)` form never needed
    // this because it carries no uppercase idents).
    if matches!(
        &kind,
        PatternKind::Insertion {
            use_name: Some(_),
            ..
        }
    ) && is_evolution_template(plus_raw.trim())
    {
        unbound_plus.clear();
    }

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
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C9",
            "`# returns=none` requires a function-item pattern",
        )));
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
                GeneralShape::Sequence => "general statement-sequence",
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
            PatternKind::FieldRename { .. } => "field-rename",
            PatternKind::VariantRename { .. } => "variant-rename",
            PatternKind::TypeAliasEvolution { .. } => "type-alias evolution",
            PatternKind::ConstEvolution { .. } => "const/static evolution",
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
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C14",
            "use a call-shape hunk or a concrete path pattern",
        )));
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
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "C14",
                "remove that `# symbol` line (symbols verify receiver/callee paths, SPEC 19.3)",
            )));
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
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C15",
            "remove the line, or add `# symbol Name=path` (SPEC 19.3)",
        )));
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
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "C15",
            "remove the line (SPEC 14.6)",
        )));
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
                PatternKind::General {
                    shape: GeneralShape::Sequence,
                    ..
                } => {
                    "`# repeat` is not supported on statement-sequence patterns in v1; emit one ordered hunk per nesting level"
                }
                _ => {
                    "`# repeat` requires a call/method-call or general-expression rewrite pattern (other shapes are future work)"
                }
            };
            return Err(
                StructuredError::new(ErrorCode::PatternConflict, reason.to_string()).with_repair(
                    StructuredError::fix_pattern_repair(
                        "repeat",
                        "emit one ordered hunk per nesting level",
                    ),
                ),
            );
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
