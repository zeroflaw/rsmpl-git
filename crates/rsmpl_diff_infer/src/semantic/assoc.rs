//! Associated-item rename raising (WI-28, SPEC D.4).
//!
//! Single-file def+uses intent: K ≥ 1 definition pairs (a `type` or
//! `const` assoc rename line in a `trait`/`impl` block, one per block)
//! plus use-site lines sharing the ident delta through a `::Old` tail
//! (`Self::Item`, `S::Item`, `Tr::Item`, `<S as Tr>::Item`). Emission
//! is one definition hunk per enclosing block (verbatim header lines,
//! shared rule) with the first hunk carrying `# follow=uses` — the
//! WI-13 (1)+(3) authored shape. The follow rewrite does the rest at
//! apply time.
//!
//! Totality is a scratch-apply line-equality probe (the `member.rs`
//! posture): the authored emission must reproduce exactly the native
//! plus lines at the changed sites with every other line
//! byte-identical. A bare-`Old` use (no `::` tail) is opaque — the
//! engine vetoes the follow — so any such pair declines the section.
//! Two-trait same-name classes decline (two intents, never grouped).
//! `None` pre-image declines (the block proof needs it). Cross-file
//! intents stay ordinary (WI-15).

use super::unique_rule;

/// One classified associated-item definition rename pair.
struct AssocDef {
    /// `true` for `type`, `false` for `const`.
    is_type: bool,
    old: String,
    new: String,
}

/// Whole-section assoc rename: K ≥ 1 definition pairs in trait/impl
/// blocks plus `::Old`/`::New` use-site lines sharing one ident delta.
/// Returns the multi-hunk follow emission, or `None` when the section
/// is not exactly this shape.
pub(crate) fn try_assoc_rename(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if changed.len() != minuses.len() {
        return None;
    }
    let pre = file_old?;
    // Classify every pair: assoc definition rename or `::`-tail use.
    // A second spelling of definition in one pair is impossible by
    // construction (parse decides once).
    let mut defs: Vec<Option<AssocDef>> = Vec::with_capacity(minuses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        defs.push(parse_assoc_def_pair(m, p));
    }
    let first_def = defs.iter().flatten().next()?;
    let (is_type, old, new) = (
        first_def.is_type,
        first_def.old.clone(),
        first_def.new.clone(),
    );
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    let mut def_idx: Vec<usize> = Vec::new();
    for (i, d) in defs.iter().enumerate() {
        match d {
            Some(d) => {
                if d.is_type != is_type || d.old != old || d.new != new {
                    return None;
                }
                def_idx.push(i);
            }
            None => {
                // Use-site gate: the minus names `::old`, the plus
                // names `::new` (ident-boundary, never substring), and
                // the pair is not itself a definition.
                let (m, p) = (minuses[i], pluses[i]);
                if m.trim() == p.trim() {
                    return None;
                }
                if !contains_path_tail(m, &old) || !contains_path_tail(p, &new) {
                    return None;
                }
                if !contains_ident(m, &old) || !contains_ident(p, &new) {
                    return None;
                }
            }
        }
    }
    if def_idx.is_empty() {
        return None;
    }
    // Every def site must sit in a trait or impl block; recover its
    // verbatim header plus block span. Use lines need no block proof
    // (the engine's follow classifies them; the probe below is
    // authoritative).
    let pre_lines: Vec<&str> = pre.lines().collect();
    let mut def_header: Vec<String> = Vec::new();
    let mut def_span: Vec<(usize, usize)> = Vec::new();
    for &i in def_idx.iter() {
        let (header, start, end) = enclosing_block(pre, changed[i])?;
        // The trimmed native def line must match its minus side
        // (trailing `,` tolerated — patterns omit member commas).
        let pre_line = pre_lines.get(changed[i].saturating_sub(1))?;
        let want = minuses[i].trim();
        let got = pre_line.trim();
        if got != want
            && !(got == format!("{want},").as_str()
                || got.trim_end_matches(',').trim() == want.trim_end_matches(',').trim())
        {
            return None;
        }
        def_header.push(header);
        def_span.push((start, end));
    }
    // Two-trait veto: distinct named traits across def headers mean two
    // intents (the engine follow would skip one side's uses).
    {
        let mut traits: Vec<String> = Vec::new();
        for h in &def_header {
            if let Some(t) = block_trait_name(h) {
                if !traits.contains(&t) {
                    traits.push(t);
                }
            }
        }
        if traits.len() > 1 {
            return None;
        }
    }
    // Group def sites by verbatim header (document order); one def per
    // block (a duplicate same-name def in one block is ambiguous).
    // Each hunk's range covers its whole block: the assoc matcher
    // filters by header and scopes candidates by range, so a def-line
    // range (the `covering_item_span` smallest item) would exclude the
    // context header the pattern needs.
    let mut groups: Vec<(String, (usize, usize), Vec<usize>)> = Vec::new();
    for ((k, h), span) in def_idx.iter().zip(def_header.iter()).zip(def_span.iter()) {
        match groups.iter_mut().find(|(gh, _, _)| gh == h) {
            Some(g) => {
                g.2.push(*k);
                if g.2.len() > 1 {
                    return None;
                }
            }
            None => groups.push((h.clone(), *span, vec![*k])),
        }
    }
    groups.sort_by_key(|(_, _, idx)| changed[idx[0]]);
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    // WI-3: the follow hunk alone suffices (WI-1 claims impl definitions
    // as `rename` spans). Emit the document-first def hunk with follow;
    // the scratch probe proves the sibling defs + uses. Multi-hunk is the
    // fallback when the single-hunk probe fails (twin headers, etc.).
    let (first_header, first_span, first_idx) = groups.first().cloned().unwrap();
    let k = first_idx[0];
    let mut single = String::new();
    single.push_str(&crate::emission_header(
        path,
        &rule,
        &[first_span.0, first_span.1],
    ));
    single.push_str("# follow=uses\n");
    single.push_str(&format!("  {first_header}\n"));
    single.push_str(&format!("- {}\n", minuses[k].trim()));
    single.push_str(&format!("+ {}\n", pluses[k].trim()));
    if scratch_reproduces(pre, path, &single, &pre_lines, changed, pluses) {
        return Some(single);
    }
    let mut out = String::new();
    // Fallback: one hunk per block, follow on the first only (a follow on
    // every hunk overlaps the siblings' covering ranges, so the engine
    // vetoes multi-follow as overlapping edits). Definition-only hunks
    // report no orphans of their own — `plan.rs` suppresses the no-follow
    // orphan arm when a same-file, same-rule sibling carries the follow.
    for (gi, (header, span, idx)) in groups.iter().enumerate() {
        let k = idx[0];
        out.push_str(&crate::emission_header(path, &rule, &[span.0, span.1]));
        if gi == 0 {
            out.push_str("# follow=uses\n");
        }
        out.push_str(&format!("  {header}\n"));
        out.push_str(&format!("- {}\n", minuses[k].trim()));
        out.push_str(&format!("+ {}\n", pluses[k].trim()));
    }
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.pop();
        return None;
    }
    Some(out)
}

/// A `-`/`+` line pair as an assoc definition rename: both sides parse
/// as the same `type`/`const` assoc shape, names differ, and the
/// normalized remainder is equal. Trailing `,` tolerated.
fn parse_assoc_def_pair(minus: &str, plus: &str) -> Option<AssocDef> {
    let m = minus.trim().trim_end_matches(',').trim();
    let p = plus.trim().trim_end_matches(',').trim();
    if m.is_empty() || p.is_empty() {
        return None;
    }
    if let (Some(a), Some(b)) = (parse_type_side(m), parse_type_side(p)) {
        if a.0 == b.0 || a.0.is_empty() || a.1 != b.1 {
            return None;
        }
        return Some(AssocDef {
            is_type: true,
            old: a.0,
            new: b.0,
        });
    }
    if let (Some(a), Some(b)) = (parse_const_side(m), parse_const_side(p)) {
        if a.0 == b.0 || a.0.is_empty() || a.1 != b.1 || a.2 != b.2 {
            return None;
        }
        return Some(AssocDef {
            is_type: false,
            old: a.0,
            new: b.0,
        });
    }
    None
}

/// Parse a `type` assoc line into (name, normalized rest).
fn parse_type_side(line: &str) -> Option<(String, String)> {
    let t = line.strip_prefix("type ")?.trim_start();
    if t.is_empty() {
        return None;
    }
    // Trait-form first (`type Item;`, `type Item = T;`), then impl-form
    // (`type Item = T;`).
    if let Ok(it) = syn::parse_str::<syn::TraitItemType>(line) {
        let generics = &it.generics;
        let bounds = &it.bounds;
        let rest = format!(
            "{}|{}|{}",
            norm(&quote::quote!(#generics)),
            it.colon_token
                .as_ref()
                .map(|_| norm(&quote::quote!(#bounds)))
                .unwrap_or_default(),
            it.default
                .as_ref()
                .map(|(_, ty)| norm(&quote::quote!(#ty)))
                .unwrap_or_default(),
        );
        return Some((it.ident.to_string(), rest));
    }
    if let Ok(it) = syn::parse_str::<syn::ImplItemType>(line) {
        let generics = &it.generics;
        let ty = &it.ty;
        let rest = format!(
            "{}|{}",
            norm(&quote::quote!(#generics)),
            norm(&quote::quote!(#ty))
        );
        return Some((it.ident.to_string(), rest));
    }
    None
}

/// Parse a `const` assoc line into (name, normalized type, normalized value).
fn parse_const_side(line: &str) -> Option<(String, String, String)> {
    let t = line.strip_prefix("const ")?.trim_start();
    // A `const fn` method header is not an assoc pair.
    let head = t
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("");
    if matches!(head, "fn" | "unsafe" | "async" | "extern") {
        return None;
    }
    if let Ok(it) = syn::parse_str::<syn::TraitItemConst>(line) {
        let ty = &it.ty;
        let rest_ty = norm(&quote::quote!(#ty));
        let rest_val = it
            .default
            .as_ref()
            .map(|(_, e)| norm(&quote::quote!(#e)))
            .unwrap_or_default();
        return Some((it.ident.to_string(), rest_ty, rest_val));
    }
    if let Ok(it) = syn::parse_str::<syn::ImplItemConst>(line) {
        let ty = &it.ty;
        let expr = &it.expr;
        let rest_ty = norm(&quote::quote!(#ty));
        let rest_val = norm(&quote::quote!(#expr));
        return Some((it.ident.to_string(), rest_ty, rest_val));
    }
    None
}

fn norm(t: &impl quote::ToTokens) -> String {
    quote::quote!(#t)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Verbatim header line plus span of the trait/impl block enclosing
/// 1-based `site` (top-level and inline-mod items), or `None` when the
/// site names no block.
fn enclosing_block(pre: &str, site: usize) -> Option<(String, usize, usize)> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    fn walk(items: &[syn::Item], blocks: &mut Vec<(usize, usize)>) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Trait(t) => {
                    blocks.push((t.span().start().line, t.span().end().line));
                }
                syn::Item::Impl(imp) => {
                    blocks.push((imp.span().start().line, imp.span().end().line));
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, blocks);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, &mut blocks);
    let mut best: Option<(usize, usize)> = None;
    for b in &blocks {
        if b.0 <= site
            && site <= b.1
            && best.is_none_or(|prev: (usize, usize)| b.1 - b.0 < prev.1 - prev.0)
        {
            best = Some(*b);
        }
    }
    let (start, end) = best?;
    let lines: Vec<&str> = pre.lines().collect();
    // The block's first line is its header (attributes never start a
    // `syn` block span; the span starts at the keyword or its attrs —
    // scan forward to the header line defensively).
    for line in lines.iter().skip(start.saturating_sub(1)).take(4) {
        let t = line.trim();
        if is_block_header_line(t) {
            return Some((t.to_string(), start, end));
        }
    }
    None
}

/// True for `trait … {` / `impl … {` header lines (incl. `unsafe`).
fn is_block_header_line(trimmed: &str) -> bool {
    if !trimmed.ends_with('{') {
        return false;
    }
    let s = trimmed.strip_prefix("unsafe ").unwrap_or(trimmed);
    if s.starts_with("trait ") || s.starts_with("trait<") {
        return true;
    }
    s == "impl" || s.starts_with("impl ") || s.starts_with("impl<") || s.starts_with("impl{")
}

/// The trait named by a block header: `trait Tr` → `Tr`,
/// `impl Tr for S` → `Tr`, inherent `impl S` → `None`.
fn block_trait_name(header: &str) -> Option<String> {
    let item: syn::Item = syn::parse_str(&format!("{header}}}")).ok()?;
    match item {
        syn::Item::Trait(t) => Some(t.ident.to_string()),
        syn::Item::Impl(imp) => imp
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last().map(|s| s.ident.to_string())),
        _ => None,
    }
}

/// Ident-boundary containment (never substring: `old` must not match
/// `older`).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    let mut hit = false;
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                hit = true;
                break;
            }
            token.clear();
        }
    }
    hit
}

/// True when `haystack` holds `::needle` with an ident boundary after
/// the name (assoc use tails are always path tails).
fn contains_path_tail(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let pat = format!("::{needle}");
    let mut rest = haystack;
    while let Some(i) = rest.find(pat.as_str()) {
        let after = rest[i + pat.len()..].chars().next();
        if after.is_none_or(|c| c != '_' && !c.is_ascii_alphanumeric()) {
            return true;
        }
        rest = &rest[i + pat.len()..];
    }
    false
}

/// WI-19b def-only single: exactly one assoc `type`/`const` definition
/// pair in a trait/impl block, no use sites, no `# follow=uses` (a
/// follow would rewrite uses the diff did not touch). Same block proof
/// + scratch line-equality probe as the follow path, def hunk only.
pub(crate) fn try_assoc_def_only(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let def = parse_assoc_def_pair(minuses[0], pluses[0])?;
    if def.old == def.new || def.old.is_empty() {
        return None;
    }
    let (header, start, end) = enclosing_block(pre, changed[0])?;
    let pre_lines: Vec<&str> = pre.lines().collect();
    let pre_line = pre_lines.get(changed[0].saturating_sub(1))?;
    let want = minuses[0].trim();
    let got = pre_line.trim();
    if got != want
        && got != format!("{want},").as_str()
        && got.trim_end_matches(',').trim() != want.trim_end_matches(',').trim()
    {
        return None;
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("rename_{}_to_{}", def.old, def.new));
    // WI-19b: concrete def-only hunk, no directives (59/77 doctrine:
    // default `allow=one` matches the one proven site).
    let mut out = String::new();
    out.push_str(&crate::emission_header(path, &rule, &[start, end]).replace("# allow=many\n", ""));
    out.push_str(&format!("  {header}\n"));
    out.push_str(&format!("- {}\n", minuses[0].trim()));
    out.push_str(&format!("+ {}\n", pluses[0].trim()));
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some((out, "assoc_rename"))
}

/// Apply `section` to `pre` in memory and require exact line equality:
/// every `changed[i]` post line trims to `pluses[i]`, every other line
/// is byte-identical to `pre`.
fn scratch_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    pre_lines: &[&str],
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    let parsed = match rsmpl_patch::parse_patch(section) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("SCRATCH parse fail: {e:?}");
            return false;
        }
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    if let Err(e) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) {
        eprintln!("SCRATCH apply fail: {}", e.render());
        eprintln!("section was:\n{section}");
        return false;
    }
    let post = &files[path];
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() {
        return false;
    }
    let changed_set: std::collections::HashSet<usize> = changed.iter().copied().collect();
    for (i, &site) in changed.iter().enumerate() {
        let Some(post_line) = post_lines.get(site.saturating_sub(1)) else {
            return false;
        };
        if post_line.trim() != pluses[i].trim() {
            return false;
        }
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if changed_set.contains(&(idx + 1)) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    const ASSOC_PRE: &str = "trait Tr {\n    type Item;\n    fn make() -> Self::Item;\n}\nstruct S;\nimpl Tr for S {\n    type Item = u8;\n    fn make() -> Self::Item {\n        Self::Item::default()\n    }\n}\nstruct T;\nimpl Tr for T {\n    type Item = u16;\n    fn make() -> S::Item {\n        S::Item::default()\n    }\n}\n";

    fn assoc_section() -> String {
        section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
                "@@ -7,1 +7,1 @@\n",
                "-    type Item = u8;\n",
                "+    type Output = u8;\n",
                "@@ -14,1 +14,1 @@\n",
                "-    type Item = u16;\n",
                "+    type Output = u16;\n",
                "@@ -3,1 +3,1 @@\n",
                "-    fn make() -> Self::Item;\n",
                "+    fn make() -> Self::Output;\n",
                "@@ -8,1 +8,1 @@\n",
                "-    fn make() -> Self::Item {\n",
                "+    fn make() -> Self::Output {\n",
                "@@ -9,1 +9,1 @@\n",
                "-        Self::Item::default()\n",
                "+        Self::Output::default()\n",
                "@@ -15,1 +15,1 @@\n",
                "-    fn make() -> S::Item {\n",
                "+    fn make() -> S::Output {\n",
                "@@ -16,1 +16,1 @@\n",
                "-        S::Item::default()\n",
                "+        S::Output::default()\n",
            ),
        )
    }

    #[test]
    fn def_pair_parses() {
        let d = super::parse_assoc_def_pair("    type Item;", "    type Output;").expect("def");
        assert_eq!(d.old, "Item");
        assert!(d.is_type);
        assert!(super::parse_assoc_def_pair("    type Item;", "    type Item;").is_none());
        assert!(
            super::parse_assoc_def_pair("    type Item = u8;", "    type Output = u16;").is_none()
        );
        let c =
            super::parse_assoc_def_pair("    const MAX: i32 = 10;", "    const LIMIT: i32 = 10;")
                .expect("const def");
        assert!(!c.is_type);
        assert!(super::parse_assoc_def_pair("    const fn f();", "    const fn g();").is_none());
        assert!(super::parse_assoc_def_pair("    old: u32,", "    new: u32,").is_none());
    }

    #[test]
    fn trait_impl_uses_fires_and_round_trips() {
        let raw = assoc_section();
        let pre = pre_map(&[("f.rs", ASSOC_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("assoc rename must compress");
        assert!(out.contains("trait Tr {"), "got:\n{out}");
        assert!(out.contains("- type Item;"), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        // WI-3: single follow hunk owns the impl definitions (was 3).
        assert_eq!(out.matches("rule=r1").count(), 1, "got:\n{out}");
        validated_live(&out, &pre);
        // WI-28 sibling suppression: the def-only hunks report no
        // orphans — the follow hunk owns the use set.
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            report.rules.iter().all(|r| r.orphans.is_empty()),
            "got: {:?}",
            report.rules.iter().map(|r| &r.orphans).collect::<Vec<_>>()
        );
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        let post = &files["f.rs"];
        assert!(post.contains("type Output;"), "got: {post}");
        assert!(post.contains("type Output = u8;"), "got: {post}");
        assert!(post.contains("Self::Output"), "got: {post}");
        assert!(!post.contains("Item"), "got: {post}");
    }

    #[test]
    fn inherent_const_fires() {
        let pre =
            "struct S;\nimpl S {\n    const MAX: i32 = 10;\n}\nfn f() -> i32 {\n    S::MAX\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -3,1 +3,1 @@\n",
                "-    const MAX: i32 = 10;\n",
                "+    const LIMIT: i32 = 10;\n",
                "@@ -6,1 +6,1 @@\n",
                "-    S::MAX\n",
                "+    S::LIMIT\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("inherent assoc rename must compress");
        assert!(out.contains("impl S {"), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn bare_old_use_declines() {
        // A bare `Old` use is opaque to the engine follow: the probe
        // would fail, so the section stays ordinary.
        let pre = "trait Tr {\n    type Item;\n}\nfn f() -> Tr::Item {\n    Item\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
                "@@ -5,1 +5,1 @@\n",
                "-    Item\n",
                "+    Output\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map);
        assert!(
            out.is_none() || !out.unwrap().contains("follow=uses"),
            "bare uses must not raise with follow"
        );
    }

    #[test]
    fn two_trait_same_name_declines_whole_section() {
        // WI-19b: two same-name defs in different traits are two
        // intents — the whole-section follow detector declines. Per-hunk,
        // each def is a sound def-only single (no follow, no ambiguity),
        // so the pipeline claims both and round-trips.
        let pre = "trait A {\n    type Item;\n}\ntrait B {\n    type Item;\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
                "@@ -5,1 +5,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let mut used = Vec::new();
        assert!(super::try_assoc_rename(
            &["    type Item;", "    type Item;"],
            &["    type Output;", "    type Output;"],
            "f.rs",
            Some(pre),
            &[2, 5],
            &mut used,
        )
        .is_none());
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("per-hunk def-only singles must fire");
        assert!(!out.contains("follow=uses"), "got:\n{out}");
        validated_live(&out, &pre_map);
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn def_only_single_fires_and_round_trips() {
        // WI-19b: a lone assoc `type` definition (uses untouched)
        // compresses to the def-only hunk with no follow.
        let pre =
            "trait Tr {\n    type Item;\n}\nstruct S;\nimpl Tr for S {\n    type Item = u8;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    type Item;\n+    type Output;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("assoc def-only must fire");
        assert!(out.contains("type Item;"), "got:\n{out}");
        assert!(out.contains("type Output;"), "got:\n{out}");
        assert!(!out.contains("follow=uses"), "got:\n{out}");
        validated_live(&out, &pre_map);
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn def_only_use_line_declines() {
        // A lone `::Item` use line is not a definition.
        let pre = "trait Tr {\n    type Item;\n    fn make() -> Self::Item;\n}\n";
        let mut used = Vec::new();
        assert!(super::try_assoc_def_only(
            &["    fn make() -> Self::Item;"],
            &["    fn make() -> Self::Output;"],
            "f.rs",
            Some(pre),
            &[3],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn hetero_kind_declines_whole_section() {
        // WI-19b: hetero kinds are never one follow intent — the
        // whole-section detector declines. Per-hunk, each def is a
        // sound def-only single, so the pipeline claims both.
        let pre = "trait Tr {\n    type Item;\n    const Item: i32;\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
                "@@ -3,1 +3,1 @@\n",
                "-    const Item: i32;\n",
                "+    const Output: i32;\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let mut used = Vec::new();
        assert!(super::try_assoc_rename(
            &["    type Item;", "    const Item: i32;"],
            &["    type Output;", "    const Output: i32;"],
            "f.rs",
            Some(pre),
            &[2, 3],
            &mut used,
        )
        .is_none());
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("per-hunk def-only singles must fire");
        assert!(!out.contains("follow=uses"), "got:\n{out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn shadowed_block_passes_probe_untouched() {
        // A second trait defining the same assoc name stays untouched:
        // the follow skips it `shadowed` and the probe still passes.
        let pre = "trait Tr {\n    type Item;\n    fn make() -> Self::Item;\n}\nstruct S;\nimpl Tr for S {\n    type Item = u8;\n    fn make() -> Self::Item {\n        1\n    }\n}\ntrait Other {\n    type Item;\n    fn mk() -> Self::Item;\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    type Item;\n",
                "+    type Output;\n",
                "@@ -7,1 +7,1 @@\n",
                "-    type Item = u8;\n",
                "+    type Output = u8;\n",
                "@@ -3,1 +3,1 @@\n",
                "-    fn make() -> Self::Item;\n",
                "+    fn make() -> Self::Output;\n",
                "@@ -8,1 +8,1 @@\n",
                "-    fn make() -> Self::Item {\n",
                "+    fn make() -> Self::Output {\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("shadowed-untouched classes must compress");
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre_map.clone();
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        let details = &report.rules[0].follow_skipped_details;
        assert!(
            details.iter().any(|d| d.reason == "shadowed"),
            "got: {details:?}"
        );
        assert!(
            files["f.rs"].contains("trait Other {\n    type Item;\n    fn mk() -> Self::Item;\n}")
        );
    }
}
