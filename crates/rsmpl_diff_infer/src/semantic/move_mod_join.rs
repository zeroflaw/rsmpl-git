//! WI-15 slice 1: file-module move + lone `mod` pair under one shared rule.
//!
//! A deleted-file + created-file single-item move paired with a lone
//! `mod old;` → `mod new;` declaration pair (file stems match the `mod`
//! idents, no use lines, unique definition) raises the `mod` pair under
//! the move cut's rule instead of as a separate item single — contents
//! never restated, file creation/deletion carried by the sections
//! themselves. Every condition that is off declines the join and the
//! caller falls back to today's split emission (move + separate `mod`
//! single), so the join can never regress a firing move.
//!
//! Narrowing (slice 1): exactly one `MovePair`, deleted-file source +
//! created-file target, exactly one 2-line `mod` hunk in a move-untouched
//! section, no other native mention of the old ident, unique `mod`
//! declaration across involved pre-images, cited site verified.
//! Fan-out, multi-item moves, mixed-hunk `mod` claims, and `mod` + use
//! lines (`mod_rename` territory) stay split by design.

use std::collections::HashMap;

use super::move_parts::{line_text, MovePair, SectionParts};

/// A proved file-module + `mod`-pair join: the `mod` run-group slot
/// and the declaration delta it carries.
#[derive(Clone)]
pub(crate) struct ModJoin {
    pub(crate) sec_idx: usize,
    /// Run-split group id of the native mixed hunk (`-mod old;` /
    /// `+mod new;` arrive as an adjacent minus-run + plus-run pair
    /// sharing one group — `push_hunk` splits every mixed `-U0` hunk).
    pub(crate) group: usize,
    pub(crate) old: String,
    pub(crate) new: String,
    /// Cited site of the `mod` line (minus-run `old_start`, 1-based).
    pub(crate) site: u32,
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
    // Slice-1 narrowing: exactly one moved item (one cut rule to share;
    // fan-out and multi-item moves stay split).
    if final_pairs.len() != 1 {
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
    let p = &final_pairs[0];
    let minus_sec = parsed.get(p.minus_hunk_key.0)?;
    let plus_sec = parsed.get(p.plus_hunk_key.0)?;
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
    // The lone `mod` pair: exactly one adjacent minus-run + plus-run
    // pair carrying a `mod` delta in a move-untouched section. Mixed
    // `-U0` hunks always arrive run-split (one group per native hunk),
    // so the pair is two 1-line pure runs, never one 2-line hunk.
    // Mixed groups (pair + other lines) and multi-hunk sections with
    // two `mod`-shaped groups decline — residues.
    let mut found: Option<(usize, usize, String, String, u32)> = None;
    for (si, sec) in parsed.iter().enumerate() {
        if si == p.minus_hunk_key.0 || si == p.plus_hunk_key.0 {
            continue;
        }
        let mut by_group: HashMap<usize, Vec<usize>> = HashMap::new();
        for (hi, hunk) in sec.hunks.iter().enumerate() {
            if let Some(g) = hunk.group {
                by_group.entry(g).or_default().push(hi);
            }
        }
        for (g, his) in &by_group {
            if his.len() != 2 {
                continue;
            }
            let (a, b) = (his[0], his[1]);
            let (ha, hb) = (&sec.hunks[a], &sec.hunks[b]);
            if ha.body.len() != 1 || hb.body.len() != 1 {
                continue;
            }
            if !ha.body[0].starts_with('-') || ha.body[0].starts_with("---") {
                continue;
            }
            if !hb.body[0].starts_with('+') || hb.body[0].starts_with("+++") {
                continue;
            }
            let Some((old, new)) = mod_delta(line_text(&ha.body[0]), line_text(&hb.body[0])) else {
                continue;
            };
            if found.is_some() {
                return None;
            }
            found = Some((si, *g, old, new, ha.old_start));
        }
    }
    let (si, group, old, new, site) = found?;
    // File stems must name the `mod` delta (never filename similarity
    // alone: the stems must equal the declaration idents exactly).
    if minus_stem != old || plus_stem != new {
        return None;
    }
    // No other native mention of the old ident: use lines belong to the
    // `mod_rename` intent (def + uses), twin `mod` lines to the
    // per-section singles — both stay split so no intent loses a pair.
    // Moved runs and the joined `mod` runs are exempt (cut/paste carry
    // them verbatim).
    for (asi, sec) in parsed.iter().enumerate() {
        for (ahi, hunk) in sec.hunks.iter().enumerate() {
            for (idx, line) in hunk.body.iter().enumerate() {
                if asi == si && hunk.group == Some(group) {
                    continue;
                }
                if asi == p.minus_hunk_key.0
                    && ahi == p.minus_hunk_key.1
                    && (p.item.first..=p.item.last).contains(&idx)
                {
                    continue;
                }
                if asi == p.plus_hunk_key.0
                    && ahi == p.plus_hunk_key.1
                    && (p.plus_first..=p.plus_last).contains(&idx)
                {
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
    fn mixed_mod_hunk_stays_split() {
        // The mod pair sharing its hunk with another line is not the
        // lone-pair shape: no join, move still fires.
        let big = big_fn("foo");
        let (m, pl) = move_sections(&big);
        let sections = file_module_sections(
            "@@ -1,2 +1,2 @@\n-mod old;\n+mod new;\n-let x = 1;\n+let x = 2;\n",
            &m,
            &pl,
        );
        let pre = pre_map(&[
            ("src/lib.rs", "mod old;\nlet x = 1;\nfn main() {}\n"),
            ("src/old.rs", &format!("fn keep() {{}}\n{big}")),
        ]);
        let mut used = Vec::new();
        let out = super::super::moves::try_move_sweep(&sections, &pre, &mut used)
            .expect("move must still fire");
        let text = out.join("");
        assert!(
            text.contains("-mod old;"),
            "mixed hunk must stay native, got:\n{text}"
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
