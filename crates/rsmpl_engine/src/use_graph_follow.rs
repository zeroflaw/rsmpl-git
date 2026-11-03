//! WI-6b1 `# follow=uses` workspace dispatcher (SPEC 14.8).
//!
//! Split from [`crate::use_graph`] (D5 hygiene: that file sat 35 lines
//! under the 2000-line gate; behaviour-preserving move, no semantic
//! change). [`crate::use_graph`] keeps the resolvers (`resolve`,
//! `resolve_workspace`, `raw_mentions`) and the `Collector`; this module
//! owns the planner-facing workspace step that dispatches each follow
//! kind (scalar / member / assoc / method) to its resolver, handles the
//! unparseable tripwire, and appends the follow edits.

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::edits::{edit_spans, TextEdit};
use crate::follow_reasons::{reason as skip_reason, FollowSkip};
use crate::lines::{FileState, LineIndex};
use crate::use_graph::{raw_mentions, resolve_workspace, FollowVeto, WorkspaceOutcome};

/// Shared C17 rejection (WI-3/WI-6 widen the legal list to trait- and
/// inherent-method renames; every other shape keeps the follow-up hunks).
fn c17_err(rh: &rsmpl_patch::RsmplHunk, def_path: &str) -> StructuredError {
    StructuredError::new(
        ErrorCode::PatternConflict,
        "# follow=uses is only supported on fn, item, field, variant, assoc, trait-method, and inherent-method rename hunks in v0.1 (other shapes keep the SPEC 14.8 follow-up hunks)",
    )
    .with_rule(rh.rule_name())
    .with_file(def_path)
    .with_suggestion("remove `# follow=uses`, or rewrite this hunk as a fn/item/field/variant/assoc rename")
    .with_repair(StructuredError::fix_pattern_repair(
        "C17",
        "remove `# follow=uses`, or rewrite this hunk as a fn/item/field/variant/assoc rename",
    ))
}

/// WI-6a `# follow=uses`: after the definition rewrite, resolve same-file
/// use sites and append their ident-span edits under the same rule
/// (SPEC 14.8). Zero new def edits (optional zero-match) means nothing to
/// WI-6b1: one follow-only entry per non-definition file touched by
/// `# follow=uses`: (file, rewritten use-site count, line spans).
pub(crate) type FollowExtras = Vec<(
    String,
    usize,
    Vec<(u32, u32)>,
    Vec<String>,
    Vec<(usize, usize, String)>,
)>;

/// Resolved follow edits per file: (path, [(start, end, replacement, label)]).
/// Labels are WI-2 evidence only (`follow` for use sites, `rename` for
/// WI-1 impl-definition claims).
type ResolvedUses = Vec<(String, Vec<(usize, usize, String, &'static str)>)>;

/// WI-2 follow-workspace outcome: the follow-only entries and the
/// structured skip details (SPEC D.2). v4: the compat `follow_skipped`
/// prose binding is gone — `details` carry the signal alone.
pub(crate) type FollowWorkspaceOutcome = (FollowExtras, Vec<crate::FollowSkipReport>);

/// Old ident claimed by a `# follow=uses` hunk (facade D13-domain
/// widening): the ident whose off-diff mentions must join the raiser
/// probe pre-image. Mirrors the dispatch match below (scalar / member
/// / assoc / method targets); `None` for non-follow shapes and for
/// hunks that fail to compile. Single call so the facade never
/// re-implements follow-kind knowledge.
pub fn follow_old_ident(rh: &rsmpl_patch::RsmplHunk) -> Option<String> {
    let compiled = rsmpl_pattern::compile(rh).ok()?;
    match &compiled.kind {
        PatternKind::Fn { minus_name, .. } => Some(minus_name.clone()),
        PatternKind::ItemRename { minus_name, .. } => Some(minus_name.clone()),
        PatternKind::StructRenameFieldEvo { old_container, .. } => Some(old_container.clone()),
        PatternKind::EnumRenameVariantEvo { old_container, .. } => Some(old_container.clone()),
        PatternKind::FieldRename { old, .. } => Some(old.clone()),
        PatternKind::VariantRename { old, .. } => Some(old.clone()),
        PatternKind::AssocRename { minus_name, .. } => Some(minus_name.clone()),
        PatternKind::ImplContext { .. } => {
            crate::use_graph_method::method_follow_target(rh, &compiled).map(|(old, _, _)| old)
        }
        _ => None,
    }
}

/// WI-6b1 `# follow=uses` workspace step (SPEC 14.8): after the definition
/// rewrite, resolve use sites across every loaded `.rs` file (unique-
/// definition fast path) and append their ident-span edits under the same
/// rule. Zero new def edits (optional zero-match) means nothing to follow
/// — a no-op, never a veto.
///
/// The caller reports the definition file as usual and
/// each extra entry as its own `RuleReport` with the same rule name
/// (SPEC D.2).
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow_uses_workspace(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    states: &HashMap<String, FileState>,
    file_edits: &mut HashMap<String, Vec<TextEdit>>,
    unparseable: &[(String, String)],
    def_path: &str,
    edits_before: usize,
    found: usize,
) -> Result<FollowWorkspaceOutcome, StructuredError> {
    let no_follow: FollowExtras = Vec::new();
    if found == 0 {
        return Ok((no_follow, Vec::new()));
    }
    // WI-9a: member renames follow through the member collector (which
    // claims only `E::Old`-spelled paths / `.old` + `S`-anchored labels);
    // WI-27: assoc renames follow through the assoc collector (which
    // claims only container-proven `Self::`/`S::`/`Tr::`/qself tails);
    // every other shape keeps the scalar resolver.
    enum FollowTarget {
        Scalar {
            macro_paths: bool,
        },
        Member {
            kind: crate::use_graph_member::MemberKind,
            container: String,
        },
        Assoc {
            impl_header: String,
            want_trait: bool,
            is_type: bool,
        },
        Method {
            impl_header: String,
        },
    }
    let (old, new, target) = match &compiled.kind {
        PatternKind::Fn {
            minus_name,
            plus_name,
            ..
        } => (
            minus_name.clone(),
            plus_name.clone(),
            FollowTarget::Scalar { macro_paths: false },
        ),
        PatternKind::ItemRename {
            minus_name,
            plus_name,
            kind,
            ..
        } => (
            minus_name.clone(),
            plus_name.clone(),
            FollowTarget::Scalar {
                macro_paths: *kind == rsmpl_pattern::ItemKind::MacroRules,
            },
        ),
        PatternKind::StructRenameFieldEvo {
            old_container,
            new_container,
            ..
        } => (
            old_container.clone(),
            new_container.clone(),
            FollowTarget::Scalar { macro_paths: false },
        ),
        PatternKind::EnumRenameVariantEvo {
            old_container,
            new_container,
            ..
        } => (
            old_container.clone(),
            new_container.clone(),
            FollowTarget::Scalar { macro_paths: false },
        ),
        PatternKind::FieldRename {
            struct_name,
            old,
            new,
            ..
        } => (
            old.clone(),
            new.clone(),
            FollowTarget::Member {
                kind: crate::use_graph_member::MemberKind::Field,
                container: struct_name.clone(),
            },
        ),
        PatternKind::VariantRename {
            enum_name,
            old,
            new,
            ..
        } => (
            old.clone(),
            new.clone(),
            FollowTarget::Member {
                kind: crate::use_graph_member::MemberKind::Variant,
                container: enum_name.clone(),
            },
        ),
        PatternKind::AssocRename {
            impl_header,
            want_trait,
            is_type,
            minus_name,
            plus_name,
            ..
        } => (
            minus_name.clone(),
            plus_name.clone(),
            FollowTarget::Assoc {
                impl_header: impl_header.clone(),
                want_trait: *want_trait,
                is_type: *is_type,
            },
        ),
        PatternKind::ImplContext { .. } => {
            let Some((old, new, header)) =
                crate::use_graph_method::method_follow_target(rh, compiled)
            else {
                return Err(c17_err(rh, def_path));
            };
            (
                old,
                new,
                FollowTarget::Method {
                    impl_header: header,
                },
            )
        }
        _ => {
            return Err(c17_err(rh, def_path));
        }
    };
    // Ranges this hunk already rewrote: a same-named definition outside
    // them shadows (the resolver decides veto vs skip, never guesses).
    let renamed: Vec<(usize, usize)> = file_edits
        .get(def_path)
        .map(|v| v[edits_before..].iter().map(|e| (e.start, e.end)).collect())
        .unwrap_or_default();
    // Deterministic file order for reports and vetoes.
    let mut refs: Vec<(String, &syn::File, &str, &LineIndex)> = Vec::new();
    let mut paths: Vec<&String> = states.keys().collect();
    paths.sort();
    for p in paths {
        let st = &states[p];
        if let Some(parsed) = st.parsed.as_ref() {
            refs.push((p.clone(), parsed, st.original.as_str(), &st.index));
        }
    }
    let veto_to_err = |veto: FollowVeto| {
        StructuredError::new(ErrorCode::SymbolConflict, veto.reason)
            .with_rule(rh.rule_name())
            .with_file(def_path)
            .with_suggestion(
                "add `# fallback=syntax` to skip ambiguous sites (listed in `follow_skipped_details`) instead of failing",
            )
            .with_repair(StructuredError::add_directive_repair("# fallback=syntax"))
    };
    // Normalized edits per file. The scalar resolver rewrites every
    // span to `new`; the member resolver carries its own replacements
    // (shorthand expansion).
    let (per_file, skipped_from_resolve): (ResolvedUses, Vec<FollowSkip>) = match &target {
        FollowTarget::Scalar { macro_paths } => {
            let outcome = resolve_workspace(
                &refs,
                &old,
                def_path,
                &renamed,
                rh.fallback_syntax(),
                *macro_paths,
            )
            .map_err(veto_to_err)?;
            let per_file = outcome
                .per_file
                .into_iter()
                .map(|(f, spans)| {
                    (
                        f,
                        spans
                            .into_iter()
                            .map(|(a, b)| (a, b, new.clone(), "follow"))
                            .collect(),
                    )
                })
                .collect();
            (per_file, outcome.skipped)
        }
        FollowTarget::Member { kind, container } => {
            let outcome = crate::use_graph_member::resolve_member_workspace(
                &refs,
                *kind,
                container,
                &old,
                &new,
                def_path,
                &renamed,
                rh.fallback_syntax(),
            )
            .map_err(veto_to_err)?;
            let per_file = outcome
                .per_file
                .into_iter()
                .map(|(f, edits)| {
                    (
                        f,
                        edits
                            .into_iter()
                            .map(|e| (e.start, e.end, e.replacement, "follow"))
                            .collect(),
                    )
                })
                .collect();
            (per_file, outcome.skipped)
        }
        FollowTarget::Assoc {
            impl_header,
            want_trait,
            is_type,
        } => {
            let domain =
                crate::use_graph_assoc::assoc_domain(impl_header, *want_trait, *is_type, &old);
            let Some(domain) = domain else {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "assoc follow header does not parse as a `trait`/`impl` block header",
                )
                .with_rule(rh.rule_name())
                .with_file(def_path)
                .with_suggestion("copy the block header verbatim from the file")
                .with_repair(StructuredError::fix_pattern_repair(
                    "assoc",
                    "copy the `trait T {` or `impl ... {` header verbatim from the file",
                )));
            };
            let Some(filter) = rsmpl_pattern::parse_impl_header(impl_header) else {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "assoc follow header does not parse as a `trait`/`impl` block header",
                )
                .with_rule(rh.rule_name())
                .with_file(def_path)
                .with_suggestion("copy the block header verbatim from the file")
                .with_repair(StructuredError::fix_pattern_repair(
                    "assoc",
                    "copy the `trait T {` or `impl ... {` header verbatim from the file",
                )));
            };
            let outcome = crate::use_graph_assoc::resolve_assoc_workspace(
                &refs,
                &domain,
                &filter,
                def_path,
                &renamed,
                rh.fallback_syntax(),
            )
            .map_err(veto_to_err)?;
            let per_file = outcome
                .per_file
                .into_iter()
                .map(|(f, spans)| {
                    (
                        f,
                        spans
                            .into_iter()
                            .map(|(a, b, is_def)| {
                                (a, b, new.clone(), if is_def { "rename" } else { "follow" })
                            })
                            .collect(),
                    )
                })
                .collect();
            (per_file, outcome.skipped)
        }
        FollowTarget::Method { impl_header } => {
            let domain = crate::use_graph_method::method_domain(impl_header).ok_or_else(|| {
                StructuredError::new(
                    ErrorCode::PatternConflict,
                    "method follow header does not parse as a `trait` or bare inherent `impl` block header",
                )
                .with_rule(rh.rule_name())
                .with_file(def_path)
                .with_suggestion("copy the `trait T {` or `impl S {` header verbatim from the file")
                .with_repair(StructuredError::fix_pattern_repair(
                    "method",
                    "copy the `trait T {` or `impl S {` header verbatim from the file",
                ))
            })?;
            let (spans, skipped) = crate::use_graph_method::resolve_method_follow(
                &refs,
                &domain,
                &old,
                def_path,
                &renamed,
                rh.fallback_syntax(),
            )
            .map_err(veto_to_err)?;
            let per_file = spans
                .into_iter()
                .map(|(f, edits)| {
                    (
                        f,
                        edits
                            .into_iter()
                            .map(|(a, b, is_def)| {
                                (a, b, new.clone(), if is_def { "rename" } else { "follow" })
                            })
                            .collect(),
                    )
                })
                .collect();
            (per_file, skipped)
        }
    };
    let outcome = WorkspaceOutcome {
        per_file: per_file
            .iter()
            .map(|(f, edits)| {
                (
                    f.clone(),
                    edits.iter().map(|(a, b, _, _)| (*a, *b)).collect(),
                )
            })
            .collect(),
        skipped: skipped_from_resolve,
    };
    // Unparseable-workspace tripwire (SPEC 14.8): a file the resolver could
    // not search that names `old` outside strings/comments fails loud
    // without fallback (never a silent stale site), or is listed and left
    // untouched with it. Token-exact via `proc_macro2` lexing; when even
    // lexing fails, assume a mention (fail loud).
    let mut tripwire: Vec<String> = Vec::new();
    for (path, text) in unparseable {
        if raw_mentions(text, &old) {
            tripwire.push(path.clone());
        }
    }
    tripwire.sort();
    tripwire.dedup();
    if !tripwire.is_empty() && !rh.fallback_syntax() {
        let path = &tripwire[0];
        return Err(StructuredError::new(
            ErrorCode::SymbolConflict,
            format!("use of `{old}` may hide in unparseable file {path}: cannot search it syntactically"),
        )
        .with_rule(rh.rule_name())
        .with_file(def_path)
        .with_suggestion(
            "add `# fallback=syntax` to skip unsearchable files (listed in `follow_skipped_details`) instead of failing",
        )
        .with_repair(StructuredError::add_directive_repair("# fallback=syntax")));
    }
    // Append use-span edits per file; record pre-push lengths so extra
    // reports span exactly the pushed edits. Use sites carry the WI-2
    // `follow` span label, WI-1 impl-definition claims `rename` (evidence
    // only).
    let mut before_lens: HashMap<String, usize> = HashMap::new();
    for (f, edits) in &per_file {
        before_lens.insert(f.clone(), file_edits.get(f).map(|v| v.len()).unwrap_or(0));
        let vec = file_edits.get_mut(f).expect("workspace file state");
        for (a, b, replacement, label) in edits {
            vec.push(TextEdit::new(*a, *b, replacement.clone()).with_label(label));
        }
    }
    let mut extras: FollowExtras = Vec::new();
    for (f, edits) in &per_file {
        if f == def_path {
            continue;
        }
        if edits.is_empty() {
            continue;
        }
        let st = states.get(f).expect("workspace file state");
        let file_edit_list = file_edits.get(f).expect("workspace file state");
        let before = before_lens.get(f).copied().unwrap_or(0);
        let line_spans = edit_spans(&st.index, &file_edit_list[before..]);
        let labels: Vec<String> = file_edit_list[before..]
            .iter()
            .map(|e| e.label.unwrap_or("follow").to_string())
            .collect();
        let byte_edits: Vec<(usize, usize, String)> = edits
            .iter()
            .map(|(a, b, r, _)| (*a, *b, r.clone()))
            .collect();
        extras.push((f.clone(), edits.len(), line_spans, labels, byte_edits));
    }
    extras.sort_by(|a, b| a.0.cmp(&b.0));
    let mut skipped = outcome.skipped;
    for path in &tripwire {
        skipped.push(FollowSkip::whole_file(
            format!("{path}: unparseable file left untouched (may name `{old}`)"),
            path,
            skip_reason::MACRO_HIDDEN,
        ));
    }
    let details = FollowSkip::details(&skipped, def_path);
    Ok((extras, details))
}

#[cfg(test)]
mod tests {
    //! D5 sibling of `use_graph_follow.rs` (`plan_tests.rs` precedent):
    //! dispatcher-level pins for `follow_uses_workspace` — one happy-path
    //! pin per `FollowTarget` arm plus the two error paths. Resolver
    //! matrices keep their own pins; this module only proves the dispatch
    //! (arm selection, `found == 0` no-op, labels, extras, tripwire).

    use super::*;
    use crate::edits::TextEdit;
    use crate::lines::{FileState, LineIndex};
    use std::collections::HashMap;

    fn hunk_of(patch: &str) -> (rsmpl_patch::RsmplHunk, rsmpl_pattern::CompiledPattern) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("patch must parse");
        let rh = match &parsed.files[0].hunks[0] {
            rsmpl_patch::Hunk::Rsmpl(r) => r.clone(),
            _ => panic!("expected rsmpl hunk"),
        };
        let compiled = rsmpl_pattern::compile(&rh).expect("hunk must compile");
        (rh, compiled)
    }

    fn states_for(
        files: &[(&str, &str)],
    ) -> (HashMap<String, FileState>, HashMap<String, Vec<TextEdit>>) {
        let mut states = HashMap::new();
        let mut edits = HashMap::new();
        for (path, src) in files {
            states.insert(
                (*path).to_string(),
                FileState {
                    original: (*src).to_string(),
                    index: LineIndex::new(src),
                    parsed: syn::parse_str::<syn::File>(src).ok(),
                },
            );
            edits.insert((*path).to_string(), Vec::new());
        }
        (states, edits)
    }

    /// Push the definition edit covering `old` inside `marker` in
    /// `def_path`, so the dispatcher's `renamed` exclusion mirrors plan.rs.
    fn push_def_edit(
        file_edits: &mut HashMap<String, Vec<TextEdit>>,
        files: &[(&str, &str)],
        def_path: &str,
        marker: &str,
        old: &str,
        new: &str,
    ) {
        let src = files
            .iter()
            .find(|(p, _)| *p == def_path)
            .expect("def file present")
            .1;
        let moff = src.find(marker).expect("marker present");
        let a = moff + marker.find(old).expect("old inside marker");
        file_edits
            .get_mut(def_path)
            .expect("edits entry")
            .push(TextEdit::new(a, a + old.len(), new.to_string()));
    }

    #[test]
    fn found_zero_is_noop() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
        let (rh, compiled) = hunk_of(patch);
        let files = [
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
        ];
        let (states, mut file_edits) = states_for(&files);
        let before: HashMap<String, usize> = file_edits
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect();
        let (extras, details) = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &[],
            "src/a.rs",
            0,
            0,
        )
        .unwrap();
        assert!(extras.is_empty(), "got: {extras:?}");
        assert!(details.is_empty(), "got: {details:?}");
        for (k, v) in &file_edits {
            assert_eq!(v.len(), before[k], "edits touched for {k}");
        }
    }

    #[test]
    fn scalar_dispatch_claims_cross_file_use() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
        let (rh, compiled) = hunk_of(patch);
        let files = [
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
        ];
        let (states, mut file_edits) = states_for(&files);
        push_def_edit(
            &mut file_edits,
            &files,
            "src/a.rs",
            "struct Old",
            "Old",
            "New",
        );
        let (extras, _) = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &[],
            "src/a.rs",
            0,
            1,
        )
        .unwrap();
        assert_eq!(extras.len(), 1, "got: {extras:?}");
        assert_eq!(extras[0].0, "src/b.rs");
        assert_ne!(extras[0].1, 0, "got: {extras:?}");
        assert!(
            extras[0].3.iter().all(|l| l == "follow"),
            "got: {:?}",
            extras[0].3
        );
        assert!(
            !file_edits["src/b.rs"].is_empty(),
            "use edits must land in file_edits"
        );
    }

    #[test]
    fn member_dispatch_claims_typed_use() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,4 @@ rule=r\n# follow=uses\n  struct S {\n- old: u32\n+ new: u32\n";
        let (rh, compiled) = hunk_of(patch);
        let files = [
            ("src/a.rs", "struct S {\n    old: u32,\n}\n"),
            ("src/b.rs", "fn f(s: S) -> u32 {\n    s.old\n}\n"),
        ];
        let (states, mut file_edits) = states_for(&files);
        push_def_edit(
            &mut file_edits,
            &files,
            "src/a.rs",
            "old: u32",
            "old",
            "new",
        );
        let (extras, _) = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &[],
            "src/a.rs",
            0,
            1,
        )
        .unwrap();
        assert_eq!(extras.len(), 1, "got: {extras:?}");
        assert_eq!(extras[0].0, "src/b.rs");
        assert!(
            !file_edits["src/b.rs"].is_empty(),
            "member use must rewrite"
        );
    }

    #[test]
    fn assoc_dispatch_labels_impl_definition_rename() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait Tr {\n- type Item;\n+ type Output;\n";
        let (rh, compiled) = hunk_of(patch);
        let src =
            "trait Tr {\n    type Item;\n}\nstruct S;\nimpl Tr for S {\n    type Item = u8;\n}\n";
        let files = [("src/a.rs", src)];
        let (states, mut file_edits) = states_for(&files);
        push_def_edit(
            &mut file_edits,
            &files,
            "src/a.rs",
            "type Item;",
            "Item",
            "Output",
        );
        let (extras, _) = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &[],
            "src/a.rs",
            0,
            1,
        )
        .unwrap();
        assert!(extras.is_empty(), "same-file only, got: {extras:?}");
        let labels: Vec<Option<&str>> = file_edits["src/a.rs"].iter().map(|e| e.label).collect();
        assert!(
            labels.contains(&Some("rename")),
            "impl def must label `rename`, got: {labels:?}"
        );
        assert!(
            labels.contains(&Some("follow")) || labels.len() >= 2,
            "use/def pair expected, got: {labels:?}"
        );
    }

    #[test]
    fn method_dispatch_accepts_trait_header() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@ rule=r\n# follow=uses\n  trait T {\n-     V Q fn old G (...) W ;\n+     V Q fn new G (...) W ;\n";
        let (rh, compiled) = hunk_of(patch);
        let src = "trait T {\n    fn old(&self);\n}\nstruct S;\nimpl T for S {\n    fn old(&self) {\n    }\n}\n";
        let files = [("src/a.rs", src)];
        let (states, mut file_edits) = states_for(&files);
        push_def_edit(&mut file_edits, &files, "src/a.rs", "fn old", "old", "new");
        let (extras, _) = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &[],
            "src/a.rs",
            0,
            1,
        )
        .unwrap();
        assert!(extras.is_empty(), "same-file only, got: {extras:?}");
        let labels: Vec<Option<&str>> = file_edits["src/a.rs"].iter().map(|e| e.label).collect();
        assert!(
            labels.contains(&Some("rename")),
            "impl def must label `rename`, got: {labels:?}"
        );
    }

    #[test]
    fn call_shape_follow_is_c17() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -2,1 +2,1 @@ rule=x\n# follow=uses\n# expr E\n- foo(E)\n+ bar(E)\n";
        let (rh, compiled) = hunk_of(patch);
        let files = [("src/a.rs", "fn f() {\n    foo(1);\n}\n")];
        let (states, mut file_edits) = states_for(&files);
        let err = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &[],
            "src/a.rs",
            0,
            1,
        )
        .unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.render().contains("# follow=uses is only supported"),
            "got: {}",
            err.render()
        );
    }

    #[test]
    fn tripwire_vetoes_without_fallback_and_skips_with_it() {
        let plain = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n- struct Old\n+ struct New\n";
        let fallback = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=r\n# follow=uses\n# fallback=syntax\n- struct Old\n+ struct New\n";
        let files = [
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
        ];
        let unparseable = vec![("src/c.rs".to_string(), "fn f( { Old \n".to_string())];
        // Without fallback: loud veto.
        let (rh, compiled) = hunk_of(plain);
        let (states, mut file_edits) = states_for(&files);
        push_def_edit(
            &mut file_edits,
            &files,
            "src/a.rs",
            "struct Old",
            "Old",
            "New",
        );
        let err = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &unparseable,
            "src/a.rs",
            0,
            1,
        )
        .unwrap_err();
        assert_eq!(err.code, rsmpl_patch::ErrorCode::SymbolConflict);
        // With fallback: skip + details, no veto.
        let (rh, compiled) = hunk_of(fallback);
        let (states, mut file_edits) = states_for(&files);
        push_def_edit(
            &mut file_edits,
            &files,
            "src/a.rs",
            "struct Old",
            "Old",
            "New",
        );
        let (_, details) = follow_uses_workspace(
            &rh,
            &compiled,
            &states,
            &mut file_edits,
            &unparseable,
            "src/a.rs",
            0,
            1,
        )
        .unwrap();
        assert!(
            details.iter().any(|d| d.file == "src/c.rs"),
            "got: {details:?}"
        );
    }
}
