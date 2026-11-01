//! WI-7d-follow-up residue pins: multi-container sweep groups verify
//! all-or-nothing (D5 sibling of `postcheck_move.rs`, `plan_tests.rs`
//! precedent — the core file sits at ~1900 lines and two inline pins
//! would push it over the 2000 gate).
//!
//! A single `struct S` sweep hunk can only record one container, so the
//! multi-container group shape is forged directly (no re-planning),
//! mirroring the forged pins in `postcheck_move.rs`'s inline tests.

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
