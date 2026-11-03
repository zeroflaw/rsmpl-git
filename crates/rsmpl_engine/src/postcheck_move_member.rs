//! WI-7d-follow-up member-addition verdict (SPEC 24.6).
//!
//! D5 sibling of `postcheck_move.rs`: the member-addition
//! container probe moved verbatim here so `postcheck_move.rs`
//! plus its inline tests stays under the 2000-line gate.
//! `postcheck_move.rs` re-exports the four names, so existing
//! `crate::postcheck_move::{...}` paths keep working.

use std::collections::HashMap;

use rsmpl_patch::StructuredError;

use crate::edits::TextEdit;
use crate::{RuleReport, RuleSemantics};

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::edits::TextEdit;
    use crate::lines::LineIndex;
    use crate::postcheck_move::{upgrade_member_additions, MemberAddition, MemberKind};
    use crate::{RuleReport, RuleSemantics};

    /// Forged two-container group: fields `S::b` + `T::y` under one
    /// (rule, file) with point-insertions at two anchors. `second_present`
    /// controls whether the post-image carries the `T::y` insert (the file
    /// still parses either way, so the probe always reaches the
    /// membership half).
    struct Harness {
        full_edits: HashMap<String, Vec<TextEdit>>,
        files: HashMap<String, String>,
        post_parsed: HashMap<String, syn::File>,
        additions: Vec<MemberAddition>,
        rules: Vec<RuleReport>,
    }

    fn two_container_harness(second_present: bool) -> Harness {
        let original = "struct S {\n    a: u32,\n}\nstruct T {\n    x: u8,\n}\n";
        let index = LineIndex::new(original);
        let pos_s = index.line_start(3).unwrap();
        let pos_t = index.line_start(6).unwrap();
        let exp_s = "    b: bool,\n".to_string();
        let exp_t = "    y: u16,\n".to_string();
        let full_edits = HashMap::from([(
            "f.rs".to_string(),
            vec![
                TextEdit::new(pos_s, pos_s, exp_s.clone()),
                TextEdit::new(pos_t, pos_t, exp_t.clone()),
            ],
        )]);
        // Higher offset first: the earlier splice must not shift the later
        // anchor.
        let mut post_text = original.to_string();
        if second_present {
            post_text.insert_str(pos_t, &exp_t);
        }
        post_text.insert_str(pos_s, &exp_s);
        let post_parsed = HashMap::from([(
            "f.rs".to_string(),
            syn::parse_str::<syn::File>(&post_text).expect("post parses"),
        )]);
        let additions = vec![
            MemberAddition {
                rule: "add".to_string(),
                file: "f.rs".to_string(),
                pos: pos_s,
                expected: exp_s,
                kind: MemberKind::Field {
                    container: "S".to_string(),
                    member: "b".to_string(),
                },
            },
            MemberAddition {
                rule: "add".to_string(),
                file: "f.rs".to_string(),
                pos: pos_t,
                expected: exp_t,
                kind: MemberKind::Field {
                    container: "T".to_string(),
                    member: "y".to_string(),
                },
            },
        ];
        let rules = vec![RuleReport {
            rule: "add".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        }];
        Harness {
            full_edits,
            files: HashMap::from([("f.rs".to_string(), post_text)]),
            post_parsed,
            additions,
            rules,
        }
    }

    #[test]
    fn multi_container_sweep_group_verifies_postcondition() {
        // Both members present in their containers: the group earns
        // `postcondition` (all-or-nothing over containers, not just
        // inserts).
        let mut h = two_container_harness(true);
        upgrade_member_additions(
            &h.additions,
            &h.full_edits,
            &h.files,
            &h.post_parsed,
            &mut h.rules,
        )
        .expect("verifies");
        assert_eq!(h.rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn multi_container_sweep_group_partial_stays_reparse_only() {
        // The second member never landed (byte slice out of range, then
        // membership would fail too): the group abstains — never a partial
        // upgrade — so the entry stays `reparse-only`.
        let mut h = two_container_harness(false);
        upgrade_member_additions(
            &h.additions,
            &h.full_edits,
            &h.files,
            &h.post_parsed,
            &mut h.rules,
        )
        .expect("abstains");
        assert_eq!(h.rules[0].semantics, RuleSemantics::ReparseOnly);
    }
}
