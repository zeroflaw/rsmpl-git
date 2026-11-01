//! WI-7d evolution-probe follow-up: member-evolution structural verdict
//! (SPEC 24.6).
//!
//! `FieldTypeEvolution` and `VariantPayloadEvolution` (tuple / struct /
//! discriminant) — plus the WI-5 joint kinds (`StructRenameFieldEvo`,
//! `EnumRenameVariantEvo`, recorded under their *new* container by
//! `plan.rs`) — prove their *content* through the plus re-match
//! (`postcheck.rs` — the synthesised `[struct S {, field: plus]` /
//! `[enum E {, variant(payload)]` pattern re-matches at the reported
//! spans with binding equality). This module adds the second structural
//! leg, mirroring the member-addition discipline: the post-image
//! container must still hold the member in the re-parsing post-image
//! (struct holds the field / enum holds the variant). Content plus
//! container integrity earns `postcondition` on the entry; sweeps need
//! every recorded evolution to verify (all-or-nothing over the
//! (rule, file) group).
//!
//! Only `Instantiated` rules upgrade (the plus gate already ran and
//! passed — ranged evolutions; sweep evolutions already report
//! `postcondition` via the sweep gate). Misses abstain, never loud:
//! content mismatch already fails loud in the plus gate, and a
//! membership miss past a passed plus gate can only mean an
//! unparseable or macro-hidden post-image, which never vetoes.
//! Discriminant evolutions record `Variant` membership: variant
//! presence plus plus-proven discriminant content is the full verdict
//! (a lost discriminant fails the plus gate first).

use std::collections::{HashMap as Map, HashSet};

use crate::postcheck_move::{member_present, MemberKind};
use crate::{RuleReport, RuleSemantics};

/// WI-7d evolution-probe pending member evolution: rule/file plus the
/// container key. One entry per evolution hunk (the container and the
/// member are concrete idents, never holes, so one key covers every
/// match of the hunk). No byte half — the plus re-match owns content.
#[derive(Clone, Debug)]
pub(crate) struct MemberEvo {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) kind: MemberKind,
}

/// WI-7d evolution-probe: upgrade verified member evolutions to
/// `postcondition`.
///
/// Groups recorded evolutions by (rule, file); a group verifies iff
/// the post-image parses and every evolution's container holds its
/// member there. Verified groups upgrade `Instantiated` entries to
/// `Postcondition` (`Postcondition` sweeps and `ReparseOnly`
/// plus-misses are untouched).
pub(crate) fn upgrade_member_evolutions(
    evos: &[MemberEvo],
    post_parsed: &Map<String, syn::File>,
    rules: &mut [RuleReport],
) {
    let mut by_rule: Map<(String, String), Vec<&MemberEvo>> = Map::new();
    for e in evos {
        by_rule
            .entry((e.rule.clone(), e.file.clone()))
            .or_default()
            .push(e);
    }
    let mut verified: HashSet<(String, String)> = HashSet::new();
    for ((rule, file), group) in &by_rule {
        let post_file = match post_parsed.get(file) {
            Some(f) => f,
            None => continue,
        };
        if !group.is_empty() && group.iter().all(|e| member_present(post_file, &e.kind)) {
            verified.insert((rule.clone(), file.clone()));
        }
    }
    for r in rules.iter_mut() {
        if r.semantics == RuleSemantics::Instantiated
            && verified.contains(&(r.rule.clone(), r.file.clone()))
        {
            r.semantics = RuleSemantics::Postcondition;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> syn::File {
        syn::parse_str::<syn::File>(text).expect("test post-image parses")
    }

    fn evo(rule: &str, file: &str, kind: MemberKind) -> MemberEvo {
        MemberEvo {
            rule: rule.to_string(),
            file: file.to_string(),
            kind,
        }
    }

    fn rules_one() -> Vec<RuleReport> {
        vec![RuleReport {
            rule: "evo".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::Instantiated,
            ..Default::default()
        }]
    }

    #[test]
    fn field_evolution_present_verifies_postcondition() {
        let post = parsed("struct S {\n    old: Box<[u8]>,\n}\n");
        let evos = vec![evo(
            "evo",
            "f.rs",
            MemberKind::Field {
                container: "S".to_string(),
                member: "old".to_string(),
            },
        )];
        let mut rules = rules_one();
        upgrade_member_evolutions(&evos, &Map::from([("f.rs".to_string(), post)]), &mut rules);
        assert_eq!(rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn variant_evolution_missing_stays_instantiated() {
        // The enum no longer holds the variant (renamed away): the
        // group abstains — never a partial upgrade, never loud.
        let post = parsed("enum E {\n    Other(u8),\n}\n");
        let evos = vec![evo(
            "evo",
            "f.rs",
            MemberKind::Variant {
                container: "E".to_string(),
                member: "Old".to_string(),
            },
        )];
        let mut rules = rules_one();
        upgrade_member_evolutions(&evos, &Map::from([("f.rs".to_string(), post)]), &mut rules);
        assert_eq!(rules[0].semantics, RuleSemantics::Instantiated);
    }

    #[test]
    fn multi_entry_group_needs_every_evo() {
        // Two evolutions under one (rule, file): one missing member
        // vetoes the group (all-or-nothing, same as additions).
        let post = parsed("struct S {\n    a: u32,\n}\nstruct T {\n    y: u16,\n}\n");
        let both = vec![
            evo(
                "evo",
                "f.rs",
                MemberKind::Field {
                    container: "S".to_string(),
                    member: "a".to_string(),
                },
            ),
            evo(
                "evo",
                "f.rs",
                MemberKind::Field {
                    container: "T".to_string(),
                    member: "missing".to_string(),
                },
            ),
        ];
        let mut rules = rules_one();
        upgrade_member_evolutions(
            &both,
            &Map::from([("f.rs".to_string(), post.clone())]),
            &mut rules,
        );
        assert_eq!(rules[0].semantics, RuleSemantics::Instantiated);
        // Both present: the group earns `postcondition`.
        let both_present = vec![
            evo(
                "evo",
                "f.rs",
                MemberKind::Field {
                    container: "S".to_string(),
                    member: "a".to_string(),
                },
            ),
            evo(
                "evo",
                "f.rs",
                MemberKind::Field {
                    container: "T".to_string(),
                    member: "y".to_string(),
                },
            ),
        ];
        upgrade_member_evolutions(
            &both_present,
            &Map::from([("f.rs".to_string(), post)]),
            &mut rules,
        );
        assert_eq!(rules[0].semantics, RuleSemantics::Postcondition);
    }

    #[test]
    fn reparse_only_never_upgrades() {
        // A plus-miss (`ReparseOnly`) stays honest even when the
        // container holds the member — membership never over-claims
        // content the plus gate did not prove.
        let post = parsed("struct S {\n    old: Vec<u8>,\n}\n");
        let evos = vec![evo(
            "evo",
            "f.rs",
            MemberKind::Field {
                container: "S".to_string(),
                member: "old".to_string(),
            },
        )];
        let mut rules = vec![RuleReport {
            rule: "evo".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        }];
        upgrade_member_evolutions(&evos, &Map::from([("f.rs".to_string(), post)]), &mut rules);
        assert_eq!(rules[0].semantics, RuleSemantics::ReparseOnly);
    }
}
