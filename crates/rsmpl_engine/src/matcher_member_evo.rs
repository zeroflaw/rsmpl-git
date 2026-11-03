//! Member-context definition RHS evolution (SPEC 14.8, WI-11b).
//!
//! Field-type, variant-payload, and discriminant rewrites. The author
//! writes the member pair under its container header with a concrete
//! preserved name; only the evolved RHS span is replaced — the container,
//! visibility, attributes, and suffixes are preserved byte-for-byte.
//!
//! ```diff
//!   struct S {
//! - old: Vec<T>
//! + old: Box<[T]>
//! ```
//!
//! ```diff
//!   enum E {
//! - Old(Vec<T>)
//! + Old(Box<[T]>)
//! ```
//!
//! Single-region rule (classifier-enforced): one hunk evolves the field
//! type XOR the tuple payload XOR the struct payload XOR the discriminant.
//! A mismatched ascribed vis/`mut` or preserved-region difference skips the
//! site (subset semantics: never rewrites under a lie).

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{Allow, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind, VariantPayload};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_item::{attrs_equal, check_attrs, span_bytes};

/// One field candidate: the named field's type span.
struct FieldCandidate<'ast> {
    attrs: Vec<syn::Attribute>,
    ty: &'ast syn::Type,
    vis_norm: String,
    container_start: u32,
    container_end: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

fn norm_vis(v: &syn::Visibility) -> String {
    quote::quote!(#v)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn collect_fields<'ast>(
    items: &'ast [syn::Item],
    struct_name: &str,
    field: &str,
    out: &mut Vec<FieldCandidate<'ast>>,
) {
    struct Collector<'a, 'ast> {
        struct_name: &'a str,
        field: &'a str,
        found: Vec<FieldCandidate<'ast>>,
    }
    impl<'ast> Visit<'ast> for Collector<'_, 'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Struct(s) = it {
                if s.ident == self.struct_name {
                    if let syn::Fields::Named(fields) = &s.fields {
                        for f in &fields.named {
                            if let Some(ident) = &f.ident {
                                if ident == self.field {
                                    self.found.push(FieldCandidate {
                                        attrs: f.attrs.clone(),
                                        ty: &f.ty,
                                        vis_norm: norm_vis(&f.vis),
                                        container_start: s.span().start().line as u32,
                                        container_end: s.span().end().line as u32,
                                        anchor_line: ident.span().start().line as u32,
                                        attr_start_line: f.span().start().line as u32,
                                    });
                                }
                            }
                        }
                    }
                }
            }
            visit::visit_item(self, it);
        }
    }
    let mut c = Collector {
        struct_name,
        field,
        found: Vec::new(),
    };
    for it in items {
        c.visit_item(it);
    }
    out.extend(c.found);
}

/// One variant candidate: tuple payload types, struct payload fields, or
/// discriminant expression.
struct VariantCandidate<'ast> {
    attrs: Vec<syn::Attribute>,
    tuple_tys: Vec<&'ast syn::Type>,
    struct_fields: Vec<(String, &'ast syn::Type)>,
    discriminant: Option<&'ast syn::Expr>,
    is_tuple: bool,
    is_struct: bool,
    container_start: u32,
    container_end: u32,
    anchor_line: u32,
    attr_start_line: u32,
}

fn collect_variants<'ast>(
    items: &'ast [syn::Item],
    enum_name: &str,
    variant: &str,
    out: &mut Vec<VariantCandidate<'ast>>,
) {
    struct Collector<'a, 'ast> {
        enum_name: &'a str,
        variant: &'a str,
        found: Vec<VariantCandidate<'ast>>,
    }
    impl<'ast> Visit<'ast> for Collector<'_, 'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Enum(e) = it {
                if e.ident == self.enum_name {
                    for v in &e.variants {
                        if v.ident == self.variant {
                            let (tuple_tys, struct_fields, is_tuple, is_struct) = match &v.fields {
                                syn::Fields::Unnamed(u) => (
                                    u.unnamed.iter().map(|f| &f.ty).collect(),
                                    Vec::new(),
                                    true,
                                    false,
                                ),
                                syn::Fields::Named(n) => (
                                    Vec::new(),
                                    n.named
                                        .iter()
                                        .filter_map(|f| {
                                            f.ident.as_ref().map(|i| (i.to_string(), &f.ty))
                                        })
                                        .collect(),
                                    false,
                                    true,
                                ),
                                syn::Fields::Unit => (Vec::new(), Vec::new(), false, false),
                            };
                            self.found.push(VariantCandidate {
                                attrs: v.attrs.clone(),
                                tuple_tys,
                                struct_fields,
                                discriminant: v.discriminant.as_ref().map(|(_, e)| e),
                                is_tuple,
                                is_struct,
                                container_start: e.span().start().line as u32,
                                container_end: e.span().end().line as u32,
                                anchor_line: v.ident.span().start().line as u32,
                                attr_start_line: v.span().start().line as u32,
                            });
                        }
                    }
                }
            }
            visit::visit_item(self, it);
        }
    }
    let mut c = Collector {
        enum_name,
        variant,
        found: Vec::new(),
    };
    for it in items {
        c.visit_item(it);
    }
    out.extend(c.found);
}

fn in_scope(
    c_start: u32,
    c_end: u32,
    anchor: u32,
    scope: Scope,
    old_start: u32,
    old_len: u32,
) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c_start >= old_start && c_end <= end.max(old_start),
        Scope::Anchor => anchor >= old_start && anchor <= end.max(old_start),
        Scope::Overlap => c_start <= end.max(old_start) && c_end >= old_start,
        Scope::Exact => old_len > 0 && c_start == old_start && c_end == end,
        Scope::File => true,
    }
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_repair(StructuredError::internal_repair())
}

fn pattern_err(rule: &str, msg: String) -> StructuredError {
    StructuredError::new(ErrorCode::PatternConflict, msg)
        .with_rule(rule)
        .with_repair(StructuredError::internal_repair())
}

/// Apply a field-type evolution hunk. Only the field type span is
/// replaced; the container, visibility, name, and attributes are
/// preserved. Vis must render equal to the authored prefix (a different
/// vis skips the site, never rewrites under a lie). WI-4 slice 2b:
/// the field type may name `# symbol`s (whole-path, nested, or
/// single-leading-prefix, same law as General types) — verified through
/// the crate graph (`symbol_graph`) or matched textually under
/// `# fallback=syntax`, with the plus side emitting the seed text.
pub fn apply_field_type_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
    symbol_graph: Option<(&crate::CrateGraph, &str)>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (struct_name, field, vis, minus_sub, plus_raw, placeholders, minus_attrs) = match &compiled
        .kind
    {
        PatternKind::FieldTypeEvolution {
            struct_name,
            field,
            vis,
            minus_sub,
            plus_raw,
            placeholders,
            minus_attrs,
            ..
        } => (
            struct_name.clone(),
            field.clone(),
            vis.clone(),
            minus_sub.clone(),
            plus_raw.clone(),
            placeholders.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected field-type evolution pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-field-type-template",
                "write a field-type evolution pair under its `struct` header (`  struct S {` context with `- x: u32` / `+ x: u64`; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    let file = "";
    let pat_ty: syn::Type = syn::parse_str(&minus_sub)
        .map_err(|e| pattern_err(&rule, format!("invalid field type pattern: {e}")))?;
    // WI-4 slice 2b: symbol posture (SPEC 19.3), mirroring the General
    // driver — seeds pre-bind into every candidate context (the plus
    // side emits the concrete seed), bare symbols verify their
    // minus-side seeds here, per-site checks run in `matcher_symbol`
    // through each candidate `MatchCx`.
    let symbols = compiled.table.symbols.clone();
    let fallback_syntax = hunk.fallback_syntax();
    let seed_canons = crate::matcher_symbol::verify_symbol_seeds(
        &symbols,
        &minus_sub,
        fallback_syntax,
        symbol_graph,
        &rule,
    )?;
    let mut candidates = Vec::new();
    collect_fields(&parsed.items, &struct_name, &field, &mut candidates);
    candidates.sort_by_key(|c| (c.container_start, c.anchor_line));
    let reverse: HashMap<String, String> = placeholders
        .iter()
        .map(|(n, ph)| (ph.clone(), n.clone()))
        .collect();
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    let mut all_bindings = Vec::new();
    for c in &candidates {
        if !in_scope(
            c.container_start,
            c.container_end,
            c.anchor_line,
            hunk.scope(),
            old_start,
            old_len,
        ) {
            continue;
        }
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Preserve
            && !minus_attrs.is_empty()
            && !attrs_equal(&minus_attrs, &c.attrs)
        {
            continue;
        }
        check_attrs(&c.attrs, hunk.attrs_policy(), &minus_attrs, &rule, file)?;
        // Subset semantics: vis must agree with the authored prefix.
        if norm_vis_of(&c.vis_norm) != norm_vis_of(&vis) {
            continue;
        }
        let mut cx = crate::matcher_symbol::symbol_cx(
            original,
            index,
            reverse.clone(),
            &symbols,
            fallback_syntax,
            symbol_graph,
            &seed_canons,
        );
        if !crate::matcher_types::match_type(&pat_ty, c.ty, &mut cx) {
            continue;
        }
        let subst = crate::matcher_expr_subst::substitute_call_plus(
            &plus_raw,
            &cx.bindings,
            &[],
            compiled.table.sigiled,
        );
        syn::parse_str::<syn::Type>(&subst).map_err(|e| {
            pattern_err(&rule, format!("substituted field type does not parse: {e}"))
        })?;
        let (a, b) = span_bytes(original, index, c.ty.span())
            .ok_or_else(|| range_err(&rule, c.anchor_line))?;
        edits.push(TextEdit::new(a, b, subst).with_label("type"));
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Strip && !c.attrs.is_empty() {
            let start = index
                .line_start(c.attr_start_line)
                .ok_or_else(|| range_err(&rule, c.attr_start_line))?;
            let end = index
                .line_start(c.anchor_line)
                .ok_or_else(|| range_err(&rule, c.anchor_line))?;
            if end > start {
                edits.push(TextEdit::new(start, end, String::new()));
            }
        }
        all_bindings.push(cx.bindings);
        matched += 1;
        if hunk.allow() == Allow::One && matched > 1 {
            break;
        }
    }
    Ok((matched, all_bindings))
}

fn norm_vis_of(s: &str) -> String {
    s.split_whitespace().collect()
}

/// Apply a variant payload/discriminant evolution hunk. Only the evolved
/// region spans are replaced; the variant name and preserved regions stay
/// byte-for-byte.
pub fn apply_variant_payload_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (enum_name, variant, payload, minus_attrs) = match &compiled.kind {
        PatternKind::VariantPayloadEvolution {
            enum_name,
            variant,
            payload,
            minus_attrs,
            ..
        } => (
            enum_name.clone(),
            variant.clone(),
            payload.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected variant-payload evolution pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-variant-payload-template",
                "write a variant-payload evolution pair under its `enum` def pair (one payload slot evolves, e.g. `- A(u32)` / `+ A(u64)`; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    let file = "";
    let (minus_subs, plus_raws, disc_minus, disc_plus, placeholders, mode) = match &payload {
        VariantPayload::Tuple {
            minus_subs,
            plus_raws,
            placeholders,
        } => (
            minus_subs.clone(),
            plus_raws.clone(),
            None,
            None,
            placeholders.clone(),
            "tuple",
        ),
        VariantPayload::Struct {
            minus_subs,
            plus_raws,
            placeholders: sph,
            ..
        } => (
            minus_subs.clone(),
            plus_raws.clone(),
            None,
            None,
            sph.clone(),
            "struct",
        ),
        VariantPayload::Discriminant {
            minus_sub,
            plus_raw,
            placeholders,
        } => (
            vec![minus_sub.clone()],
            vec![plus_raw.clone()],
            Some(minus_sub.clone()),
            Some(plus_raw.clone()),
            placeholders.clone(),
            "disc",
        ),
    };
    let mut candidates = Vec::new();
    collect_variants(&parsed.items, &enum_name, &variant, &mut candidates);
    candidates.sort_by_key(|c| (c.container_start, c.anchor_line));
    let reverse: HashMap<String, String> = placeholders
        .iter()
        .map(|(n, ph)| (ph.clone(), n.clone()))
        .collect();
    // Parse minus templates once per hunk.
    let pat_tys: Vec<syn::Type> = if mode == "disc" {
        Vec::new()
    } else {
        minus_subs
            .iter()
            .map(|s| {
                syn::parse_str::<syn::Type>(s)
                    .map_err(|e| pattern_err(&rule, format!("invalid payload type pattern: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let pat_disc: Option<syn::Expr> = if mode == "disc" {
        Some(
            syn::parse_str::<syn::Expr>(disc_minus.as_ref().expect("disc"))
                .map_err(|e| pattern_err(&rule, format!("invalid discriminant pattern: {e}")))?,
        )
    } else {
        None
    };
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    let mut all_bindings = Vec::new();
    for c in &candidates {
        if !in_scope(
            c.container_start,
            c.container_end,
            c.anchor_line,
            hunk.scope(),
            old_start,
            old_len,
        ) {
            continue;
        }
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Preserve
            && !minus_attrs.is_empty()
            && !attrs_equal(&minus_attrs, &c.attrs)
        {
            continue;
        }
        check_attrs(&c.attrs, hunk.attrs_policy(), &minus_attrs, &rule, file)?;
        let mut cx =
            crate::matcher_general::MatchCx::with_reverse(original, index, reverse.clone());
        // Structural match with one shared binding set (SPEC 15.7 holds
        // across positions).
        let ok = match mode {
            "tuple" => {
                if !c.is_tuple || c.tuple_tys.len() != pat_tys.len() {
                    false
                } else {
                    pat_tys
                        .iter()
                        .zip(c.tuple_tys.iter())
                        .all(|(p, t)| crate::matcher_types::match_type(p, t, &mut cx))
                }
            }
            "struct" => {
                if !c.is_struct {
                    false
                } else {
                    let want: Vec<String> = match &payload {
                        VariantPayload::Struct { fields, .. } => fields.clone(),
                        _ => Vec::new(),
                    };
                    let have: Vec<String> =
                        c.struct_fields.iter().map(|(n, _)| n.clone()).collect();
                    if want != have {
                        false
                    } else {
                        pat_tys
                            .iter()
                            .zip(c.struct_fields.iter().map(|(_, t)| *t))
                            .all(|(p, t)| crate::matcher_types::match_type(p, t, &mut cx))
                    }
                }
            }
            _ => match (&pat_disc, c.discriminant) {
                (Some(p), Some(t)) => crate::matcher_general::match_expr(p, t, &mut cx),
                _ => false,
            },
        };
        if !ok {
            continue;
        }
        // Emit one span edit per evolved position.
        match mode {
            "tuple" => {
                for (i, target) in c.tuple_tys.iter().enumerate() {
                    let subst = crate::matcher_expr_subst::substitute_call_plus(
                        &plus_raws[i],
                        &cx.bindings,
                        &[],
                        compiled.table.sigiled,
                    );
                    syn::parse_str::<syn::Type>(&subst).map_err(|e| {
                        pattern_err(
                            &rule,
                            format!("substituted payload type does not parse: {e}"),
                        )
                    })?;
                    let (a, b) = span_bytes(original, index, target.span())
                        .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                    edits.push(TextEdit::new(a, b, subst).with_label("payload"));
                }
            }
            "struct" => {
                for (i, (_, target)) in c.struct_fields.iter().enumerate() {
                    let subst = crate::matcher_expr_subst::substitute_call_plus(
                        &plus_raws[i],
                        &cx.bindings,
                        &[],
                        compiled.table.sigiled,
                    );
                    syn::parse_str::<syn::Type>(&subst).map_err(|e| {
                        pattern_err(
                            &rule,
                            format!("substituted payload type does not parse: {e}"),
                        )
                    })?;
                    let (a, b) = span_bytes(original, index, target.span())
                        .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                    edits.push(TextEdit::new(a, b, subst).with_label("payload"));
                }
            }
            _ => {
                let target = c.discriminant.expect("discriminant");
                let subst = crate::matcher_expr_subst::substitute_call_plus(
                    disc_plus.as_ref().expect("disc"),
                    &cx.bindings,
                    &[],
                    compiled.table.sigiled,
                );
                syn::parse_str::<syn::Expr>(&subst).map_err(|e| {
                    pattern_err(
                        &rule,
                        format!("substituted discriminant does not parse: {e}"),
                    )
                })?;
                let (a, b) = span_bytes(original, index, target.span())
                    .ok_or_else(|| range_err(&rule, c.anchor_line))?;
                edits.push(TextEdit::new(a, b, subst).with_label("discriminant"));
            }
        }
        if hunk.attrs_policy() == rsmpl_patch::AttrsPolicy::Strip && !c.attrs.is_empty() {
            let start = index
                .line_start(c.attr_start_line)
                .ok_or_else(|| range_err(&rule, c.attr_start_line))?;
            let end = index
                .line_start(c.anchor_line)
                .ok_or_else(|| range_err(&rule, c.anchor_line))?;
            if end > start {
                edits.push(TextEdit::new(start, end, String::new()));
            }
        }
        all_bindings.push(cx.bindings);
        matched += 1;
        if hunk.allow() == Allow::One && matched > 1 {
            break;
        }
    }
    Ok((matched, all_bindings))
}

/// Member-line indent for point insertions (WI-12b slice 1): the
/// indent of the first non-empty member line inside the container, or
/// the closing-brace indent plus four spaces when the container is
/// empty. Keeps inserted lines byte-identical to the file's own
/// style instead of column 0.
fn member_indent(original: &str, start: u32, end: u32) -> String {
    let lines: Vec<&str> = original.lines().collect();
    let n = lines.len();
    // `start`/`end` are 1-based syn lines; the span may begin at doc
    // comments or attributes above the header, so the member region
    // starts after the opening `{`, not at `start`.
    let s = (start as usize).saturating_sub(1).min(n);
    let e = (end as usize).min(n);
    let mut body_start = e;
    for (i, line) in lines.iter().enumerate().take(e).skip(s) {
        if line.contains('{') {
            body_start = i + 1;
            break;
        }
    }
    // First non-blank, non-closing line after the brace carries the
    // member indent (attributes share it).
    for line in &lines[body_start..e.saturating_sub(1)] {
        let t = line.trim();
        if t.is_empty() || t == "}" || t == "}," {
            continue;
        }
        return line.chars().take_while(|c| c.is_whitespace()).collect();
    }
    // Empty container: closing-brace indent plus one level.
    if end >= 1 && (end as usize) <= n {
        let close = lines[end as usize - 1];
        let base: String = close.chars().take_while(|c| c.is_whitespace()).collect();
        return format!("{base}    ");
    }
    "    ".to_string()
}

/// Apply a field-addition hunk (WI-12b slice 1): point-insert one
/// `name: Ty` line (plus its attr lines) inside `struct S`.
///
/// Ranged hunks insert after `header.old_start` (the `-N,0` point);
/// the point must lie strictly inside the container
/// (`container_start <= N < container_end`). File-scope hunks
/// (`# sweep`) insert before the closing brace of every matching
/// container that lacks the field. An already-present field skips the
/// site (0 matches); the inserted line gains a trailing comma when the
/// authored text omits one.
pub fn apply_field_addition_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (struct_name, field, plus_raw, plus_attrs) = match &compiled.kind {
        PatternKind::FieldAddition {
            struct_name,
            field,
            plus_raw,
            plus_attrs,
            ..
        } => (
            struct_name.clone(),
            field.clone(),
            plus_raw.clone(),
            plus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected field-addition pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-field-add-template",
                "write a field-addition hunk (`struct S {` context plus one `+ field: Ty` line; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    // All `struct S` containers with their spans + field-name sets.
    struct StructProbe {
        start: u32,
        end: u32,
        fields: Vec<String>,
    }
    struct Probe<'ast> {
        name: &'ast str,
        out: Vec<StructProbe>,
    }
    impl<'ast> Visit<'ast> for Probe<'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Struct(s) = it {
                if s.ident == self.name {
                    let fields = match &s.fields {
                        syn::Fields::Named(f) => f
                            .named
                            .iter()
                            .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                            .collect(),
                        _ => Vec::new(),
                    };
                    self.out.push(StructProbe {
                        start: s.span().start().line as u32,
                        end: s.span().end().line as u32,
                        fields,
                    });
                }
            }
            visit::visit_item(self, it);
        }
    }
    let mut probe = Probe {
        name: &struct_name,
        out: Vec::new(),
    };
    for it in &parsed.items {
        probe.visit_item(it);
    }
    let mut member = plus_raw.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    let render = |indent: &str| -> String {
        let mut t = String::new();
        for a in &plus_attrs {
            t.push_str(indent);
            t.push_str(a.trim());
            t.push('\n');
        }
        t.push_str(indent);
        t.push_str(&member);
        t.push('\n');
        t
    };
    let mut matched = 0usize;
    if hunk.scope() == Scope::File {
        let mut targets: Vec<(u32, u32)> = Vec::new();
        for c in &probe.out {
            if c.fields.iter().any(|f| f == &field) {
                continue;
            }
            // Insert before the closing brace line.
            targets.push((c.start, c.end));
        }
        targets.sort_unstable();
        targets.dedup();
        for (start, end) in targets {
            let at = index.line_start(end).ok_or_else(|| range_err(&rule, end))?;
            let indent = member_indent(original, start, end);
            edits.push(TextEdit::new(at, at, render(&indent)).with_label("addition"));
            matched += 1;
            if hunk.allow() == Allow::One && matched > 1 {
                break;
            }
        }
    } else {
        let point = hunk.header.old_start;
        let mut target: Option<(u32, u32, u32)> = None;
        for c in &probe.out {
            if c.fields.iter().any(|f| f == &field) {
                continue;
            }
            if c.start <= point && point < c.end {
                target = Some((c.start, c.end, point + 1));
                break;
            }
        }
        let Some((cstart, cend, at_line)) = target else {
            return Ok((0, Vec::new()));
        };
        let at = index
            .line_start(at_line)
            .ok_or_else(|| range_err(&rule, at_line))?;
        let indent = member_indent(original, cstart, cend);
        edits.push(TextEdit::new(at, at, render(&indent)).with_label("addition"));
        matched = 1;
    }
    let empties = vec![HashMap::new(); matched];
    Ok((matched, empties))
}

/// Apply a variant-addition hunk (WI-12b slice 1): point-insert one
/// variant line (plus its attr lines) inside `enum E`. Same
/// range/file-scope discipline as field addition.
pub fn apply_variant_addition_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (enum_name, variant, plus_raw, plus_attrs) = match &compiled.kind {
        PatternKind::VariantAddition {
            enum_name,
            variant,
            plus_raw,
            plus_attrs,
            ..
        } => (
            enum_name.clone(),
            variant.clone(),
            plus_raw.clone(),
            plus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected variant-addition pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-variant-add-template",
                "write a variant-addition hunk (`enum E {` context plus one `+ Variant` line; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    struct EnumProbe {
        start: u32,
        end: u32,
        variants: Vec<String>,
    }
    struct Probe<'ast> {
        name: &'ast str,
        out: Vec<EnumProbe>,
    }
    impl<'ast> Visit<'ast> for Probe<'ast> {
        fn visit_item(&mut self, it: &'ast syn::Item) {
            if let syn::Item::Enum(e) = it {
                if e.ident == self.name {
                    self.out.push(EnumProbe {
                        start: e.span().start().line as u32,
                        end: e.span().end().line as u32,
                        variants: e.variants.iter().map(|v| v.ident.to_string()).collect(),
                    });
                }
            }
            visit::visit_item(self, it);
        }
    }
    let mut probe = Probe {
        name: &enum_name,
        out: Vec::new(),
    };
    for it in &parsed.items {
        probe.visit_item(it);
    }
    let mut member = plus_raw.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    let render = |indent: &str| -> String {
        let mut t = String::new();
        for a in &plus_attrs {
            t.push_str(indent);
            t.push_str(a.trim());
            t.push('\n');
        }
        t.push_str(indent);
        t.push_str(&member);
        t.push('\n');
        t
    };
    let mut matched = 0usize;
    if hunk.scope() == Scope::File {
        let mut targets: Vec<(u32, u32)> = Vec::new();
        for c in &probe.out {
            if c.variants.iter().any(|v| v == &variant) {
                continue;
            }
            targets.push((c.start, c.end));
        }
        targets.sort_unstable();
        targets.dedup();
        for (start, end) in targets {
            let at = index.line_start(end).ok_or_else(|| range_err(&rule, end))?;
            let indent = member_indent(original, start, end);
            edits.push(TextEdit::new(at, at, render(&indent)).with_label("addition"));
            matched += 1;
            if hunk.allow() == Allow::One && matched > 1 {
                break;
            }
        }
    } else {
        let point = hunk.header.old_start;
        let mut target: Option<(u32, u32, u32)> = None;
        for c in &probe.out {
            if c.variants.iter().any(|v| v == &variant) {
                continue;
            }
            if c.start <= point && point < c.end {
                target = Some((c.start, c.end, point + 1));
                break;
            }
        }
        let Some((cstart, cend, at_line)) = target else {
            return Ok((0, Vec::new()));
        };
        let at = index
            .line_start(at_line)
            .ok_or_else(|| range_err(&rule, at_line))?;
        let indent = member_indent(original, cstart, cend);
        edits.push(TextEdit::new(at, at, render(&indent)).with_label("addition"));
        matched = 1;
    }
    let empties = vec![HashMap::new(); matched];
    Ok((matched, empties))
}

/// Apply a match-arm addition hunk (WI-12d): point-insert one
/// concrete `E::C => body,` arm line (plus its attr lines) inside a
/// `match <scrut> {`.
///
/// Ranged-only in v1: the point (`@@ -N,1 @@` old_start) must lie
/// strictly inside the innermost `match` lacking an arm for the
/// variant (`container_start <= N < container_end`), and that match's
/// scrutinee must equal the context scrutinee token-for-token. An
/// already-present arm (qualified or bare, guarded or not —
/// last-segment naming) skips the site (0 matches). Sweep/file-scope
/// is rejected loud: a sweep would plant the arm into matches on
/// other enums. Insertion lands before the closing brace; a brace
/// sharing its line with other code vetoes (0 matches) rather than
/// splicing mid-line.
pub fn apply_match_arm_addition_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (variant, arm_raw, scrutinee, plus_attrs) = match &compiled.kind {
        PatternKind::ArmAddition {
            variant,
            arm_raw,
            scrutinee,
            plus_attrs,
            ..
        } => (
            variant.clone(),
            arm_raw.clone(),
            scrutinee.clone(),
            plus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected match-arm addition pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-arm-dispatch-template",
                "write a match-arm addition hunk (`match <scrut> {` context plus one `+ <arm>,` line, ranged-only; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    if hunk.scope() == Scope::File {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "match-arm insertion is ranged-only in v1 (a sweep would plant the arm into matches on other enums); write `@@ -N,1 @@` citing the line before the insertion point, or use an ordinary hunk".to_string(),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "arm-addition",
            "write `@@ -N,1 @@` with a `match <scrut> {` context line and one `+ <arm>,` line, or use an ordinary hunk",
        )));
    }
    struct MatchProbe {
        start: u32,
        end: u32,
        scrutinee: String,
        arm_names: Vec<String>,
    }
    struct Probe {
        out: Vec<MatchProbe>,
    }
    impl<'ast> Visit<'ast> for Probe {
        fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
            let mut arm_names = Vec::new();
            for arm in &node.arms {
                // A lone ident parses as `Pat::Ident` (binding position)
                // — a bare uppercase arm is the glob-import spelling of
                // a unit variant and counts as present.
                if let syn::Pat::Ident(pi) = &arm.pat {
                    if pi.by_ref.is_none() && pi.mutability.is_none() && pi.subpat.is_none() {
                        arm_names.push(pi.ident.to_string());
                        continue;
                    }
                }
                let path = match &arm.pat {
                    syn::Pat::Path(pp) if pp.qself.is_none() => Some(&pp.path),
                    syn::Pat::TupleStruct(ps) => Some(&ps.path),
                    syn::Pat::Struct(ps) => Some(&ps.path),
                    _ => None,
                };
                if let Some(p) = path {
                    if let Some(last) = p.segments.last() {
                        arm_names.push(last.ident.to_string());
                    }
                }
            }
            self.out.push(MatchProbe {
                start: node.span().start().line as u32,
                end: node.span().end().line as u32,
                scrutinee: {
                    let e = &*node.expr;
                    quote::quote!(#e)
                        .to_string()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                },
                arm_names,
            });
            visit::visit_expr_match(self, node);
        }
    }
    // Expression-position matches (`let x = match …`) are `Expr`, not
    // `Item`: walk every expression via a file-level visitor that
    // routes matches through the probe (nested matches recurse via
    // `visit_expr_match` above).
    struct FileWalk {
        probe: Probe,
    }
    impl<'ast> Visit<'ast> for FileWalk {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Match(m) = node {
                self.probe.visit_expr_match(m);
            } else {
                visit::visit_expr(self, node);
            }
        }
    }
    let mut walk = FileWalk {
        probe: Probe { out: Vec::new() },
    };
    for it in &parsed.items {
        walk.visit_item(it);
    }
    let point = hunk.header.old_start;
    // The point names one match: the innermost spanning it. It must
    // lack the variant (an inner dup vetoes even when an outer match
    // lacks it — the point pins the inner scope) and its scrutinee
    // must agree exactly.
    let mut inner: Option<(u32, u32)> = None;
    let mut best_start = 0u32;
    for c in &walk.probe.out {
        if c.start <= point && point < c.end && c.start >= best_start {
            best_start = c.start;
            inner = Some((c.start, c.end));
        }
    }
    let Some((cstart, cend)) = inner else {
        return Ok((0, Vec::new()));
    };
    let found_match = walk
        .probe
        .out
        .iter()
        .find(|c| c.start == cstart && c.end == cend);
    let Some(found_match) = found_match else {
        return Ok((0, Vec::new()));
    };
    if found_match.arm_names.iter().any(|a| a == &variant) {
        return Ok((0, Vec::new()));
    }
    if found_match.scrutinee != scrutinee {
        return Ok((0, Vec::new()));
    }
    // The closing brace must own its line; anything else vetoes
    // rather than splicing mid-line.
    let close = original.lines().nth(cend as usize - 1).unwrap_or("");
    if !matches!(close.trim(), "}" | "}," | "};") {
        return Ok((0, Vec::new()));
    }
    let mut member = arm_raw.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    let render = |indent: &str| -> String {
        let mut t = String::new();
        for a in &plus_attrs {
            t.push_str(indent);
            t.push_str(a.trim());
            t.push('\n');
        }
        t.push_str(indent);
        t.push_str(&member);
        t.push('\n');
        t
    };
    let at = index
        .line_start(cend)
        .ok_or_else(|| range_err(&rule, cend))?;
    let indent = member_indent(original, cstart, cend);
    edits.push(TextEdit::new(at, at, render(&indent)).with_label("addition"));
    Ok((1, vec![HashMap::new()]))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::RuleSemantics;

    pub(super) fn apply_report(
        patch_text: &str,
        file_text: &str,
    ) -> Result<(String, crate::ApplyReport), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        let report = crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok((files["src/lib.rs"].clone(), report))
    }

    pub(super) fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        apply_report(patch_text, file_text).map(|(s, _)| s)
    }

    const FIELD_CTX: &str = "  struct S {\n";

    fn field_patch(minus: &str, plus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evo\n{FIELD_CTX}- {minus}\n+ {plus}\n"
        )
    }

    const VAR_CTX: &str = "  enum E {\n";

    fn var_patch(minus: &str, plus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evo\n{VAR_CTX}- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn field_type_concrete_fire_verifies_postcondition() {
        let patch = field_patch("old: Vec<u8>", "old: Box<[u8]>");
        let (out, report) = apply_report(&patch, "struct S {\n    old: Vec<u8>,\n}\n").unwrap();
        assert!(out.contains("old: Box<[u8]>"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn field_type_hole_fire_verifies_postcondition() {
        let patch = field_patch("old: Vec<T>", "old: Box<[T]>");
        let (out, report) = apply_report(&patch, "struct S {\n    old: Vec<u8>,\n}\n").unwrap();
        assert!(out.contains("old: Box<[u8]>"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
        assert_eq!(
            report.rules[0].sites[0].bindings,
            vec![("T".to_string(), "u8".to_string())]
        );
    }

    #[test]
    fn field_type_repeat_equality_vetoes() {
        let patch = field_patch("old: (T, T)", "old: (T, u8)");
        let err = apply(&patch, "struct S {\n    old: (u8, bool),\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn field_type_name_change_fails_fast() {
        // Same-type different-name is the rename spelling (claimed by the
        // rename classifier via fallthrough), not an evolution error.
        let patch = field_patch("old: Vec<u8>", "new: Vec<u8>");
        let out = apply(&patch, "struct S {\n    old: Vec<u8>,\n}\n").unwrap();
        assert!(out.contains("new: Vec<u8>"), "got: {out:?}");
        // Both differ never combines rename + evolution.
        let patch = field_patch("old: Vec<u8>", "new: Box<[u8]>");
        let err = apply(&patch, "struct S {\n    old: Vec<u8>,\n}\n").unwrap_err();
        assert!(err.contains("never combine"), "got: {err}");
    }

    #[test]
    fn field_type_needs_struct_context() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=evo\n- old: Vec<u8>\n+ old: Box<[u8]>\n";
        let err = apply(patch, "struct S {\n    old: Vec<u8>,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("struct S"), "got: {err}");
    }

    #[test]
    fn field_type_vis_mismatch_rejected() {
        let patch = field_patch("pub old: Vec<u8>", "old: Box<[u8]>");
        let err = apply(&patch, "struct S {\n    pub old: Vec<u8>,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn field_type_other_container_untouched() {
        let patch = field_patch("old: Vec<u8>", "old: Box<[u8]>");
        let src = "struct S {\n    x: u8,\n}\nstruct T {\n    old: Vec<u8>,\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    // WI-4 slice 2b: `# symbol` in field-type evolution (whole-path,
    // nested, and prefix types verify through the crate graph or match
    // textually under `# fallback=syntax`; the plus side emits the seed).
    fn symbol_field_patch(minus: &str, plus: &str, fallback: bool) -> String {
        symbol_field_patch_seeds(minus, plus, fallback, "crate::old::Old", "crate::New")
    }

    fn symbol_field_patch_seeds(
        minus: &str,
        plus: &str,
        fallback: bool,
        old_seed: &str,
        new_seed: &str,
    ) -> String {
        let fb = if fallback { "# fallback=syntax\n" } else { "" };
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=evo\n# scope=file\n{fb}# symbol Old={old_seed}\n# symbol New={new_seed}\n  struct S {{\n- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn field_type_symbol_fallback_fires() {
        let patch = symbol_field_patch_seeds("x: Old", "x: New", true, "crate::Old", "crate::New");
        let (out, report) = apply_report(&patch, "struct S {\n    x: crate::Old,\n}\n").unwrap();
        assert!(out.contains("x: crate::New,"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn field_type_symbol_resolved_short_spelling_fires() {
        let patch = symbol_field_patch("x: Old", "x: New", false);
        let src = "pub mod old {\n    pub struct Old;\n}\npub struct New;\nuse crate::old::Old;\nstruct S {\n    x: Old,\n}\n";
        let (out, report) = apply_report(&patch, src).unwrap();
        assert!(out.contains("x: crate::New,"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn field_type_symbol_nested_vec_fires() {
        let patch = symbol_field_patch("x: Vec<Old>", "x: Vec<New>", false);
        let src = "pub mod old {\n    pub struct Old;\n}\npub struct New;\nuse crate::old::Old;\nstruct S {\n    x: Vec<Old>,\n}\n";
        let (out, _) = apply_report(&patch, src).unwrap();
        assert!(out.contains("x: Vec<crate::New>,"), "got: {out:?}");
    }

    #[test]
    fn field_type_symbol_vendored_twin_skipped() {
        // Same text, different definition: never a match (D2 honesty).
        let patch = symbol_field_patch("x: Old", "x: New", false);
        let src = "pub mod old {\n    pub struct Old;\n}\npub mod vendored {\n    pub struct Old;\n}\npub struct New;\nuse crate::vendored::Old;\nstruct S {\n    x: Old,\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn field_type_symbol_unresolvable_seed_fails() {
        let patch = symbol_field_patch("x: Old", "x: New", false).replace(
            "# symbol Old=crate::old::Old",
            "# symbol Old=crate::missing",
        );
        let err = apply(&patch, "struct S {\n    x: crate::missing,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
    }

    #[test]
    fn tuple_payload_concrete_fire_verifies_postcondition() {
        let patch = var_patch("Old(Vec<u8>)", "Old(Box<[u8]>)");
        let (out, report) = apply_report(&patch, "enum E {\n    Old(Vec<u8>),\n}\n").unwrap();
        assert!(out.contains("Old(Box<[u8]>)"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn tuple_payload_hole_fire() {
        let patch = var_patch("Old(Vec<T>)", "Old(Box<[T]>)");
        let (out, _) = apply_report(&patch, "enum E {\n    Old(Vec<u8>),\n}\n").unwrap();
        assert!(out.contains("Old(Box<[u8]>)"), "got: {out:?}");
    }

    #[test]
    fn tuple_payload_arity_mismatch_rejected() {
        let patch = var_patch("Old(Vec<T>)", "Old(Box<[T]>, u8)");
        let err = apply(&patch, "enum E {\n    Old(Vec<u8>),\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("arity"), "got: {err}");
    }

    #[test]
    fn struct_payload_fire_verifies_postcondition() {
        let patch = var_patch("Old { f: Vec<T> }", "Old { f: Box<[T]> }");
        let (out, report) = apply_report(&patch, "enum E {\n    Old { f: Vec<u8> },\n}\n").unwrap();
        assert!(out.contains("Old { f: Box<[u8]> }"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn struct_payload_field_mismatch_rejected() {
        let patch = var_patch("Old { f: Vec<T> }", "Old { g: Box<[T]> }");
        let err = apply(&patch, "enum E {\n    Old { f: Vec<u8> },\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn discriminant_fire_verifies_postcondition() {
        let patch = var_patch("Old = E", "Old = E + 1");
        let (out, report) = apply_report(&patch, "enum E {\n    Old = 1,\n}\n").unwrap();
        assert!(out.contains("Old = 1 + 1"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
        assert_eq!(
            report.rules[0].sites[0].bindings,
            vec![("E".to_string(), "1".to_string())]
        );
    }

    #[test]
    fn discriminant_without_header_is_not_member_evolution() {
        // No `enum E {` header: the pair falls through to the
        // general-expression classifier (assignment reading) instead of
        // the member classifier — never the member missing-header error.
        // (The field-pair twin keeps the rename missing-header error via
        // fallthrough; see `field_type_needs_struct_context`.)
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=evo\n- Old = E\n+ Old = E + 1\n";
        let err = apply(patch, "enum E {\n    Old = 1,\n}\n").unwrap_err();
        assert!(
            !err.contains("needs an `enum E {` context"),
            "must not claim member evolution without a header, got: {err}"
        );
    }

    #[test]
    fn evolution_rejects_follow_uses() {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evo\n# follow=uses\n{FIELD_CTX}- old: Vec<u8>\n+ old: Box<[u8]>\n"
        );
        let err = apply(&patch, "struct S {\n    old: Vec<u8>,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("C17"), "got: {err}");
    }

    #[test]
    fn tuple_payload_other_enum_untouched() {
        let patch = var_patch("Old(Vec<u8>)", "Old(Box<[u8]>)");
        let src = "enum E {\n    Keep,\n}\nenum F {\n    Old(Vec<u8>),\n}\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    // WI-12b slice 1: member-addition authoring matrix.

    fn add_field_patch(plus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n{FIELD_CTX}+ {plus}\n"
        )
    }

    fn add_variant_patch(plus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n{VAR_CTX}+ {plus}\n"
        )
    }

    #[test]
    fn field_addition_fire_verifies_postcondition() {
        let patch = add_field_patch("b: bool,");
        let (out, report) = apply_report(&patch, "struct S {\n    a: u32,\n}\n").unwrap();
        assert!(out.contains("    b: bool,"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn field_addition_mid_point_indented() {
        // Insertion point inside the container (after line 2, before
        // the last member) still lands indented.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n  struct S {\n+ b: bool,\n";
        let (out, _) = apply_report(patch, "struct S {\n    a: u32,\n    c: u8,\n}\n").unwrap();
        assert!(out.contains("    b: bool,"), "got: {out:?}");
        assert!(out.contains("    a: u32,"), "got: {out:?}");
        assert!(out.contains("    c: u8,"), "got: {out:?}");
    }

    #[test]
    fn variant_addition_bare_fire_verifies_postcondition() {
        let patch = add_variant_patch("C,");
        let (out, report) = apply_report(&patch, "enum E {\n    A,\n}\n").unwrap();
        assert!(out.contains("    C,"), "got: {out:?}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn variant_addition_tuple_fire() {
        let patch = add_variant_patch("C(i32),");
        let (out, _) = apply_report(&patch, "enum E {\n    A,\n}\n").unwrap();
        assert!(out.contains("    C(i32),"), "got: {out:?}");
    }

    #[test]
    fn variant_addition_struct_fire() {
        let patch = add_variant_patch("C { f: u32 },");
        let (out, _) = apply_report(&patch, "enum E {\n    A,\n}\n").unwrap();
        assert!(out.contains("    C { f: u32 },"), "got: {out:?}");
    }

    #[test]
    fn variant_addition_discriminant_fire() {
        let patch = add_variant_patch("C = 3,");
        let (out, _) = apply_report(&patch, "enum E {\n    A,\n}\n").unwrap();
        assert!(out.contains("    C = 3,"), "got: {out:?}");
    }

    #[test]
    fn addition_two_plus_lines_veto() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n  struct S {\n+ b: bool,\n+ c: u8,\n";
        let err = apply(patch, "struct S {\n    a: u32,\n}\n").unwrap_err();
        assert!(
            err.contains("RSMPL_PATTERN_CONFLICT") || err.contains("RSMPL_MATCH_CONFLICT"),
            "two-plus insertion must not claim addition, got: {err}"
        );
    }

    #[test]
    fn addition_missing_container_veto() {
        // No container header: the plus line falls through to a plain
        // insertion (not the member-addition shape), so it lands
        // unindented at the insertion point instead of container-indented.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1 @@ rule=add\n+ b: bool,\n";
        let (out, _) = apply_report(patch, "struct S {\n    a: u32,\n}\n").unwrap();
        assert!(
            !out.contains("    b: bool,"),
            "headerless plus must not take the addition path, got: {out:?}"
        );
    }

    #[test]
    fn addition_rejects_follow_uses() {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n# follow=uses\n{FIELD_CTX}+ b: bool,\n"
        );
        let err = apply(&patch, "struct S {\n    a: u32,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
        assert!(err.contains("C17"), "got: {err}");
    }

    #[test]
    fn addition_duplicate_member_zero_matches() {
        let patch = add_field_patch("a: u32,");
        let err = apply(&patch, "struct S {\n    a: u32,\n}\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn addition_indent_ignores_attrs_above_container() {
        // History shape (a734ee0): doc + `#[derive]` lines sit above
        // the container, and the member indent must come from inside
        // the braces — not from the 0-indent attribute lines.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -5,0 +6 @@ rule=add\n  enum Scope {\n+ File,\n";
        let src = "/// Scope modes.\n#[derive(Debug, Clone, Copy)]\npub enum Scope {\n    #[default]\n    Contained,\n    Exact,\n}\n";
        let (out, _) = apply_report(patch, src).unwrap();
        assert!(out.contains("    File,"), "got: {out:?}");
        assert!(
            out.contains("    Contained,\n    File,\n    Exact,"),
            "got: {out:?}"
        );
    }
}

#[cfg(test)]
mod arm_addition_tests {
    use super::tests::{apply, apply_report};

    const ARM_PRE: &str =
        "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";

    fn arm_patch(point: u32, context: &str, plus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -{point},1 +{point},1 @@ rule=arm\n{context}\n+ {plus}\n"
        )
    }

    #[test]
    fn arm_appends_before_closing_brace() {
        let patch = arm_patch(6, "  match e {", "E::C => 3,");
        let (out, report) = apply_report(&patch, ARM_PRE).unwrap();
        assert!(
            out.contains("        E::A => 1,\n        E::C => 3,\n    }"),
            "got: {out:?}"
        );
        assert_eq!(report.rules[0].matches, 1);
        assert_eq!(
            report.rules[0].semantics,
            super::super::RuleSemantics::Postcondition
        );
    }

    #[test]
    fn arm_bare_fires() {
        let patch = arm_patch(6, "  match e {", "C => 3,");
        let (out, _) = apply_report(&patch, ARM_PRE).unwrap();
        assert!(out.contains("        C => 3,\n"), "got: {out:?}");
    }

    #[test]
    fn arm_dup_vetoes() {
        // The variant arm is already present (bare or qualified —
        // last-segment naming): 0 matches, loud conflict.
        for pre in [
            "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n        E::C => 9,\n    }\n}\n",
            "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n        C => 9,\n    }\n}\n",
        ] {
            let patch = arm_patch(6, "  match e {", "E::C => 3,");
            let err = apply(&patch, pre).unwrap_err();
            assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        }
    }

    #[test]
    fn arm_scrutinee_mismatch_vetoes() {
        // The point sits in `match e`, but the context claims
        // `match other` — no silent cross-match splice.
        let patch = arm_patch(6, "  match other {", "E::C => 3,");
        let err = apply(&patch, ARM_PRE).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn arm_lands_in_innermost_match() {
        let pre = "fn f(e: E) -> u32 {\n    match e {\n        E::A => match e {\n            E::A => 1,\n        },\n    }\n}\n";
        // Point 4 is the inner arm line; both matches share the
        // scrutinee, so only innermost-wins keeps this precise.
        let patch = arm_patch(4, "  match e {", "E::C => 2,");
        let (out, _) = apply_report(&patch, pre).unwrap();
        assert!(
            out.contains("            E::A => 1,\n            E::C => 2,\n"),
            "got: {out:?}"
        );
    }

    #[test]
    fn arm_inner_dup_blocks_outer_fallthrough() {
        // The inner match already covers C: the point names the inner
        // scope, so no edit lands in the outer match either.
        let pre = "fn f(e: E) -> u32 {\n    match e {\n        E::A => match e {\n            E::C => 1,\n        },\n    }\n}\n";
        let patch = arm_patch(4, "  match e {", "E::C => 2,");
        let err = apply(&patch, pre).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn arm_let_bound_match_fires() {
        // `match` as a `let` tail (`};` close) inserts before the brace.
        let pre =
            "fn f(e: E) -> u32 {\n    let x = match e {\n        E::A => 1,\n    };\n    x\n}\n";
        let patch = arm_patch(3, "  match e {", "E::C => 3,");
        let (out, _) = apply_report(&patch, pre).unwrap();
        assert!(
            out.contains("        E::A => 1,\n        E::C => 3,\n    };"),
            "got: {out:?}"
        );
    }

    #[test]
    fn arm_sweep_rejects_loud() {
        // File-scope would plant the arm into matches on other enums:
        // fail fast with a template, never a broad rewrite.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=arm\n# sweep\n  match e {\n+ E::C => 3,\n";
        let err = apply(patch, ARM_PRE).unwrap_err();
        assert!(err.contains("ranged-only"), "got: {err}");
        assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    }

    #[test]
    fn arm_outside_any_match_vetoes() {
        let patch = arm_patch(1, "  match e {", "E::C => 3,");
        let err = apply(&patch, ARM_PRE).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }
}

#[cfg(test)]
mod wi18_slice12_tests {
    // WI-18 slice 12: the five member-evo dispatch else-arms carry
    // site-specific `member-evo-*-template` repairs (defence-in-depth:
    // dispatch routes by kind, so the arms are forced with a wrong-kind
    // `Raw` overwrite, slice-7/10/11 idiom).
    use super::tests::apply_report;

    use crate::edits::TextEdit;

    const ARM_SRC: &str =
        "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";

    fn setup(
        patch_text: &str,
        src: &str,
    ) -> (
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        syn::File,
        crate::lines::LineIndex,
        String,
    ) {
        let mut parsed_patch = rsmpl_patch::parse_patch(patch_text).expect("test patch parses");
        let hunk = match parsed_patch.files.remove(0).hunks.remove(0) {
            rsmpl_patch::Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&hunk).expect("member patch compiles");
        let src = src.to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn repair_of(err: &rsmpl_patch::StructuredError) -> (String, String) {
        let repair = err
            .repair
            .as_ref()
            .expect("WI-18 slice 12: must carry repair");
        let code = repair
            .get("code")
            .and_then(|v| v.as_str())
            .expect("code str")
            .to_string();
        let write_instead = repair
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str")
            .to_string();
        assert_ne!(code, "pattern", "must beat the generic filler");
        assert!(
            !write_instead.contains(&err.message),
            "repair must not echo the reason, got: {write_instead}"
        );
        (code, write_instead)
    }

    fn raw(mut compiled: rsmpl_pattern::CompiledPattern) -> rsmpl_pattern::CompiledPattern {
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        compiled
    }

    #[test]
    fn wi18_slice12_field_type_dispatch_repair_is_specific() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evo\n  struct S {\n- old: Vec<u8>\n+ old: Box<[u8]>\n";
        let (hunk, compiled, parsed, index, src) =
            setup(patch, "struct S {\n    old: Vec<u8>,\n}\n");
        let compiled = raw(compiled);
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_field_type_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected field-type evolution pattern");
        let (code, w) = repair_of(&err);
        assert_eq!(code, "member-evo-field-type-template");
        assert!(w.contains("field-type evolution"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_variant_payload_dispatch_repair_is_specific() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evo\n  enum E {\n- Old(Vec<u8>)\n+ Old(Box<[u8]>)\n";
        let (hunk, compiled, parsed, index, src) = setup(patch, "enum E {\n    Old(Vec<u8>),\n}\n");
        let compiled = raw(compiled);
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_variant_payload_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected variant-payload evolution pattern");
        let (code, w) = repair_of(&err);
        assert_eq!(code, "member-evo-variant-payload-template");
        assert!(w.contains("variant-payload evolution"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_field_add_dispatch_repair_is_specific() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n  struct S {\n+ b: bool,\n";
        let (hunk, compiled, parsed, index, src) = setup(patch, "struct S {\n    a: u32,\n}\n");
        let compiled = raw(compiled);
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_field_addition_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected field-addition pattern");
        let (code, w) = repair_of(&err);
        assert_eq!(code, "member-evo-field-add-template");
        assert!(w.contains("field-addition"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_variant_add_dispatch_repair_is_specific() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +3 @@ rule=add\n  enum E {\n+ C,\n";
        let (hunk, compiled, parsed, index, src) = setup(patch, "enum E {\n    A,\n}\n");
        let compiled = raw(compiled);
        let mut edits = Vec::<TextEdit>::new();
        let err =
            super::apply_variant_addition_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected variant-addition pattern");
        let (code, w) = repair_of(&err);
        assert_eq!(code, "member-evo-variant-add-template");
        assert!(w.contains("variant-addition"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_arm_dispatch_repair_is_specific() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -6,1 +6,1 @@ rule=arm\n  match e {\n+ E::C => 3,\n";
        let (hunk, compiled, parsed, index, src) = setup(patch, ARM_SRC);
        let compiled = raw(compiled);
        let mut edits = Vec::<TextEdit>::new();
        let err = super::apply_match_arm_addition_hunk(
            &parsed, &src, &index, &hunk, &compiled, &mut edits,
        )
        .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected match-arm addition pattern");
        let (code, w) = repair_of(&err);
        assert_eq!(code, "member-evo-arm-dispatch-template");
        assert!(w.contains("match-arm addition"), "got: {w}");
    }

    #[test]
    fn wi18_slice12_valid_shapes_still_fire() {
        // Repair appends only: a valid field-type pair still fires through
        // the full apply (no `Raw` overwrite).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=evo\n  struct S {\n- old: Vec<u8>\n+ old: Box<[u8]>\n";
        let (out, _) =
            apply_report(patch, "struct S {\n    old: Vec<u8>,\n}\n").expect("valid shape fires");
        assert!(out.contains("old: Box<[u8]>"), "got: {out:?}");
    }
}
