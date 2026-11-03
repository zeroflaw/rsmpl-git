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

/// One root-to-leaf path through a `use` tree, with per-segment spans and
/// the group-boundary depths on the path (segment counts before each
/// `{...}` entered). A boundary strictly inside the rewritten span means
/// sibling leaves share the outer prefix — the edit covers the member
/// portion only (never the shared outer, which would overlap its
/// siblings).
struct UseLeaf {
    segs: Vec<(String, proc_macro2::Span)>,
    boundaries: Vec<usize>,
}

/// Collect every root-to-leaf path (`Path` descends, `Group` fans out per
/// member with the outer prefix shared, `Glob` ends the leaf with the
/// `::*` tail outside the ident spans — preserved byte-for-byte).
fn collect_use_leaves(
    tree: &syn::UseTree,
    prefix: &mut Vec<(String, proc_macro2::Span)>,
    boundaries: &mut Vec<usize>,
    out: &mut Vec<UseLeaf>,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push((p.ident.to_string(), p.ident.span()));
            collect_use_leaves(&p.tree, prefix, boundaries, out);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            prefix.push((n.ident.to_string(), n.ident.span()));
            out.push(UseLeaf {
                segs: prefix.clone(),
                boundaries: boundaries.clone(),
            });
            prefix.pop();
        }
        syn::UseTree::Rename(r) => {
            prefix.push((r.ident.to_string(), r.ident.span()));
            out.push(UseLeaf {
                segs: prefix.clone(),
                boundaries: boundaries.clone(),
            });
            prefix.pop();
        }
        syn::UseTree::Glob(_) => {
            out.push(UseLeaf {
                segs: prefix.clone(),
                boundaries: boundaries.clone(),
            });
        }
        syn::UseTree::Group(g) => {
            boundaries.push(prefix.len());
            for member in &g.items {
                collect_use_leaves(member, prefix, boundaries, out);
            }
            boundaries.pop();
        }
    }
}

/// WI-4 slice 2b: symbol-head `use`-prefix matching over leaf paths.
///
/// The minus head symbol expands to its seed (text or canon); the minus
/// suffix locates the head/tail split per leaf (suffix compares
/// textually — compile-gated concrete). The head verifies by exact seed
/// text (`minus_canon` is `None` under `# fallback=syntax`) or by
/// `graph.resolve_call(head) == seed_canon + suffix` (re-export
/// spellings resolve; vendored twins miss; glob-file/unresolvable heads
/// miss, never veto). The replacement is the expanded plus head plus the
/// preserved tail (`::*`/`as` tails sit outside the ident spans).
/// Leading-`::` parity is ignored (the historical walk never owned it).
///
/// Group sharing: a boundary strictly inside the rewritten span means the
/// outer prefix is shared with sibling leaves — the edit covers the
/// member portion only, guarded by the outer being literally preserved
/// (`plus_expanded[..d] == leaf[..d]`, the historical `plus[..k]` law
/// restated for expanded heads); a divergent outer skips the member,
/// never guesses. A match wholly inside the outer prefix emits one
/// shared edit (exact duplicates collapse below).
#[allow(clippy::too_many_arguments)]
fn collect_symbol_use_edits(
    tree: &syn::UseTree,
    minus_segs: &[String],
    minus_head_sym: &Option<String>,
    symbols: &std::collections::HashMap<String, String>,
    plus_expanded: &[String],
    minus_canon: &Option<String>,
    symbol_graph: Option<(&crate::CrateGraph, &str)>,
    out: &mut Vec<UseEdit>,
) {
    let Some(head_name) = minus_head_sym else {
        return;
    };
    let Some(seed) = symbols.get(head_name) else {
        return;
    };
    let seed_text: Vec<String> = seed
        .trim_start_matches("::")
        .split("::")
        .map(str::to_string)
        .collect();
    let minus_suffix: &[String] = if minus_segs.len() > 1 {
        &minus_segs[1..]
    } else {
        &[]
    };
    let mut leaves = Vec::new();
    collect_use_leaves(tree, &mut Vec::new(), &mut Vec::new(), &mut leaves);
    for leaf in &leaves {
        let ids: Vec<String> = leaf.segs.iter().map(|(s, _)| s.clone()).collect();
        let sfx = minus_suffix.len();
        if ids.len() < sfx + 1 {
            continue;
        }
        // Every head split, shortest head first; the first verifying split
        // wins (a path and its own prefix never resolve to the same
        // definition, so two splits cannot both verify).
        for h in 1..=(ids.len() - sfx) {
            if !ids[h..h + sfx]
                .iter()
                .zip(minus_suffix.iter())
                .all(|(a, b)| a == b)
            {
                continue;
            }
            let head = &ids[..h];
            let verified = match minus_canon {
                Some(want) => {
                    let want_full = if minus_suffix.is_empty() {
                        want.clone()
                    } else {
                        format!("{want}::{}", minus_suffix.join("::"))
                    };
                    match symbol_graph {
                        Some((graph, file)) => {
                            graph.resolve_call(file, false, head) == Some(want_full)
                        }
                        None => false,
                    }
                }
                None => head == seed_text.as_slice(),
            };
            if !verified {
                continue;
            }
            // Group sharing: deepest boundary strictly inside the span.
            let end = h + sfx;
            let boundary = leaf.boundaries.iter().filter(|b| **b < end).max().copied();
            match boundary {
                Some(d) if d > 0 => {
                    // Member portion only; the outer must be literally
                    // preserved or the splice is unstatable (skip, never
                    // guess — mirrors the historical `plus[..k]` guard).
                    if plus_expanded.len() < d
                        || !plus_expanded[..d]
                            .iter()
                            .zip(ids[..d].iter())
                            .all(|(a, b)| a == b)
                    {
                        continue;
                    }
                    let Some(rep) = plus_expanded.get(d..) else {
                        continue;
                    };
                    if rep.is_empty() {
                        continue;
                    }
                    out.push(UseEdit {
                        first: leaf.segs[d].1,
                        last: leaf.segs[end - 1].1,
                        replacement: rep.join("::"),
                    });
                }
                _ => {
                    // Whole head+suffix span (d == 0, or the match sits
                    // wholly in the outer prefix — shared edits collapse
                    // as exact duplicates below). The tail stays outside
                    // the span, preserved byte-for-byte.
                    out.push(UseEdit {
                        first: leaf.segs[0].1,
                        last: leaf.segs[end - 1].1,
                        replacement: plus_expanded.join("::"),
                    });
                }
            }
            break;
        }
    }
    // Shared-outer matches (a match wholly inside a group prefix) emit one
    // edit per descendant leaf — collapse exact duplicates so siblings
    // never overlap each other.
    let mut seen = std::collections::HashSet::new();
    out.retain(|e| {
        let key = (
            e.first.start().line,
            e.first.start().column,
            e.last.end().line,
            e.last.end().column,
            e.replacement.clone(),
        );
        seen.insert(key)
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
///
/// WI-4 slice 2b: the minus/plus heads may each name a declared
/// `# symbol` (compile-gated leading-only, SPEC 19.3). A concrete minus
/// head keeps the historical segment-wise walk (the plus head symbol
/// expands to its seed text for emission). A minus head symbol takes the
/// leaf walk below: the minus suffix locates the split, the head verifies
/// by exact seed text (`# fallback=syntax`) or crate-graph proof (bare
/// `# symbol`, re-export spellings included, vendored twins skipped),
/// and the replacement is the plus seed text + plus suffix + preserved
/// tail. Leading-`::` parity is ignored on both postures (the historical
/// walk never owned it either — patterns normalize it away).
pub fn apply_use_rename_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
    symbol_graph: Option<(&crate::CrateGraph, &str)>,
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
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "dispatch-use-rename-template",
                "write a use-path prefix pair (`- use crate::old` / `+ use crate::new`, same tail; see SPEC 14.9)",
            )));
        }
    };
    let rule = hunk.rule_name();
    // Re-walk: collect use items, then match each tree recursively (T183
    // reaches group members at the matched position).
    let plus: Vec<String> = plus_path.split("::").map(str::to_string).collect();
    // WI-4 slice 2b: symbol heads (compile-gated leading-only). A head
    // segment naming a declared `# symbol` expands to its seed segments;
    // anything else is concrete.
    let symbols = compiled.table.symbols.clone();
    let fallback_syntax = hunk.fallback_syntax();
    let minus_head_sym = minus_segs
        .first()
        .filter(|s| symbols.contains_key(*s))
        .cloned();
    let plus_head_sym = plus.first().filter(|s| symbols.contains_key(*s)).cloned();
    let expand_head = |segs: &[String], head: &Option<String>| -> Vec<String> {
        match head {
            Some(name) => {
                let seed = symbols.get(name).cloned().unwrap_or_default();
                let mut out: Vec<String> = seed
                    .trim_start_matches("::")
                    .split("::")
                    .map(str::to_string)
                    .collect();
                out.extend(segs.iter().skip(1).cloned());
                out
            }
            None => segs.to_vec(),
        }
    };
    let plus_expanded = expand_head(&plus, &plus_head_sym);
    // Resolved minus-head canon (plus-only symbols need no proof — the
    // seed names the rewrite target the crate may not define yet, same
    // law as the call/general matchers). Fallback and concrete heads
    // need no canon either.
    let minus_canon: Option<String> = match (&minus_head_sym, fallback_syntax) {
        (Some(name), false) => {
            let mut only = std::collections::HashMap::new();
            only.insert(name.clone(), symbols[name].clone());
            let canons = crate::matcher_symbol::verify_symbol_seeds(
                &only,
                &format!("__RSMPL_{name}"),
                fallback_syntax,
                symbol_graph,
                &rule,
            )?;
            canons.get(name).cloned()
        }
        _ => None,
    };
    let mut items = Vec::new();
    collect_use_items(&parsed.items, &mut items);

    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let mut matched = 0;
    for v in &items {
        let mut found: Vec<UseEdit> = Vec::new();
        if minus_head_sym.is_some() {
            // WI-4 slice 2b: symbol minus head — leaf walk with
            // head/tail split (the historical walk's `plus[..k]` alignment
            // assumes shared leading segments; expanded seeds may share
            // none, so the whole head+suffix span is replaced at once).
            // `minus_canon` is `Some` under the resolved posture, `None`
            // under `# fallback=syntax` (exact seed-text match).
            collect_symbol_use_edits(
                v.tree,
                &minus_segs,
                &minus_head_sym,
                &symbols,
                &plus_expanded,
                &minus_canon,
                symbol_graph,
                &mut found,
            );
        } else {
            // Concrete minus head (a plus head symbol already expanded to
            // its seed text above): historical segment-wise walk.
            walk_use_tree(v.tree, &minus_segs, &plus_expanded, 0, None, &mut found);
        }
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
    vis: syn::Visibility,
    attrs: &'a [syn::Attribute],
    anchor_line: u32,
    attr_start_line: u32,
}

/// Apply a `use`-regroup merge hunk (SPEC 14.9). Returns the number of
/// merged sets. Line-wise: the first-in-file member line is replaced by
/// the verbatim plus line (keeping that line's indent), every other
/// member line is deleted whole (attribute lines above a deleted item
/// go with it). Only single-line `use` items match; visibility must
/// equal the pattern's uniform vis (whitespace-insensitive); leading
/// `::` is normalized away on both sides.
pub fn apply_use_regroup_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (prefix, members, vis, plus_line) = match &compiled.kind {
        PatternKind::UseRegroup {
            prefix,
            members,
            vis,
            plus_line,
        } => (
            prefix.clone(),
            members.clone(),
            vis.clone(),
            plus_line.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected use-regroup merge pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "dispatch-use-regroup-template",
                "write a merge-only use-regroup pair (N≥2 `use P::Ti;` minus lines plus one grouped `use P::{...}` plus line; see SPEC 14.9)",
            )));
        }
    };
    let rule = hunk.rule_name();
    let want_vis: String = vis.chars().filter(|c| !c.is_whitespace()).collect();
    let minus_paths: Vec<Vec<String>> = members
        .iter()
        .map(|m| {
            let mut p = prefix.clone();
            p.push(m.clone());
            p
        })
        .collect();
    let mut items = Vec::new();
    collect_use_items(&parsed.items, &mut items);
    // Candidate lines per minus path (scope-filtered, single-line
    // plain items, uniform vis).
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let end = old_start + old_len.saturating_sub(1);
    let mut per_path: Vec<Vec<(u32, u32)>> = vec![Vec::new(); minus_paths.len()];
    for v in &items {
        let Some(path) = plain_use_path(v.tree) else {
            continue;
        };
        if norm_use_vis(&v.vis) != want_vis {
            continue;
        }
        let (start, last) = (v.anchor_line, v.anchor_line);
        let in_scope = match hunk.scope() {
            Scope::Exact => old_len > 0 && start == old_start && last == end.max(old_start),
            Scope::File => true,
            _ => start >= old_start && last <= end.max(old_start),
        };
        if !in_scope {
            continue;
        }
        // Single-line items only: a multi-line `use` never matches a
        // single regroup minus line.
        if !is_single_line_tree(v.tree) {
            continue;
        }
        for (i, mp) in minus_paths.iter().enumerate() {
            if path == *mp {
                per_path[i].push((v.anchor_line, v.attr_start_line));
            }
        }
    }
    for lines in per_path.iter_mut() {
        lines.sort();
        lines.dedup();
    }
    if per_path.iter().any(|l| l.is_empty()) {
        return Ok(0);
    }
    let allow_many = hunk.allow() == rsmpl_patch::Allow::Many;
    let counts: Vec<usize> = per_path.iter().map(|l| l.len()).collect();
    if !allow_many {
        if counts.iter().any(|&c| c > 1) {
            return Ok(counts.into_iter().max().unwrap_or(0));
        }
    } else if counts.windows(2).any(|w| w[0] != w[1]) {
        return Err(StructuredError::new(
            ErrorCode::MatchConflict,
            "use-regroup members match different numbers of sites (each member must occur the same number of times under `# allow=many`)",
        )
        .with_rule(&rule)
        .with_repair(StructuredError::internal_repair()));
    }
    let sets = if allow_many { counts[0] } else { 1 };
    let mut merged = 0;
    for k in 0..sets {
        let mut set: Vec<(u32, u32)> = per_path.iter().map(|l| l[k]).collect();
        set.sort();
        set.dedup();
        if set.len() != minus_paths.len() {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "use-regroup members share one line (each member must be its own `use` item)",
            )
            .with_rule(&rule)
            .with_repair(StructuredError::internal_repair()));
        }
        // Same-line items (`use a; use b;` on one line) can never merge
        // line-wise — the tuples above differ by attr span, so check the
        // lines themselves.
        {
            let mut lines: Vec<u32> = set.iter().map(|(l, _)| *l).collect();
            lines.sort();
            lines.dedup();
            if lines.len() != set.len() {
                return Err(StructuredError::new(
                    ErrorCode::MatchConflict,
                    "use-regroup members share one line (each member must be its own `use` item)",
                )
                .with_rule(&rule)
                .with_repair(StructuredError::internal_repair()));
            }
        }
        // Attribute policies are per item and checked only for items
        // with edits (same posture as the prefix rename).
        for (line, _) in &set {
            let v = items
                .iter()
                .find(|v| v.anchor_line == *line)
                .expect("regroup line resolves to its item");
            check_attrs(v.attrs, hunk.attrs_policy(), &[], &rule, "")?;
        }
        let (first_line, _) = set[0];
        let first_start = index
            .line_start(first_line)
            .ok_or_else(|| range_err(&rule, first_line))?;
        let first_end = line_end_excl_nl(original, index, first_line)
            .ok_or_else(|| range_err(&rule, first_line))?;
        let indent: String = original[first_start..first_end]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        // `# attrs=strip` deletes attribute lines above the kept item
        // (mirrors the prefix rename, once per item).
        let first_item = items
            .iter()
            .find(|v| v.anchor_line == first_line)
            .expect("regroup first line resolves");
        if hunk.attrs_policy() == AttrsPolicy::Strip && !first_item.attrs.is_empty() {
            let start = index
                .line_start(first_item.attr_start_line)
                .ok_or_else(|| range_err(&rule, first_item.attr_start_line))?;
            if first_start > start {
                edits.push(TextEdit::new(start, first_start, String::new()));
            }
        }
        edits.push(TextEdit::new(
            first_start,
            first_end,
            format!("{indent}{plus_line}"),
        ));
        for (line, attr_start) in set.iter().skip(1) {
            let del_start = index
                .line_start(*attr_start)
                .ok_or_else(|| range_err(&rule, *attr_start))?;
            // Delete through the newline; a last line without trailing
            // newline deletes to EOF instead.
            let del_end = index.line_start(line + 1).unwrap_or(original.len());
            if del_end <= del_start {
                return Err(StructuredError::new(
                    ErrorCode::RangeConflict,
                    format!("span line {line} outside file"),
                )
                .with_rule(&rule)
                .with_repair(StructuredError::internal_repair()));
            }
            edits.push(TextEdit::new(del_start, del_end, String::new()));
        }
        merged += 1;
        if !allow_many {
            break;
        }
    }
    Ok(merged)
}

/// Apply a `use`-regroup split hunk (SPEC 14.9). Returns the number of
/// split sets. Line-wise: the grouped `use` item line is replaced by the
/// N verbatim plus lines (each with the matched line's indent). Only
/// single-line grouped `use` items match; visibility must equal the
/// pattern's uniform vis (whitespace-insensitive); leading `::` is
/// normalized away on both sides.
pub fn apply_use_split_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let (prefix, members, vis, plus_lines) = match &compiled.kind {
        PatternKind::UseSplit {
            prefix,
            members,
            vis,
            plus_lines,
            ..
        } => (
            prefix.clone(),
            members.clone(),
            vis.clone(),
            plus_lines.clone(),
        ),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected use-split pattern".to_string(),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "dispatch-use-split-template",
                "write a split-only use-regroup pair (one grouped `use P::{...}` minus line plus N≥2 `use P::Ti;` plus lines; see SPEC 14.9)",
            )));
        }
    };
    let rule = hunk.rule_name();
    let want_vis: String = vis.chars().filter(|c| !c.is_whitespace()).collect();
    let mut items = Vec::new();
    collect_use_items(&parsed.items, &mut items);
    let old_start = hunk.header.old_start;
    let old_len = hunk.header.old_len;
    let end = old_start + old_len.saturating_sub(1);
    let mut candidates: Vec<(u32, u32)> = Vec::new();
    for v in &items {
        let Some((group_prefix, group_members)) = grouped_use_path(v.tree) else {
            continue;
        };
        if group_prefix != prefix || group_members != members {
            continue;
        }
        if norm_use_vis(&v.vis) != want_vis {
            continue;
        }
        let (start, last) = (v.anchor_line, v.anchor_line);
        let in_scope = match hunk.scope() {
            Scope::Exact => old_len > 0 && start == old_start && last == end.max(old_start),
            Scope::File => true,
            _ => start >= old_start && last <= end.max(old_start),
        };
        if !in_scope {
            continue;
        }
        if !is_single_line_tree(v.tree) {
            continue;
        }
        candidates.push((v.anchor_line, v.attr_start_line));
    }
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        return Ok(0);
    }
    let allow_many = hunk.allow() == rsmpl_patch::Allow::Many;
    if !allow_many && candidates.len() > 1 {
        return Ok(candidates.len());
    }
    // Same-line grouped items can never split line-wise.
    {
        let mut lines: Vec<u32> = candidates.iter().map(|(l, _)| *l).collect();
        lines.sort();
        lines.dedup();
        if lines.len() != candidates.len() {
            return Err(StructuredError::new(
                ErrorCode::MatchConflict,
                "use-split groups share one line (each group must be its own `use` item)",
            )
            .with_rule(&rule)
            .with_repair(StructuredError::internal_repair()));
        }
    }
    let sets = if allow_many { candidates.len() } else { 1 };
    let mut split = 0;
    for (line, attr_start) in candidates.iter().take(sets) {
        let v = items
            .iter()
            .find(|v| v.anchor_line == *line)
            .expect("split line resolves to its item");
        check_attrs(v.attrs, hunk.attrs_policy(), &[], &rule, "")?;
        let line_start = index
            .line_start(*line)
            .ok_or_else(|| range_err(&rule, *line))?;
        let line_end =
            line_end_excl_nl(original, index, *line).ok_or_else(|| range_err(&rule, *line))?;
        let indent: String = original[line_start..line_end]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let mut replacement = String::new();
        for (i, pl) in plus_lines.iter().enumerate() {
            if i > 0 {
                replacement.push('\n');
                replacement.push_str(&indent);
            } else {
                replacement.push_str(&indent);
            }
            replacement.push_str(pl.trim());
        }
        // `# attrs=strip` deletes attribute lines above the grouped item.
        let start = if hunk.attrs_policy() == AttrsPolicy::Strip && !v.attrs.is_empty() {
            let attr_line = index
                .line_start(*attr_start)
                .ok_or_else(|| range_err(&rule, *attr_start))?;
            if line_start > attr_line {
                attr_line
            } else {
                line_start
            }
        } else {
            line_start
        };
        let _ = attr_start;
        edits.push(TextEdit::new(start, line_end, replacement));
        split += 1;
        if !allow_many {
            break;
        }
    }
    Ok(split)
}

/// Prefix + members of a grouped `use` tree (`a::b::{X, Y}` only);
/// `None` for plain paths, globs, renames, nested groups, or empty groups.
fn grouped_use_path(tree: &syn::UseTree) -> Option<(Vec<String>, Vec<String>)> {
    let mut prefix = Vec::new();
    let mut t = tree;
    loop {
        match t {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.to_string());
                t = &p.tree;
            }
            syn::UseTree::Group(g) => {
                let mut members = Vec::new();
                for m in &g.items {
                    match m {
                        syn::UseTree::Name(n) => members.push(n.ident.to_string()),
                        _ => return None,
                    }
                }
                if members.is_empty() {
                    return None;
                }
                return Some((prefix, members));
            }
            _ => return None,
        }
    }
}

/// Full path segments of a plain `use` tree (`a::b::X` only);
/// `None` for groups, globs, renames, and qualified trees.
fn plain_use_path(tree: &syn::UseTree) -> Option<Vec<String>> {
    let mut segs = Vec::new();
    let mut t = tree;
    loop {
        match t {
            syn::UseTree::Path(p) => {
                segs.push(p.ident.to_string());
                t = &p.tree;
            }
            syn::UseTree::Name(n) => {
                segs.push(n.ident.to_string());
                return Some(segs);
            }
            _ => return None,
        }
    }
}

/// Visibility normalized for regroup comparison (all whitespace
/// removed, so `pub(crate)` equals syn's `pub (crate)` render).
fn norm_use_vis(vis: &syn::Visibility) -> String {
    let text = match vis {
        syn::Visibility::Inherited => String::new(),
        syn::Visibility::Public(_) => "pub".to_string(),
        syn::Visibility::Restricted(r) => {
            let path = r
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if r.in_token.is_some() {
                format!("pub(in {path})")
            } else {
                format!("pub({path})")
            }
        }
    };
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// True when a `use` tree sits on one line (its span starts and ends
/// on the same line). Multi-line items never match a single regroup
/// minus line.
fn is_single_line_tree(tree: &syn::UseTree) -> bool {
    use syn::spanned::Spanned;
    let span = tree.span();
    span.start().line == span.end().line
}

/// Byte offset of a 1-based line's end excluding its `\n` (EOF when the
/// last line has no trailing newline).
fn line_end_excl_nl(original: &str, index: &LineIndex, line: u32) -> Option<usize> {
    let start = index.line_start(line)?;
    let next = index
        .line_start(line + 1)
        .unwrap_or_else(|| original.len() + 1);
    if next > original.len() + 1 {
        return None;
    }
    if next == original.len() + 1 {
        return Some(original.len());
    }
    if next == start {
        return Some(start);
    }
    let bytes = original.as_bytes();
    if next > start && bytes[next - 1] == b'\n' {
        Some(next - 1)
    } else {
        Some(next)
    }
}

/// Collect file- and inline-`mod`-level `use` items.
fn collect_use_items<'a>(items: &'a [syn::Item], out: &mut Vec<UseItemView<'a>>) {
    for it in items {
        match it {
            syn::Item::Use(u) => {
                let item_span = it.span();
                out.push(UseItemView {
                    tree: &u.tree,
                    vis: u.vis.clone(),
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

    fn regroup_patch_in(range: &str, minuses: &[&str], plus: &str, extra: &str) -> String {
        // Directive-free terse regroup (SPEC 7.1: `rule=` alone marks a
        // semantic hunk). `extra` appends directive lines when non-empty.
        let mut out = format!("diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=u\n");
        if !extra.is_empty() {
            out.push_str(extra);
            if !extra.ends_with('\n') {
                out.push('\n');
            }
        }
        for m in minuses {
            out.push_str(&format!("- {m}\n"));
        }
        out.push_str(&format!("+ {plus}\n"));
        out
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

    /// WI-4 slice 2b: symbol use-prefix patches (whole-prefix or
    /// single-leading head; the engine expands the seed on both postures).
    fn symbol_use_patch(minus: &str, plus: &str, fallback: bool, range: &str) -> String {
        let fb = if fallback { "# fallback=syntax\n" } else { "" };
        let scope = if range == "@@ -1,1 @@" {
            "# scope=file\n# allow=many\n"
        } else {
            ""
        };
        format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=u\n{scope}{fb}# symbol Old=crate::old\n# symbol New=crate::newmod\n- {minus}\n+ {plus}\n"
        )
    }

    #[test]
    fn use_prefix_symbol_fallback_fires() {
        let patch = symbol_use_patch("use Old", "use New", true, "@@ -1,1 @@");
        let src = "use crate::old::Thing;\nuse crate::old::{A, B};\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::newmod::Thing;"), "got: {out}");
        assert!(out.contains("use crate::newmod::{A, B};"), "got: {out}");
        assert!(!out.contains("crate::old"), "got: {out}");
    }

    #[test]
    fn use_prefix_symbol_group_member_rewrites() {
        // A member sitting directly inside `{...}` at the matched position
        // rewrites; siblings are preserved.
        let patch = symbol_use_patch("use Old", "use New", true, "@@ -1,1 @@");
        let src = "use crate::{old::Deep, other};\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(out, "use crate::{newmod::Deep, other};\n", "got: {out}");
    }

    #[test]
    fn use_prefix_symbol_suffix_restricts() {
        // The minus suffix locates the split: only sites under the suffix
        // rewrite, and the plus suffix replaces it.
        let patch = symbol_use_patch("use Old::sub", "use New::sub", true, "@@ -1,1 @@");
        let src = "use crate::old::sub::Thing;\nuse crate::old::other::Thing;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::newmod::sub::Thing;"), "got: {out}");
        assert!(out.contains("use crate::old::other::Thing;"), "got: {out}");
    }

    #[test]
    fn use_prefix_symbol_resolved_reexport_fires() {
        // Short spelling through a one-hop `pub use` alias resolves to the
        // seed canon; the plus side emits the seed text.
        let patch = symbol_use_patch("use Old", "use New", false, "@@ -8,1 @@");
        let src = "pub mod old {\n    pub struct Thing;\n}\npub mod newmod {\n    pub struct Thing;\n}\npub use crate::old as mid;\nuse crate::mid::Thing;\n";
        let out = apply(&patch, src).unwrap();
        assert!(out.contains("use crate::newmod::Thing;"), "got: {out}");
        assert!(
            out.contains("pub use crate::old as mid;"),
            "ranged hunk must not touch line 3, got: {out}"
        );
    }

    #[test]
    fn use_prefix_symbol_wrong_module_misses() {
        let patch = symbol_use_patch("use Old", "use New", false, "@@ -1,1 @@");
        let src = "pub mod old {\n    pub struct Thing;\n}\npub mod newmod {\n    pub struct Thing;\n}\nuse crate::newmod::Thing;\n";
        let err = apply(&patch, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn use_prefix_symbol_plus_only_concrete_minus_fires() {
        // A concrete minus head with a symbol plus head: the historical
        // walk matches textually, the plus head expands to its seed.
        let patch = symbol_use_patch("use crate::old", "use New", true, "@@ -1,1 @@");
        let src = "use crate::old::Thing;\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(out, "use crate::newmod::Thing;\n", "got: {out}");
    }

    #[test]
    fn use_prefix_symbol_unresolvable_seed_fails() {
        let patch = symbol_use_patch("use Old", "use New", false, "@@ -1,1 @@")
            .replace("# symbol Old=crate::old", "# symbol Old=crate::missing");
        let err = apply(&patch, "use crate::missing::Thing;\n").unwrap_err();
        assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
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

    #[test]
    fn regroup_merges_at_first_line() {
        // Merge-only: the first member line becomes the grouped line,
        // later member lines are deleted whole.
        let patch = regroup_patch_in(
            "@@ -1,2 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "",
        );
        let src = "use a::b::X;\nuse a::b::Y;\nfn m() {}\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(out, "use a::b::{X, Y};\nfn m() {}\n", "got: {out:?}");
    }

    #[test]
    fn regroup_keeps_indent_in_inline_mod() {
        // Inline-`mod` items are collected; the replacement keeps the
        // target indent while the deleted line goes whole.
        let patch = regroup_patch_in(
            "@@ -2,2 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "",
        );
        let src = "mod outer {\n    use a::b::X;\n    use a::b::Y;\n}\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(
            out, "mod outer {\n    use a::b::{X, Y};\n}\n",
            "got: {out:?}"
        );
    }

    #[test]
    fn regroup_double_set_one_fails_many_merges() {
        let src = "use a::b::X;\nuse a::b::Y;\nuse a::b::X;\nuse a::b::Y;\nfn m() {}\n";
        let one = regroup_patch_in(
            "@@ -1,4 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "",
        );
        let err = apply(&one, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        let many = regroup_patch_in(
            "@@ -1,4 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "# allow=many\n",
        );
        let out = apply(&many, src).unwrap();
        assert_eq!(
            out, "use a::b::{X, Y};\nuse a::b::{X, Y};\nfn m() {}\n",
            "got: {out:?}"
        );
    }

    #[test]
    fn regroup_no_match_without_optional_fails() {
        let patch = regroup_patch_in(
            "@@ -1,2 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "",
        );
        let err = apply(&patch, "use a::b::X;\nuse a::b::Z;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn regroup_vis_and_tails_gated() {
        // `pub` pattern never matches plain items.
        let patch = regroup_patch_in(
            "@@ -1,2 @@",
            &["pub use a::b::X;", "pub use a::b::Y;"],
            "pub use a::b::{X, Y};",
            "",
        );
        let err = apply(&patch, "use a::b::X;\nuse a::b::Y;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        // Grouped target members never match plain minus paths.
        let plain = regroup_patch_in(
            "@@ -1,2 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "",
        );
        let err = apply(&plain, "use a::b::{X};\nuse a::b::Y;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn split_vis_and_shape_gated() {
        // `pub` pattern never matches plain items.
        let patch = split_patch_in(
            "@@ -1,1 @@",
            "pub use a::b::{X, Y};",
            &["pub use a::b::X;", "pub use a::b::Y;"],
            "",
        );
        let err = apply(&patch, "use a::b::{X, Y};\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        // Plain minus paths never match grouped targets.
        let err = apply(&patch, "use a::b::X;\nuse a::b::Y;\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    #[test]
    fn split_leaves_hetero_group_untouched() {
        // A `{X, Z}` group is not the `{X, Y}` pattern: no match, never a guess.
        let patch = split_patch_in(
            "@@ -1,1 @@",
            "use a::b::{X, Y};",
            &["use a::b::X;", "use a::b::Y;"],
            "",
        );
        let err = apply(&patch, "use a::b::{X, Z};\n").unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    }

    fn split_patch_in(range: &str, minus: &str, pluses: &[&str], extra: &str) -> String {
        let mut out = format!("diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{range} rule=u\n");
        if !extra.is_empty() {
            out.push_str(extra);
            if !extra.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str(&format!("- {minus}\n"));
        for p in pluses {
            out.push_str(&format!("+ {p}\n"));
        }
        out
    }

    #[test]
    fn split_expands_in_place_with_indent() {
        let patch = split_patch_in(
            "@@ -1,1 @@",
            "use a::b::{X, Y};",
            &["use a::b::X;", "use a::b::Y;"],
            "",
        );
        let src = "use a::b::{X, Y};\nfn m() {}\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(
            out, "use a::b::X;\nuse a::b::Y;\nfn m() {}\n",
            "got: {out:?}"
        );
    }

    #[test]
    fn split_keeps_indent_in_inline_mod() {
        let patch = split_patch_in(
            "@@ -2,1 @@",
            "use a::b::{X, Y};",
            &["use a::b::X;", "use a::b::Y;"],
            "",
        );
        let src = "mod outer {\n    use a::b::{X, Y};\n}\n";
        let out = apply(&patch, src).unwrap();
        assert_eq!(
            out, "mod outer {\n    use a::b::X;\n    use a::b::Y;\n}\n",
            "got: {out:?}"
        );
    }

    #[test]
    fn split_double_group_one_fails_many_splits() {
        let src = "use a::b::{X, Y};\nuse a::b::{X, Y};\nfn m() {}\n";
        let one = split_patch_in(
            "@@ -1,2 @@",
            "use a::b::{X, Y};",
            &["use a::b::X;", "use a::b::Y;"],
            "",
        );
        let err = apply(&one, src).unwrap_err();
        assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
        let many = split_patch_in(
            "@@ -1,2 @@",
            "use a::b::{X, Y};",
            &["use a::b::X;", "use a::b::Y;"],
            "# allow=many\n",
        );
        let out = apply(&many, src).unwrap();
        assert_eq!(
            out, "use a::b::X;\nuse a::b::Y;\nuse a::b::X;\nuse a::b::Y;\nfn m() {}\n",
            "got: {out:?}"
        );
    }

    fn wi18_use_setup(
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
        let compiled = rsmpl_pattern::compile(&hunk).expect("use patch compiles");
        let src = src.to_string();
        let parsed: syn::File = syn::parse_str(&src).expect("src parses");
        let index = crate::lines::LineIndex::new(&src);
        (hunk, compiled, parsed, index, src)
    }

    fn wi18_use_repair(err: &rsmpl_patch::StructuredError) -> (String, String) {
        let repair = err
            .repair
            .as_ref()
            .expect("WI-18 slice 10: must carry repair");
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

    #[test]
    fn wi18_slice10_use_rename_dispatch_repair_is_specific() {
        let patch = use_patch_in(
            "@@ -1,1 @@",
            "use crate::old",
            "use crate::new",
            "# allow=many\n",
        );
        let (hunk, mut compiled, parsed, index, src) =
            wi18_use_setup(&patch, "use crate::old::Thing;\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err =
            super::apply_use_rename_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits, None)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected use-path rename pattern");
        let (code, w) = wi18_use_repair(&err);
        assert_eq!(code, "dispatch-use-rename-template");
        assert!(w.contains("`- use crate::old`"), "got: {w}");
    }

    #[test]
    fn wi18_use_split_dispatch_repair_is_specific() {
        let patch = split_patch_in(
            "@@ -1,1 @@",
            "use a::b::{X, Y};",
            &["use a::b::X;", "use a::b::Y;"],
            "",
        );
        let (hunk, mut compiled, parsed, index, src) =
            wi18_use_setup(&patch, "use a::b::{X, Y};\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err = super::apply_use_split_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
            .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected use-split pattern");
        let (code, w) = wi18_use_repair(&err);
        assert_eq!(code, "dispatch-use-split-template");
        assert!(w.contains("split-only"), "got: {w}");
    }

    #[test]
    fn wi18_slice10_use_regroup_dispatch_repair_is_specific() {
        let patch = regroup_patch_in(
            "@@ -1,2 @@",
            &["use a::b::X;", "use a::b::Y;"],
            "use a::b::{X, Y};",
            "",
        );
        let (hunk, mut compiled, parsed, index, src) =
            wi18_use_setup(&patch, "use a::b::X;\nuse a::b::Y;\n");
        compiled.kind = rsmpl_pattern::PatternKind::Raw {
            minus: "x".to_string(),
            plus: "y".to_string(),
        };
        let mut edits = Vec::new();
        let err =
            super::apply_use_regroup_hunk(&parsed, &src, &index, &hunk, &compiled, &mut edits)
                .expect_err("wrong kind must fail");
        assert_eq!(err.message, "expected use-regroup merge pattern");
        let (code, w) = wi18_use_repair(&err);
        assert_eq!(code, "dispatch-use-regroup-template");
        assert!(w.contains("merge-only"), "got: {w}");
    }
}
