//! Struct-field deletion (WI-21 slice 1).
//!
//! D5 sibling of `matcher_member_evo` (1656 lines, over the 1600 gate):
//! the `FieldDeletion` lowering (`struct S {` context + one `- field`
//! line, no plus member). Mirrors `apply_field_addition_hunk` with the
//! membership test inverted (the named field must be *present*; the
//! deleted type text must normalize-equal the site type, else the site
//! is skipped) and the edit a whole-line deletion via the shared
//! `matcher_fn_item::delete_lines` (blank-absorption included).
//!
//! ```diff
//!   struct S {
//! - old: u32
//! ```
//!
//! Scope discipline mirrors addition: `# sweep` (or any file-scope
//! range) deletes the field in every containing `struct Name`
//! (`allow=many`; `allow=one` stops after the first); a ranged hunk
//! deletes it in the single container spanning the range point.
//! Zero targets is `Ok(0)` (the planner reports the no-match); a
//! wrong-kind call is a loud `PatternConflict` with the
//! site-specific `member-evo-field-delete-template` repair (WI-18
//! ceiling holds — this site carries `with_repair`).

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
}
