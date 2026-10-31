//! `use`-path prefix renames (SPEC 14.9, T19a; group members T183).
//!
//! A minus/plus prefix pair (`use crate::old` / `use crate::new`) rewrites
//! the leading-segment span of every matching `use` item — including
//! members sitting directly inside a `{...}` group at the matched position
//! (`use crate::{old, other}` rewrites the `old` member span). The target's
//! tail (final name, groups, globs, `as` renames) sits outside the replaced
//! span and is preserved byte-for-byte, exactly like the ident-span fast
//! path of item renames. Visibility, attributes, and formatting are likewise
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

/// One recorded prefix match: the span to replace, its replacement text
/// (the plus suffix aligned to the matched minus suffix), and the anchor
/// line (match start) for scope filtering.
struct UseEdit {
    first: proc_macro2::Span,
    last: proc_macro2::Span,
    replacement: String,
}

/// Recursive prefix walk (T183). `consumed` counts the minus segments
/// matched along the current path; `start` carries the match-start span
/// plus the consumed count when it started (`None` until the first segment
/// matches — groups reset it so member matches stay contiguous and never
/// span braces).
///
/// `Path` descends on minus-head equality and records when minus is fully
/// consumed (the remaining subtree is the preserved tail). `Group` recurses
/// into each member with the current suffix — this is what reaches members
/// at the matched position (`crate::{old, other}`) as well as root groups
/// (`use {crate::old, x}`). `Name`/`Rename` record on exact full
/// consumption (a rename keeps `as o`: only the leading ident is replaced).
/// `Glob` records nothing (a glob tail after full consumption was already
/// recorded by its parent `Path`).
fn walk_use_tree(
    tree: &syn::UseTree,
    minus: &[String],
    plus: &[String],
    consumed: usize,
    start: Option<(proc_macro2::Span, usize)>,
    out: &mut Vec<UseEdit>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            if consumed < minus.len() && p.ident == minus[consumed] {
                let st = start.or(Some((p.ident.span(), consumed)));
                if consumed + 1 == minus.len() {
                    record_match(st, p.ident.span(), minus, plus, out);
                } else {
                    walk_use_tree(&p.tree, minus, plus, consumed + 1, st, out);
                }
            }
        }
        syn::UseTree::Name(n) => {
            if consumed < minus.len() && consumed + 1 == minus.len() && n.ident == minus[consumed] {
                record_match(
                    start.or(Some((n.ident.span(), consumed))),
                    n.ident.span(),
                    minus,
                    plus,
                    out,
                );
            }
        }
        syn::UseTree::Rename(r) => {
            if consumed < minus.len() && consumed + 1 == minus.len() && r.ident == minus[consumed] {
                record_match(
                    start.or(Some((r.ident.span(), consumed))),
                    r.ident.span(),
                    minus,
                    plus,
                    out,
                );
            }
        }
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(g) => {
            for member in &g.items {
                walk_use_tree(member, minus, plus, consumed, None, out);
            }
        }
    }
}

/// Record a full minus consumption. The rewritten span covers `minus[k..]`
/// (k = consumed when the match started), so the replacement must cover the
/// same suffix of plus — which requires the leading k segments to be
/// literally shared (`plus[..k] == minus[..k]`). Otherwise the member is
/// skipped, never guessed: divergent prefixes and shrinking replacements
/// (`plus.len() <= k`, empty text) stay fail-safe for an ordinary hunk.
fn record_match(
    start: Option<(proc_macro2::Span, usize)>,
    last: proc_macro2::Span,
    minus: &[String],
    plus: &[String],
    out: &mut Vec<UseEdit>,
) {
    let Some((first, k)) = start else { return };
    if plus.len() <= k || plus[..k] != minus[..k] {
        return;
    }
    out.push(UseEdit {
        first,
        last,
        replacement: plus[k..].join("::"),
    });
}

/// Scope filtering (SPEC 17.3) applied per rewritten span. `anchor` uses the
/// match-start line as the first concrete non-ellipsis token.
fn in_scope(edit: &UseEdit, scope: Scope, old_start: u32, old_len: u32) -> bool {
    let start_line = edit.first.start().line as u32;
    let end_line = edit.last.end().line as u32;
    let end = old_start + old_len.saturating_sub(1);
    match scope {
        Scope::Contained => start_line >= old_start && end_line <= end.max(old_start),
        Scope::Anchor => start_line >= old_start && start_line <= end.max(old_start),
        Scope::Overlap => start_line <= end.max(old_start) && end_line >= old_start,
        Scope::Exact => old_len > 0 && start_line == old_start && end_line == end,
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
    // Re-walk: collect use items, then match each tree recursively (T183
    // reaches group members at the matched position).
    let plus: Vec<String> = plus_path.split("::").map(str::to_string).collect();
    let mut items = Vec::new();
    collect_use_items(&parsed.items, &mut items);

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for v in &items {
        let mut found: Vec<UseEdit> = Vec::new();
        walk_use_tree(v.tree, &minus_segs, &plus, 0, None, &mut found);
        if found.is_empty() {
            continue;
        }
        // Attribute policies are per item and checked only for items with
        // edits: unrelated items must never trip `# attrs=absent`.
        check_attrs(v.attrs, hunk.attrs_policy(), &[], &rule, "")?;
        let mut item_edits = 0;
        for e in &found {
            if !in_scope(e, hunk.scope(), old_start, old_len) {
                continue;
            }
            let (a, b) = span_bytes(original, index, e.first)
                .and_then(|(a, _)| span_bytes(original, index, e.last).map(|(_, b)| (a, b)))
                .ok_or_else(|| range_err(&rule, e.first.start().line as u32))?;
            if a >= b {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "use prefix span is empty (file changed during matching?)",
                )
                .with_rule(&rule)
                .with_repair(StructuredError::internal_repair()));
            }
            edits.push(TextEdit::new(a, b, e.replacement.clone()));
            item_edits += 1;
            matched += 1;
            if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
                break; // count validation reports the error
            }
        }
        // `# attrs=strip`: delete attribute lines above the item once per
        // item (not once per member edit — duplicate identical edits would
        // trip the overlap check).
        if item_edits > 0 && hunk.attrs_policy() == AttrsPolicy::Strip && !v.attrs.is_empty() {
            let start = index
                .line_start(v.attr_start_line)
                .ok_or_else(|| range_err(&rule, v.attr_start_line))?;
            let end = index
                .line_start(v.anchor_line)
                .ok_or_else(|| range_err(&rule, v.anchor_line))?;
            if end > start {
                edits.push(TextEdit::new(start, end, String::new()));
            }
        }
        if hunk.allow() == rsmpl_patch::Allow::One && matched > 1 {
            break; // count validation reports the error
        }
    }
    Ok(matched)
}

/// One collected `use` item: its tree plus the attribute/line info the
/// match and strip steps need.
struct UseItemView<'a> {
    tree: &'a syn::UseTree,
    attrs: &'a [syn::Attribute],
    anchor_line: u32,
    attr_start_line: u32,
}

/// Collect file- and inline-`mod`-level `use` items.
fn collect_use_items<'a>(items: &'a [syn::Item], out: &mut Vec<UseItemView<'a>>) {
    for it in items {
        match it {
            syn::Item::Use(u) => {
                let item_span = it.span();
                out.push(UseItemView {
                    tree: &u.tree,
                    attrs: &u.attrs,
                    anchor_line: u.use_token.span.start().line as u32,
                    attr_start_line: item_span.start().line as u32,
                });
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_use_items(inner, out);
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
    fn rewrites_group_member_at_matched_position() {
        // T183: a member sitting directly inside `{...}` at the matched
        // position rewrites; siblings (incl. `as` renames) are preserved.
        let patch = use_patch_in(
            "@@ -1,3 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "use crate::{old, other as o};\nuse crate::old::Thing;\nfn m() {}\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::{new, other as o};"), "got: {out}");
        assert!(out.contains("use crate::new::Thing;"), "got: {out}");
    }

    #[test]
    fn rewrites_group_member_with_tail() {
        // T183: `old::Deep` inside a group rewrites the `old` span only.
        let patch = use_patch_in(
            "@@ -1,1 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "use crate::{old::Deep, other};\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(out, "use crate::{new::Deep, other};\n", "got: {out}");
    }

    #[test]
    fn group_member_suffix_never_overreaches() {
        // T183: `x::old` is not the `crate::old` prefix — members match from
        // member start only, so this fails safe instead of corrupting.
        let patch = use_patch_in("@@ -1,1 @@", "use crate::old", "use crate::new", "");
        let src = "use crate::{x::old};\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn root_group_members_match() {
        // T183: a group at the root matches the full minus path per member.
        let patch = use_patch_in(
            "@@ -1,1 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let src = "use {crate::old, x};\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(out, "use {crate::new, x};\n", "got: {out}");
    }

    #[test]
    fn divergent_and_shrinking_prefix_skip_group_members() {
        // T183: the member replacement is the plus suffix aligned to the
        // matched minus suffix, which requires the leading segments to be
        // literally shared — otherwise the member is skipped, never guessed.
        // (Non-grouped sites still rewrite with the full plus path.)
        let grouped = "use crate::{old::T};\n";
        let divergent = use_patch_in("@@ -1,1 @@", "use crate::old", "use changed::new", "");
        let err = apply(&divergent, grouped).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        let shrinking = use_patch_in("@@ -1,1 @@", "use crate::old", "use new", "");
        let err = apply(&shrinking, grouped).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        // Non-grouped sites still rewrite with the full plus path.
        let out = apply(&divergent, "use crate::old::U;\n").unwrap();
        assert_eq!(out, "use changed::new::U;\n", "got: {out}");
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
