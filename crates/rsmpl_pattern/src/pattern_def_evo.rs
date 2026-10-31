//! Definition RHS evolution classification (SPEC 14.8, WI-11a).
//!
//! Split from `pattern_classify` per D5 — the parent file sits over the
//! 1600-line gate. The routing hook in `pattern_classify::classify` stays
//! (three lines); every helper lives here.

use rsmpl_patch::{ErrorCode, RsmplHunk, StructuredError};

use crate::metavar::MetavarTable;
use crate::pattern::PatternKind;
use crate::pattern_classify::{find_field_colon, is_member_ident};

/// Definition RHS evolution (SPEC 14.8, WI-11a): two context-free shapes
/// with a concrete preserved name.
///
/// - `type NAME = RHS;` — RHS is a type template (`T` holes ride the
///   existing diet/`# type T` inference);
/// - `(const | static [mut]) NAME : TY = EXPR;` — `TY` token-equal on both
///   sides, `EXPR` a general-expr template.
///
/// Visibility, generics, `where` clauses, and attributes are preserved
/// from the target (patterns stay bare, like item renames). Returns
/// `Ok(None)` when neither side carries the shape — bare renames,
/// `const { ... }` block expressions, and every other shape fall through
/// untouched. A claimed shape with a malformed half is a loud
/// `PatternConflict` (never a silent fallthrough, SPEC 2.6).
pub(crate) fn classify_definition_evolution(
    minus_raw: &str,
    plus_raw: &str,
    table: &MetavarTable,
    hunk: &RsmplHunk,
) -> Result<Option<PatternKind>, StructuredError> {
    let (minus_attrs, minus_rest) = crate::pattern_general::split_leading_attrs(minus_raw);
    let (plus_attrs, plus_rest) = crate::pattern_general::split_leading_attrs(plus_raw);
    // T153: interior lines of multi-line patterns keep the one-space
    // diff-prefix artifact — trim per side (matchers are
    // whitespace-insensitive; replacements are span-based).
    let minus_rest = minus_rest.trim().to_string();
    let plus_rest = plus_rest.trim().to_string();
    // Multi-line bodies (context headers, block shapes) are not definition
    // evolution: single-line pairs only, so `mod m {` context never routes
    // here (the matcher collects from inline modules regardless).
    if minus_rest.contains('\n') || plus_rest.contains('\n') {
        return Ok(None);
    }
    let minus_head = def_head(&minus_rest);
    let plus_head = def_head(&plus_rest);
    match (&minus_head, &plus_head) {
        (None, None) => Ok(None),
        (Some(m), Some(p)) if m == p => {
            let head = *m;
            // Attribute mirror laws (SPEC 14.6 rename parity): written lines must
            // mirror. Attr add/remove beside an evolution stays an ordinary hunk
            // in this slice (no attr delta here).
    if !plus_attrs.is_empty() && minus_attrs.is_empty() {
        return Err(def_err(
            "plus side writes attributes the minus side omitted (ambiguous merge); write the attributes on both sides or neither",
            "write the attributes on both sides or neither",
        ));
    }
    if hunk.attrs_policy() != rsmpl_patch::AttrsPolicy::Strip
        && !minus_attrs.is_empty()
        && norm_attr_join(&minus_attrs) != norm_attr_join(&plus_attrs)
    {
        return Err(def_err(
            "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
            "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
        ));
    }
    match head {
        DefHead::Type => {
            let (mvis, mname, mrhs) = parse_type_side(&minus_rest)?;
            let (pvis, pname, prhs) = parse_type_side(&plus_rest)?;
            if mvis != pvis {
                return Err(def_err(
                    format!(
                        "type alias visibility differs (`{mvis}` vs `{pvis}`); evolution preserves visibility — write the same prefix on both sides or neither"
                    ),
                    "write the same visibility prefix on both sides or neither",
                ));
            }
            if mname != pname {
                return Err(rename_instead(&mname, &pname, &mrhs, &prhs));
            }
            let (minus_sub, placeholders) = compile_def_rhs(&mrhs, table, false)?;
            compile_def_rhs(&prhs, table, false)?;
            Ok(Some(PatternKind::TypeAliasEvolution {
                name: mname,
                minus_sub,
                plus_raw: prhs,
                placeholders,
                minus_attrs,
                plus_attrs,
            }))
        }
        DefHead::Const {
            is_static,
            is_mut,
        } => {
            let (mvis, mname, mty, mexpr) = parse_const_side(&minus_rest, is_static, is_mut)?;
            let (pvis, pname, pty, pexpr) = parse_const_side(&plus_rest, is_static, is_mut)?;
            if mvis != pvis {
                return Err(def_err(
                    format!(
                        "const/static visibility differs (`{mvis}` vs `{pvis}`); evolution preserves visibility — write the same prefix on both sides or neither"
                    ),
                    "write the same visibility prefix on both sides or neither",
                ));
            }
            if mname != pname {
                return Err(rename_instead(&mname, &pname, &mexpr, &pexpr));
            }
            if mty != pty {
                return Err(def_err(
                    format!(
                        "const/static ascribed types differ (`{mty}` vs `{pty}`); evolution preserves the type — write the same type on both sides, or use an ordinary hunk"
                    ),
                    "write the same type on both sides, or use an ordinary hunk",
                ));
            }
            let (minus_sub, placeholders) = compile_def_rhs(&mexpr, table, true)?;
            compile_def_rhs(&pexpr, table, true)?;
            Ok(Some(PatternKind::ConstEvolution {
                is_static,
                is_mut,
                name: mname,
                ty: mty,
                minus_sub,
                plus_raw: pexpr,
                placeholders,
                minus_attrs,
                plus_attrs,
            }))
        }
        }
        }
        _ => Err(def_err(
            "definition evolution minus and plus sides must carry the same shape (`type NAME = ...;` on both sides, or `const`/`static [mut] NAME: TY = ...;` on both sides)",
            "write the same definition shape on both sides, or use an ordinary hunk",
        )),
    }
}

/// The claimed definition head: `type`, or `const`/`static (+ mut)`.
/// `None` when the text carries no definition shape (bare renames,
/// `const { ... }` block expressions, unclaimed halves).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DefHead {
    Type,
    Const { is_static: bool, is_mut: bool },
}

/// Head probe: `type NAME =`, `const NAME :`, `static [mut] NAME :`.
/// The `=`/`:` gate keeps bare renames (`type Old`) and block
/// expressions (`const { ... }`) unclaimed.
fn def_head(text: &str) -> Option<DefHead> {
    let (_, rest) = strip_def_vis(text);
    if let Some(after) = eat_def_word(rest, "type") {
        let name_end = after
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(after.len());
        let (name, tail) = (&after[..name_end], after[name_end..].trim_start());
        if is_member_ident(name) && tail.starts_with('=') {
            return Some(DefHead::Type);
        }
        return None;
    }
    for (kw, is_static) in [("const", false), ("static", true)] {
        if let Some(mut after) = eat_def_word(rest, kw) {
            let mut is_mut = false;
            if is_static {
                if let Some(next) = eat_def_word(after, "mut") {
                    is_mut = true;
                    after = next;
                }
            }
            let name_end = after
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(after.len());
            let (name, tail) = (&after[..name_end], &after[name_end..]);
            if is_member_ident(name) && find_field_colon(tail).is_some() {
                return Some(DefHead::Const { is_static, is_mut });
            }
            return None;
        }
    }
    None
}

/// Split an optional `pub` / `pub(...)` visibility prefix; returns the
/// whitespace-normalized prefix plus the rest. Only consumes `pub` when
/// followed by a delimiter so `public` (an ident) is never visibility.
fn strip_def_vis(s: &str) -> (String, &str) {
    let t = s.trim_start();
    let Some(rest) = t.strip_prefix("pub") else {
        return (String::new(), s);
    };
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        return ("pub".to_string(), rest.trim_start());
    }
    if let Some(after) = rest.trim_start().strip_prefix('(') {
        let mut depth = 1i32;
        for (i, c) in after.char_indices() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    let raw = &s[..s.len() - after[i + 1..].trim_start().len()];
                    let norm: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                    return (norm, after[i + 1..].trim_start());
                }
            }
        }
        return (String::new(), s); // unbalanced: don't claim
    }
    (String::new(), s)
}

/// Consume one lowercase keyword when followed by a delimiter.
fn eat_def_word<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        Some(rest.trim_start())
    } else {
        None
    }
}

/// Parse one `type` evolution side (`[pub] type NAME = RHS;`) into
/// (vis-prefix, name, raw RHS without the terminator).
fn parse_type_side(side: &str) -> Result<(String, String, String), StructuredError> {
    let (vis, rest) = strip_def_vis(side);
    let after = eat_def_word(rest, "type").expect("definition evolution type head");
    let name_end = after
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let (name, tail) = (&after[..name_end], after[name_end..].trim_start());
    if !is_member_ident(name) {
        return Err(def_err(
            format!("type evolution needs a bare identifier name (got `{name}`); generics are preserved from the target, never authored"),
            "write `type NAME = RHS;` with a bare identifier name",
        ));
    }
    if name_end < after.len() && after[name_end..].starts_with('<') {
        return Err(def_err(
            "type alias generic parameters cannot evolve in v1 (write the bare `type NAME = RHS;` form; generics are preserved)",
            "write the bare `type NAME = RHS;` form; generics are preserved",
        ));
    }
    let Some(rhs) = tail.strip_prefix('=') else {
        return Err(def_err(
            "type evolution needs `= RHS;` after the name (bare `type NAME` is a rename — write the bare `<kind> <Name>` pair on both sides)",
            "write `type NAME = RHS;` on both sides, or the bare `<kind> <Name>` pair",
        ));
    };
    let rhs = rhs.trim();
    let Some(rhs) = rhs.strip_suffix(';') else {
        return Err(def_err(
            "type evolution needs a trailing `;` (write `type NAME = RHS;`)",
            "write `type NAME = RHS;`",
        ));
    };
    let rhs = rhs.trim();
    if rhs.is_empty() {
        return Err(def_err(
            "type evolution names no RHS (write `type NAME = RHS;`)",
            "write `type NAME = RHS;`",
        ));
    }
    Ok((vis, name.to_string(), rhs.to_string()))
}

/// Parse one `const`/`static` evolution side
/// (`[pub] const NAME: TY = EXPR;`, `static [mut]` twin) into
/// (vis-prefix, name, normalized ascribed type, raw value without terminator).
fn parse_const_side(
    side: &str,
    is_static: bool,
    is_mut: bool,
) -> Result<(String, String, String, String), StructuredError> {
    let kw = if is_static { "static" } else { "const" };
    let (vis, rest) = strip_def_vis(side);
    let mut after = eat_def_word(rest, kw).expect("definition evolution const/static head");
    if is_static {
        if is_mut {
            after = eat_def_word(after, "mut").ok_or_else(|| {
                def_err(
                    "`static mut` must be written on both sides (mutability never evolves)",
                    "write `static mut NAME: TY = EXPR;` on both sides",
                )
            })?;
        } else if eat_def_word(after, "mut").is_some() {
            return Err(def_err(
                "`static mut` must be written on both sides (mutability never evolves)",
                "write `static NAME: TY = EXPR;` on both sides, or `static mut` on both",
            ));
        }
    }
    let name_end = after
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let (name, tail) = (&after[..name_end], &after[name_end..]);
    if !is_member_ident(name) {
        return Err(def_err(
            format!("{kw} evolution needs a bare identifier name (got `{name}`)"),
            "write `const NAME: TY = EXPR;` with a bare identifier name",
        ));
    }
    let Some(colon) = find_field_colon(tail) else {
        return Err(def_err(
            format!("{kw} evolution needs `: TY = EXPR;` after the name (write `{kw} NAME: TY = EXPR;`)"),
            "write `const NAME: TY = EXPR;` (or the `static [mut]` twin)",
        ));
    };
    let after_ty = tail[colon + 1..].trim();
    let Some(eq) = after_ty.find('=') else {
        return Err(def_err(
            format!("{kw} evolution needs `= EXPR;` after the ascribed type (write `{kw} NAME: TY = EXPR;`)"),
            "write `const NAME: TY = EXPR;` (or the `static [mut]` twin)",
        ));
    };
    let ty_raw = after_ty[..eq].trim();
    if ty_raw.is_empty() {
        return Err(def_err(
            format!("{kw} evolution names no ascribed type (write `{kw} NAME: TY = EXPR;`)"),
            "write `const NAME: TY = EXPR;` (or the `static [mut]` twin)",
        ));
    }
    let ty: syn::Type = syn::parse_str(ty_raw).map_err(|_| {
        def_err(
            format!("{kw} ascribed type `{ty_raw}` does not parse as a Rust type"),
            "write a Rust type between `:` and `=`",
        )
    })?;
    let norm_ty = quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let expr = after_ty[eq + 1..].trim();
    let Some(expr) = expr.strip_suffix(';') else {
        return Err(def_err(
            format!("{kw} evolution needs a trailing `;` (write `{kw} NAME: TY = EXPR;`)"),
            "write `const NAME: TY = EXPR;` with a trailing `;`",
        ));
    };
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(def_err(
            format!("{kw} evolution names no value (write `{kw} NAME: TY = EXPR;`)"),
            "write `const NAME: TY = EXPR;` (or the `static [mut]` twin)",
        ));
    }
    Ok((vis, name.to_string(), norm_ty, expr.to_string()))
}

/// Compile one definition RHS (type template or general-expr template)
/// through the placeholder pipeline; returns the substituted minus text
/// plus the (name → placeholder) map. The plus side is validated the
/// same way (parse must succeed) but stored raw for match-time
/// substitution. `...` outside literals is rejected (v1 templates carry
/// no ellipsis here, SPEC 16).
fn compile_def_rhs(
    rhs: &str,
    table: &MetavarTable,
    is_expr: bool,
) -> Result<(String, Vec<(String, String)>), StructuredError> {
    let (masked, sentinels) = crate::quote_utils::mask_literal_spans(rhs)?;
    if masked.contains("...") {
        return Err(def_err(
            "ellipsis is not supported in definition RHS templates in v1 (write the full RHS)",
            "write the full RHS without `...`",
        ));
    }
    let (mut sub, maps) = crate::expr_pattern::ExprPattern::placeholder_expr(&masked, table);
    sub = crate::quote_utils::unmask_literal_spans(&sub, &sentinels);
    if is_expr {
        syn::parse_str::<syn::Expr>(&sub).map_err(|e| {
            def_err(
                format!("definition value `{rhs}` does not parse as a Rust expression: {e}"),
                "write a Rust expression template for the value",
            )
        })?;
    } else {
        syn::parse_str::<syn::Type>(&sub).map_err(|e| {
            def_err(
                format!("definition RHS `{rhs}` does not parse as a Rust type: {e}"),
                "write a Rust type template for the RHS",
            )
        })?;
    }
    let placeholders: Vec<(String, String)> = maps
        .expr_placeholders
        .into_iter()
        .map(|(ph, n)| (n, ph))
        .collect();
    Ok((sub, placeholders))
}

/// A name-changing "evolution": fail fast naming the rename spelling.
/// Same RHS on both sides is a plain rename written with a concrete
/// suffix (point at the bare form); differing RHS on both sides never
/// combines rename + evolution (point at two hunks).
fn rename_instead(mname: &str, pname: &str, mrhs: &str, phs: &str) -> StructuredError {
    if mrhs.trim() == phs.trim() {
        def_err(
            format!(
                "definition names differ (`{mname}` vs `{pname}`) with the same RHS; renames are ident-only — write the bare `<kind> <Name>` pair on both sides (visibility, generics and bodies are preserved)"
            ),
            "write the bare `<kind> <Name>` pair on both sides",
        )
    } else {
        def_err(
            format!(
                "definition names differ (`{mname}` vs `{pname}`) and the RHS differs; rename and evolution never combine — write a bare rename hunk plus an evolution hunk, or use an ordinary hunk"
            ),
            "write a bare rename hunk plus an evolution hunk, or use an ordinary hunk",
        )
    }
}

fn norm_attr_join(attrs: &[String]) -> Vec<String> {
    attrs
        .iter()
        .map(|a| a.split_whitespace().collect::<String>())
        .collect()
}

fn def_err(msg: impl Into<String>, write_instead: &str) -> StructuredError {
    StructuredError::new(ErrorCode::PatternConflict, msg.into()).with_repair(
        StructuredError::fix_pattern_repair("definition", write_instead),
    )
}
