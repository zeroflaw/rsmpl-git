//! `use`-path prefix renames (SPEC 14.9, T19a).
//!
//! A minus/plus prefix pair (`use crate::old` / `use crate::new`) rewrites
//! the leading-segment span of every matching `use` item. The target's tail
//! (final name, groups, globs, `as` renames) sits outside the replaced span
//! and is preserved byte-for-byte, exactly like the ident-span fast path of
//! item renames. Visibility, attributes, and formatting are likewise
//! preserved (attribute policies mirror item renames).
//!
//! Only file- and inline-`mod`-level `use` items are collected; `use`
//! declarations nested inside function bodies are not matched in v0.1.

use syn::spanned::Spanned;

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_item::{check_attrs, span_bytes};

/// One prefix-match candidate: the byte span of the matched leading
/// segments plus line spans for scope filtering.
struct UseCandidate {
    start_line: u32,
    end_line: u32,
    anchor_line: u32,
    attr_start_line: u32,
    attrs: Vec<syn::Attribute>,
}

/// Leading plain segments of a `use` tree with their spans, descending
/// through nested `Path` nodes. Stops at (and excludes) `Group`/`Glob`
/// tails; includes a trailing `Name`/`Rename` ident as the final segment.
fn leading_segments(tree: &syn::UseTree, out: &mut Vec<(String, proc_macro2::Span)>) {
    match tree {
        syn::UseTree::Path(p) => {
            out.push((p.ident.to_string(), p.ident.span()));
            leading_segments(&p.tree, out);
        }
        syn::UseTree::Name(n) => out.push((n.ident.to_string(), n.ident.span())),
        syn::UseTree::Rename(r) => out.push((r.ident.to_string(), r.ident.span())),
        syn::UseTree::Glob(_) | syn::UseTree::Group(_) => {}
    }
}

/// Scope filtering (SPEC 17.3). `anchor` uses the prefix-start line as the
/// first concrete non-ellipsis token; other modes use the item span.
fn in_scope(c: &UseCandidate, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => c.start_line >= old_start && c.end_line <= end.max(old_start),
        Scope::Anchor => c.anchor_line >= old_start && c.anchor_line <= end.max(old_start),
        Scope::Overlap => c.start_line <= end.max(old_start) && c.end_line >= old_start,
        Scope::Exact => old_len > 0 && c.start_line == old_start && c.end_line == end,
        Scope::File => true,
    }
}

fn range_err(rule: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
}

/// Apply a `use`-path prefix rename hunk. Returns the number of matches.
pub fn apply_use_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (minus_segs, plus_path) = match &compiled.kind {
        PatternKind::UseRename {
            minus_segs,
            plus_path,
        } => (minus_segs.clone(), plus_path.clone()),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected use-path rename pattern".to_string(),
            ));
        }
    };
    let rule = hunk.rule_name();
    // Re-walk: collect candidate segment spans alongside line info.
    let mut cands: Vec<(UseCandidate, Vec<(String, proc_macro2::Span)>)> = Vec::new();
    collect_with_segs(&parsed.items, &mut cands);

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for (c, segs) in &cands {
        if segs.len() < minus_segs.len() {
            continue;
        }
        if !segs
            .iter()
            .take(minus_segs.len())
            .map(|(s, _)| s)
            .eq(minus_segs.iter())
        {
            continue;
        }
        if !in_scope(c, hunk.scope(), old_start, old_len) {
            continue;
        }
        check_attrs(&c.attrs, hunk.attrs_policy(), &[], &rule, "")?;
        let first_span = segs[0].1;
        let last_span = segs[minus_segs.len() - 1].1;
        let (a, b) = span_bytes(original, index, first_span)
            .and_then(|(a, _)| span_bytes(original, index, last_span).map(|(_, b)| (a, b)))
            .ok_or_else(|| range_err(&rule, first_span.start().line as u32))?;
        if a >= b {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "use prefix span is empty (file changed during matching?)",
            )
            .with_rule(&rule));
        }
        edits.push(TextEdit::new(a, b, plus_path.clone()));
        // `# attrs=strip`: delete attribute lines above the item.
        if hunk.attrs_policy() == AttrsPolicy::Strip && !c.attrs.is_empty() {
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
        matched += 1;
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

/// Collect `use` items with their leading-segment spans.
fn collect_with_segs(
    items: &[syn::Item],
    out: &mut Vec<(UseCandidate, Vec<(String, proc_macro2::Span)>)>,
) {
    for it in items {
        match it {
            syn::Item::Use(u) => {
                let mut segs = Vec::new();
                leading_segments(&u.tree, &mut segs);
                if segs.is_empty() {
                    continue;
                }
                let item_span = it.span();
                let first = segs[0].1;
                out.push((
                    UseCandidate {
                        start_line: item_span.start().line as u32,
                        end_line: item_span.end().line as u32,
                        anchor_line: first.start().line as u32,
                        attr_start_line: item_span.start().line as u32,
                        attrs: u.attrs.clone(),
                    },
                    segs,
                ));
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_with_segs(inner, out);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn apply(patch_text: &str, file_text: &str) -> Result<String, String> {
        let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
        let mut files = HashMap::new();
        files.insert("src/lib.rs".to_string(), file_text.to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        crate::apply_patch_to_tree(&mut files, &patch).map_err(|e| e.render())?;
        Ok(files["src/lib.rs"].clone())
    }

    fn use_patch_in(range: &str, minus: &str, plus: &str, extra: &str) -> String {
        // A hunk with no `#` line at all is an ordinary hunk: bare renames
        // keep `# lang=rust` as the RSmPL marker (SPEC 7.1).
        let extra = if extra.is_empty() {
            "# lang=rust\n"
        } else {
            extra
        };
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=u\n{extra}- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn rewrites_plain_prefix() {
        let patch = use_patch_in(
            "@@ -1,3 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "use crate::old::Thing;\nuse crate::old::other::Widget;\nfn a() {}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::new::Thing;"), "got: {out}");
        assert!(out.contains("use crate::new::other::Widget;"), "got: {out}");
        assert!(!out.contains("crate::old"), "got: {out}");
    }

    #[test]
    fn preserves_group_glob_as_tails() {
        let patch = use_patch_in(
            "@@ -1,3 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "use crate::old::{A, B};\nuse crate::old::*;\nuse crate::old::Thing as T;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::new::{A, B};"), "got: {out}");
        assert!(out.contains("use crate::new::*;"), "got: {out}");
        assert!(out.contains("use crate::new::Thing as T;"), "got: {out}");
    }

    #[test]
    fn preserves_pub_and_attrs() {
        let patch = use_patch_in(
            "@@ -1,3 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "pub use crate::old::Thing;\n#[allow(unused_imports)]\nuse crate::old::Other;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("pub use crate::new::Thing;"), "got: {out}");
        assert!(out.contains("#[allow(unused_imports)]"), "got: {out}");
        assert!(out.contains("use crate::new::Other;"), "got: {out}");
    }

    #[test]
    fn segment_boundary_not_string_prefix() {
        let patch = use_patch_in(
            "@@ -1,2 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "use crate::old2::Thing;\nuse crate::old::Thing;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::old2::Thing;"), "got: {out}");
        assert!(out.contains("use crate::new::Thing;"), "got: {out}");
    }

    #[test]
    fn allow_one_rejects_two_imports() {
        let patch = use_patch_in("@@ -1,2 @@", "use crate::old", "use crate::new", "");
        let src = "use crate::old::A;\nuse crate::old::B;\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn no_match_without_optional_fails() {
        let patch = use_patch_in("@@ -1,1 @@", "use crate::gone", "use crate::new", "");
        let src = "use crate::old::Thing;\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn attrs_absent_and_strip() {
        let absent1 = use_patch_in(
            "@@ -1,1 @@",
            "use crate::old",
            "use crate::new",
            "# attrs=absent\n",
        );
        let absent2 = use_patch_in(
            "@@ -1,2 @@",
            "use crate::old",
            "use crate::new",
            "# attrs=absent\n",
        );
        assert!(apply(&absent1, "use crate::old::A;\n").is_ok());
        assert!(apply(&absent2, "#[allow(unused)]\nuse crate::old::A;\n").is_err());
        let strip = use_patch_in(
            "@@ -1,2 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n# attrs=strip\n",
        );
        let out = apply(&strip, "#[allow(unused)]\nuse crate::old::A;\n").unwrap();
        assert!(!out.contains("allow(unused)"), "got: {out}");
        assert!(out.contains("use crate::new::A;"), "got: {out}");
    }

    #[test]
    fn nested_mod_items_found() {
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 @@ rule=u\n# lang=rust\n- use crate::old\n+ use crate::new\n";
        let src = "mod outer {\n    use crate::old::Thing;\n}\n";
        let out = apply(patch, src).unwrap();
        assert!(out.contains("use crate::new::Thing;"), "got: {out}");
    }

    #[test]
    fn different_depth_prefix() {
        // Plus may be longer/shorter than minus: pure prefix swap.
        let patch = use_patch_in(
            "@@ -1,1 @@",
            "use old",
            "use crate::new::sub",
            "# allow=many\n",
        );
        let src = "use old::Thing;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::new::sub::Thing;"), "got: {out}");
    }

    #[test]
    fn malformed_use_shapes_fail_fast() {
        for (minus, plus) in [
            ("use crate::old::{A}", "use crate::new"),
            ("use crate::old", "use crate::new::*"),
            ("use crate::old", "use crate::old"),
        ] {
            let patch = use_patch_in("@@ -1,1 @@", minus, plus, "");
            let err = apply(&patch, "use crate::old::A;\n").unwrap_err();
            assert!(
                err.contains("RSMPL_PATTERN_CONFLICT"),
                "got: {err} for {minus}"
            );
        }
    }
}
