//! D5 sibling of `postcheck_move.rs` (`plan_tests.rs` precedent):
//! the inline `mod tests` body moved verbatim here (dedented one level).
//! Prod code stays in `postcheck_move.rs`; new move-probe pins land here
//! (or in `postcheck_move_sweep_tests.rs` for forged sweep groups).

use super::*;
use crate::lines::LineIndex;

const SRC: &str = "fn a() {}\nfn helper() {}\nfn z() {}\n";
const CUT_PATCH: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -0,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";

fn apply_report(
    patch_text: &str,
    initial: Vec<(&str, &str)>,
) -> (HashMap<String, String>, crate::ApplyReport) {
    let patch = rsmpl_patch::parse_patch(patch_text).expect("patch parses");
    let mut files: HashMap<String, String> = initial
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let report = crate::plan::apply_with_captures(&mut files, &patch).expect("applies");
    (files, report)
}

fn semantics_of(report: &crate::ApplyReport, rule: &str) -> String {
    report
        .rules
        .iter()
        .find(|r| r.rule == rule)
        .map(|r| r.semantics.as_str().to_string())
        .unwrap_or_else(|| "missing".to_string())
}

#[test]
fn single_file_verbatim_move_verifies_postcondition() {
    let (files, report) = apply_report(CUT_PATCH, vec![("src/lib.rs", SRC)]);
    assert_eq!(
        files["src/lib.rs"], "fn helper() {}\nfn a() {}\nfn z() {}\n",
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_helper"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_helper"), "postcondition");
}

#[test]
fn downward_move_verifies_postcondition() {
    // Paste anchor past the cut: translation shifts both directions.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert_eq!(
        files["src/lib.rs"], "fn a() {}\nfn z() {}\nfn helper() {}\n",
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_helper"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_helper"), "postcondition");
}

#[test]
fn cross_file_verbatim_move_verifies_postcondition() {
    let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n\ndiff --git a/src/new.rs b/src/new.rs\n--- a/src/new.rs\n+++ b/src/new.rs\n@@ -1,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n";
    let (files, report) = apply_report(
        patch,
        vec![("src/old.rs", SRC), ("src/new.rs", "fn main() {}\n")],
    );
    assert_eq!(files["src/old.rs"], "fn a() {}\nfn z() {}\n");
    assert_eq!(files["src/new.rs"], "fn main() {}\nfn helper() {}\n");
    assert_eq!(semantics_of(&report, "cut_helper"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_helper"), "postcondition");
}

#[test]
fn evolving_paste_verifies_postcondition() {
    // WI-7c: scalar evolving paste verifies per item via the
    // template replay (old ident absent, evolved bytes present).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_new\n# lang=rust\n# use=Helper\n+ pub fn renamed() { ... }\n\n@@ -2,3 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
    let src = "fn a() {}\nfn helper() {\n    1\n}\nfn z() {}\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", src)]);
    assert!(
        files["src/lib.rs"].contains("pub fn renamed()"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert!(
        !files["src/lib.rs"].contains("fn helper()"),
        "old ident must be gone, got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_helper"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_new"), "postcondition");
}

#[test]
fn evolving_cross_file_verifies_postcondition() {
    // WI-7c cross-file: presence in the paste file, old-absence in
    // the cut file. Single-line capture needs the abstract
    // `(...)` template (concrete signatures need multi-line bodies).
    let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n\ndiff --git a/src/new.rs b/src/new.rs\n--- a/src/new.rs\n+++ b/src/new.rs\n@@ -1,0 @@ rule=paste_new\n# lang=rust\n# use=Helper\n+ pub fn renamed(...) { ... }\n";
    let (files, report) = apply_report(
        patch,
        vec![("src/old.rs", SRC), ("src/new.rs", "fn main() {}\n")],
    );
    assert_eq!(files["src/old.rs"], "fn a() {}\nfn z() {}\n");
    assert!(
        files["src/new.rs"].contains("fn renamed()"),
        "got: {}",
        files["src/new.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_helper"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_new"), "postcondition");
}

#[test]
fn evolving_collection_verifies_per_item() {
    // WI-7c: a 2-item collection cut + `new_N` evolving paste replays
    // per item (`new_a`, `new_b`) and verifies both entries.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,0 @@ rule=paste_all\n# lang=rust\n# use=Helpers\n+ fn new_N(...) { ... }\n\n@@ -1,2 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n";
    let src = "fn a() {}\nfn b() {}\nfn z() {}\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", src)]);
    assert!(
        files["src/lib.rs"].contains("fn new_a()"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert!(
        files["src/lib.rs"].contains("fn new_b()"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_all"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_all"), "postcondition");
}

#[test]
fn tampered_evolving_post_fails_loud() {
    // WI-7c in-range mismatch fails loud: forge an evolving pair
    // whose post-image carries the wrong new ident at the offset.
    let original = "fn a() {}\nfn helper() {}\nfn z() {}\n";
    let index = LineIndex::new(original);
    let cut_start = index.line_start(2).unwrap();
    let cut_end = index.line_start(3).unwrap();
    let captured = original[cut_start..cut_end].to_string();
    let template = "pub fn renamed(...) { ... }";
    let spliced = crate::capture::splice_evolved_header(&captured, template, "paste_h")
        .expect("splice works");
    let mut expected = spliced.trim_end().to_string();
    expected.push('\n');
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![
            TextEdit::new(0, 0, expected.clone()),
            TextEdit::new(cut_start, cut_end, String::new()),
        ],
    )]);
    // Post carries a different new ident at the same offset.
    let post_text = "pub fn wrong() {}\nfn a() {}\nfn z() {}\n";
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(post_text).expect("post parses"),
    )]);
    let mut captures = CaptureStore::default();
    captures
        .define("H", captured, "cut_h", "f.rs")
        .expect("bind");
    let cuts = vec![MoveCut {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        spans: vec![(2, 2)],
        name: "H".to_string(),
    }];
    let pastes = vec![MovePaste {
        rule: "paste_h".to_string(),
        file: "f.rs".to_string(),
        name: "H".to_string(),
        pos: 0,
        bare: false,
        template: Some(template.to_string()),
    }];
    let mut rules = vec![
        RuleReport {
            rule: "cut_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
        RuleReport {
            rule: "paste_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
    ];
    let states = HashMap::from([(
        "f.rs".to_string(),
        FileState {
            original: original.to_string(),
            index: LineIndex::new(original),
            parsed: Some(syn::parse_str::<syn::File>(original).expect("pre parses")),
        },
    )]);
    let files = HashMap::from([("f.rs".to_string(), post_text.to_string())]);
    let err = upgrade_verified_moves(
        &cuts,
        &pastes,
        &captures,
        &states,
        &full_edits,
        &files,
        &post_parsed,
        &mut rules,
    )
    .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn same_anchor_double_evolving_paste_stays_reparse_only() {
    // T249 join mutates the evolving replacement too, so two
    // evolving pastes of one capture at one anchor abstain.
    // Single-line capture needs the abstract `(...)` template.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,0 @@ rule=paste_one\n# lang=rust\n# use=Helper\n+ pub fn renamed(...) { ... }\n\n@@ -3,0 @@ rule=paste_two\n# lang=rust\n# use=Helper\n+ pub fn renamed(...) { ... }\n\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
    let (_, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert_eq!(semantics_of(&report, "cut_helper"), "reparse-only");
    assert_eq!(semantics_of(&report, "paste_one"), "reparse-only");
    assert_eq!(semantics_of(&report, "paste_two"), "reparse-only");
}

#[test]
fn evolved_expected_rejects_hole_less_collection_template() {
    // Hole-less collection templates cannot name per-item idents:
    // the probe abstains (`None`), mirroring the apply-time loud
    // rejection without ever verifying.
    let texts = vec!["fn a() {}\n".to_string(), "fn b() {}\n".to_string()];
    assert!(evolved_expected(&texts, true, Some("fn renamed() { ... }"), "r").is_none());
}

#[test]
fn pure_insertion_verifies_postcondition() {
    // WI-7d: a pure insertion (`+ <text>`, no `# use=`) verifies by
    // byte-exact presence plus a standalone re-parse of the slice.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=ins\n+ fn inserted() {}\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert!(
        files["src/lib.rs"].contains("fn inserted() {}"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "ins"), "postcondition");
}

#[test]
fn unpaired_scalar_cut_verifies_postcondition() {
    // WI-7d: a cut with no same-name paste (a deletion) verifies by
    // old kind+ident absence in the translated cut span.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert!(
        !files["src/lib.rs"].contains("helper"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_helper"), "postcondition");
}

#[test]
fn unpaired_collection_cut_verifies_per_item() {
    // WI-7d: a collection cut with no paste verifies per item.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert!(
        !files["src/lib.rs"].contains("fn a()"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert!(
        !files["src/lib.rs"].contains("helper"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert!(
        files["src/lib.rs"].contains("fn z()"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_all"), "postcondition");
}

#[test]
fn unpaired_cut_returning_item_fails_loud() {
    // WI-7d in-range mismatch fails loud: forge an unpaired cut whose
    // post-image still carries the cut item inside the cut span.
    let original = "fn a() {}\nfn helper() {}\nfn z() {}\n";
    let index = LineIndex::new(original);
    let cut_start = index.line_start(2).unwrap();
    let cut_end = index.line_start(3).unwrap();
    let captured = original[cut_start..cut_end].to_string();
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![TextEdit::new(cut_start, cut_end, String::new())],
    )]);
    // Post still carries the item at the cut span: absence must fail.
    let post_text = original.to_string();
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(&post_text).expect("post parses"),
    )]);
    let mut captures = CaptureStore::default();
    captures
        .define("H", captured, "cut_h", "f.rs")
        .expect("bind");
    let cuts = vec![MoveCut {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        spans: vec![(2, 2)],
        name: "H".to_string(),
    }];
    let mut rules = vec![RuleReport {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        semantics: RuleSemantics::ReparseOnly,
        ..Default::default()
    }];
    let states = HashMap::from([(
        "f.rs".to_string(),
        FileState {
            original: original.to_string(),
            index: LineIndex::new(original),
            parsed: Some(syn::parse_str::<syn::File>(original).expect("pre parses")),
        },
    )]);
    let err = upgrade_unpaired_cuts(
        &cuts,
        &[],
        &captures,
        &states,
        &full_edits,
        &post_parsed,
        &mut rules,
    )
    .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn unparseable_insertion_stays_reparse_only() {
    // WI-7d: an inserted slice that parses neither as item/stmt/expr/
    // type/file abstains — never a structural verdict.
    assert!(!insertion_text_parses("} garbage { { {"));
}

#[test]
fn same_anchor_double_pure_insert_stays_reparse_only() {
    // WI-7d: two pure inserts at one anchor share the position, so
    // neither earns a verdict (ambiguous attribution).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=ins_one\n+ fn one() {}\n\n@@ -1,0 @@ rule=ins_two\n+ fn two() {}\n";
    let (_, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert_eq!(semantics_of(&report, "ins_one"), "reparse-only");
    assert_eq!(semantics_of(&report, "ins_two"), "reparse-only");
}

#[test]
fn field_addition_verifies_postcondition() {
    // WI-7d-follow-up: a struct-field point-insertion verifies by
    // byte-exact presence plus container membership.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 @@ rule=add\n  struct S {\n+ b: bool,\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", "struct S {\n    a: u32,\n}\n")]);
    assert!(
        files["src/lib.rs"].contains("b: bool,"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "add"), "postcondition");
}

#[test]
fn variant_addition_verifies_postcondition() {
    // WI-7d-follow-up: an enum-variant point-insertion verifies by
    // byte-exact presence plus container membership.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 @@ rule=add\n  enum E {\n+ C,\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", "enum E {\n    A,\n}\n")]);
    assert!(
        files["src/lib.rs"].contains("C,"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "add"), "postcondition");
}

#[test]
fn arm_addition_verifies_postcondition() {
    // WI-7d-follow-up: a match-arm point-insertion verifies by
    // byte-exact presence plus same-scrutinee membership.
    let pre =
        "enum E {\n    A,\n}\n\nfn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -6,1 @@ rule=arm\n  match e {\n+ E::C => 3,\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", pre)]);
    assert!(
        files["src/lib.rs"].contains("E::C => 3,"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "arm"), "postcondition");
}

#[test]
fn member_addition_sweep_spelling_verifies() {
    // WI-7d-follow-up: the file-scope (`# sweep`) spelling records the
    // same per-insert probe input and verifies (single-container pin;
    // multi-container sweeps need every insert to verify by
    // construction of the group loop).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=add\n# sweep\n  struct S {\n+ b: bool,\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", "struct S {\n    a: u32,\n}\n")]);
    assert!(
        files["src/lib.rs"].contains("b: bool,"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "add"), "postcondition");
}

#[test]
fn tampered_member_addition_post_fails_loud() {
    // WI-7d-follow-up in-range mismatch fails loud: forge a field
    // addition whose post-image carries different bytes at the
    // anchor offset.
    let original = "struct S {\n    a: u32,\n}\n";
    let index = LineIndex::new(original);
    let pos = index.line_start(3).unwrap();
    let expected = "    b: bool,\n";
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![TextEdit::new(pos, pos, expected.to_string())],
    )]);
    let post_text = "struct S {\n    a: u32,\n    c: u8,\n}\n";
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(post_text).expect("post parses"),
    )]);
    let additions = vec![MemberAddition {
        rule: "add".to_string(),
        file: "f.rs".to_string(),
        pos,
        expected: expected.to_string(),
        kind: MemberKind::Field {
            container: "S".to_string(),
            member: "b".to_string(),
        },
    }];
    let mut rules = vec![RuleReport {
        rule: "add".to_string(),
        file: "f.rs".to_string(),
        semantics: RuleSemantics::ReparseOnly,
        ..Default::default()
    }];
    let files = HashMap::from([("f.rs".to_string(), post_text.to_string())]);
    let err = upgrade_member_additions(&additions, &full_edits, &files, &post_parsed, &mut rules)
        .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn member_addition_missing_member_stays_reparse_only() {
    // WI-7d-follow-up abstains when the container does not hold the
    // member in the post-image (here the recorded scrutinee names a
    // different match, so the arm never counts as present even though
    // the bytes landed): no verdict, never loud.
    let original = "fn f(e: E) -> u32 {\n    match e {\n        E::A => 1,\n    }\n}\n";
    let index = LineIndex::new(original);
    let pos = index.line_start(4).expect("line 4");
    let expected = "        E::C => 3,\n";
    let mut post_text = original.to_string();
    post_text.insert_str(pos, expected);
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(&post_text).expect("post parses"),
    )]);
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![TextEdit::new(pos, pos, expected.to_string())],
    )]);
    let additions = vec![MemberAddition {
        rule: "arm".to_string(),
        file: "f.rs".to_string(),
        pos,
        expected: expected.to_string(),
        kind: MemberKind::Arm {
            member: "C".to_string(),
            scrutinee: "other".to_string(),
        },
    }];
    let mut rules = vec![RuleReport {
        rule: "arm".to_string(),
        file: "f.rs".to_string(),
        semantics: RuleSemantics::ReparseOnly,
        ..Default::default()
    }];
    let files = HashMap::from([("f.rs".to_string(), post_text.to_string())]);
    upgrade_member_additions(&additions, &full_edits, &files, &post_parsed, &mut rules)
        .expect("abstains");
    assert_eq!(rules[0].semantics, RuleSemantics::ReparseOnly);
}

#[test]
fn same_anchor_double_member_addition_stays_reparse_only() {
    // WI-7d-follow-up: two member inserts at one anchor share the
    // position, so neither earns a verdict (ambiguous attribution).
    // Forged directly: the single-edit-at-pos gate sees two edits.
    let original = "struct S {\n    a: u32,\n}\n";
    let index = LineIndex::new(original);
    let pos = index.line_start(3).unwrap();
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![
            TextEdit::new(pos, pos, "    b: bool,\n".to_string()),
            TextEdit::new(pos, pos, "    c: u8,\n".to_string()),
        ],
    )]);
    let post_text = "struct S {\n    a: u32,\n    b: bool,\n    c: u8,\n}\n";
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(post_text).expect("post parses"),
    )]);
    let additions = vec![MemberAddition {
        rule: "add".to_string(),
        file: "f.rs".to_string(),
        pos,
        expected: "    b: bool,\n".to_string(),
        kind: MemberKind::Field {
            container: "S".to_string(),
            member: "b".to_string(),
        },
    }];
    let mut rules = vec![RuleReport {
        rule: "add".to_string(),
        file: "f.rs".to_string(),
        semantics: RuleSemantics::ReparseOnly,
        ..Default::default()
    }];
    let files = HashMap::from([("f.rs".to_string(), post_text.to_string())]);
    upgrade_member_additions(&additions, &full_edits, &files, &post_parsed, &mut rules)
        .expect("abstains");
    assert_eq!(rules[0].semantics, RuleSemantics::ReparseOnly);
}

#[test]
fn tampered_insertion_post_fails_loud() {
    // WI-7d in-range mismatch fails loud: forge a pure insertion whose
    // post-image carries different bytes at the anchor offset.
    let expected = "fn inserted() {}\n";
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![TextEdit::new(0, 0, expected.to_string())],
    )]);
    let post_text = "fn tampered() {}\nfn a() {}\n";
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(post_text).expect("post parses"),
    )]);
    let inserts = vec![PureInsert {
        rule: "ins".to_string(),
        file: "f.rs".to_string(),
        pos: 0,
        expected: expected.to_string(),
    }];
    let mut rules = vec![RuleReport {
        rule: "ins".to_string(),
        file: "f.rs".to_string(),
        semantics: RuleSemantics::ReparseOnly,
        ..Default::default()
    }];
    let files = HashMap::from([("f.rs".to_string(), post_text.to_string())]);
    let err = upgrade_pure_insertions(&inserts, &full_edits, &files, &post_parsed, &mut rules)
        .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn unnameable_unpaired_cut_stays_reparse_only() {
    // WI-7d: an `impl`-block cut names no item, so absence cannot be
    // proven — the cut abstains like the WI-7b move does. The impl
    // range also covers the inner method span, so the cut binds the
    // collection (scalar would fail the count gate, not the probe).
    let src = "struct S;\nimpl S {\n    fn m(&self) {}\n}\nfn z() {}\n";
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n";
    let (_, report) = apply_report(patch, vec![("src/lib.rs", src)]);
    assert_eq!(semantics_of(&report, "cut_all"), "reparse-only");
}

#[test]
fn collection_move_verifies_postcondition_per_item() {
    // WI-7b: a 3-item collection cut + bare paste verifies per
    // item — joined presence plus per-item absence.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,0 @@ rule=paste_all\n# lang=rust\n# use=Helpers\n+ Helpers\n\n@@ -1,3 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert_eq!(
        files["src/lib.rs"], "fn a() {}\n\nfn helper() {}\n\nfn z() {}\n",
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_all"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_all"), "postcondition");
}

#[test]
fn collection_cross_file_move_verifies_postcondition() {
    // WI-7b cross-file: presence in the paste file, absence per
    // item in the cut file.
    let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -1,2 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n\ndiff --git a/src/new.rs b/src/new.rs\n--- a/src/new.rs\n+++ b/src/new.rs\n@@ -1,0 @@ rule=paste_all\n# lang=rust\n# use=Helpers\n+ Helpers\n";
    let (files, report) = apply_report(
        patch,
        vec![("src/old.rs", SRC), ("src/new.rs", "fn main() {}\n")],
    );
    assert_eq!(files["src/old.rs"], "fn z() {}\n");
    assert_eq!(
        files["src/new.rs"],
        "fn main() {}\nfn a() {}\n\nfn helper() {}\n"
    );
    assert_eq!(semantics_of(&report, "cut_all"), "postcondition");
    assert_eq!(semantics_of(&report, "paste_all"), "postcondition");
}

#[test]
fn tampered_collection_post_fails_loud() {
    // WI-7b in-range mismatch fails loud: forge a hand-built pair
    // whose post-image drops the middle item at the paste offset.
    let original = "fn a() {}\nfn helper() {}\nfn z() {}\n";
    let index = LineIndex::new(original);
    let cut_start = index.line_start(1).unwrap();
    let cut_end = index.line_start(3).unwrap();
    let t1 = original[index.line_start(1).unwrap()..index.line_start(2).unwrap()].to_string();
    let t2 = original[index.line_start(2).unwrap()..index.line_start(3).unwrap()].to_string();
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![
            TextEdit::new(
                original.len(),
                original.len(),
                format!("{}\n\n{}", t1.trim_end(), t2.trim_end()) + "\n",
            ),
            TextEdit::new(cut_start, cut_end, String::new()),
        ],
    )]);
    // Post swaps the second item: the presence slice stays in
    // range but mismatches, so the probe fails loud.
    let post_text = format!("fn z() {{}}\n{}\nfn tampered() {{}}\n", t1.trim_end());
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(&post_text).expect("post parses"),
    )]);
    let mut captures = CaptureStore::default();
    captures
        .define_collection("H", vec![t1, t2], "cut_h", "f.rs")
        .expect("bind");
    let cuts = vec![MoveCut {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        spans: vec![(1, 1), (2, 2)],
        name: "H".to_string(),
    }];
    let pastes = vec![MovePaste {
        rule: "paste_h".to_string(),
        file: "f.rs".to_string(),
        name: "H".to_string(),
        pos: original.len(),
        bare: true,
        template: None,
    }];
    let mut rules = vec![
        RuleReport {
            rule: "cut_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
        RuleReport {
            rule: "paste_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
    ];
    let states = HashMap::from([(
        "f.rs".to_string(),
        FileState {
            original: original.to_string(),
            index: LineIndex::new(original),
            parsed: Some(syn::parse_str::<syn::File>(original).expect("pre parses")),
        },
    )]);
    let files = HashMap::from([("f.rs".to_string(), post_text)]);
    let err = upgrade_verified_moves(
        &cuts,
        &pastes,
        &captures,
        &states,
        &full_edits,
        &files,
        &post_parsed,
        &mut rules,
    )
    .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn same_anchor_double_paste_stays_reparse_only() {
    // T249 blank-join mutates the replacement, so two bare pastes
    // of one capture at one anchor abstain (WI-7b skip).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,0 @@ rule=paste_one\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -3,0 @@ rule=paste_two\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
    let (_, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
    assert_eq!(semantics_of(&report, "cut_helper"), "reparse-only");
    assert_eq!(semantics_of(&report, "paste_one"), "reparse-only");
    assert_eq!(semantics_of(&report, "paste_two"), "reparse-only");
}

#[test]
fn unnameable_collection_stays_reparse_only() {
    // `impl` items decline `named_item`: the pair abstains.
    let src = "struct S;\nimpl S {\n    fn m(&self) {}\n}\nfn z() {}\n";
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -4,0 @@ rule=paste_all\n# lang=rust\n# use=Helpers\n+ Helpers\n\n@@ -1,4 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n";
    let (files, report) = apply_report(patch, vec![("src/lib.rs", src)]);
    // The pair applies (items move) but abstains: `impl` has no
    // named-item identity, so no structural verdict.
    assert!(
        files["src/lib.rs"].contains("impl S"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(semantics_of(&report, "cut_all"), "reparse-only");
    assert_eq!(semantics_of(&report, "paste_all"), "reparse-only");
}

/// Hand-built move state: original with a cut deletion + a paste
/// insertion, so the probe inputs can be forged without re-planning.
struct Harness {
    states: HashMap<String, FileState>,
    full_edits: HashMap<String, Vec<TextEdit>>,
    files: HashMap<String, String>,
    post_parsed: HashMap<String, syn::File>,
    captures: CaptureStore,
    cuts: Vec<MoveCut>,
    pastes: Vec<MovePaste>,
    rules: Vec<RuleReport>,
}

fn hand_built(post_override: Option<&str>) -> Harness {
    // Original lines: 1 `fn a`, 2 `fn helper`, 3 `fn z`.
    let original = SRC.to_string();
    let index = LineIndex::new(&original);
    let cut_start = index.line_start(2).unwrap();
    let cut_end = index.line_start(3).unwrap();
    let captured = original[cut_start..cut_end].to_string();
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![
            TextEdit::new(0, 0, captured.clone()),
            TextEdit::new(cut_start, cut_end, String::new()),
        ],
    )]);
    // Real post-image: helper moved to the top.
    let real_post = format!("{captured}fn a() {{}}\nfn z() {{}}\n");
    let post_text = post_override.unwrap_or(&real_post).to_string();
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(&post_text).expect("post parses"),
    )]);
    let mut captures = CaptureStore::default();
    captures
        .define("H", captured, "cut_h", "f.rs")
        .expect("bind");
    let cuts = vec![MoveCut {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        spans: vec![(2, 2)],
        name: "H".to_string(),
    }];
    let pastes = vec![MovePaste {
        rule: "paste_h".to_string(),
        file: "f.rs".to_string(),
        name: "H".to_string(),
        pos: 0,
        bare: true,
        template: None,
    }];
    let rules = vec![
        RuleReport {
            rule: "cut_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
        RuleReport {
            rule: "paste_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
    ];
    let states = HashMap::from([(
        "f.rs".to_string(),
        FileState {
            original,
            index,
            parsed: Some(syn::parse_str::<syn::File>(SRC).expect("pre parses")),
        },
    )]);
    let files = HashMap::from([("f.rs".to_string(), post_text)]);
    Harness {
        states,
        full_edits,
        files,
        post_parsed,
        captures,
        cuts,
        pastes,
        rules,
    }
}

#[test]
fn reverted_post_image_fails_loud() {
    // The post-image still carries the old text at the paste offset:
    // in-range mismatch must fail, never silently verify.
    let mut h = hand_built(Some(SRC));
    // `post_parsed` must match the (unmoved) post text.
    h.post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(SRC).expect("post parses"),
    )]);
    let err = upgrade_verified_moves(
        &h.cuts,
        &h.pastes,
        &h.captures,
        &h.states,
        &h.full_edits,
        &h.files,
        &h.post_parsed,
        &mut h.rules,
    )
    .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn cut_item_returning_fails_loud() {
    // Paste landed, but the cut item is also still at the cut span.
    let mut h = hand_built(Some(
        "fn helper() {}\nfn a() {}\nfn helper() {}\nfn z() {}\n",
    ));
    let err = upgrade_verified_moves(
        &h.cuts,
        &h.pastes,
        &h.captures,
        &h.states,
        &h.full_edits,
        &h.files,
        &h.post_parsed,
        &mut h.rules,
    )
    .unwrap_err();
    assert!(
        err.render().contains("RSMPL_MATCH_CONFLICT"),
        "got: {}",
        err.render()
    );
}

#[test]
fn clean_hand_built_pair_upgrades() {
    let mut h = hand_built(None);
    upgrade_verified_moves(
        &h.cuts,
        &h.pastes,
        &h.captures,
        &h.states,
        &h.full_edits,
        &h.files,
        &h.post_parsed,
        &mut h.rules,
    )
    .expect("verifies");
    assert_eq!(h.rules[0].semantics, RuleSemantics::Postcondition);
    assert_eq!(h.rules[1].semantics, RuleSemantics::Postcondition);
}

/// WI-7a follow-up: paste exactly at `cut_start` (boundary, not
/// interior — `TextEdit::overlaps` uses strict `<`, so interior
/// anchors are rejected pre-probe in `check_overlaps` and never
/// reach here). The move is a no-op (`post == original`); presence
/// holds at the byte-exact offset and absence holds via the
/// same-file `paste_block` exclusion over the translated cut span.
#[test]
fn paste_at_cut_start_boundary_verifies() {
    let original = SRC.to_string();
    let index = LineIndex::new(&original);
    let cut_start = index.line_start(2).unwrap();
    let cut_end = index.line_start(3).unwrap();
    let captured = original[cut_start..cut_end].to_string();
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![
            TextEdit::new(cut_start, cut_start, captured.clone()),
            TextEdit::new(cut_start, cut_end, String::new()),
        ],
    )]);
    // No-op post: insertion at the cut start + deletion of the cut
    // range restores the original text (`apply_edits` applies the
    // wider deletion first at tied starts).
    let post_text = original.clone();
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(&post_text).expect("post parses"),
    )]);
    let mut captures = CaptureStore::default();
    captures
        .define("H", captured, "cut_h", "f.rs")
        .expect("bind");
    let cuts = vec![MoveCut {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        spans: vec![(2, 2)],
        name: "H".to_string(),
    }];
    let pastes = vec![MovePaste {
        rule: "paste_h".to_string(),
        file: "f.rs".to_string(),
        name: "H".to_string(),
        pos: cut_start,
        bare: true,
        template: None,
    }];
    let mut rules = vec![
        RuleReport {
            rule: "cut_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
        RuleReport {
            rule: "paste_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
    ];
    let states = HashMap::from([(
        "f.rs".to_string(),
        FileState {
            original: original.clone(),
            index: LineIndex::new(&original),
            parsed: Some(syn::parse_str::<syn::File>(SRC).expect("pre parses")),
        },
    )]);
    let files = HashMap::from([("f.rs".to_string(), post_text)]);
    upgrade_verified_moves(
        &cuts,
        &pastes,
        &captures,
        &states,
        &full_edits,
        &files,
        &post_parsed,
        &mut rules,
    )
    .expect("boundary paste verifies");
    assert_eq!(rules[0].semantics, RuleSemantics::Postcondition);
    assert_eq!(rules[1].semantics, RuleSemantics::Postcondition);
}

/// WI-7a follow-up: paste exactly at `cut_end` (bottom boundary).
/// Complementary translation arm to the `cut_start` pin above
/// (neither edit's `end <= boundary`, so the cut span does not
/// shift); the `paste_block` exclusion still carries the absence
/// half. Interior anchors stay rejected pre-probe.
#[test]
fn paste_at_cut_end_boundary_verifies() {
    let original = SRC.to_string();
    let index = LineIndex::new(&original);
    let cut_start = index.line_start(2).unwrap();
    let cut_end = index.line_start(3).unwrap();
    let captured = original[cut_start..cut_end].to_string();
    let full_edits = HashMap::from([(
        "f.rs".to_string(),
        vec![
            TextEdit::new(cut_end, cut_end, captured.clone()),
            TextEdit::new(cut_start, cut_end, String::new()),
        ],
    )]);
    // No-op post: insertion at the cut end + deletion of the cut
    // range restores the original text (insertion at the higher
    // offset applies first).
    let post_text = original.clone();
    let post_parsed = HashMap::from([(
        "f.rs".to_string(),
        syn::parse_str::<syn::File>(&post_text).expect("post parses"),
    )]);
    let mut captures = CaptureStore::default();
    captures
        .define("H", captured, "cut_h", "f.rs")
        .expect("bind");
    let cuts = vec![MoveCut {
        rule: "cut_h".to_string(),
        file: "f.rs".to_string(),
        spans: vec![(2, 2)],
        name: "H".to_string(),
    }];
    let pastes = vec![MovePaste {
        rule: "paste_h".to_string(),
        file: "f.rs".to_string(),
        name: "H".to_string(),
        pos: cut_end,
        bare: true,
        template: None,
    }];
    let mut rules = vec![
        RuleReport {
            rule: "cut_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
        RuleReport {
            rule: "paste_h".to_string(),
            file: "f.rs".to_string(),
            semantics: RuleSemantics::ReparseOnly,
            ..Default::default()
        },
    ];
    let states = HashMap::from([(
        "f.rs".to_string(),
        FileState {
            original: original.clone(),
            index: LineIndex::new(&original),
            parsed: Some(syn::parse_str::<syn::File>(SRC).expect("pre parses")),
        },
    )]);
    let files = HashMap::from([("f.rs".to_string(), post_text)]);
    upgrade_verified_moves(
        &cuts,
        &pastes,
        &captures,
        &states,
        &full_edits,
        &files,
        &post_parsed,
        &mut rules,
    )
    .expect("boundary paste verifies");
    assert_eq!(rules[0].semantics, RuleSemantics::Postcondition);
    assert_eq!(rules[1].semantics, RuleSemantics::Postcondition);
}
