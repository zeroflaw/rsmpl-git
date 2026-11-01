//! WI-7a + WI-7b + WI-7c cut+paste move verification plus WI-7d
//! pure-insertion and unpaired-cut verdicts plus the WI-7d-follow-up
//! member-addition verdict (SPEC 24.6).
//!
//! A move pair — one item cut (`- I` + `# bind=N`, scalar or
//! `# allow=many` collection in source order) and one paste (`+ N` +
//! `# use=N` bare, or `+ <template>` + `# use=N` evolving) — earns
//! `postcondition` on **both** entries iff two span-local checks hold
//! in the post-image:
//!
//! * presence: the pasted bytes equal the joined captured item texts
//!   (bare: per-item `trim_end`, `"\n\n"` join, trailing `"\n"`), or
//!   the template-replayed bytes (WI-7c: scalar splice, or per-item
//!   concretize+splice joined identically), at the paste edit's
//!   byte-exact post-image offset (WI-6b `post_starts_for` machinery —
//!   occurrence elsewhere never counts);
//! * absence per item: no same-kind same-ident (old ident for evolving)
//!   item starts inside the cut spans translated to post-image
//!   coordinates (structural, never a substring search).
//!
//! WI-7d adds the two non-pair verdicts:
//!
//! * pure insertion (`+ <text>` with no `# use=`): presence at the
//!   byte-exact post offset plus a standalone re-parse of the inserted
//!   slice (`Item`/`Stmt`/`Expr`/`Type`/multi-item `File`) plus a
//!   post-image file re-parse — earns `postcondition` on the entry;
//! * unpaired cut (a cut with no same-name paste — a deletion):
//!   per-item old kind+ident absence against the translated cut spans
//!   plus a post-image file re-parse — earns `postcondition` on the
//!   cut entry.
//!
//! WI-7d-follow-up adds the member-addition verdict:
//!
//! * member addition (`FieldAddition` / `VariantAddition` / `ArmAddition`
//!   point-insertion): presence at the byte-exact post offset plus
//!   container membership in the re-parsing post-image (struct holds the
//!   field / enum holds the variant / a same-scrutinee `match` holds the
//!   arm) — earns `postcondition` on the entry; sweeps need every
//!   recorded insert to verify.
//!
//! Everything else stays `reparse-only`: empty collections,
//! multi-inserts sharing one anchor, multi-pastes sharing one anchor
//! (T249 blank-join), unparseable post-images, standalone-unparseable
//! insertions, unnameable items (`impl` blocks and friends),
//! hole-less collection templates, and kind mismatches.
//! Out-of-range probe math abstains (skip, never fail); in-range
//! mismatch fails loud with `RSMPL_MATCH_CONFLICT` (a planner bug,
//! never a user error — WI-6 tamper doctrine).

use std::collections::HashMap;

use rsmpl_patch::StructuredError;
use syn::spanned::Spanned;

use crate::capture::{splice_evolved_header, CaptureStore};
use crate::capture_evolve::{
    captured_old_ident, collection_template_parts, concrete_new_ident, concretize_template,
    parse_collection_name_hole,
};
use crate::edits::{translate_spans_to_post, TextEdit};
use crate::lines::FileState;
use crate::{RuleReport, RuleSemantics};

/// WI-7a pending scalar cut: rule/file/spans plus the capture name that
/// links it to its paste. WI-7b records collection cuts identically
/// (all target spans, source order); the probe pairs by name and
/// verifies per item.
pub(crate) struct MoveCut {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) spans: Vec<(u32, u32)>,
    pub(crate) name: String,
}

/// WI-7a pending paste: rule/file plus the capture name and the
/// original-coordinate insertion byte offset. `bare` is true for
/// verbatim pastes (`+ N` trims to the name, `template` is `None`);
/// evolving pastes carry the raw template for the WI-7c per-item probe.
pub(crate) struct MovePaste {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) name: String,
    pub(crate) pos: usize,
    pub(crate) bare: bool,
    pub(crate) template: Option<String>,
}

/// WI-7d pending pure insertion: rule/file plus the original-coordinate
/// insertion byte offset and the exact bytes the planner spliced
/// (`apply_insertion` normalises to trailing `"\n"`). The probe checks
/// byte-exact presence at the post offset and a standalone re-parse of
/// the inserted slice.
pub(crate) struct PureInsert {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) pos: usize,
    pub(crate) expected: String,
}

/// WI-7d-follow-up container key for a member-addition insertion. Field
/// and variant carry their container name plus the member name;
/// arm carries the variant plus the token-normalised scrutinee from the
/// classifier (compared against the same normalisation in the probe —
/// mismatch abstains, never verifies).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MemberKind {
    Field { container: String, member: String },
    Variant { container: String, member: String },
    Arm { member: String, scrutinee: String },
}

/// WI-7d-follow-up pending member addition: rule/file plus the
/// original-coordinate insertion byte offset, the exact bytes the
/// planner spliced, and the container key above. One entry per new
/// insertion edit (ranged: exactly one; sweep: one per lacking
/// container). The probe checks byte-exact presence at the post offset
/// plus container membership in the re-parsing post-image.
#[derive(Clone, Debug)]
pub(crate) struct MemberAddition {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) pos: usize,
    pub(crate) expected: String,
    pub(crate) kind: MemberKind,
}

/// WI-7a + WI-7b + WI-7c: upgrade verified cut+paste pairs to `postcondition`.
///
/// Pairing is by capture name. Presence checks the paste bytes at the
/// byte-exact post offset — joined captured bytes for bare pastes
/// (scalar and 1-item collections share the single-item form), or the
/// template-replayed bytes for evolving pastes (scalar splice, or
/// per-item concretize+splice joined identically for collections);
/// absence checks every captured item's old kind+ident against the
/// translated cut spans. Skips stay `reparse-only`; in-range mismatches
/// fail loud.
#[allow(clippy::too_many_arguments)]
pub(crate) fn upgrade_verified_moves(
    cuts: &[MoveCut],
    pastes: &[MovePaste],
    captures: &CaptureStore,
    states: &HashMap<String, FileState>,
    full_edits: &HashMap<String, Vec<TextEdit>>,
    files: &HashMap<String, String>,
    post_parsed: &HashMap<String, syn::File>,
    rules: &mut [RuleReport],
) -> Result<(), StructuredError> {
    for p in pastes {
        let cut = match cuts.iter().find(|c| c.name == p.name) {
            Some(c) => c,
            None => continue,
        };
        let (texts, collection) = match captures.get_all(&p.name, &p.rule, &p.file) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if texts.is_empty() {
            continue;
        }
        // WI-7b bare insertion text vs WI-7c template-replayed text.
        // Both mirror `paste_capture` byte-for-byte (per-item
        // `trim_end`, `"\n\n"` join, trailing `"\n"`); any drift between
        // the two call sites abstains via the replacement-equality
        // guard below, never verifies.
        let expected = if p.bare {
            // WI-7b joined insertion text: per-item `trim_end`, `"\n\n"`
            // join, trailing `"\n"` — exactly what `paste_capture`
            // splices for a bare collection paste; the scalar
            // single-item form coincides (`trim_end + "\n"`).
            let joined = texts
                .iter()
                .map(|t| t.trim_end().to_string())
                .collect::<Vec<_>>()
                .join("\n\n");
            if joined.is_empty() {
                continue;
            }
            format!("{joined}\n")
        } else {
            match evolved_expected(texts, collection, p.template.as_deref(), &p.rule) {
                Some(e) => e,
                None => continue,
            }
        };
        // Single edit at the paste position: the T249 same-anchor join
        // (WI-7b) and any same-pos sibling insertion abstain.
        let edits = full_edits.get(&p.file).map(Vec::as_slice).unwrap_or(&[]);
        let at_pos: Vec<&TextEdit> = edits
            .iter()
            .filter(|e| e.start == p.pos && e.end == p.pos)
            .collect();
        if at_pos.len() != 1 {
            continue;
        }
        if at_pos[0].replacement != expected {
            continue;
        }
        let post = crate::postcheck_follow::post_starts_for(
            &[(
                at_pos[0].start,
                at_pos[0].end,
                at_pos[0].replacement.clone(),
            )],
            edits,
        );
        if post.len() != 1 {
            continue;
        }
        let post_text = match files.get(&p.file) {
            Some(t) => t,
            None => continue,
        };
        // `get` (never indexing): non-boundary or out-of-range math
        // abstains instead of panicking; in-range mismatch fails loud.
        match post_text.get(post[0].0..post[0].0.saturating_add(expected.len())) {
            Some(s) if s == expected => {}
            Some(_) => {
                return Err(move_error(
                    &p.rule,
                    &p.file,
                    "pasted item text does not match the captured cut text in the post-image",
                ));
            }
            None => continue,
        }
        // Cut absence per item: every captured text must name an item,
        // and no same-kind same-ident item may start inside the
        // translated cut spans. Unnameable captures (`impl`/exotic)
        // abstain the pair, never verify it.
        let mut wanted: Vec<(&'static str, String)> = Vec::with_capacity(texts.len());
        let mut nameable = true;
        for t in texts {
            match named_item(t) {
                Some(v) => wanted.push(v),
                None => {
                    nameable = false;
                    break;
                }
            }
        }
        if !nameable {
            continue;
        }
        let state = match states.get(&cut.file) {
            Some(s) => s,
            None => continue,
        };
        let cut_edits = full_edits.get(&cut.file).map(Vec::as_slice).unwrap_or(&[]);
        let translated =
            translate_spans_to_post(&state.original, &state.index, cut_edits, &cut.spans);
        let post_file = match post_parsed.get(&cut.file) {
            Some(f) => f,
            None => continue,
        };
        // WI-7b same-file exclusion: the pasted block itself starts
        // inside (or exactly over) the translated cut spans whenever
        // the move stays in one file — the absence scan must ignore
        // the paste it just proved, or every same-file collection
        // move fails loud on its own output. Byte offset → start
        // line, block length = newline count of `expected` (trailing
        // `"\n"` included, blank separators counted).
        let paste_block: Option<(u32, u32)> = if p.file == cut.file {
            post_text.get(..post[0].0).map(|prefix| {
                let start = prefix.matches('\n').count() as u32 + 1;
                let len = expected.matches('\n').count() as u32;
                (start, start + len.saturating_sub(1))
            })
        } else {
            None
        };
        for item in &post_file.items {
            let (k2, id2) = match named_item_of(item) {
                Some(v) => v,
                None => continue,
            };
            if !wanted.iter().any(|(k, id)| *k == k2 && *id == id2) {
                continue;
            }
            let line = item.span().start().line as u32;
            if let Some((bs, be)) = paste_block {
                if line >= bs && line <= be {
                    continue;
                }
            }
            let line = item.span().start().line as u32;
            if translated.iter().any(|(s, e)| line >= *s && line <= *e) {
                return Err(move_error(
                    &cut.rule,
                    &cut.file,
                    "cut item is still present at the cut span in the post-image",
                ));
            }
        }
        for r in rules.iter_mut() {
            if r.rule == p.rule && r.file == p.file {
                r.semantics = RuleSemantics::Postcondition;
            }
            if r.rule == cut.rule && r.file == cut.file {
                r.semantics = RuleSemantics::Postcondition;
            }
        }
    }
    Ok(())
}

/// WI-7d: upgrade verified pure insertions to `postcondition`.
///
/// A pure insertion (`+ <text>`, no `# use=`) verifies iff the post
/// image carries exactly the spliced bytes at the byte-exact post
/// offset, the file re-parses, and the inserted slice parses
/// standalone as Rust. Single-edit-at-pos plus
/// `replacement == expected` guards mirror the pair probe (two
/// inserts at one anchor, or any drift between planning and probing,
/// abstain — never verify).
pub(crate) fn upgrade_pure_insertions(
    inserts: &[PureInsert],
    full_edits: &HashMap<String, Vec<TextEdit>>,
    files: &HashMap<String, String>,
    post_parsed: &HashMap<String, syn::File>,
    rules: &mut [RuleReport],
) -> Result<(), StructuredError> {
    for ins in inserts {
        // File must re-parse: the insertion is only structural when it
        // lands in a valid Rust file (global reparse already gates,
        // the missing parse here just abstains).
        if post_parsed.get(&ins.file).is_none() {
            continue;
        }
        if !insertion_text_parses(&ins.expected) {
            continue;
        }
        let edits = full_edits.get(&ins.file).map(Vec::as_slice).unwrap_or(&[]);
        let at_pos: Vec<&TextEdit> = edits
            .iter()
            .filter(|e| e.start == ins.pos && e.end == ins.pos)
            .collect();
        if at_pos.len() != 1 {
            continue;
        }
        if at_pos[0].replacement != ins.expected {
            continue;
        }
        let post = crate::postcheck_follow::post_starts_for(
            &[(
                at_pos[0].start,
                at_pos[0].end,
                at_pos[0].replacement.clone(),
            )],
            edits,
        );
        if post.len() != 1 {
            continue;
        }
        let post_text = match files.get(&ins.file) {
            Some(t) => t,
            None => continue,
        };
        match post_text.get(post[0].0..post[0].0.saturating_add(ins.expected.len())) {
            Some(s) if s == ins.expected => {}
            Some(_) => {
                return Err(StructuredError::new(
                    rsmpl_patch::ErrorCode::MatchConflict,
                    "postcondition: insertion did not land as authored (inserted text does not match the post-image at the anchor)",
                )
                .with_rule(&ins.rule)
                .with_file(&ins.file)
                .with_repair(StructuredError::fix_pattern_repair(
                    "insertion",
                    "read the reported spans in the post-image file; fix the insertion anchor so the inserted item lands as authored",
                )));
            }
            None => continue,
        }
        for r in rules.iter_mut() {
            if r.rule == ins.rule && r.file == ins.file {
                r.semantics = RuleSemantics::Postcondition;
            }
        }
    }
    Ok(())
}

/// WI-7d: upgrade verified unpaired cuts (deletions) to `postcondition`.
///
/// A cut with no same-name paste verifies iff every captured item's
/// old kind+ident is absent from the translated cut spans in a
/// re-parsing post-image. There is no paste block to exclude (unlike
/// the pair probe). Cuts that do have a same-name paste belong to
/// the pair probe and are skipped here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn upgrade_unpaired_cuts(
    cuts: &[MoveCut],
    pastes: &[MovePaste],
    captures: &CaptureStore,
    states: &HashMap<String, FileState>,
    full_edits: &HashMap<String, Vec<TextEdit>>,
    post_parsed: &HashMap<String, syn::File>,
    rules: &mut [RuleReport],
) -> Result<(), StructuredError> {
    use std::collections::HashSet;
    let paired: HashSet<&str> = pastes.iter().map(|p| p.name.as_str()).collect();
    for cut in cuts {
        if paired.contains(cut.name.as_str()) {
            continue;
        }
        let (texts, _) = match captures.get_all(&cut.name, &cut.rule, &cut.file) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if texts.is_empty() {
            continue;
        }
        let mut wanted: Vec<(&'static str, String)> = Vec::with_capacity(texts.len());
        let mut nameable = true;
        for t in texts {
            match named_item(t) {
                Some(v) => wanted.push(v),
                None => {
                    nameable = false;
                    break;
                }
            }
        }
        if !nameable {
            continue;
        }
        let state = match states.get(&cut.file) {
            Some(s) => s,
            None => continue,
        };
        let cut_edits = full_edits.get(&cut.file).map(Vec::as_slice).unwrap_or(&[]);
        let translated =
            translate_spans_to_post(&state.original, &state.index, cut_edits, &cut.spans);
        if translated.is_empty() {
            continue;
        }
        let post_file = match post_parsed.get(&cut.file) {
            Some(f) => f,
            None => continue,
        };
        for item in &post_file.items {
            let (k2, id2) = match named_item_of(item) {
                Some(v) => v,
                None => continue,
            };
            if !wanted.iter().any(|(k, id)| *k == k2 && *id == id2) {
                continue;
            }
            let line = item.span().start().line as u32;
            if translated.iter().any(|(s, e)| line >= *s && line <= *e) {
                return Err(move_error(
                    &cut.rule,
                    &cut.file,
                    "cut item is still present at the cut span in the post-image",
                ));
            }
        }
        for r in rules.iter_mut() {
            if r.rule == cut.rule && r.file == cut.file {
                r.semantics = RuleSemantics::Postcondition;
            }
        }
    }
    Ok(())
}

/// WI-7d-follow-up: upgrade verified member additions to `postcondition`.
///
/// A member addition (`FieldAddition` / `VariantAddition` / `ArmAddition`
/// point-insertion) verifies iff the post image carries exactly the
/// spliced bytes at the byte-exact post offset, the file re-parses, and
/// the container holds the member in the post-image (`struct` holds the
/// field / `enum` holds the variant / a same-scrutinee `match` holds the
/// arm). Entries group by (rule, file) so a file-scope sweep needs every
/// recorded insert to verify (all-or-nothing). Single-edit-at-pos plus
/// `replacement == expected` guards mirror the pure-insertion probe (two
/// inserts at one anchor, or any drift between planning and probing,
/// abstain — never verify).
pub(crate) fn upgrade_member_additions(
    additions: &[MemberAddition],
    full_edits: &HashMap<String, Vec<TextEdit>>,
    files: &HashMap<String, String>,
    post_parsed: &HashMap<String, syn::File>,
    rules: &mut [RuleReport],
) -> Result<(), StructuredError> {
    use std::collections::{HashMap as Map, HashSet};
    // Group recorded inserts by (rule, file): a sweep verifies only when
    // every insert verifies.
    let mut by_rule: Map<(String, String), Vec<&MemberAddition>> = Map::new();
    for a in additions {
        by_rule
            .entry((a.rule.clone(), a.file.clone()))
            .or_default()
            .push(a);
    }
    // Track which (rule, file) groups earn the upgrade; apply after the
    // loop so a loud failure below still surfaces before any mutation.
    let mut verified: HashSet<(String, String)> = HashSet::new();
    for ((rule, file), group) in &by_rule {
        let post_file = match post_parsed.get(file) {
            Some(f) => f,
            None => continue,
        };
        let mut group_ok = true;
        for add in group {
            let edits = full_edits.get(file).map(Vec::as_slice).unwrap_or(&[]);
            let at_pos: Vec<&TextEdit> = edits
                .iter()
                .filter(|e| e.start == add.pos && e.end == add.pos)
                .collect();
            if at_pos.len() != 1 {
                group_ok = false;
                break;
            }
            if at_pos[0].replacement != add.expected {
                group_ok = false;
                break;
            }
            let post = crate::postcheck_follow::post_starts_for(
                &[(
                    at_pos[0].start,
                    at_pos[0].end,
                    at_pos[0].replacement.clone(),
                )],
                edits,
            );
            if post.len() != 1 {
                group_ok = false;
                break;
            }
            let post_text = match files.get(file) {
                Some(t) => t,
                None => {
                    group_ok = false;
                    break;
                }
            };
            match post_text.get(post[0].0..post[0].0.saturating_add(add.expected.len())) {
                Some(s) if s == add.expected => {}
                Some(_) => {
                    return Err(StructuredError::new(
                        rsmpl_patch::ErrorCode::MatchConflict,
                        "postcondition: member addition did not land as authored (inserted text does not match the post-image at the anchor)",
                    )
                    .with_rule(rule)
                    .with_file(file)
                    .with_repair(StructuredError::fix_pattern_repair(
                        "member-addition",
                        "read the reported spans in the post-image file; fix the addition anchor so the member lands inside its container as authored",
                    )));
                }
                None => {
                    group_ok = false;
                    break;
                }
            }
            if !member_present(post_file, &add.kind) {
                group_ok = false;
                break;
            }
        }
        if group_ok && !group.is_empty() {
            verified.insert((rule.clone(), file.clone()));
        }
    }
    for r in rules.iter_mut() {
        if verified.contains(&(r.rule.clone(), r.file.clone())) {
            r.semantics = RuleSemantics::Postcondition;
        }
    }
    Ok(())
}

/// WI-7d-follow-up container membership: the structural half of the
/// member-addition verdict (mirrors the matcher-side naming discipline:
/// field ident equality, variant last-segment equality, arm
/// last-segment equality under a token-normalised scrutinee).
pub(crate) fn member_present(post_file: &syn::File, kind: &MemberKind) -> bool {
    match kind {
        MemberKind::Field { container, member } => {
            struct FieldProbe<'ast> {
                name: &'ast str,
                want: &'ast str,
                found: bool,
            }
            impl<'ast> syn::visit::Visit<'ast> for FieldProbe<'ast> {
                fn visit_item_struct(&mut self, s: &'ast syn::ItemStruct) {
                    if s.ident == self.name {
                        if let syn::Fields::Named(f) = &s.fields {
                            if f.named
                                .iter()
                                .any(|f| f.ident.as_ref().is_some_and(|i| i == self.want))
                            {
                                self.found = true;
                            }
                        }
                    }
                    syn::visit::visit_item_struct(self, s);
                }
            }
            let mut probe = FieldProbe {
                name: container,
                want: member,
                found: false,
            };
            syn::visit::Visit::visit_file(&mut probe, post_file);
            probe.found
        }
        MemberKind::Variant { container, member } => {
            struct VariantProbe<'ast> {
                name: &'ast str,
                want: &'ast str,
                found: bool,
            }
            impl<'ast> syn::visit::Visit<'ast> for VariantProbe<'ast> {
                fn visit_item_enum(&mut self, e: &'ast syn::ItemEnum) {
                    if e.ident == self.name && e.variants.iter().any(|v| v.ident == self.want) {
                        self.found = true;
                    }
                    syn::visit::visit_item_enum(self, e);
                }
            }
            let mut probe = VariantProbe {
                name: container,
                want: member,
                found: false,
            };
            syn::visit::Visit::visit_file(&mut probe, post_file);
            probe.found
        }
        MemberKind::Arm { member, scrutinee } => {
            struct ArmProbe {
                want: String,
                scrutinee: String,
                found: bool,
            }
            impl<'ast> syn::visit::Visit<'ast> for ArmProbe {
                fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
                    // Shared with the `ArmAddition` classifier.
                    let got: String = rsmpl_pattern::norm_arm_expr(&node.expr);
                    if got == self.scrutinee {
                        for arm in &node.arms {
                            let name: Option<String> = match &arm.pat {
                                syn::Pat::Ident(pi)
                                    if pi.by_ref.is_none()
                                        && pi.mutability.is_none()
                                        && pi.subpat.is_none() =>
                                {
                                    Some(pi.ident.to_string())
                                }
                                syn::Pat::Path(pp) if pp.qself.is_none() => {
                                    pp.path.segments.last().map(|s| s.ident.to_string())
                                }
                                syn::Pat::TupleStruct(ps) => {
                                    ps.path.segments.last().map(|s| s.ident.to_string())
                                }
                                syn::Pat::Struct(ps) => {
                                    ps.path.segments.last().map(|s| s.ident.to_string())
                                }
                                _ => None,
                            };
                            if name.as_deref() == Some(self.want.as_str()) {
                                self.found = true;
                            }
                        }
                    }
                    syn::visit::visit_expr_match(self, node);
                }
            }
            // Expression-position matches are `Expr`, not `Item`: route
            // every expression through the match probe (matcher
            // discipline in `matcher_member_evo`).
            struct FileWalk {
                probe: ArmProbe,
            }
            impl<'ast> syn::visit::Visit<'ast> for FileWalk {
                fn visit_expr(&mut self, node: &'ast syn::Expr) {
                    if let syn::Expr::Match(m) = node {
                        syn::visit::Visit::visit_expr_match(&mut self.probe, m);
                    } else {
                        syn::visit::visit_expr(self, node);
                    }
                }
            }
            let mut walk = FileWalk {
                probe: ArmProbe {
                    want: member.clone(),
                    scrutinee: scrutinee.clone(),
                    found: false,
                },
            };
            syn::visit::Visit::visit_file(&mut walk, post_file);
            walk.probe.found
        }
    }
}

/// WI-7d: the inserted slice must parse standalone as Rust — an item,
/// a statement, an expression, a type, or a multi-item file. Anything
/// else (a lone `}`, a half-line fragment) abstains, never verifies.
fn insertion_text_parses(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return false;
    }
    if syn::parse_str::<syn::Item>(t).is_ok() {
        return true;
    }
    if syn::parse_str::<syn::Stmt>(t).is_ok() {
        return true;
    }
    if syn::parse_str::<syn::Expr>(t).is_ok() {
        return true;
    }
    if syn::parse_str::<syn::Type>(t).is_ok() {
        return true;
    }
    if syn::parse_str::<syn::File>(t)
        .map(|f| !f.items.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    false
}

/// WI-7c loud failure: the pair claimed a move but the post-image
/// disagrees at the recorded span (planner bug, WI-6 doctrine —
/// `RSMPL_MATCH_CONFLICT` so agents reuse the match-conflict repair
/// path).
fn move_error(rule: &str, file: &str, detail: &str) -> StructuredError {
    StructuredError::new(
        rsmpl_patch::ErrorCode::MatchConflict,
        format!("postcondition: move did not land as authored ({detail})"),
    )
    .with_rule(rule)
    .with_file(file)
    .with_repair(StructuredError::fix_pattern_repair(
        "move",
        "read the reported spans in the post-image file; fix the cut range or paste anchor so the pair names one item move",
    ))
}

/// WI-7c: template-replayed expected bytes for an evolving paste.
///
/// Mirrors `paste_capture` byte-for-byte without touching the edit
/// list: scalar splices once (`trim_end + "\n"`), collections replay
/// per item (`N`-hole concretize + splice, `"\n\n"` join, trailing
/// `"\n"`). Returns `None` (abstain, never loud) when the template is
/// missing, hole-less, ident-less, kind-mismatched, or unspliceable —
/// the same shapes `paste_capture` rejects loud at apply time or that
/// cannot name items structurally.
fn evolved_expected(
    texts: &[String],
    collection: bool,
    template: Option<&str>,
    rule: &str,
) -> Option<String> {
    let tmpl = template?;
    if tmpl.trim().is_empty() {
        return None;
    }
    if !collection {
        if texts.len() != 1 {
            return None;
        }
        let old_named = named_item(&texts[0])?;
        let spliced = splice_evolved_header(&texts[0], tmpl, rule).ok()?;
        let new_named = named_item(&spliced)?;
        // Evolving keeps the item kind; a kind change is shape drift,
        // never a verified move.
        if new_named.0 != old_named.0 {
            return None;
        }
        let mut out = spliced.trim_end().to_string();
        if out.is_empty() {
            return None;
        }
        out.push('\n');
        return Some(out);
    }
    let (_, hole_name) = collection_template_parts(tmpl)?;
    let (pre, post) = parse_collection_name_hole(&hole_name)?;
    if texts.is_empty() {
        return None;
    }
    let mut items = Vec::with_capacity(texts.len());
    for captured in texts {
        let old_ident = captured_old_ident(captured)?;
        let old_named = named_item(captured)?;
        let new_ident = concrete_new_ident(&pre, &old_ident, &post);
        let concrete = concretize_template(tmpl, &hole_name, &new_ident);
        let spliced = splice_evolved_header(captured, &concrete, rule).ok()?;
        let new_named = named_item(&spliced)?;
        if new_named.0 != old_named.0 || new_named.1 != new_ident {
            return None;
        }
        items.push(spliced);
    }
    let joined = items
        .iter()
        .map(|t| t.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n\n");
    if joined.is_empty() {
        return None;
    }
    Some(format!("{joined}\n"))
}

/// Named-item identity of a captured text: kind tag + ident for the nine
/// nameable item kinds. `impl` blocks and exotic items decline (`None`).
fn named_item(captured: &str) -> Option<(&'static str, String)> {
    let item: syn::Item = syn::parse_str(captured.trim()).ok()?;
    named_item_of(&item)
}

/// Named-item identity of a parsed item (same discipline as `named_item`).
fn named_item_of(item: &syn::Item) -> Option<(&'static str, String)> {
    match item {
        syn::Item::Fn(i) => Some(("fn", i.sig.ident.to_string())),
        syn::Item::Struct(i) => Some(("struct", i.ident.to_string())),
        syn::Item::Enum(i) => Some(("enum", i.ident.to_string())),
        syn::Item::Type(i) => Some(("type", i.ident.to_string())),
        syn::Item::Const(i) => Some(("const", i.ident.to_string())),
        syn::Item::Static(i) => Some(("static", i.ident.to_string())),
        syn::Item::Mod(i) => Some(("mod", i.ident.to_string())),
        syn::Item::Trait(i) => Some(("trait", i.ident.to_string())),
        syn::Item::Union(i) => Some(("union", i.ident.to_string())),
        _ => None,
    }
}

#[cfg(test)]
#[path = "postcheck_move_sweep_tests.rs"]
mod sweep_tests;
#[cfg(test)]
#[path = "postcheck_move_tests.rs"]
mod tests;
