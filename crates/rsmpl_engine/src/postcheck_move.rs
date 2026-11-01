//! WI-7a + WI-7b + WI-7c cut+paste move verification (SPEC 24.6).
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
//! Everything else stays `reparse-only`: pure insertions, unpaired
//! cuts, empty collections, multi-pastes sharing one anchor (T249
//! blank-join), unparseable post-images, unnameable items (`impl`
//! blocks and friends), hole-less collection templates, and kind
//! mismatches. Out-of-range probe math abstains (skip, never fail);
//! in-range mismatch fails loud with `RSMPL_MATCH_CONFLICT` (a planner
//! bug, never a user error — WI-6 tamper doctrine).

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
    fn pure_insertion_and_unpaired_cut_stay_reparse_only() {
        // WI-7d owns these: no pair, no structural verdict.
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=ins\n+ fn inserted() {}\n\n@@ -2,1 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n";
        let (_, report) = apply_report(patch, vec![("src/lib.rs", SRC)]);
        assert_eq!(semantics_of(&report, "ins"), "reparse-only");
        assert_eq!(semantics_of(&report, "cut_helper"), "reparse-only");
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
}
