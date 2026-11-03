//! WI-1b reprobe floor (IMPROVE D18): never emit a semantic hunk that
//! cannot re-apply to its own pre-image.
//!
//! After the detector chain (whole-patch pre-passes + per-section chain)
//! and the shared-rule grouping assemble the final emission, this module
//! scratch-applies each semantic file (all same-path sections joined —
//! per-hunk emissions repeat one header per hunk) against the pre-image
//! map the library already holds and degrades failures to the true
//! native ordinary section with `veto:reprobe-failed` provenance.
//! Totality counting cannot see citation bugs (the count is right, the
//! cited range/anchor is wrong — the WI-1a F1 shape); the reprobe is
//! the per-hunk soundness floor beneath `diffrs --check`'s end-to-end
//! verdict.
//!
//! Three ordered checks per semantic file (first failure degrades):
//! (a) section-visible rule — no pre-image entry + a move cut/paste
//! that cites minus-side content never emits (pure additions and the
//! bare-`- I` delete spelling keep today's behavior); (b) engine-apply
//! — the file text parses and applies to its pre-image file (catches
//! `RSMPL_MATCH_CONFLICT` citations like F1); (c) T361 post-compare —
//! where the native section parses under the minimal U0 applier below,
//! engine-post must equal native-post (catches over-match sweeps that
//! apply cleanly while rewriting untouched lines). Files whose rules
//! span files (shared-rule joint intents: move cut/paste, follow
//! follows) and empty-base creates skip the local legs and prove only
//! jointly: a whole-patch T361 gate with shared-rule-atomic bisection
//! backs the local checks so multi-file intents cannot slip through
//! file-local leniency.
//!
//! Files the floor cannot prove anything about (no pre-image entry
//! beyond rule (a), empty base, or a native section that does not apply
//! to the pre-image — coordinate-unfaithful caller input, rename
//! preludes) keep today's behavior: the floor never invents a failure
//! it cannot derive from the pre-image.

use std::collections::{HashMap, HashSet};

use crate::semantic::prov::{
    emitted_hunk_ranges, has_semantic_hunk, push_ordinary_prov, reason_veto, HunkDeclined,
    HunkProvenance,
};
use crate::totality::changed_old_lines;

/// Scratch-apply every semantic file of the assembled emission and
/// degrade failures to ordinary. `native` are the pre-pre-pass input
/// sections (the ordinary fallback text, keyed by b-side path); `out`
/// is the final grouped emission, rewritten in place; `prov` drops the
/// degraded rules' semantic entries and gains one ordinary entry per
/// degraded file. No-op when nothing semantic was emitted.
///
/// File granularity (not section): per-hunk emissions repeat one
/// `diff --git` header per hunk of the same file, so a lone section
/// proves nothing against the file's full native post — the unit is
/// all emitted sections of one path joined.
pub(crate) fn reprobe_sections(
    native: &[String],
    out: &mut String,
    pre: &HashMap<String, String>,
    prov: &mut Vec<HunkProvenance>,
) {
    if !has_semantic_hunk(out) {
        return;
    }
    let mut native_by_path: HashMap<String, &str> = HashMap::new();
    for sec in native {
        if let Some(path) = section_path(sec) {
            native_by_path.insert(path, sec.as_str());
        }
    }
    let mut sections = split_sections(out);
    // Rules claimed by more than one file prove only jointly (shared-rule
    // intents: move cut/paste, joint `impl`, fn-joint, member/mod
    // follows) — the joint gate below owns every file carrying one.
    let mut shared: HashSet<String> = HashSet::new();
    {
        let mut rule_files: HashMap<String, HashSet<String>> = HashMap::new();
        for sec in &sections {
            if sec.is_empty() || !has_semantic_hunk(sec) {
                continue;
            }
            if let Some(path) = section_path(sec) {
                for (rule, _, _) in emitted_hunk_ranges(sec) {
                    rule_files.entry(rule).or_default().insert(path.clone());
                }
            }
        }
        for (rule, files) in &rule_files {
            if files.len() > 1 {
                shared.insert(rule.clone());
            }
        }
    }
    for path in file_order(&sections) {
        if !file_has_semantic(&sections, &path) {
            continue;
        }
        if file_shares_rule(&sections, &path, &shared) {
            continue;
        }
        let fallback = match native_by_path.get(&path) {
            Some(f) => (*f).to_string(),
            None => continue,
        };
        let file_old = pre.get(&path).map(String::as_str);
        if file_needs_degrade(&sections, &path, &fallback, file_old) {
            degrade_file(&mut sections, &path, &fallback, file_old, prov);
        }
    }
    // The joint T361 gate still runs when nothing degraded: it owns
    // multi-file move intents (cut in A, paste in B) the local checks
    // cannot prove file by file.
    if whole_verifies(&sections, &native_by_path, pre) {
        *out = sections.join("");
        return;
    }
    // Joint verification fails: bisect the gateable semantic files in
    // emission order until the patch verifies or nothing semantic is
    // left. Files sharing a rule bisect together (one shared intent —
    // degrading half a joint intent can only trade one failure for a
    // dangling-reference failure). The ordinary passthrough parsed as
    // RSmPL upstream, so the loop always terminates verified or wholly
    // ordinary.
    for path in file_order(&sections) {
        if !file_has_semantic(&sections, &path) {
            continue;
        }
        if !gateable(&path, &native_by_path, pre) {
            continue;
        }
        // Shared-rule partners bisect with the file: half a joint
        // intent left standing dangles (missing bind / half follow).
        for partner in rule_partners(&sections, &path) {
            if !gateable(&partner, &native_by_path, pre) {
                continue;
            }
            let fallback = match native_by_path.get(&partner) {
                Some(f) => (*f).to_string(),
                None => continue,
            };
            let file_old = pre.get(&partner).map(String::as_str);
            degrade_file(&mut sections, &partner, &fallback, file_old, prov);
        }
        if whole_verifies(&sections, &native_by_path, pre) {
            break;
        }
    }
    *out = sections.join("");
}

/// The file plus every other emitted file sharing one of its semantic
/// rules (one shared intent bisects atomically). The file leads.
fn rule_partners(sections: &[String], path: &str) -> Vec<String> {
    let mut rules: HashSet<String> = HashSet::new();
    for sec in sections {
        if sec.is_empty() || section_path(sec).as_deref() != Some(path) {
            continue;
        }
        for (rule, _, _) in emitted_hunk_ranges(sec) {
            rules.insert(rule);
        }
    }
    let mut partners = vec![path.to_string()];
    for sec in sections {
        if sec.is_empty() || !has_semantic_hunk(sec) {
            continue;
        }
        let Some(other) = section_path(sec) else {
            continue;
        };
        if other == path || partners.contains(&other) {
            continue;
        }
        let shares = emitted_hunk_ranges(sec)
            .iter()
            .any(|(r, _, _)| rules.contains(r));
        if shares {
            partners.push(other);
        }
    }
    partners
}

/// True when any semantic rule of `path` is claimed by another file.
fn file_shares_rule(sections: &[String], path: &str, shared: &HashSet<String>) -> bool {
    sections.iter().any(|sec| {
        !sec.is_empty()
            && section_path(sec).as_deref() == Some(path)
            && emitted_hunk_ranges(sec)
                .iter()
                .any(|(r, _, _)| shared.contains(r))
    })
}

/// First-appearance file order over the emitted sections (cleared
/// slots skipped).
fn file_order(sections: &[String]) -> Vec<String> {
    let mut order = Vec::new();
    for sec in sections {
        if sec.is_empty() {
            continue;
        }
        if let Some(path) = section_path(sec) {
            if !order.contains(&path) {
                order.push(path);
            }
        }
    }
    order
}

/// True when any emitted section of `path` carries a semantic hunk.
fn file_has_semantic(sections: &[String], path: &str) -> bool {
    sections.iter().any(|sec| {
        !sec.is_empty() && section_path(sec).as_deref() == Some(path) && has_semantic_hunk(sec)
    })
}

/// All emitted sections of one path joined (the unit the floor
/// proves). Cleared slots contribute nothing.
fn file_joined(sections: &[String], path: &str) -> String {
    sections
        .iter()
        .filter(|sec| !sec.is_empty() && section_path(sec).as_deref() == Some(path))
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("")
}

/// Replace every emitted section of `path` with the single ordinary
/// fallback (first slot keeps it, the rest clear) and repair
/// provenance once for the file.
fn degrade_file(
    sections: &mut [String],
    path: &str,
    fallback: &str,
    file_old: Option<&str>,
    prov: &mut Vec<HunkProvenance>,
) {
    let rules: HashSet<String> = sections
        .iter()
        .filter(|sec| !sec.is_empty() && section_path(sec).as_deref() == Some(path))
        .flat_map(|sec| emitted_hunk_ranges(sec))
        .map(|(r, _, _)| r)
        .collect();
    let mut first = true;
    for sec in sections.iter_mut() {
        if sec.is_empty() || section_path(sec).as_deref() != Some(path) {
            continue;
        }
        if first {
            *sec = fallback.to_string();
            first = false;
        } else {
            sec.clear();
        }
    }
    degrade_rules(&rules, fallback, path, file_old, prov);
}

/// Per-file verdict: true when the emitted file text must degrade to
/// its ordinary fallback.
fn file_needs_degrade(
    sections: &[String],
    path: &str,
    fallback: &str,
    file_old: Option<&str>,
) -> bool {
    let joined = file_joined(sections, path);
    // (a) Section-visible cut rule: no pre-image, so neither the engine
    // check nor the post-compare can run — a move cut/paste that CITES
    // minus-side content never emits from section-visible evidence
    // (IMPROVE WI-1; the F1 shape). Pure additions (new-file pastes,
    // creates) cite nothing and keep today's behavior, as does a bare
    // `- I` without bind/use (the whole-file delete spelling,
    // SPEC 18.2).
    let Some(base) = file_old else {
        return carries_move_cut(&joined) && has_minus_lines(&joined);
    };
    // Empty bases (creates) and joint intents (move bind/use pairs and
    // `# follow=uses` follows whose proof spans files — the joint
    // `impl` header, fn-joint callee uses, member/mod follows) belong
    // to the joint gate below: a lone file cannot prove them without
    // its partner files.
    if base.is_empty() || carries_joint_intent(&joined) {
        return false;
    }
    // Native fidelity first: when the native section itself does not
    // apply to the pre-image (coordinate-unfaithful caller stub, rename
    // prelude), the floor has no `want` to compare against and the
    // engine leg would fail on the same coordinates — skip, keeping
    // today's behavior. In production the diff came from this base, so
    // the leg always runs.
    let want = match native_post(base, fallback) {
        Some(w) => w,
        None => return false,
    };
    // (b)+(c) Engine-apply then T361 post-compare against the file.
    match engine_post(&joined, base) {
        Some(got) => want != got,
        None => true,
    }
}

/// True when the file text belongs to a joint intent whose proof spans
/// files: a move `# bind=` cut / `# use=` paste (binds are
/// patch-scoped), or a `# follow=uses` follow (uses may live in partner
/// files — the joint `impl` header, fn-joint callees, member/mod
/// follows). These files verify jointly (the whole gate), never alone.
/// A bare `- I` without bind/use is the whole-file delete spelling
/// (SPEC 18.2) and proves file-locally like any other hunk.
fn carries_joint_intent(sec: &str) -> bool {
    carries_move_cut(sec)
        || sec
            .lines()
            .any(|l| l.trim_start().starts_with("# follow=uses"))
}

/// True when the file text carries a whole-item move cut or paste.
fn carries_move_cut(sec: &str) -> bool {
    sec.lines()
        .any(|l| l.starts_with("# bind=") || l.starts_with("# use="))
}

/// True when the file text cites minus-side content (any `-` body
/// line; `---` file markers excluded).
fn has_minus_lines(sec: &str) -> bool {
    sec.lines()
        .any(|l| l.starts_with('-') && !l.starts_with("---"))
}

/// A file joins the joint gate only when its pre-image is present,
/// non-empty (creates prove nothing alone), and the native section
/// applies to it (native fidelity — without a `want` there is no
/// verdict to compute).
fn gateable(
    path: &str,
    native_by_path: &HashMap<String, &str>,
    pre: &HashMap<String, String>,
) -> bool {
    match (pre.get(path), native_by_path.get(path)) {
        (Some(base), Some(native_sec)) => {
            !base.is_empty() && native_post(base, native_sec).is_some()
        }
        _ => false,
    }
}

/// Engine post-tree for one emitted file text applied to `base`.
/// `None` on parse or apply failure (the miscitation class). A file
/// the engine deletes reads back as `""`, matching the native deleted
/// arm — removal both sides agree on is not a divergence.
fn engine_post(sec: &str, base: &str) -> Option<String> {
    let patch = rsmpl_patch::parse_patch(sec).ok()?;
    let path = section_path(sec)?;
    let mut tree: HashMap<String, String> = HashMap::new();
    tree.insert(path.clone(), base.to_string());
    // New-file targets have no base entry; seed them like `verify.rs`
    // so creates apply without engine missing-target leniency.
    for file in &patch.files {
        tree.entry(file.target_path.clone()).or_default();
    }
    rsmpl_engine::apply_patch_to_tree(&mut tree, &patch).ok()?;
    Some(tree.get(&path).cloned().unwrap_or_default())
}

/// Joint T361 gate over the gateable files: parse + apply the
/// gateable sections against the pre-image map (seeding missing targets
/// the way `verify.rs` does), then byte-compare each gateable file
/// whose emitted section is still semantic against its native post.
/// Exempt files (no pre-image, empty base, or native-unfaithful) are
/// dropped from the candidate — exempt, never a gate failure — so the
/// gate proves exactly what the pre-image can prove.
fn whole_verifies(
    sections: &[String],
    native_by_path: &HashMap<String, &str>,
    pre: &HashMap<String, String>,
) -> bool {
    let mut candidate = String::new();
    let mut compare: Vec<String> = Vec::new();
    for sec in sections {
        let Some(path) = section_path(sec) else {
            candidate.push_str(sec);
            continue;
        };
        if !gateable(&path, native_by_path, pre) {
            continue;
        }
        candidate.push_str(sec);
        if has_semantic_hunk(sec) && !compare.contains(&path) {
            compare.push(path);
        }
    }
    let patch = match rsmpl_patch::parse_patch(&candidate) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let mut raised = pre.clone();
    for file in &patch.files {
        raised.entry(file.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut raised, &patch).is_err() {
        return false;
    }
    for path in &compare {
        let base = &pre[path];
        let want = match native_post(base, native_by_path[path.as_str()]) {
            Some(w) => w,
            None => return false,
        };
        // Deleted files leave no tree entry; both sides agree on "".
        let got = raised.get(path).map(String::as_str).unwrap_or("");
        if want != got {
            return false;
        }
    }
    true
}

/// Repair provenance for a degraded file: drop its semantic entries
/// for the degraded rules, then record one ordinary entry naming the
/// floor.
fn degrade_rules(
    rules: &HashSet<String>,
    fallback: &str,
    path: &str,
    file_old: Option<&str>,
    prov: &mut Vec<HunkProvenance>,
) {
    prov.retain(|p| !(p.file == path && rules.contains(&p.rule)));
    // A remainder entry for this file (mixed emission) no longer has a
    // meaning once the whole section is ordinary; the fresh entry below
    // covers all changed lines.
    prov.retain(|p| !(p.file == path && p.detector == "ordinary" && p.rule.is_empty()));
    let declined = vec![HunkDeclined {
        detector: "reprobe".to_string(),
        reason: reason_veto("reprobe-failed"),
    }];
    // `push_ordinary_prov` dedupes, completes with `not-tried` parity,
    // and stays silent on hunk-less fallbacks (headers-only sections).
    push_ordinary_prov(
        prov,
        path,
        &declined,
        &changed_old_lines(fallback),
        file_old,
        fallback,
    );
}

/// B-side path of a `diff --git` section, `None` on bad headers.
fn section_path(sec: &str) -> Option<String> {
    let header = sec.lines().next()?;
    let rest = header.strip_prefix("diff --git ")?;
    crate::b_side_path(rest)
}

/// Split patch text on line-start `diff --git ` (byte-level, mirroring
/// `rsmpl_diff::verify::split_sections` without depending on it —
/// `rsmpl_diff` depends on this crate, so the helper is local).
fn split_sections(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut starts = vec![0usize];
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' && text[i + 1..].starts_with("diff --git ") {
            starts.push(i + 1);
        }
        i += 1;
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let e = if k + 1 < starts.len() {
                starts[k + 1]
            } else {
                text.len()
            };
            text[*s..e].to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

/// Minimal native U0 applier for one ordinary section: apply its hunks
/// at original coordinates to `base`. Handles modify/create/delete,
/// pure-insertion anchors (`-N,0` inserts after original line N, `-0,0`
/// at the top), context lines, and the `\ No newline` marker.
/// Returns `None` on any shape it does not understand (rename
/// preludes, unparsable headers, pre-image mismatches) so the caller
/// keeps the engine verdict instead of inventing a failure.
fn native_post(base: &str, section: &str) -> Option<String> {
    let mut lines: Vec<String> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('\n').map(str::to_string).collect()
    };
    // `split('\n')` on `"a\n"` yields `["a", ""]`; drop the phantom.
    let mut trailing_newline = base.is_empty() || base.ends_with('\n');
    if !base.is_empty() && base.ends_with('\n') {
        lines.pop();
    }
    let mut created = false;
    let mut deleted = false;
    let mut delta: i64 = 0;
    let mut hunks: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut active = false;
    for line in section.lines().skip(1) {
        if line.starts_with("@@ ") {
            if active {
                hunks.push(std::mem::take(&mut cur));
            }
            active = true;
            cur.push(line);
        } else if active {
            cur.push(line);
        } else if line.starts_with("new file mode") {
            created = true;
        } else if line.starts_with("deleted file mode") {
            deleted = true;
        } else if line == "--- /dev/null" {
            created = true;
        } else if line == "+++ /dev/null" {
            deleted = true;
        } else if line.starts_with("rename from ") || line.starts_with("rename to ") {
            // Renames carry prelude the minimal applier does not model.
            return None;
        }
    }
    if active {
        hunks.push(cur);
    }
    if deleted {
        return Some(String::new());
    }
    if created {
        lines.clear();
        trailing_newline = true;
    }
    // The `-`/context lines must cite the pre-image; the per-line
    // checks below return `None` (skip the leg) on any mismatch.
    for hunk in &hunks {
        let (old_start, old_len, new_len) = parse_hunk_header(hunk.first()?)?;
        let start_i = if old_len == 0 {
            old_start as i64 + delta
        } else if old_start == 0 {
            return None;
        } else {
            old_start as i64 - 1 + delta
        };
        if start_i < 0 {
            return None;
        }
        let start = start_i as usize;
        let mut idx = start;
        let mut old_consumed = 0usize;
        let mut plus: Vec<String> = Vec::new();
        let mut new_trailing = trailing_newline;
        for (k, line) in hunk.iter().skip(1).enumerate() {
            if let Some(content) = line.strip_prefix('-') {
                if lines.get(idx).map(String::as_str) != Some(content) {
                    return None;
                }
                idx += 1;
                old_consumed += 1;
            } else if let Some(content) = line.strip_prefix('+') {
                plus.push(content.to_string());
            } else if let Some(content) = line.strip_prefix(' ') {
                if lines.get(idx).map(String::as_str) != Some(content) {
                    return None;
                }
                idx += 1;
                old_consumed += 1;
                plus.push(content.to_string());
            } else if *line == "\\ No newline at end of file" {
                // `k` indexes `hunk[1..]`; the annotated line is `hunk[k]`.
                let prev = hunk.get(k).copied().unwrap_or("");
                let prev_is_plus =
                    prev.starts_with('+') && !prev.starts_with("+++") || prev.starts_with(' ');
                if prev_is_plus {
                    new_trailing = false;
                }
            } else {
                return None;
            }
        }
        if old_consumed != old_len || plus.len() != new_len {
            return None;
        }
        if start + old_consumed > lines.len() {
            return None;
        }
        lines.splice(start..start + old_consumed, plus);
        trailing_newline = new_trailing;
        delta += new_len as i64 - old_len as i64;
    }
    if hunks.is_empty() && !created {
        return None;
    }
    let mut post = lines.join("\n");
    if !lines.is_empty() && trailing_newline {
        post.push('\n');
    }
    Some(post)
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize)> {
    let inner = line.strip_prefix("@@ ")?;
    let end = inner.find(" @@")?;
    let mut parts = inner[..end].split(' ');
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let parse_side = |s: &str| -> Option<(usize, usize)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = parse_side(old)?;
    let (_, new_len) = parse_side(new)?;
    Some((old_start, old_len, new_len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn pre(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn native_section() -> String {
        "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-fn foo() {}\n+fn bar() {}\n"
            .to_string()
    }

    fn good_semantic() -> String {
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n- fn foo\n+ fn bar\n".to_string()
    }

    #[test]
    fn clean_rename_passes_through_untouched() {
        // Guard against over-correction: an appliable rename keeps its
        // bytes and its provenance.
        let native = vec![native_section()];
        let mut out = good_semantic();
        let pre = pre(&[("src/lib.rs", "fn foo() {}\nfn other() {}\n")]);
        let mut prov = vec![HunkProvenance {
            rule: "r1".to_string(),
            file: "src/lib.rs".to_string(),
            detector: "test",
            sites: vec![1],
            sites_exact: true,
            totality: "pre-image",
            proven: true,
            saved_bytes: 1,
            group: String::new(),
            declined: Vec::new(),
        }];
        reprobe_sections(&native, &mut out, &pre, &mut prov);
        assert_eq!(out, good_semantic(), "got: {out:?}");
        assert_eq!(prov.len(), 1, "got: {prov:?}");
        assert_eq!(prov[0].rule, "r1");
    }

    #[test]
    fn miscited_cut_degrades_with_named_veto() {
        // Synthetic F1 fault: a bind cut citing range 836 of a 1-line
        // file cannot apply — the floor degrades it to ordinary with
        // `veto:reprobe-failed` (the TASKS.md synthetic-fault test).
        let ordinary = "diff --git src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-mod sweep_tests;\n+mod other;\n"
            .to_string();
        let bad = "diff --git src/a.rs\n@@ -836,1 @@ rule=r9\n# bind=M5\n- I\n".to_string();
        let native = vec![ordinary.clone()];
        let mut out = bad.clone();
        let pre = pre(&[("src/a.rs", "mod sweep_tests;\n")]);
        let mut prov = vec![HunkProvenance {
            rule: "r9".to_string(),
            file: "src/a.rs".to_string(),
            detector: "move",
            sites: vec![836],
            sites_exact: false,
            totality: "pre-image",
            proven: true,
            saved_bytes: 0,
            group: String::new(),
            declined: Vec::new(),
        }];
        reprobe_sections(&native, &mut out, &pre, &mut prov);
        assert_eq!(out, ordinary, "got: {out:?}");
        assert!(
            prov.iter().all(|p| p.rule != "r9"),
            "semantic entry must go, got: {prov:?}"
        );
        let ord = prov
            .iter()
            .find(|p| p.detector == "ordinary" && p.file == "src/a.rs")
            .expect("ordinary entry must exist");
        assert!(
            ord.declined
                .iter()
                .any(|d| d.detector == "reprobe" && d.reason == "veto:reprobe-failed"),
            "got: {:?}",
            ord.declined
        );
        assert!(
            crate::semantic::prov::reason_is_closed("veto:reprobe-failed"),
            "reason must stay in the closed taxonomy"
        );
    }

    #[test]
    fn section_visible_bind_cut_never_emits() {
        // No pre-image entry: a move cut degrades even though the
        // engine check cannot run (cuts need a pre-image to prove).
        let ordinary = "diff --git src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-mod sweep_tests;\n+mod other;\n"
            .to_string();
        let cut = "diff --git src/a.rs\n@@ -1,1 @@ rule=r9\n# bind=M5\n- I\n".to_string();
        let native = vec![ordinary.clone()];
        let mut out = cut.clone();
        let pre = pre(&[]);
        let mut prov = vec![HunkProvenance {
            rule: "r9".to_string(),
            file: "src/a.rs".to_string(),
            detector: "move",
            sites: vec![],
            sites_exact: false,
            totality: "section-visible",
            proven: false,
            saved_bytes: 0,
            group: String::new(),
            declined: Vec::new(),
        }];
        reprobe_sections(&native, &mut out, &pre, &mut prov);
        assert_eq!(out, ordinary, "got: {out:?}");
        assert!(prov.iter().all(|p| p.rule != "r9"));
    }

    #[test]
    fn section_visible_pure_addition_paste_survives() {
        // A new-file paste cites no minus-side content, so the
        // section-visible rule cannot falsify it: it keeps today's
        // behavior (the `32-new-file-move` cycle shape — degrading it
        // would trade a `--check`-clean paste for ordinary content and
        // break the S/S′ canonical fixed point).
        let ordinary = "diff --git src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,1 @@\n+fn parse_pair() {}\n"
            .to_string();
        let paste =
            "diff --git src/new.rs\nnew file mode 100644\n@@ -0,0 @@ rule=r1\n# use=M1\n+ M1\n"
                .to_string();
        let native = vec![ordinary.clone()];
        let mut out = paste.clone();
        let pre = pre(&[]);
        let mut prov = vec![HunkProvenance {
            rule: "r1".to_string(),
            file: "src/new.rs".to_string(),
            detector: "move",
            sites: vec![],
            sites_exact: false,
            totality: "section-visible",
            proven: false,
            saved_bytes: 10,
            group: String::new(),
            declined: Vec::new(),
        }];
        reprobe_sections(&native, &mut out, &pre, &mut prov);
        assert_eq!(out, paste, "pure-addition paste must survive, got: {out:?}");
        assert_eq!(prov.len(), 1, "got: {prov:?}");
        assert_eq!(prov[0].rule, "r1");
    }

    #[test]
    fn over_match_sweep_degrades_by_post_compare() {
        // The `verify.rs` T361 contrast shape: two identical call
        // sites, native changes one; a sweep rewriting both applies
        // cleanly but diverges post-compare, so it degrades.
        let ordinary = "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2 +2 @@\n-foo(1);\n+bar(1);\n"
            .to_string();
        let sweep = "diff --git src/lib.rs\n@@ -2,2 @@ rule=r1\n# allow=many\n- foo(E)\n+ bar(E)\n"
            .to_string();
        let native = vec![ordinary.clone()];
        let mut out = sweep.clone();
        let pre = pre(&[("src/lib.rs", "fn m() {\nfoo(1);\nfoo(1);\n}\n")]);
        let mut prov = vec![HunkProvenance {
            rule: "r1".to_string(),
            file: "src/lib.rs".to_string(),
            detector: "call_sweep",
            sites: vec![2, 3],
            sites_exact: true,
            totality: "pre-image",
            proven: true,
            saved_bytes: 1,
            group: String::new(),
            declined: Vec::new(),
        }];
        reprobe_sections(&native, &mut out, &pre, &mut prov);
        assert_eq!(out, ordinary, "got: {out:?}");
        assert!(prov.iter().all(|p| p.rule != "r1"));
    }

    #[test]
    fn multi_section_file_degrades_to_single_fallback() {
        // Per-hunk emissions repeat one `diff --git` header per hunk of
        // the same file: degrading the file must yield its ordinary
        // section exactly once, never one fallback copy per section
        // (the bench-24 triplication shape).
        let ordinary =
            "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+A\n"
                .to_string();
        let bad_one =
            "diff --git src/lib.rs\n@@ -9,1 @@ rule=r1\n# allow=many\n- a\n+ A\n".to_string();
        let bad_two =
            "diff --git src/lib.rs\n@@ -10,1 @@ rule=r2\n# allow=many\n- a\n+ A\n".to_string();
        let native = vec![ordinary.clone()];
        let mut out = format!("{bad_one}{bad_two}");
        let pre = pre(&[("src/lib.rs", "a\nb\n")]);
        let mut prov = vec![
            HunkProvenance {
                rule: "r1".to_string(),
                file: "src/lib.rs".to_string(),
                detector: "call_sweep",
                sites: vec![9],
                sites_exact: true,
                totality: "pre-image",
                proven: true,
                saved_bytes: 1,
                group: String::new(),
                declined: Vec::new(),
            },
            HunkProvenance {
                rule: "r2".to_string(),
                file: "src/lib.rs".to_string(),
                detector: "call_sweep",
                sites: vec![10],
                sites_exact: true,
                totality: "pre-image",
                proven: true,
                saved_bytes: 1,
                group: String::new(),
                declined: Vec::new(),
            },
        ];
        reprobe_sections(&native, &mut out, &pre, &mut prov);
        assert_eq!(out, ordinary, "one fallback copy, got: {out:?}");
        assert!(prov.iter().all(|p| p.rule != "r1" && p.rule != "r2"));
        assert_eq!(
            prov.iter()
                .filter(|p| p.detector == "ordinary" && p.file == "src/lib.rs")
                .count(),
            1,
            "one ordinary entry, got: {prov:?}"
        );
    }

    #[test]
    fn native_u0_applier_derives_post_text() {
        // Modify + pure-insertion + new-file arms of the local applier.
        let base = "a\nb\nc\n";
        let sec = "diff --git f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+A\n@@ -3,0 +4 @@ c\n+X\n";
        assert_eq!(native_post(base, sec), Some("A\nb\nc\nX\n".to_string()));
        let create = "diff --git f.txt\nnew file mode 100644\n--- /dev/null\n+++ b/f.txt\n@@ -0,0 +1,1 @@\n+n\n";
        assert_eq!(native_post("", create), Some("n\n".to_string()));
        // Rename preludes are out of scope: skip the leg, keep the
        // engine verdict.
        let ren =
            "diff --git a/o.rs b/n.rs\nrename from o.rs\nrename to n.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(native_post("a\n", ren), None);
    }
}
