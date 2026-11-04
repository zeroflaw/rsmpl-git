//! WI-15 residue: file-module move + `mod` pair under one shared rule.
//!
//! A deleted-file + created-file move (one file pair, N≥1 moved items)
//! paired with a `mod old;` → `mod new;` declaration pair (file stems
//! match the `mod` idents, no use lines, unique definition) raises the
//! `mod` pair under the first cut's rule instead of as a separate item
//! single — contents never restated, file creation/deletion carried by
//! the sections themselves. Every condition that is off declines the
//! join and the caller falls back to today's split emission (move +
//! separate `mod` single), so the join can never regress a firing move.
//!
//! Narrowing: all pairs share one deleted-file source + one created-file
//! target (same dir, stems naming the `mod` idents), exactly one
//! adjacent minus-run + plus-run `mod` pair in a move-untouched section
//! (lone-pair group or one adjacent pair inside a mixed group, whose
//! other runs stay ordinary leftovers), no other native mention of the
//! old ident, unique `mod` declaration workspace-wide, cited site
//! verified. Fan-out across file pairs, collection moves, `mod` + use
//! lines (`mod_rename` territory), stem mismatches, twin declarations,
//! and rename sections stay split by design.

use std::collections::HashMap;

use super::move_parts::{line_text, MovePair, SectionParts};

/// A proved file-module + `mod`-pair join: the `mod` run-group slot
/// and the declaration delta it carries.
#[derive(Clone)]
pub(crate) struct ModJoin {
    pub(crate) sec_idx: usize,
    /// Run-split group id of the native mixed hunk (`-mod old;` /
    /// `+mod new;` arrive inside one adjacent minus-run + plus-run pair
    /// sharing one group — `push_hunk` splits every mixed `-U0` hunk into
    /// maximal pure runs, so real `git diff -U0` shapes are multi-line
    /// runs, e.g. `[-mod old;, -fn k() {}]` then `[+mod new;, +fn k2() {}]`).
    pub(crate) group: usize,
    /// Hunk indices of the two `mod`-carrying runs inside
    /// `parsed[sec_idx]` (adjacent minus-run then plus-run).
    pub(crate) mod_minus_hi: usize,
    pub(crate) mod_plus_hi: usize,
    /// Line indices of the `mod` lines inside their run bodies (other
    /// lines of the runs stay ordinary leftovers).
    pub(crate) mod_minus_idx: usize,
    pub(crate) mod_plus_idx: usize,
    pub(crate) old: String,
    pub(crate) new: String,
    /// Cited site of the `mod` line (minus-run `old_start` +
    /// `mod_minus_idx`, 1-based).
    pub(crate) site: u32,
}

/// One `mod`-delta line pair candidate: its section, run group, the two
/// carrying runs, the line positions inside those runs, the delta, and
/// the cited site.
struct ModCandidate {
    sec_idx: usize,
    group: usize,
    minus_hi: usize,
    plus_hi: usize,
    minus_idx: usize,
    plus_idx: usize,
    old: String,
    new: String,
    site: u32,
}

/// File stem without the `.rs` suffix (`src/old.rs` → `old`), or `None`
/// for non-`.rs` basenames (parsed sections are `.rs`-only, so this is a
/// second gate, never the first).
fn stem(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?;
    name.strip_suffix(".rs").filter(|s| !s.is_empty())
}

/// Parent directory (`src/old.rs` → `src`, `old.rs` → `""`).
fn dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Ident-boundary containment (never substring): mirrors the `mod_rename`
/// census scan.
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                return true;
            }
            token.clear();
        }
    }
    false
}

/// The declaration delta of a 2-line hunk body, or `None`: same prefix
/// and kind, `mod` on both sides, `;` suffix on both sides, idents
/// differ. Mirrors `mod_rename::split_mod_delta` (which owns the
/// pre-pass intent; this owns the move-join claim).
fn mod_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    let (old_prefix, old_kind, old_ident, old_suffix) = super::split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = super::split_item(plus.trim_end())?;
    if old_kind != "mod" || new_kind != "mod" {
        return None;
    }
    if old_prefix != new_prefix {
        return None;
    }
    if old_suffix.trim() != ";" || new_suffix.trim() != ";" {
        return None;
    }
    if old_ident == new_ident || old_ident.is_empty() || new_ident.is_empty() {
        return None;
    }
    Some((old_ident, new_ident))
}

/// Count `mod <ident>;` declarations across every pre-image path.
/// The join claims the lone declaration workspace-wide (its rule names
/// one intent), so a twin anywhere vetoes — unlike the `mod_rename`
/// census, which scopes to claimed pairs because the follow proves the
/// use set.
fn count_mod_defs(pre: &HashMap<String, String>, ident: &str) -> usize {
    let mut count = 0usize;
    for text in pre.values() {
        for line in text.lines() {
            if let Some((_, kind, id, suffix)) = super::split_item(line) {
                if kind == "mod" && id == ident && suffix.trim() == ";" {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Find the file-module join candidate: `None` for every off-narrowing
/// shape (caller keeps today's split emission).
pub(crate) fn find_mod_join(
    parsed: &[SectionParts],
    sections: &[String],
    final_pairs: &[MovePair],
    pre: &HashMap<String, String>,
) -> Option<ModJoin> {
    // Residue narrowing: N≥1 moved items sharing one deleted-file
    // source + one created-file target (one file pair; fan-out across
    // file pairs stays split). Collection moves keep today's names
    // (the rebuild declines the join when cells exist).
    if final_pairs.is_empty() {
        return None;
    }
    // Rename sections own their shape (headers-only passthrough,
    // content-carrying per-section singles): never join beside one.
    for sec in sections {
        let header = sec.lines().next().unwrap_or("");
        if super::is_rename_header(header) {
            return None;
        }
    }
    let mut minus_secs: Vec<usize> = Vec::new();
    let mut plus_secs: Vec<usize> = Vec::new();
    for p in final_pairs {
        if !minus_secs.contains(&p.minus_hunk_key.0) {
            minus_secs.push(p.minus_hunk_key.0);
        }
        if !plus_secs.contains(&p.plus_hunk_key.0) {
            plus_secs.push(p.plus_hunk_key.0);
        }
    }
    // One file pair only: every minus run lives in one deleted-file
    // section, every plus run in one created-file section.
    if minus_secs.len() != 1 || plus_secs.len() != 1 {
        return None;
    }
    if minus_secs[0] == plus_secs[0] {
        return None;
    }
    let minus_sec = parsed.get(minus_secs[0])?;
    let plus_sec = parsed.get(plus_secs[0])?;
    // File-module shape only: deleted-file source, created-file target.
    if !minus_sec.is_deleted {
        return None;
    }
    if plus_sec.is_deleted || !plus_sec.prelude.contains("new file mode") {
        return None;
    }
    let minus_stem = stem(&minus_sec.path)?;
    let plus_stem = stem(&plus_sec.path)?;
    if dir(&minus_sec.path) != dir(&plus_sec.path) {
        return None;
    }
    // The `mod` pair: exactly one `mod old;` → `mod new;` line pair
    // inside one adjacent minus-run + plus-run pair of a move-untouched
    // section. Mixed `-U0` hunks always arrive run-split (one group per
    // native hunk, maximal pure runs), so the pair is one line pair at
    // run-relative positions `(i, j)` — either the whole group (two
    // 1-line runs) or one line pair inside larger multi-line runs
    // (whose other lines stay ordinary leftovers).
    let mut found: Option<ModCandidate> = None;
    for (si, sec) in parsed.iter().enumerate() {
        if minus_secs.contains(&si) || plus_secs.contains(&si) {
            continue;
        }
        let mut by_group: HashMap<usize, Vec<usize>> = HashMap::new();
        for (hi, hunk) in sec.hunks.iter().enumerate() {
            if let Some(g) = hunk.group {
                by_group.entry(g).or_default().push(hi);
            }
        }
        for (g, his) in &by_group {
            let mut ordered = his.clone();
            ordered.sort_unstable();
            for w in ordered.windows(2) {
                let (a, b) = (w[0], w[1]);
                let (ha, hb) = (&sec.hunks[a], &sec.hunks[b]);
                if ha.body.is_empty() || hb.body.is_empty() {
                    continue;
                }
                if !ha.body[0].starts_with('-') || ha.body[0].starts_with("---") {
                    continue;
                }
                if !hb.body[0].starts_with('+') || hb.body[0].starts_with("+++") {
                    continue;
                }
                for (i, mline) in ha.body.iter().enumerate() {
                    for (j, pline) in hb.body.iter().enumerate() {
                        let Some((old, new)) = mod_delta(line_text(mline), line_text(pline)) else {
                            continue;
                        };
                        if found.is_some() {
                            return None;
                        }
                        found = Some(ModCandidate {
                            sec_idx: si,
                            group: *g,
                            minus_hi: a,
                            plus_hi: b,
                            minus_idx: i,
                            plus_idx: j,
                            old,
                            new,
                            site: ha.old_start + i as u32,
                        });
                    }
                }
            }
        }
    }
    let cand = found?;
    let ModCandidate {
        sec_idx: si,
        group,
        minus_hi: mod_minus_hi,
        plus_hi: mod_plus_hi,
        minus_idx: mod_minus_idx,
        plus_idx: mod_plus_idx,
        old,
        new,
        site,
    } = cand;
    // File stems must name the `mod` delta (never filename similarity
    // alone: the stems must equal the declaration idents exactly).
    if minus_stem != old || plus_stem != new {
        return None;
    }
    // No other native mention of the old ident: use lines belong to the
    // `mod_rename` intent (def + uses), twin `mod` lines to the
    // per-section singles — both stay split so no intent loses a pair.
    // Moved runs and the two joined `mod` lines are exempt (cut/paste
    // carry them verbatim); other lines of mixed runs are scanned.
    for (asi, sec) in parsed.iter().enumerate() {
        for (ahi, hunk) in sec.hunks.iter().enumerate() {
            for (idx, line) in hunk.body.iter().enumerate() {
                if asi == si && ahi == mod_minus_hi && idx == mod_minus_idx {
                    continue;
                }
                if asi == si && ahi == mod_plus_hi && idx == mod_plus_idx {
                    continue;
                }
                let mut moved = false;
                for p in final_pairs {
                    if asi == p.minus_hunk_key.0
                        && ahi == p.minus_hunk_key.1
                        && (p.item.first..=p.item.last).contains(&idx)
                    {
                        moved = true;
                        break;
                    }
                    if asi == p.plus_hunk_key.0
                        && ahi == p.plus_hunk_key.1
                        && (p.plus_first..=p.plus_last).contains(&idx)
                    {
                        moved = true;
                        break;
                    }
                }
                if moved {
                    continue;
                }
                if contains_ident(line_text(line), &old) {
                    return None;
                }
            }
        }
    }
    // Unique `mod old;` declaration workspace-wide (mirrors the
    // `mod_rename` twin veto, scoped to every pre-image since the join
    // names one intent) + the pair's minus line at the cited site.
    let mod_path = &parsed[si].path;
    if count_mod_defs(pre, &old) != 1 {
        return None;
    }
    if site < 1 {
        return None;
    }
    let pre_text = pre.get(mod_path)?;
    let at_site = pre_text.lines().nth(site as usize - 1)?.trim();
    if at_site != format!("mod {old};") {
        return None;
    }
    Some(ModJoin {
        sec_idx: si,
        group,
        mod_minus_hi,
        mod_plus_hi,
        mod_minus_idx,
        mod_plus_idx,
        old,
        new,
        site,
    })
}

/// The semantic `mod`-pair hunk text under `rule` (today's item-single
/// spelling, engine-proven: bare `- mod old`, no semicolon).
pub(crate) fn mod_hunk_text(join: &ModJoin, rule: &str) -> String {
    format!(
        "@@ -{},1 @@ rule={rule}\n- mod {old}\n+ mod {new}\n",
        join.site,
        old = join.old,
        new = join.new
    )
}

/// The replaced native `mod`-hunk byte count: the run-split group's
/// original header + body (what leaves the section), matching the T375
/// accounting in `move_rebuild`. `None` when the group carries no
/// original (never for run-split groups — defensive).
pub(crate) fn replaced_bytes(parsed: &[SectionParts], join: &ModJoin) -> Option<usize> {
    for hunk in &parsed[join.sec_idx].hunks {
        if hunk.group == Some(join.group) {
            if let Some((header, body)) = &hunk.unsplit {
                return Some(header.len() + 1 + body.iter().map(|l| l.len() + 1).sum::<usize>());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A long-enough fn body to clear the T375 byte-win guard.
    fn big_fn(name: &str) -> String {
        let mut s = format!("fn {name}() {{\n");
        for i in 0..40 {
            s.push_str(&format!("    let v{i} = {i};\n"));
        }
        s.push_str("}\n");
        s
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    /// Sections for a file-module move: lib.rs `mod` pair hunk plus the
    /// deleted old.rs / created new.rs sections carrying `body`.
    /// `old_name`/`new_name` override the file stems (default old/new).
    fn file_module_sections_for(
        lib_body: &str,
        old_name: &str,
        old_minus: &str,
        new_name: &str,
        new_plus: &str,
    ) -> Vec<String> {
        vec![
            format!("diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n{lib_body}"),
            format!(
                "diff --git src/{old_name}.rs\ndeleted file mode 100644\n--- a/src/{old_name}.rs\n+++ /dev/null\n{old_minus}"
            ),
            format!(
                "diff --git src/{new_name}.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/{new_name}.rs\n{new_plus}"
            ),
        ]
    }

    fn file_module_sections(lib_body: &str, old_minus: &str, new_plus: &str) -> Vec<String> {
        file_module_sections_for(lib_body, "old", old_minus, "new", new_plus)
    }

    fn move_sections(big: &str) -> (String, String) {
        let minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        (
            format!("@@ -2,42 +2,0 @@\n{minus}"),
            format!("@@ -0,0 +1,42 @@\n{plus}"),
        )
    }

    #[test]
    fn file_module_join_fires_byte_identical() {
        // WI-15 slice 1: identical 40-line move + lone `mod` pair share
        // the cut's rule; real-apply carries file deletion/creation.
        let big = big_fn("foo");
        let minus: String = big.lines().map(|l| format!("-{l}\n")).collect();
        let plus: String = big.lines().map(|l| format!("+{l}\n")).collect();
        let n = big.lines().count();
        let sections = file_module_sections(
            "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n",
            &format!("@@ -1,{n} +1,0 @@\n{minus}"),
            &format!("@@ -0,0 +1,{n} @@\n{plus}"),
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", &big),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("join must fire");
        let text = out.join("");
        // One shared rule over cut + paste + mod pair (no separate mod rule).
        assert_eq!(text.matches("rule=").count(), 3, "got:\n{text}");
        assert!(text.contains("- mod old\n+ mod new\n"), "got:\n{text}");
        // The mod hunk carries the same rule as the cut…
        let lines: Vec<&str> = text.lines().collect();
        let rule_of_header = |h: &str| {
            h.split_once("rule=")
                .map(|(_, r)| r.to_string())
                .unwrap_or_default()
        };
        let mod_header = lines
            .windows(3)
            .find(|w| w[1] == "- mod old" && w[2] == "+ mod new")
            .map(|w| rule_of_header(w[0]));
        let cut_header = lines
            .windows(2)
            .find(|w| w[0].starts_with("@@") && w[1] == "# bind=M1")
            .map(|w| rule_of_header(w[0]));
        assert!(mod_header.is_some() && cut_header.is_some(), "got:\n{text}");
        assert_eq!(mod_header.unwrap(), cut_header.unwrap());
        // …and no separate mod single survives.
        assert!(
            !text.contains("rule=r3"),
            "mod must not ride r3, got:\n{text}"
        );
        validated_live(&text, &pre);
        // Real-apply carries the file deletion + creation.
        let parsed = rsmpl_patch::parse_patch(&text).unwrap();
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        // Real-apply carries the file deletion + creation (the engine
        // leaves the emptied source as `""`; `git apply` drops it via
        // the `deleted file mode` prelude).
        assert!(
            files.get("src/old.rs").is_none_or(|s| s.is_empty()),
            "old file must empty: {files:?}"
        );
        assert!(
            files["src/new.rs"].contains("fn foo()"),
            "got: {}",
            files["src/new.rs"]
        );
        assert!(
            files["src/lib.rs"].contains("mod new;"),
            "got: {}",
            files["src/lib.rs"]
        );
    }

    #[test]
    fn file_module_join_fires_sig_evolved() {
        // Move+sig (WI-8 slice 9) joins the same way: concrete template
        // paste + mod pair under the cut's rule.
        let minus_big = big_fn("foo").replace("fn foo()", "fn foo(x: i32) -> i32");
        let plus_big = big_fn("bar").replace("fn bar()", "fn bar(x: i64) -> i64");
        let m: String = minus_big.lines().map(|l| format!("-{l}\n")).collect();
        let pl: String = plus_big.lines().map(|l| format!("+{l}\n")).collect();
        let sections = file_module_sections(
            "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n",
            &format!("@@ -2,42 +2,0 @@\n{m}"),
            &format!("@@ -0,0 +1,42 @@\n{pl}"),
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{minus_big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("sig-evolved join must fire");
        let text = out.join("");
        assert!(
            text.contains("+ fn bar(x: i64) -> i64 { ... }"),
            "got:\n{text}"
        );
        assert!(text.contains("- mod old\n+ mod new\n"), "got:\n{text}");
        assert_eq!(text.matches("rule=").count(), 3, "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn stem_mismatch_stays_split() {
        // `mod old;`→`mod new;` beside an unrelated a.rs→b.rs move:
        // the move still fires, the mod pair stays native (split).
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let sections =
            file_module_sections_for("@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n", "a", &m, "b", &pl);
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/a.rs", &format!("fn keep() {{}}\n{big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("move must still fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut missing, got:\n{text}");
        assert!(
            text.contains("-mod old;"),
            "mod minus must stay native, got:\n{text}"
        );
        assert!(
            text.contains("+mod new;"),
            "mod plus must stay native, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn use_lines_present_stay_split() {
        // A `crate::old` use line beside the pair is `mod_rename`
        // territory: the join declines, the mod hunk stays native.
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let sections = file_module_sections(
            "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n@@ -2,1 +2,1 @@\n-use crate::old::f;\n+use crate::new::f;\n",
            &m,
            &pl,
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nuse crate::old::f;\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{big}")),
            ("src/other.rs", "use crate::old::f;\n"),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("move must still fire");
        let text = out.join("");
        eprintln!("STEM_MISMATCH_OUT:\n{text}\nEND");
        assert!(text.contains("# bind="), "cut missing, got:\n{text}");
        assert!(
            text.contains("-mod old;"),
            "mod pair must stay native for mod_rename, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn twin_mod_definition_stays_split() {
        // Two `mod old;` declarations: the census vetoes the join.
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let sections = file_module_sections("@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n", &m, &pl);
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/twin.rs", "mod old;\nfn other() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("move must still fire");
        let text = out.join("");
        assert!(
            text.contains("-mod old;"),
            "twin def must keep the mod pair native, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn mixed_mod_hunk_joins_with_leftover() {
        // WI-15 residue: the mod pair sharing its run-split group with
        // another edit joins (semantic mod hunk under the cut's rule)
        // while the other lines stay ordinary leftovers. Real
        // `git diff -U0` shape: all minus lines first, then plus lines
        // (two 2-line runs, mod at index 0 of each).
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let sections = file_module_sections(
            "@@ -1,2 +1,2 @@\n-mod old;\n-fn keep() {}\n+mod new;\n+fn keep2() {}\n",
            &m,
            &pl,
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn keep() {}\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("mixed-group join must fire");
        let text = out.join("");
        assert!(
            text.contains("- mod old\n+ mod new\n"),
            "mod pair must join, got:\n{text}"
        );
        assert!(
            text.contains("-fn keep() {}") && text.contains("+fn keep2() {}"),
            "mixed-group leftover must stay ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn mixed_mod_hunk_joins_mod_second() {
        // WI-15 residue: same mixed shape with the mod pair second in
        // each run (mod at index 1) — leftover spans fire on both sides
        // of the semantic hunk.
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let sections = file_module_sections(
            "@@ -1,2 +1,2 @@\n-fn keep() {}\n-mod old;\n+fn keep2() {}\n+mod new;\n",
            &m,
            &pl,
        );
        let pre = pre_map(&[
            ("src/lib.rs", "fn keep() {}\nmod old;\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("mod-second mixed-group join must fire");
        let text = out.join("");
        assert!(
            text.contains("- mod old\n+ mod new\n"),
            "mod pair must join, got:\n{text}"
        );
        assert!(
            text.contains("-fn keep() {}") && text.contains("+fn keep2() {}"),
            "mixed-group leftover must stay ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn diet_scale_move_joins() {
        // The join's net T375 cost is ~3 bytes (36 replaced vs 39
        // emitted), covered by any firing move's margin — even a
        // diet-scale 3-line move joins instead of splitting.
        let sections = file_module_sections(
            "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n",
            "@@ -2,3 +2,0 @@\n-fn m() {\n-    1\n-}\n",
            "@@ -1,0 +2,3 @@\n+fn m() {\n+    1\n+}\n",
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", "fn keep() {}\nfn m() {\n    1\n}\n"),
            ("src/new.rs", "use x;\n"),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("diet move must still fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut missing, got:\n{text}");
        assert!(
            text.contains("- mod old\n+ mod new\n"),
            "diet move must join the mod pair, got:\n{text}"
        );
        assert_eq!(text.matches("rule=").count(), 3, "got:\n{text}");
        validated_live(&text, &pre);
    }

    #[test]
    fn rename_section_blocks_the_join() {
        // A headers-only rename section anywhere in the patch keeps the
        // rename path authoritative: no join.
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let mut sections = file_module_sections("@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n", &m, &pl);
        sections.push(
            "diff --git a/src/gone.rs b/src/here.rs\nsimilarity index 100%\nrename from src/gone.rs\nrename to src/here.rs\n"
                .to_string(),
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("move must still fire");
        let text = out.join("");
        assert!(
            text.contains("-mod old;"),
            "rename section must block the join, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn capped_multi_item_joins_with_leftover() {
        // EOF capping narrows two pairs to one cut; the surviving pair
        // still joins the mod pair while the dropped item rides ordinary
        // leftovers on both sides (the join claims nothing about it).
        let one = big_fn("one");
        let two = big_fn("two");
        let mut m = String::new();
        for l in one.lines() {
            m.push_str(&format!("-{l}\n"));
        }
        m.push_str("-\n");
        for l in two.lines() {
            m.push_str(&format!("-{l}\n"));
        }
        let mut pl = String::new();
        for l in one.lines() {
            pl.push_str(&format!("+{l}\n"));
        }
        pl.push_str("+\n");
        for l in two.lines() {
            pl.push_str(&format!("+{l}\n"));
        }
        let n = one.lines().count() + two.lines().count() + 1;
        let sections = file_module_sections(
            "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n",
            &format!("@@ -2,{n} +2,0 @@\n{m}"),
            &format!("@@ -2,0 +3,{n} @@\n{pl}"),
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{one}\n{two}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("capped move must still fire");
        let text = out.join("");
        assert!(
            text.contains("- mod old\n+ mod new\n"),
            "capped single-cut move joins the mod pair, got:\n{text}"
        );
        assert!(
            text.contains("-fn two() {\n"),
            "dropped item stays ordinary, got:\n{text}"
        );
        assert!(
            text.contains("+fn two() {\n"),
            "dropped item stays ordinary, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn multi_item_file_module_joins() {
        // WI-15 residue: a whole-file two-item move (old.rs deleted,
        // new.rs created, WI-18 whole-file soundness keeps both pairs)
        // joins the mod pair under the FIRST cut's rule; the second
        // cut keeps its own rule.
        let one = big_fn("one");
        let two = big_fn("two");
        let old_text = format!("{one}\n{two}");
        let mut m = String::new();
        for l in old_text.lines() {
            m.push_str(&format!("-{l}\n"));
        }
        let mut pl = String::new();
        for l in old_text.lines() {
            pl.push_str(&format!("+{l}\n"));
        }
        let n = old_text.lines().count();
        let sections = file_module_sections(
            "@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n",
            &format!("@@ -1,{n} +1,0 @@\n{m}"),
            &format!("@@ -0,0 +1,{n} @@\n{pl}"),
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", &old_text),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("two-item move must still fire");
        let text = out.join("");
        assert_eq!(
            text.matches("# bind=").count(),
            2,
            "two cuts expected, got:\n{text}"
        );
        assert!(
            text.contains("- mod old\n+ mod new\n"),
            "mod pair must join, got:\n{text}"
        );
        // The mod hunk shares the first cut's rule (document order).
        let lines: Vec<&str> = text.lines().collect();
        let rule_of_header = |h: &str| {
            h.split_once("rule=")
                .map(|(_, r)| r.to_string())
                .unwrap_or_default()
        };
        let mod_rule = lines
            .windows(3)
            .find(|w| w[1] == "- mod old" && w[2] == "+ mod new")
            .map(|w| rule_of_header(w[0]));
        let first_cut_rule = lines
            .windows(2)
            .find(|w| w[0].starts_with("@@") && w[1].starts_with("# bind="))
            .map(|w| rule_of_header(w[0]));
        assert!(mod_rule.is_some() && first_cut_rule.is_some());
        assert_eq!(mod_rule.unwrap(), first_cut_rule.unwrap());
        validated_live(&text, &pre);
    }

    #[test]
    fn fan_out_across_targets_stays_split() {
        // WI-15 residue: one deleted source feeding two created targets
        // is fan-out, not one file pair — the move still fires but the
        // mod pair stays native (split).
        let one = big_fn("one");
        let two = big_fn("two");
        let old_text = format!("{one}\n{two}");
        let mut m = String::new();
        for l in old_text.lines() {
            m.push_str(&format!("-{l}\n"));
        }
        let mut p1 = String::new();
        for l in one.lines() {
            p1.push_str(&format!("+{l}\n"));
        }
        let mut p2 = String::new();
        for l in two.lines() {
            p2.push_str(&format!("+{l}\n"));
        }
        let n = old_text.lines().count();
        let n1 = one.lines().count();
        let n2 = two.lines().count();
        let sections = vec![
            "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-mod old;\n+mod new;\n".to_string(),
            format!(
                "diff --git src/old.rs\ndeleted file mode 100644\n--- a/src/old.rs\n+++ /dev/null\n@@ -1,{n} +1,0 @@\n{m}"
            ),
            format!(
                "diff --git src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,{n1} @@\n{p1}"
            ),
            format!(
                "diff --git src/other.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/other.rs\n@@ -0,0 +1,{n2} @@\n{p2}"
            ),
        ];
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nfn main() {}\n"),
            ("src/old.rs", &old_text),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("fan-out move must still fire");
        let text = out.join("");
        assert!(text.contains("# bind="), "cut missing, got:\n{text}");
        assert!(
            text.contains("-mod old;"),
            "fan-out must keep the mod pair native, got:\n{text}"
        );
        validated_live(&text, &pre);
    }

    #[test]
    fn joined_t375_failure_falls_back_to_split() {
        // WI-15 residue fallback pin: a hand-built move whose split
        // margin is exactly 1 byte joins to -2 (fails T375) and falls
        // back to the split — the caller retries unjoined, so the move
        // still fires. Padding one minus line by one char raises the
        // split margin by exactly one byte (emitted is body-independent),
        // so the first pad that fires has margin exactly 1.
        use super::super::move_parts::{HunkParts, MoveItem, MovePair, SectionParts};
        fn build(pad: usize) -> (Vec<SectionParts>, Vec<String>, MovePair, ModJoin) {
            let pad_line = format!("-{}", "x".repeat(pad));
            let minus_body = vec!["-fn m() {".to_string(), pad_line, "-}".to_string()];
            let plus_body = vec!["+fn m() {".to_string(), "+y".to_string(), "+}".to_string()];
            let minus_hunk = HunkParts {
                header: "@@ -1,3 +1,0 @@".to_string(),
                old_start: 1,
                old_len: 3,
                new_start: 1,
                new_len: 0,
                body: minus_body,
                group: None,
                unsplit: None,
            };
            let plus_hunk = HunkParts {
                header: "@@ -0,0 +1,3 @@".to_string(),
                old_start: 0,
                old_len: 0,
                new_start: 1,
                new_len: 3,
                body: plus_body,
                group: None,
                unsplit: None,
            };
            let orig_header = "@@ -1,1 +1,1 @@".to_string();
            let orig_body = vec!["-mod old;".to_string(), "+mod new;".to_string()];
            let mod_minus = HunkParts {
                header: "@@ -1,1 +1,0 @@".to_string(),
                old_start: 1,
                old_len: 1,
                new_start: 1,
                new_len: 0,
                body: vec!["-mod old;".to_string()],
                group: Some(7),
                unsplit: Some((orig_header.clone(), orig_body.clone())),
            };
            let mod_plus = HunkParts {
                header: "@@ -1,0 +1,1 @@".to_string(),
                old_start: 1,
                old_len: 0,
                new_start: 1,
                new_len: 1,
                body: vec!["+mod new;".to_string()],
                group: Some(7),
                unsplit: Some((orig_header, orig_body)),
            };
            let parsed = vec![
                SectionParts {
                    orig_idx: 0,
                    path: "src/old.rs".to_string(),
                    is_deleted: true,
                    prelude: "diff --git src/old.rs\ndeleted file mode 100644\n--- a/src/old.rs\n+++ /dev/null\n".to_string(),
                    hunks: vec![minus_hunk],
                },
                SectionParts {
                    orig_idx: 1,
                    path: "src/new.rs".to_string(),
                    is_deleted: false,
                    prelude: "diff --git src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n".to_string(),
                    hunks: vec![plus_hunk],
                },
                SectionParts {
                    orig_idx: 2,
                    path: "src/lib.rs".to_string(),
                    is_deleted: false,
                    prelude: "diff --git src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n".to_string(),
                    hunks: vec![mod_minus, mod_plus],
                },
            ];
            let sections = vec![String::new(), String::new(), String::new()];
            let pair = MovePair {
                minus_hunk_key: (0, 0),
                plus_hunk_key: (1, 0),
                item: MoveItem {
                    ident: "m".to_string(),
                    text: "fn m() {}".to_string(),
                    lines: vec!["fn m() {}".to_string()],
                    first: 0,
                    last: 2,
                },
                plus_first: 0,
                plus_last: 2,
                evolved: None,
            };
            let join = ModJoin {
                sec_idx: 2,
                group: 7,
                mod_minus_hi: 0,
                mod_plus_hi: 1,
                mod_minus_idx: 0,
                mod_plus_idx: 0,
                old: "old".to_string(),
                new: "new".to_string(),
                site: 1,
            };
            (parsed, sections, pair, join)
        }
        let pre = pre_map(&[("src/lib.rs", "mod old;\n")]);
        // Find the first pad where the split fires (margin exactly 1).
        let mut fired_pad = None;
        for pad in 0..400 {
            let (parsed, sections, pair, _) = build(pad);
            let mut used = Vec::new();
            let out = super::super::move_rebuild::rebuild_sections(
                vec![pair],
                &parsed,
                &sections,
                &pre,
                &mut used,
                0,
                None,
            );
            if out.is_some() {
                fired_pad = Some(pad);
                break;
            }
        }
        let pad = fired_pad.expect("split must fire within the pad window");
        // At margin exactly 1 the joined (+3 B) emission fails T375.
        let (parsed, sections, pair, join) = build(pad);
        let mut used = Vec::new();
        assert!(
            super::super::move_rebuild::rebuild_sections(
                vec![pair.clone()],
                &parsed,
                &sections,
                &pre,
                &mut used,
                0,
                Some(&join),
            )
            .is_none(),
            "joined T375 must fail at split margin 1 (pad {pad})"
        );
        // …and the caller fallback (unjoined retry) still fires.
        let mut used = Vec::new();
        assert!(
            super::super::move_rebuild::rebuild_sections(
                vec![pair],
                &parsed,
                &sections,
                &pre,
                &mut used,
                0,
                None,
            )
            .is_some(),
            "split retry must fire (pad {pad})"
        );
    }

    #[test]
    fn mod_delta_gate() {
        assert_eq!(
            mod_delta("mod old;", "mod new;"),
            Some(("old".to_string(), "new".to_string()))
        );
        assert_eq!(mod_delta("mod old;", "mod old;"), None);
        assert_eq!(mod_delta("mod old {", "mod new {"), None);
        assert_eq!(
            mod_delta("pub mod old;", "pub mod new;"),
            Some(("old".to_string(), "new".to_string()))
        );
        assert_eq!(mod_delta("mod old;", "struct New;"), None);
        assert_eq!(mod_delta("use crate::old;", "use crate::new;"), None);
    }

    #[test]
    fn stem_dir_gates() {
        assert_eq!(stem("src/old.rs"), Some("old"));
        assert_eq!(stem("old.rs"), Some("old"));
        assert_eq!(stem("src/old.txt"), None);
        assert_eq!(dir("src/old.rs"), "src");
        assert_eq!(dir("old.rs"), "");
    }
}
