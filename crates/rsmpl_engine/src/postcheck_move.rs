//! WI-7a cut+paste move verification (SPEC 24.6).
//!
//! A scalar verbatim move pair — one item cut (`- I` + `# bind=N`) and one
//! bare paste (`+ N` + `# use=N`, template trims to the name) — earns
//! `postcondition` on **both** entries iff two span-local checks hold in
//! the post-image:
//!
//! * presence: the pasted bytes equal the captured item text
//!   (`captured.trim_end() + "\n"`) at the paste edit's byte-exact
//!   post-image offset (WI-6b `post_starts_for` machinery — occurrence
//!   elsewhere never counts);
//! * absence: no same-kind same-ident item starts inside the cut span
//!   translated to post-image coordinates (structural, never a substring
//!   search).
//!
//! Everything else stays `reparse-only`: evolving pastes, collections,
//! pure insertions, unpaired cuts, multi-pastes sharing one anchor
//! (T249 blank-join), unparseable post-images, and unnameable items
//! (`impl` blocks and friends). Out-of-range probe math abstains (skip,
//! never fail); in-range mismatch fails loud with
//! `RSMPL_MATCH_CONFLICT` (a planner bug, never a user error — WI-6
//! tamper doctrine).

use std::collections::HashMap;

use rsmpl_patch::StructuredError;
use syn::spanned::Spanned;

use crate::capture::CaptureStore;
use crate::edits::{translate_spans_to_post, TextEdit};
use crate::lines::FileState;
use crate::{RuleReport, RuleSemantics};

/// WI-7a pending scalar cut: rule/file/spans plus the capture name that
/// links it to its paste. Collection cuts never enter (WI-7b owns them).
pub(crate) struct MoveCut {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) spans: Vec<(u32, u32)>,
    pub(crate) name: String,
}

/// WI-7a pending bare paste: rule/file plus the capture name and the
/// original-coordinate insertion byte offset. `bare` is false for
/// evolving templates (WI-7c owns those).
pub(crate) struct MovePaste {
    pub(crate) rule: String,
    pub(crate) file: String,
    pub(crate) name: String,
    pub(crate) pos: usize,
    pub(crate) bare: bool,
}

/// WI-7a: upgrade verified cut+paste pairs to `postcondition`.
///
/// Pairing is by capture name. Each pair verifies presence (paste bytes
/// equal the captured item at the byte-exact post offset) and absence
/// (no same-kind same-ident item starts in the translated cut span).
/// Skips stay `reparse-only`; in-range mismatches fail loud.
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
        if !p.bare {
            continue;
        }
        let cut = match cuts.iter().find(|c| c.name == p.name) {
            Some(c) => c,
            None => continue,
        };
        let (texts, collection) = match captures.get_all(&p.name, &p.rule, &p.file) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if collection || texts.len() != 1 {
            continue;
        }
        let captured = texts[0].trim_end().to_string();
        if captured.is_empty() {
            continue;
        }
        let expected = format!("{captured}\n");
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
        // Cut absence: the translated cut span must not start a same-kind
        // same-ident item in the post-image parse.
        let (kind, ident) = match named_item(&captured) {
            Some(v) => v,
            None => continue,
        };
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
        for item in &post_file.items {
            let (k2, id2) = match named_item_of(item) {
                Some(v) => v,
                None => continue,
            };
            if k2 != kind || id2 != ident {
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

/// WI-7a loud failure: the pair claimed a verbatim move but the
/// post-image disagrees at the recorded span (planner bug, WI-6
/// doctrine — `RSMPL_MATCH_CONFLICT` so agents reuse the match-conflict
/// repair path).
fn move_error(rule: &str, file: &str, detail: &str) -> StructuredError {
    StructuredError::new(
        rsmpl_patch::ErrorCode::MatchConflict,
        format!("postcondition: verbatim move did not land as authored ({detail})"),
    )
    .with_rule(rule)
    .with_file(file)
    .with_repair(StructuredError::fix_pattern_repair(
        "move",
        "read the reported spans in the post-image file; fix the cut range or paste anchor so the pair names one verbatim item move",
    ))
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
mod tests {
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
    fn evolving_paste_stays_reparse_only() {
        // WI-7c owns evolving pastes: template != name abstains.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_new\n# lang=rust\n# use=Helper\n+ pub fn renamed() { ... }\n\n@@ -2,3 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
        let src = "fn a() {}\nfn helper() {\n    1\n}\nfn z() {}\n";
        let (_, report) = apply_report(patch, vec![("src/lib.rs", src)]);
        assert_eq!(semantics_of(&report, "cut_helper"), "reparse-only");
        assert_eq!(semantics_of(&report, "paste_new"), "reparse-only");
    }

    #[test]
    fn pure_insertion_and_unpaired_cut_stay_reparse_only() {
        // WI-7d owns these: no pair, no structural verdict.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=ins\n+ fn inserted() {}\n\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
        let (_, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
        assert_eq!(semantics_of(&report, "ins"), "reparse-only");
        assert_eq!(semantics_of(&report, "cut_helper"), "reparse-only");
    }

    #[test]
    fn collection_paste_stays_reparse_only() {
        // WI-7b owns collections: multi-item capture abstains.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,0 @@ rule=paste_all\n# lang=rust\n# use=Helpers\n+ Helpers\n\n@@ -1,3 @@ rule=cut_all\n# lang=rust\n# allow=many\n# item I\n# bind=Helpers\n- I\n";
        let (files, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
        assert!(
            files["src/lib.rs"].matches("fn helper").count() >= 1,
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
}
