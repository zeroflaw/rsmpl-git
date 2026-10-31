//! WI-3: `# follow=uses` for trait-method definition renames (SPEC 14.8).
//!
//! Minimal definition follow only (no call-site anchoring — deferred).
//! For a trait-header `ImplContext` rename `old` → `new`, every
//! `impl <Trait> for <Self>` block whose trait tail names the renamed
//! trait contributes its same-name `ImplItem::Fn` definition ident as a
//! *definition claim* (rewritten, labelled `rename`). Inherent impls
//! (`impl S { fn old }` with no trait) never claim — they are a different
//! item and become `shadowed` evidence. Call sites are untouched in v1
//! (ordinary hunks own them); the follow never vetoes on them.

use syn::spanned::Spanned;
use syn::visit::{self, Visit};

use crate::follow_reasons::{reason as skip_reason, FollowSkip};
use crate::lines::LineIndex;
use crate::matcher_item::span_bytes;
use crate::use_graph::FollowVeto;

/// Last `::` segment, cut at generics/whitespace (`"S < T >"` → `"S"`).
fn last_ident(normed: &str) -> String {
    let seg = normed.rsplit("::").next().unwrap_or(normed).trim();
    let mut s = String::new();
    for c in seg.chars() {
        if c.is_alphanumeric() || c == '_' {
            s.push(c);
        } else {
            break;
        }
    }
    s
}

fn norm_path(path: &syn::Path) -> String {
    quote::quote!(#path)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Trait name from a trait-header filter line (`trait Tr {` → `Tr`).
/// `None` when the header does not parse as a trait header.
pub(crate) fn method_trait_name(impl_header: &str) -> Option<String> {
    let filter = rsmpl_pattern::parse_impl_header(impl_header)?;
    if !filter.want_trait {
        return None;
    }
    if filter.trait_path.is_empty() {
        return None;
    }
    Some(filter.trait_path.clone())
}

/// Extracted method rename names from an `ImplContext` hunk: (old, new).
/// Diet `V Q fn old …` spellings carry concrete idents, so a light
/// `fn <ident>` scan suffices; name holes (`N`, `$M`) decline (no claim).
pub(crate) fn method_rename_names(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<(String, String)> {
    let old_pat = crate::matcher_fn::minus_fn_pattern(rh, compiled).ok()?;
    let old = match &old_pat.name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(_) => return None,
    };
    // Plus block mirrors the minus extraction (see matcher_impl).
    let full_plus: String = rh
        .lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let (vis, qual, gen, wh, ty) = crate::matcher_fn::slot_metas(compiled);
    let ident = compiled.table.ident_slot_metas();
    let block =
        crate::matcher_fn::extract_fn_block(&full_plus, &[vis.clone(), qual.clone()].concat());
    let plus_pat = rsmpl_pattern::parse_fn_pattern(
        &block,
        &vis,
        &qual,
        &gen,
        &wh,
        &ty,
        &ident,
        rh.returns_none(),
    )
    .ok()?;
    let new = match &plus_pat.name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(_) => return None,
    };
    if old.is_empty() || new.is_empty() || old == new {
        return None;
    }
    Some((old, new))
}

/// WI-1-style definition claims for trait methods: same-name `fn` idents
/// in `impl <Trait> for <Self>` blocks naming `trait_name`.
/// Returns (path, start, end, line). Inherent impls never claim.
pub(crate) fn collect_impl_fn_claims(
    files: &[(String, &syn::File, &str, &LineIndex)],
    trait_name: &str,
    old: &str,
) -> Vec<(String, usize, usize, u32)> {
    let mut out = Vec::new();
    for (path, parsed, original, index) in files {
        struct V<'a> {
            trait_name: &'a str,
            old: &'a str,
            original: &'a str,
            index: &'a LineIndex,
            out: Vec<(usize, usize, u32)>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                let names_trait = match &node.trait_ {
                    Some((_, p, _)) => last_ident(&norm_path(p)) == *self.trait_name,
                    None => false,
                };
                if names_trait {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                if let Some((a, b)) =
                                    span_bytes(self.original, self.index, m.sig.ident.span())
                                {
                                    if self.original.get(a..b) == Some(self.old) {
                                        let line = m.sig.ident.span().start().line as u32;
                                        self.out.push((a, b, line));
                                    }
                                }
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            trait_name,
            old,
            original,
            index,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for (a, b, line) in v.out {
            out.push((path.clone(), a, b, line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Inherent same-name `fn` definitions (`impl S { fn old }`): evidence
/// only (`shadowed`), never claims.
pub(crate) fn collect_inherent_fn_lines(
    files: &[(String, &syn::File, &str, &LineIndex)],
    old: &str,
) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
                if node.trait_.is_none() {
                    for it in &node.items {
                        if let syn::ImplItem::Fn(m) = it {
                            if m.sig.ident == self.old {
                                self.out.push(m.sig.ident.span().start().line as u32);
                            }
                        }
                    }
                }
                visit::visit_item_impl(self, node);
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            out.push((path.clone(), line));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// One method follow edit: byte span plus whether it is a definition claim.
pub(crate) type MethodEdit = (usize, usize, bool);

/// Per-file method follow spans (path + edits).
pub(crate) type MethodFollowSpans = Vec<(String, Vec<MethodEdit>)>;

/// Workspace outcome for trait-method definition follow: definition claims
/// plus inherent-`shadowed` evidence. No use-site claims in v1; opaque
/// macro mentions of `old` veto without fallback (assoc parity).
pub(crate) struct MethodOutcome {
    pub per_file: MethodFollowSpans,
    pub skipped: Vec<FollowSkip>,
}

pub(crate) fn resolve_method_workspace(
    files: &[(String, &syn::File, &str, &LineIndex)],
    trait_name: &str,
    old: &str,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback_syntax: bool,
) -> Result<MethodOutcome, FollowVeto> {
    let claims = collect_impl_fn_claims(files, trait_name, old);
    // Opaque gate: a macro-token / attribute mention of `old` vetoes
    // without fallback (assoc parity, never a silent skip).
    let mut opaque: Vec<(String, u32)> = Vec::new();
    for (path, parsed, _, _) in files {
        struct V<'a> {
            old: &'a str,
            out: Vec<u32>,
        }
        impl<'ast> Visit<'ast> for V<'_> {
            fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
                let toks = quote::quote!(#node).to_string();
                if toks
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|t| t == self.old)
                {
                    self.out.push(node.span().start().line as u32);
                }
            }
            fn visit_impl_item_macro(&mut self, node: &'ast syn::ImplItemMacro) {
                let toks = quote::quote!(#node).to_string();
                if toks
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|t| t == self.old)
                {
                    self.out.push(node.span().start().line as u32);
                }
            }
        }
        let mut v = V {
            old,
            out: Vec::new(),
        };
        v.visit_file(parsed);
        for line in v.out {
            opaque.push((path.clone(), line));
        }
    }
    if !opaque.is_empty() && !fallback_syntax {
        let (path, line) = &opaque[0];
        return Err(FollowVeto {
            reason: format!(
                "use of `{old}` hides in a macro body at {path}:{line}: cannot attribute it syntactically"
            ),
        });
    }
    let mut skipped: Vec<FollowSkip> = opaque
        .into_iter()
        .map(|(f, l)| {
            FollowSkip::new(
                format!("{f}:{l}: use of `{old}` inside a macro body left untouched (opaque)"),
                &f,
                l,
                skip_reason::MACRO_HIDDEN,
            )
        })
        .collect();
    for (f, l) in collect_inherent_fn_lines(files, old) {
        skipped.push(FollowSkip::new(
            format!("{f}:{l}: inherent method `{old}` (different item) left untouched"),
            &f,
            l,
            skip_reason::SHADOWED,
        ));
    }
    // Claims outside already-renamed definition spans only.
    let mut per_file: MethodFollowSpans = Vec::new();
    for (path, a, b, _) in claims {
        if path == def_path && renamed.iter().any(|(s, e)| *s == a && *e == b) {
            continue;
        }
        match per_file.iter_mut().find(|(f, _)| f == &path) {
            Some((_, v)) => v.push((a, b, true)),
            None => per_file.push((path.clone(), vec![(a, b, true)])),
        }
    }
    skipped.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    skipped.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.reason == b.reason);
    Ok(MethodOutcome { per_file, skipped })
}

/// Follow target for a trait-header `ImplContext` rename hunk:
/// (old, new, header). `None` for inherent headers, evolutions, and
/// name holes. Single call so `use_graph.rs` stays small (D5).
pub(crate) fn method_follow_target(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<(String, String, String)> {
    let rsmpl_pattern::PatternKind::ImplContext { impl_header } = &compiled.kind else {
        return None;
    };
    if !rsmpl_pattern::is_trait_header(impl_header) {
        return None;
    }
    method_rename_names(rh, compiled).map(|(o, n)| (o, n, impl_header.clone()))
}

/// Resolve trait-method definition follow to engine edits + skips.
/// Single call so `use_graph.rs` keeps only the label mapping (D5).
pub(crate) fn resolve_method_follow(
    refs: &[(String, &syn::File, &str, &crate::lines::LineIndex)],
    impl_header: &str,
    old: &str,
    def_path: &str,
    renamed: &[(usize, usize)],
    fallback_syntax: bool,
) -> Result<(MethodFollowSpans, Vec<FollowSkip>), FollowVeto> {
    let Some(trait_name) = method_trait_name(impl_header) else {
        return Err(FollowVeto {
            reason: "method follow header does not parse as a `trait` block header".to_string(),
        });
    };
    let outcome =
        resolve_method_workspace(refs, &trait_name, old, def_path, renamed, fallback_syntax)?;
    Ok((outcome.per_file, outcome.skipped))
}

/// New name for a trait-method follow hunk, if it is a concrete rename.
/// Single call for `plan.rs`/`validate.rs` so the planners stay small.
pub(crate) fn method_follow_new_name(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<String> {
    method_rename_names(rh, compiled).map(|(_, n)| n)
}

/// Plan-layer orphan entry: trait-header `ImplContext` renames list
/// leftover impl definitions; everything else lists nothing. Keeps
/// `plan.rs` to a one-line arm (D5 file-size gate).
pub(crate) fn method_orphans_for(
    states: &std::collections::HashMap<String, crate::lines::FileState>,
    file_edits: &std::collections::HashMap<String, Vec<crate::edits::TextEdit>>,
    path: &str,
    edits_before: usize,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Vec<crate::OrphanReport> {
    let rsmpl_pattern::PatternKind::ImplContext { impl_header } = &compiled.kind else {
        return Vec::new();
    };
    let Some(trait_name) = method_trait_name(impl_header) else {
        return Vec::new();
    };
    let Some((old, _)) = method_rename_names(rh, compiled) else {
        return Vec::new();
    };
    method_orphans(states, file_edits, path, edits_before, &trait_name, &old)
}

/// Orphan evidence for definition-only trait-method renames: leftover
/// trait-proven impl `fn` definitions. Mirrors `assoc_orphans` (WI-1b).
#[allow(clippy::too_many_arguments)]
pub(crate) fn method_orphans(
    states: &std::collections::HashMap<String, crate::lines::FileState>,
    file_edits: &std::collections::HashMap<String, Vec<crate::edits::TextEdit>>,
    path: &str,
    edits_before: usize,
    trait_name: &str,
    old: &str,
) -> Vec<crate::OrphanReport> {
    let renamed: Vec<(usize, usize)> = file_edits
        .get(path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        if let Some(st) = states.get(p) {
            if let Some(parsed) = st.parsed.as_ref() {
                refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
            }
        }
    }
    let claims = collect_impl_fn_claims(&refs, trait_name, old);
    let mut out = Vec::new();
    for (f, _, _, line) in claims {
        // Skip spans the definition hunk already rewrote.
        if f == path {
            continue;
        }
        let text = states
            .get(&f)
            .map(|st| {
                let s = st.index.line_start(line).unwrap_or(0);
                let e = st.index.line_start(line + 1).unwrap_or(st.original.len());
                st.original.get(s..e).unwrap_or("").trim().to_string()
            })
            .unwrap_or_default();
        out.push(crate::OrphanReport {
            file: f,
            line,
            text,
            reason: "off-target".to_string(),
        });
    }
    // Same-file leftover claims (not in `renamed`) are orphans too.
    for (f, a, b, line) in collect_impl_fn_claims(&refs, trait_name, old) {
        if f != path {
            continue;
        }
        if renamed.iter().any(|(s, e)| *s == a && *e == b) {
            continue;
        }
        // The definition hunk owns exactly one block; any other claimed
        // def in the same file is leftover evidence.
        let _ = (a, b);
        let text = states
            .get(&f)
            .map(|st| {
                let s = st.index.line_start(line).unwrap_or(0);
                let e = st.index.line_start(line + 1).unwrap_or(st.original.len());
                st.original.get(s..e).unwrap_or("").trim().to_string()
            })
            .unwrap_or_default();
        out.push(crate::OrphanReport {
            file: f,
            line,
            text,
            reason: "off-target".to_string(),
        });
    }
    out.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    fn apply(patch: &str, files: &[(&str, &str)]) -> Result<HashMap<String, String>, String> {
        let parsed = rsmpl_patch::parse_patch(patch).map_err(|e| format!("{e:?}"))?;
        let mut map: HashMap<String, String> = files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        crate::apply_patch_to_tree(&mut map, &parsed).map_err(|e| e.render())?;
        Ok(map)
    }

    fn apply_files(
        patch: &str,
        files: &[(&str, &str)],
    ) -> Result<(HashMap<String, String>, crate::ApplyReport), String> {
        let parsed = rsmpl_patch::parse_patch(patch).map_err(|e| format!("{e:?}"))?;
        let mut map: HashMap<String, String> = files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let report = crate::apply_patch_to_tree(&mut map, &parsed).map_err(|e| e.render())?;
        Ok((map, report))
    }

    const SRC: &str = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";

    fn trait_patch() -> String {
        "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n"
            .to_string()
    }

    #[test]
    fn follow_rewrites_trait_proven_impl_def() {
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", SRC)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("fn new(&self);"), "got: {out}");
        assert!(out.contains("fn new(&self) {"), "got: {out}");
        assert!(!out.contains("fn old"), "got: {out}");
        assert_eq!(
            report.rules[0].semantics,
            crate::RuleSemantics::Instantiated
        );
    }

    #[test]
    fn follow_cross_file_impl_def() {
        let patch = trait_patch().replace("src/lib.rs", "src/a.rs");
        let (files, _) = apply_files(
            &patch,
            &[
                ("src/a.rs", "trait T {\n    fn old(&self);\n}\n"),
                (
                    "src/b.rs",
                    "struct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n",
                ),
            ],
        )
        .unwrap();
        assert!(files["src/a.rs"].contains("fn new(&self);"));
        assert!(files["src/b.rs"].contains("fn new(&self) {"));
        assert!(!files["src/b.rs"].contains("fn old"));
    }

    #[test]
    fn inherent_namesake_untouched_shadowed() {
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\nstruct U;\nimpl U {\n    fn old(&self) {\n    }\n}\n";
        let (files, report) = apply_files(&trait_patch(), &[("src/lib.rs", src)]).unwrap();
        let out = &files["src/lib.rs"];
        assert!(out.contains("impl U {\n    fn old(&self) {"), "got: {out}");
        assert!(
            report.rules[0]
                .follow_skipped_details
                .iter()
                .any(|d| d.reason == "shadowed"),
            "got: {:?}",
            report.rules[0].follow_skipped_details
        );
    }

    #[test]
    fn no_follow_lists_impl_def_orphans() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n";
        let (_, report) = apply_files(p, &[("src/lib.rs", SRC)]).unwrap();
        assert!(
            report.rules[0]
                .orphans
                .iter()
                .any(|o| o.text.contains("fn old")),
            "got: {:?}",
            report.rules[0].orphans
        );
    }

    #[test]
    fn inherent_header_follow_rejected() {
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  impl S {\n-     V Q fn old G (...) W {\n+     V Q fn new G (...) W {\n";
        let err = apply(
            p,
            &[(
                "src/lib.rs",
                "struct S;\nimpl S {\n    fn old(&self) {\n    }\n}\n",
            )],
        )
        .unwrap_err();
        assert!(
            err.contains("C17") || err.contains("# follow=uses"),
            "got: {err}"
        );
    }

    #[test]
    fn evolution_follow_rejected() {
        // Same name both sides: not a rename, follow stays rejected.
        let p = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn old G (...) W ;\n";
        let err = apply(p, &[("src/lib.rs", SRC)]).unwrap_err();
        assert!(!err.is_empty(), "evolution follow must fail");
    }
}
