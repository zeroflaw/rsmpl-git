//! Struct-field + enum-variant deletion (WI-21 slices 1–3).
//!
//! D5 sibling of `matcher_member_evo` (1656 lines, over the 1600 gate):
//! the `FieldDeletion` lowering (`struct S {` context + one `- field`
//! line, no plus member) and the `VariantDeletion` lowering (`enum E {`
//! context + one `- Variant` line, no plus member). Each mirrors its
//! addition hunk with the membership test inverted (the named member
//! must be *present*; the deleted text must normalize-equal the site,
//! else the site is skipped) and the edit a whole-line deletion via the
//! shared `matcher_fn_item::delete_lines` (blank-absorption included).
//!
//! ```diff
//!   struct S {
//! - old: u32
//! ```
//!
//! ```diff
//!   enum E {
//! - Old
//! ```
//!
//! Scope discipline mirrors addition: `# sweep` (or any file-scope
//! range) deletes the member in every containing container
//! (`allow=many`; `allow=one` stops after the first); a ranged hunk
//! deletes it in the single container spanning the range point.
//! Zero targets is `Ok(0)` (the planner reports the no-match); a
//! wrong-kind call is a loud `PatternConflict` with the
//! site-specific `member-evo-*-delete-template` repair (WI-18
//! ceiling holds — each site carries `with_repair`).

use std::collections::HashMap;

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use rsmpl_patch::{Allow, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_fn_item::delete_lines;

/// One deletion candidate: the named field's span inside its container.
struct DelCandidate {
    container_start: u32,
    container_end: u32,
    field_start: u32,
    field_end: u32,
    ty_norm: String,
}

struct Probe<'ast> {
    name: &'ast str,
    out: Vec<DelCandidate>,
}

fn norm_ty(ty: &syn::Type) -> String {
    quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

impl<'ast> Visit<'ast> for Probe<'ast> {
    fn visit_item(&mut self, it: &'ast syn::Item) {
        if let syn::Item::Struct(s) = it {
            if s.ident == self.name {
                if let syn::Fields::Named(named) = &s.fields {
                    for f in &named.named {
                        if let Some(ident) = f.ident.as_ref() {
                            let start = f.span().start().line as u32;
                            let mut end = f.span().end().line as u32;
                            if end < start {
                                end = start;
                            }
                            self.out.push(DelCandidate {
                                container_start: s.span().start().line as u32,
                                container_end: s.span().end().line as u32,
                                field_start: start,
                                field_end: end,
                                ty_norm: norm_ty(&f.ty),
                            });
                            let _ = ident;
                        }
                    }
                }
            }
        }
        visit::visit_item(self, it);
    }
}

/// Split a `name: Ty` field line on its top-level colon (no `::`
/// adjacency). Mirrors the classifier's field split; `None` when the
/// line is not a field shape.
fn split_field_colon(line: &str) -> Option<(String, String)> {
    let t = line
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_end_matches(';')
        .trim();
    let bytes = t.as_bytes();
    let mut dp = 0i32;
    let mut da = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] as char {
            '(' | '[' => dp += 1,
            ')' | ']' => dp -= 1,
            '<' => da += 1,
            '>' => {
                if da > 0 {
                    da -= 1;
                }
            }
            ':' if dp == 0 && da == 0 => {
                let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
                let next = bytes.get(i + 1).copied();
                if prev != Some(b':') && next != Some(b':') {
                    let left = t[..i].trim();
                    let right = t[i + 1..].trim();
                    if right.is_empty() {
                        return None;
                    }
                    let mut parts: Vec<&str> = left.split_whitespace().collect();
                    if parts.is_empty() {
                        return None;
                    }
                    let name = parts.pop()?.to_string();
                    return Some((name, right.to_string()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Apply a field-deletion hunk. See module docs.
pub fn apply_field_deletion_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (struct_name, field, minus_raw, minus_attrs) = match &compiled.kind {
        PatternKind::FieldDeletion {
            struct_name,
            field,
            minus_raw,
            minus_attrs,
            ..
        } => (
            struct_name.clone(),
            field.clone(),
            minus_raw.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected field-deletion pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-field-delete-template",
                "write a field-deletion hunk (`struct S {` context plus one `- field: Ty` line; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    // The authored minus type must normalize-equal the site type, so a
    // stale hunk never deletes a field whose type already changed.
    let want_ty = split_field_colon(&minus_raw)
        .and_then(|(_, ty)| syn::parse_str::<syn::Type>(&ty).ok())
        .map(|t| norm_ty(&t));
    let mut probe = Probe {
        name: &struct_name,
        out: Vec::new(),
    };
    for it in &parsed.items {
        probe.visit_item(it);
    }
    // Keep only the named field with a matching type (when the authored
    // type parses; an unparseable authored type never matches — loud
    // no-match, never a guess).
    let mut cands: Vec<&DelCandidate> = probe
        .out
        .iter()
        .filter(|c| {
            let (_, site_name) = match field_line_name(original, index, c.field_start) {
                Some(v) => v,
                None => return false,
            };
            if site_name != field {
                return false;
            }
            match &want_ty {
                Some(w) => &c.ty_norm == w,
                None => false,
            }
        })
        .collect();
    cands.sort_by_key(|c| (c.container_start, c.field_start));
    cands.dedup_by_key(|c| (c.container_start, c.field_start));
    // Attr gate: authored minus attrs must sit trimmed-equal directly
    // above the field line (contiguous); otherwise the site is skipped.
    let mut targets: Vec<(u32, u32)> = Vec::new();
    for c in cands {
        let mut top = c.field_start;
        if !minus_attrs.is_empty() {
            let mut ok = true;
            for (i, a) in minus_attrs.iter().rev().enumerate() {
                let line_no = match top.checked_sub((i as u32) + 1) {
                    Some(n) if n >= 1 => n,
                    _ => {
                        ok = false;
                        break;
                    }
                };
                match original
                    .lines()
                    .nth((line_no - 1) as usize)
                    .map(|l| l.trim().to_string())
                {
                    Some(t) if t == a.trim() => {}
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            top -= minus_attrs.len() as u32;
        }
        targets.push((top, c.field_end));
    }
    targets.sort_unstable();
    targets.dedup();
    let mut matched = 0usize;
    if hunk.scope() == Scope::File {
        for (start, end) in targets {
            edits.push(
                delete_lines(original, index, start, end, &rule).map_err(|e| {
                    StructuredError::new(
                        ErrorCode::RangeConflict,
                        format!("field deletion span outside file: {e}"),
                    )
                    .with_rule(rule.clone())
                    .with_repair(StructuredError::fix_pattern_repair(
                        "member-evo-field-delete-template",
                        "write a field-deletion hunk (`struct S {` context plus one `- field: Ty` line; see SPEC 14.8)",
                    ))
                })?,
            );
            matched += 1;
            if hunk.allow() == Allow::One && matched > 1 {
                break;
            }
        }
    } else {
        let point = hunk.header.old_start;
        let mut target: Option<(u32, u32)> = None;
        for c in &probe.out {
            let (_, site_name) = match field_line_name(original, index, c.field_start) {
                Some(v) => v,
                None => continue,
            };
            if site_name != field {
                continue;
            }
            if !(c.container_start <= point && point <= c.container_end) {
                continue;
            }
            if let Some(w) = &want_ty {
                if &c.ty_norm != w {
                    continue;
                }
            } else {
                continue;
            }
            let mut top = c.field_start;
            if !minus_attrs.is_empty() {
                let mut ok = true;
                for (i, a) in minus_attrs.iter().rev().enumerate() {
                    let line_no = match top.checked_sub((i as u32) + 1) {
                        Some(n) if n >= 1 => n,
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                    match original
                        .lines()
                        .nth((line_no - 1) as usize)
                        .map(|l| l.trim().to_string())
                    {
                        Some(t) if t == a.trim() => {}
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                top -= minus_attrs.len() as u32;
            }
            target = Some((top, c.field_end));
            break;
        }
        let Some((start, end)) = target else {
            return Ok((0, Vec::new()));
        };
        edits.push(
            delete_lines(original, index, start, end, &rule).map_err(|e| {
                StructuredError::new(
                    ErrorCode::RangeConflict,
                    format!("field deletion span outside file: {e}"),
                )
                .with_rule(rule.clone())
                .with_repair(StructuredError::fix_pattern_repair(
                    "member-evo-field-delete-template",
                    "write a field-deletion hunk (`struct S {` context plus one `- field: Ty` line; see SPEC 14.8)",
                ))
            })?,
        );
        matched = 1;
    }
    let empties = vec![HashMap::new(); matched];
    Ok((matched, empties))
}

/// The field name on 1-based `line` (`name` from `name: Ty`, vis
/// stripped), or `None` when the line is not a field shape.
fn field_line_name(original: &str, index: &LineIndex, line: u32) -> Option<(u32, String)> {
    let _ = index;
    let text = original.lines().nth(line.checked_sub(1)? as usize)?;
    let (name, _) = split_field_colon(text)?;
    let mut parts: Vec<&str> = name.split_whitespace().collect();
    let last = parts.pop()?.to_string();
    if last.is_empty() || !last.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((line, last))
}

/// One variant-deletion candidate: the variant's span inside its enum.
struct VariantDelCandidate {
    container_start: u32,
    container_end: u32,
    variant_start: u32,
    variant_end: u32,
    variant_norm: String,
}

struct EnumProbe<'ast> {
    name: &'ast str,
    out: Vec<VariantDelCandidate>,
}

fn norm_variant(v: &syn::Variant) -> String {
    quote::quote!(#v)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_variant_norm(line: &str) -> Option<String> {
    let body = line
        .trim()
        .trim_end_matches(',')
        .trim_end()
        .trim_end_matches(';')
        .trim();
    if body.is_empty() {
        return None;
    }
    syn::parse_str::<syn::Variant>(body)
        .ok()
        .map(|v| norm_variant(&v))
}

impl<'ast> Visit<'ast> for EnumProbe<'ast> {
    fn visit_item(&mut self, it: &'ast syn::Item) {
        if let syn::Item::Enum(e) = it {
            if e.ident == self.name {
                for v in &e.variants {
                    let start = v.span().start().line as u32;
                    let mut end = v.span().end().line as u32;
                    if end < start {
                        end = start;
                    }
                    self.out.push(VariantDelCandidate {
                        container_start: e.span().start().line as u32,
                        container_end: e.span().end().line as u32,
                        variant_start: start,
                        variant_end: end,
                        variant_norm: norm_variant(v),
                    });
                }
            }
        }
        visit::visit_item(self, it);
    }
}

/// Apply a variant-deletion hunk (WI-21 slice 2). See module docs.
/// The authored minus line must normalize-equal the site variant
/// (stale payloads are zero matches, never wrong-line deletes).
pub fn apply_variant_deletion_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let (enum_name, variant, minus_raw, minus_attrs) = match &compiled.kind {
        PatternKind::VariantDeletion {
            enum_name,
            variant,
            minus_raw,
            minus_attrs,
            ..
        } => (
            enum_name.clone(),
            variant.clone(),
            minus_raw.clone(),
            minus_attrs.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected variant-deletion pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "member-evo-variant-delete-template",
                "write a variant-deletion hunk (`enum E {` context plus one `- Variant` line; see SPEC 14.8)",
            )));
        }
    };
    let rule = hunk.rule_name();
    // The authored variant must parse; an unparseable authored
    // variant never matches — loud no-match, never a guess.
    let Some(want_norm) = parse_variant_norm(&minus_raw) else {
        return Ok((0, Vec::new()));
    };
    let mut probe = EnumProbe {
        name: &enum_name,
        out: Vec::new(),
    };
    for it in &parsed.items {
        probe.visit_item(it);
    }
    // Keep only the named variant with a normalize-equal body.
    let mut cands: Vec<&VariantDelCandidate> = probe
        .out
        .iter()
        .filter(|c| {
            let site_ident = variant_ident_at(original, c.variant_start);
            if site_ident.as_deref() != Some(variant.as_str()) {
                return false;
            }
            c.variant_norm == want_norm
        })
        .collect();
    cands.sort_by_key(|c| (c.container_start, c.variant_start));
    cands.dedup_by_key(|c| (c.container_start, c.variant_start));
    // Attr gate: authored minus attrs must sit trimmed-equal directly
    // above the variant line (contiguous); otherwise the site is skipped.
    let mut targets: Vec<(u32, u32)> = Vec::new();
    for c in cands {
        let mut top = c.variant_start;
        if !minus_attrs.is_empty() {
            let mut ok = true;
            for (i, a) in minus_attrs.iter().rev().enumerate() {
                let line_no = match top.checked_sub((i as u32) + 1) {
                    Some(n) if n >= 1 => n,
                    _ => {
                        ok = false;
                        break;
                    }
                };
                match original
                    .lines()
                    .nth((line_no - 1) as usize)
                    .map(|l| l.trim().to_string())
                {
                    Some(t) if t == a.trim() => {}
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            top -= minus_attrs.len() as u32;
        }
        targets.push((top, c.variant_end));
    }
    targets.sort_unstable();
    targets.dedup();
    let mut matched = 0usize;
    if hunk.scope() == Scope::File {
        for (start, end) in targets {
            edits.push(
                delete_lines(original, index, start, end, &rule).map_err(|e| {
                    StructuredError::new(
                        ErrorCode::RangeConflict,
                        format!("variant deletion span outside file: {e}"),
                    )
                    .with_rule(rule.clone())
                    .with_repair(StructuredError::fix_pattern_repair(
                        "member-evo-variant-delete-template",
                        "write a variant-deletion hunk (`enum E {` context plus one `- Variant` line; see SPEC 14.8)",
                    ))
                })?,
            );
            matched += 1;
            if hunk.allow() == Allow::One && matched > 1 {
                break;
            }
        }
    } else {
        let point = hunk.header.old_start;
        let mut target: Option<(u32, u32)> = None;
        for c in &probe.out {
            let site_ident = variant_ident_at(original, c.variant_start);
            if site_ident.as_deref() != Some(variant.as_str()) {
                continue;
            }
            if c.variant_norm != want_norm {
                continue;
            }
            if !(c.container_start <= point && point <= c.container_end) {
                continue;
            }
            let mut top = c.variant_start;
            if !minus_attrs.is_empty() {
                let mut ok = true;
                for (i, a) in minus_attrs.iter().rev().enumerate() {
                    let line_no = match top.checked_sub((i as u32) + 1) {
                        Some(n) if n >= 1 => n,
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                    match original
                        .lines()
                        .nth((line_no - 1) as usize)
                        .map(|l| l.trim().to_string())
                    {
                        Some(t) if t == a.trim() => {}
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                top -= minus_attrs.len() as u32;
            }
            target = Some((top, c.variant_end));
            break;
        }
        let Some((start, end)) = target else {
            return Ok((0, Vec::new()));
        };
        edits.push(
            delete_lines(original, index, start, end, &rule).map_err(|e| {
                StructuredError::new(
                    ErrorCode::RangeConflict,
                    format!("variant deletion span outside file: {e}"),
                )
                .with_rule(rule.clone())
                .with_repair(StructuredError::fix_pattern_repair(
                    "member-evo-variant-delete-template",
                    "write a variant-deletion hunk (`enum E {` context plus one `- Variant` line; see SPEC 14.8)",
                ))
            })?,
        );
        matched = 1;
    }
    let empties = vec![HashMap::new(); matched];
    Ok((matched, empties))
}

/// Leading ident of the variant on 1-based `line`, or `None` when the
/// line is not a variant shape (mirrors the raiser's
/// `parse_variant_add` gate: ident + `syn::Variant` validation).
fn variant_ident_at(original: &str, line: u32) -> Option<String> {
    let text = original.lines().nth(line.checked_sub(1)? as usize)?;
    let mut t = text.trim();
    t = t.strip_suffix(',').map(str::trim_end).unwrap_or(t);
    t = t.strip_suffix(';').map(str::trim_end).unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    let len = ident_len_local(t)?;
    let (ident, suffix) = t.split_at(len);
    let suffix = suffix.trim();
    if !suffix.is_empty()
        && !(suffix.starts_with('(') || suffix.starts_with('{') || suffix.starts_with('='))
    {
        return None;
    }
    let body = text
        .trim()
        .trim_end_matches(',')
        .trim_end()
        .trim_end_matches(';')
        .trim();
    syn::parse_str::<syn::Variant>(body).ok()?;
    Some(ident.to_string())
}

fn ident_len_local(s: &str) -> Option<usize> {
    let mut len = 0usize;
    for (i, c) in s.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' {
            len = i + c.len_utf8();
        } else {
            break;
        }
    }
    if len == 0 {
        return None;
    }
    let head = s[..len].chars().next()?;
    if head.is_ascii_digit() {
        return None;
    }
    Some(len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuleSemantics;

    pub(super) fn apply_report(
        patch_text: &str,
        file_text: &str,
    ) -> Result<(String, crate::ApplyReport), String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = std::collections::HashMap::new();
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

    const DEL_CTX: &str = "  struct Old {\n";

    fn del_patch(range: &str, minus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=del\n{DEL_CTX}- {minus}\n"
        )
    }

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

    #[test]
    fn deletes_exact_field_line() {
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\n";
        let patch = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -3,1 @@ rule=r1\n  struct Old {\n- y: u32,\n";
        let post = apply(patch, pre).expect("applies");
        assert_eq!(post, "struct Old {\n    x: u32,\n}\n", "got:\n{post}");
    }

    #[test]
    fn stale_type_skips_loud() {
        // Site type already changed: zero matches (the planner reports
        // the no-match; never deletes under a lie).
        let pre = "struct Old {\n    x: u32,\n    y: u64,\n}\n";
        let (hunk, compiled, parsed, index, src) = setup(&del_patch("@@ -3,1 @@", "y: u32,"), pre);
        let mut edits = Vec::new();
        let (n, _) = apply_field_deletion_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect("returns zero");
        assert_eq!(n, 0);
        assert!(edits.is_empty());
    }

    #[test]
    fn missing_field_is_zero() {
        let pre = "struct Old {\n    x: u32,\n}\n";
        let (hunk, compiled, parsed, index, src) = setup(&del_patch("@@ -3,1 @@", "y: u32,"), pre);
        let mut edits = Vec::new();
        let (n, _) = apply_field_deletion_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect("returns zero");
        assert_eq!(n, 0);
    }

    #[test]
    fn sweep_deletes_in_every_same_named_container() {
        // Two same-named structs (uncompilable but syn-parseable) both
        // lose the field under `# sweep` + `# allow=many`.
        let pre = "struct S {\n    y: u32,\n}\nstruct S {\n    y: u32,\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=del\n# sweep\n# allow=many\n  struct S {\n- y: u32,\n";
        let (post, report) = apply_report(patch, pre).expect("applies");
        assert_eq!(post, "struct S {\n}\nstruct S {\n}\n", "got:\n{post}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::ReparseOnly);
    }

    #[test]
    fn wrong_kind_fails_with_specific_repair() {
        let add_patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 @@ rule=add\n  struct S {\n+ z: u32,\n";
        let (hunk, mut compiled, parsed, index, src) =
            setup(add_patch, "struct S {\n    y: u32,\n}\n");
        // Slice-7/10/11 idiom: force the dispatch else-arm by
        // overwriting the kind with `Raw` (dispatch routes by kind).
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = apply_field_deletion_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("wrong kind must fail");
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        let repair = err.repair.expect("WI-18: must carry repair");
        assert_eq!(
            repair.get("code").and_then(|v| v.as_str()),
            Some("member-evo-field-delete-template")
        );
    }

    const VDEL_CTX: &str = "  enum E {\n";

    fn vdel_patch(range: &str, minus: &str) -> String {
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=vdel\n{VDEL_CTX}- {minus}\n"
        )
    }

    fn vsetup(
        patch_text: &str,
        src: &str,
    ) -> (
        rsmpl_patch::RsmplHunk,
        rsmpl_pattern::CompiledPattern,
        syn::File,
        crate::lines::LineIndex,
        String,
    ) {
        setup(patch_text, src)
    }

    #[test]
    fn deletes_exact_variant_line() {
        let pre = "enum E {\n    A,\n    B,\n}\n";
        let patch = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -3,1 @@ rule=r1\n  enum E {\n- B,\n";
        let post = apply(patch, pre).expect("applies");
        assert_eq!(post, "enum E {\n    A,\n}\n", "got:\n{post}");
    }

    #[test]
    fn deletes_tuple_variant_line() {
        let pre = "enum E {\n    A,\n    B(u32),\n}\n";
        let patch = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -3,1 @@ rule=r1\n  enum E {\n- B(u32),\n";
        let post = apply(patch, pre).expect("applies");
        assert_eq!(post, "enum E {\n    A,\n}\n", "got:\n{post}");
    }

    #[test]
    fn stale_variant_payload_skips_loud() {
        // Site payload already changed: zero matches (the planner
        // reports the no-match; never deletes under a lie).
        let pre = "enum E {\n    A,\n    B(u64),\n}\n";
        let (hunk, compiled, parsed, index, src) =
            vsetup(&vdel_patch("@@ -3,1 @@", "B(u32),"), pre);
        let mut edits = Vec::new();
        let (n, _) =
            apply_variant_deletion_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
                .expect("returns zero");
        assert_eq!(n, 0);
        assert!(edits.is_empty());
    }

    #[test]
    fn missing_variant_is_zero() {
        let pre = "enum E {\n    A,\n}\n";
        let (hunk, compiled, parsed, index, src) = vsetup(&vdel_patch("@@ -3,1 @@", "B,"), pre);
        let mut edits = Vec::new();
        let (n, _) =
            apply_variant_deletion_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
                .expect("returns zero");
        assert_eq!(n, 0);
    }

    #[test]
    fn variant_sweep_deletes_in_every_same_named_container() {
        // Two same-named enums (uncompilable but syn-parseable) both
        // lose the variant under `# sweep` + `# allow=many`.
        let pre = "enum E {\n    B,\n}\nenum E {\n    B,\n}\n";
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=vdel\n# sweep\n# allow=many\n  enum E {\n- B,\n";
        let (post, report) = apply_report(patch, pre).expect("applies");
        assert_eq!(post, "enum E {\n}\nenum E {\n}\n", "got:\n{post}");
        assert_eq!(report.rules[0].semantics, RuleSemantics::ReparseOnly);
    }

    #[test]
    fn variant_wrong_kind_fails_with_specific_repair() {
        let add_patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 @@ rule=add\n  enum E {\n+ C,\n";
        let (hunk, mut compiled, parsed, index, src) = vsetup(add_patch, "enum E {\n    B,\n}\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = apply_variant_deletion_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("wrong kind must fail");
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        let repair = err.repair.expect("WI-18: must carry repair");
        assert_eq!(
            repair.get("code").and_then(|v| v.as_str()),
            Some("member-evo-variant-delete-template")
        );
    }
}
