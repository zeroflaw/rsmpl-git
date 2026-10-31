//! Verbatim per-block impl/trait-context emission (WI-4a engine
//! filter + WI-4b trait-side grouping).
//!
//! Under the WI-4a header filter the engine matches only the named
//! block, so a multi-block method sweep cannot share one advisory
//! `impl Svc` hunk: it emits one hunk per enclosing `impl` block with
//! the block's verbatim header line from the pre-image (WI-4b adds the
//! `trait` half: one hunk per enclosing trait block the same way).
//! Grouping walks
//! the pre-image parse tree (top-level and inline-mod items); the
//! engine probe replays each emitted hunk on a scratch tree so twin
//! headers and untouched twins veto instead of mis-rewriting.

use syn::spanned::Spanned;

/// One enclosing `impl` block: its verbatim header source line plus the
/// changed 1-based site lines it contains, in document order.
pub(crate) struct BlockGroup {
    pub header_line: String,
    pub sites: Vec<usize>,
}

/// Word-bounded `impl`-header line: `impl … {` or `unsafe impl … {`,
/// already trimmed. Attribute lines and comments never match.
fn is_impl_header_line(trimmed: &str) -> bool {
    let rest = trimmed.strip_prefix("unsafe ").unwrap_or(trimmed);
    rest == "impl"
        || rest.starts_with("impl ")
        || rest.starts_with("impl<")
        || rest.starts_with("impl{")
}

/// Group changed 1-based lines by enclosing `impl` block (top-level
/// and inline-mod items, smallest containing span wins). Returns the
/// groups in document order with each block's verbatim header source
/// line. `None` when the pre-image is absent/unparseable, a site lies
/// outside every `impl` block (trait blocks and free fns never group —
/// the indented slot/sig-evolution classes are impl-only), or a header
/// line cannot be read back (callers decline to ordinary).
pub(crate) fn group_sites_by_impl_block(
    file_old: Option<&str>,
    changed: &[usize],
) -> Option<Vec<BlockGroup>> {
    let pre = file_old?;
    let file: syn::File = syn::parse_str(pre).ok()?;
    // (impl_token_line, span_start, span_end) per `impl` block.
    let mut blocks: Vec<(usize, usize, usize)> = Vec::new();
    fn walk(items: &[syn::Item], blocks: &mut Vec<(usize, usize, usize)>) {
        for it in items {
            match it {
                syn::Item::Impl(imp) => {
                    blocks.push((
                        imp.impl_token.span().start().line,
                        imp.span().start().line,
                        imp.span().end().line,
                    ));
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
    let lines: Vec<&str> = pre.lines().collect();
    let mut groups: Vec<BlockGroup> = Vec::new();
    for &site in changed {
        let mut best: Option<(usize, usize, usize)> = None;
        for b in &blocks {
            if b.1 <= site
                && site <= b.2
                && best.is_none_or(|prev: (usize, usize, usize)| b.2 - b.1 < prev.2 - prev.1)
            {
                best = Some(*b);
            }
        }
        let (header_no, _, _) = best?;
        let raw = lines.get(header_no.wrapping_sub(1))?;
        // `impl_token` names the keyword line, but a defensive scan
        // skips attribute lines when the span starts above the header.
        let mut header_line = raw.to_string();
        if !is_impl_header_line(raw.trim_start()) {
            header_line = lines
                .iter()
                .skip(header_no)
                .map(|l| l.to_string())
                .find(|l| is_impl_header_line(l.trim_start()))?;
        }
        match groups.iter_mut().find(|g| g.header_line == header_line) {
            Some(g) => g.sites.push(site),
            None => groups.push(BlockGroup {
                header_line,
                sites: vec![site],
            }),
        }
    }
    if groups.is_empty() {
        return None;
    }
    Some(groups)
}

/// One enclosing `trait` block: its verbatim header source line plus
/// the changed 1-based site lines it contains, in document order.
pub(crate) struct TraitBlockGroup {
    pub header_line: String,
    pub sites: Vec<usize>,
}

/// Word-bounded `trait`-header line: `trait … {` or
/// `unsafe trait … {`, already trimmed. Attribute lines and comments
/// never match.
fn is_trait_header_line(trimmed: &str) -> bool {
    let rest = trimmed.strip_prefix("unsafe ").unwrap_or(trimmed);
    rest == "trait"
        || rest.starts_with("trait ")
        || rest.starts_with("trait<")
        || rest.starts_with("trait{")
}

/// Group changed 1-based lines by enclosing `trait` block (top-level
/// and inline-mod items, smallest containing span wins). Returns the
/// groups in document order with each block's verbatim header source
/// line. `None` when the pre-image is absent/unparseable, a site lies
/// outside every `trait` block, or a header line cannot be read back
/// (callers decline to ordinary).
pub(crate) fn group_sites_by_trait_block(
    file_old: Option<&str>,
    changed: &[usize],
) -> Option<Vec<TraitBlockGroup>> {
    let pre = file_old?;
    let file: syn::File = syn::parse_str(pre).ok()?;
    // (trait_keyword_line, span_start, span_end) per `trait` block.
    let mut blocks: Vec<(usize, usize, usize)> = Vec::new();
    fn walk(items: &[syn::Item], blocks: &mut Vec<(usize, usize, usize)>) {
        for it in items {
            match it {
                syn::Item::Trait(tr) => {
                    blocks.push((
                        tr.trait_token.span().start().line,
                        tr.span().start().line,
                        tr.span().end().line,
                    ));
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
    let lines: Vec<&str> = pre.lines().collect();
    let mut groups: Vec<TraitBlockGroup> = Vec::new();
    for &site in changed {
        let mut best: Option<(usize, usize, usize)> = None;
        for b in &blocks {
            if b.1 <= site
                && site <= b.2
                && best.is_none_or(|prev: (usize, usize, usize)| b.2 - b.1 < prev.2 - prev.1)
            {
                best = Some(*b);
            }
        }
        let (header_no, _, _) = best?;
        let raw = lines.get(header_no.wrapping_sub(1))?;
        // `trait_token` names the keyword line, but a defensive scan
        // skips attribute lines when the span starts above the header.
        let mut header_line = raw.to_string();
        if !is_trait_header_line(raw.trim_start()) {
            header_line = lines
                .iter()
                .skip(header_no)
                .map(|l| l.to_string())
                .find(|l| is_trait_header_line(l.trim_start()))?;
        }
        match groups.iter_mut().find(|g| g.header_line == header_line) {
            Some(g) => g.sites.push(site),
            None => groups.push(TraitBlockGroup {
                header_line,
                sites: vec![site],
            }),
        }
    }
    if groups.is_empty() {
        return None;
    }
    Some(groups)
}
/// Render one per-block hunk body (context marker +
/// verbatim source lines), shared by emitters and the probe below so
/// the probed text is byte-identical to the emitted text. The header
/// line is verbatim source (trailing `{` included), so no brace is
/// added — only the marker prefix.
pub(crate) fn block_hunk_body(header_line: &str, minus_fn: &str, plus_fn: &str) -> String {
    format!(
        "  {header_line}\n      ...\n-     {minus_fn}\n+     {plus_fn}\n          ...\n      }}\n  }}\n"
    )
}

/// Engine totality probe for one emitted block hunk: replay the exact
/// emission (header filter + fn pair) on a scratch tree and require
/// the match count to equal `n`. Twin headers (`impl A` twice in one
/// file) and untouched same-name twins count extra and veto; probe
/// parse/apply failure vetoes. Conservative by construction: the
/// `# sweep` probe counts the whole block, so an unchanged same-name
/// method in the same block vetoes even a ranged emission that would
/// not touch it (the `slot_holds` posture).
pub(crate) fn probe_block_hunk(
    path: &str,
    file_old: &str,
    header_line: &str,
    minus_fn: &str,
    plus_fn: &str,
    n: usize,
) -> bool {
    if n == 0 {
        return false;
    }
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n{}",
        crate::git_header(path),
        block_hunk_body(header_line, minus_fn, plus_fn)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), file_old.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRE: &str = "struct A;\nstruct B;\nimpl A {\n    fn old(&self) {}\n}\nimpl B {\n    fn old(&self) {}\n}\n";

    #[test]
    fn groups_sites_by_enclosing_block() {
        let groups = group_sites_by_impl_block(Some(PRE), &[4, 7]).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].header_line, "impl A {");
        assert_eq!(groups[0].sites, vec![4]);
        assert_eq!(groups[1].header_line, "impl B {");
        assert_eq!(groups[1].sites, vec![7]);
    }

    #[test]
    fn same_block_sites_share_one_group() {
        let pre = "impl A {\n    fn old(&self) {}\n    fn old2(&self) {}\n}\n";
        let groups = group_sites_by_impl_block(Some(pre), &[2, 3]).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].header_line, "impl A {");
        assert_eq!(groups[0].sites, vec![2, 3]);
    }

    #[test]
    fn trait_block_site_declines() {
        let pre = "trait T {\n    fn old(&self);\n}\n";
        assert!(group_sites_by_impl_block(Some(pre), &[2]).is_none());
    }

    #[test]
    fn groups_sites_by_enclosing_trait_block() {
        // WI-4b: trait-side grouping mirrors the impl half — verbatim
        // headers, document order, smallest-span wins.
        let pre = "trait T {\n    fn old(&self);\n}\ntrait U<T: Clone> {\n    fn old(&self);\n}\n";
        let groups = group_sites_by_trait_block(Some(pre), &[2, 5]).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].header_line, "trait T {");
        assert_eq!(groups[0].sites, vec![2]);
        assert_eq!(groups[1].header_line, "trait U<T: Clone> {");
        assert_eq!(groups[1].sites, vec![5]);
    }

    #[test]
    fn impl_block_site_declines_trait_grouping() {
        let pre = "struct S;\nimpl S {\n    fn old(&self) {}\n}\n";
        assert!(group_sites_by_trait_block(Some(pre), &[3]).is_none());
    }

    #[test]
    fn trait_decl_probe_fires_and_wrong_header_vetoes() {
        // WI-4b: the shared probe replays trait hunks too — the `;`
        // decl pair counts exactly its block; a wrong header matches
        // nothing; twin trait headers veto a single-block emission.
        let pre =
            "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl S {\n    fn old(&self) {}\n}\n";
        assert!(probe_block_hunk(
            "f.rs",
            pre,
            "trait T {",
            "V Q fn old G (...) W ;",
            "V Q fn new G (...) W ;",
            1
        ));
        assert!(!probe_block_hunk(
            "f.rs",
            pre,
            "trait ZZZ {",
            "V Q fn old G (...) W ;",
            "V Q fn new G (...) W ;",
            1
        ));
        let twins = "trait T {\n    fn old(&self);\n}\ntrait T {\n    fn old(&self);\n}\n";
        assert!(!probe_block_hunk(
            "f.rs",
            twins,
            "trait T {",
            "V Q fn old G (...) W ;",
            "V Q fn new G (...) W ;",
            1
        ));
    }

    #[test]
    fn twin_headers_probe_vetoes() {
        // Two `impl A` blocks: the probe counts both, vetoing a
        // single-block emission.
        let pre =
            "struct A;\nimpl A {\n    fn old(&self) {}\n}\nimpl A {\n    fn old(&self) {}\n}\n";
        assert!(!probe_block_hunk(
            "f.rs",
            pre,
            "impl A {",
            "fn old(...) {",
            "fn new(...) {",
            1
        ));
        assert!(probe_block_hunk(
            "f.rs",
            pre,
            "impl A {",
            "fn old(...) {",
            "fn new(...) {",
            2
        ));
    }

    #[test]
    fn verbatim_header_probe_fires() {
        assert!(probe_block_hunk(
            "f.rs",
            PRE,
            "impl A {",
            "fn old(...) {",
            "fn new(...) {",
            1
        ));
        // Wrong header matches nothing by construction.
        assert!(!probe_block_hunk(
            "f.rs",
            PRE,
            "impl ZZZ {",
            "fn old(...) {",
            "fn new(...) {",
            1
        ));
    }
}
