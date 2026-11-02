//! WI-19 partial partition (SPEC D.4): hunk-atomic section split.
//!
//! Split from `semantic/chain.rs` under the 2000-line gate: pure move
//! of the WI-19 partial emitter plus its matrix, no behavior change.
//! `chain.rs` keeps the whole-section path and the per-hunk fallback;
//! this module owns the mixed-section middle.

/// WI-19 partial partition: like [`try_partition_section`] but a
/// single unclaimable pair no longer vetoes the section. Native-hunk
/// atomicity is kept: a hunk is claimed only when *every* pair in it
/// is a single-pair item rename (same per-pair discipline as above —
/// `try_item_rename`, duplicate vetoes, per-pair `template_pair_holds`
/// n=1 probe); claimed hunks emit the same per-pair semantic hunks,
/// unclaimed hunks stay ordinary verbatim in the same file section
/// (SPEC 10.6 disjointness: hunk ranges never overlap, so mixing is
/// sound). A single mixed hunk therefore stays ordinary whole — the
/// per-hunk fallback below owns it.
///
/// Gates carried over from the all-or-nothing path: identical minus
/// lines anywhere in the section decline the whole section (`None` —
/// identical pairs belong to the verbatim sweep), and an emitted
/// minus pattern claimed twice demotes the repeat hunk to ordinary
/// (terse shapes would overlap-match). Zero claimed hunks returns
/// `None` with `used_rules` untouched.
///
/// Returns the mixed emission plus the claimed-pair count so the
/// caller can scale the precision guard
/// (`semantic <= ordinary + PRECISION_GUARD * claimed`).
pub(crate) fn try_partial_partition_section(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
) -> Option<(String, usize)> {
    // Split the prelude (`diff --git`/`---`/`+++`/mode lines) from the
    // native hunks; a section without hunks has nothing to partition.
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        if let Some(h) = current.as_mut() {
            h.push_str(line);
            h.push('\n');
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return None;
    }
    // Per-hunk pairs + old-side sites (whole balanced sections only;
    // the caller guarantees balance, re-checked per hunk here).
    struct HunkPairs<'a> {
        minuses: Vec<&'a str>,
        pluses: Vec<&'a str>,
        sites: Vec<usize>,
        native: String,
    }
    let mut parsed: Vec<HunkPairs> = Vec::with_capacity(hunks.len());
    let mut all_minuses: Vec<&str> = Vec::new();
    for h in &hunks {
        let mut minuses: Vec<&str> = Vec::new();
        let mut pluses: Vec<&str> = Vec::new();
        for line in h.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(&line[1..]);
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(&line[1..]);
            }
        }
        if minuses.len() != pluses.len() || minuses.is_empty() {
            return None;
        }
        let sites = crate::totality::changed_old_lines(h);
        if sites.len() != minuses.len() {
            return None;
        }
        all_minuses.extend(minuses.iter().copied());
        parsed.push(HunkPairs {
            minuses,
            pluses,
            sites,
            native: h.clone(),
        });
    }
    let total_pairs: usize = parsed.iter().map(|h| h.minuses.len()).sum();
    if total_pairs < 2 {
        return None;
    }
    // Identical pairs belong to the verbatim sweep (same as above).
    let mut ordered = all_minuses.clone();
    ordered.sort();
    ordered.dedup();
    if ordered.len() != all_minuses.len() {
        return None;
    }
    let checkpoint = used_rules.len();
    // First pass: classify each hunk without reserving rules.
    let mut seen_pats: Vec<String> = Vec::new();
    let mut hunk_claimed: Vec<bool> = Vec::with_capacity(parsed.len());
    // WI-19c: per-claimed-pair sweep-count flag, aligned with `parsed`
    // order (item pairs keep the sweep-count totality; impl pairs carry
    // their own scratch-apply totality and skip it).
    let mut hunk_impl_flags: Vec<Vec<bool>> = Vec::with_capacity(parsed.len());
    for h in &parsed {
        let mut ok = true;
        let mut pats: Vec<String> = Vec::new();
        let mut impl_flags: Vec<bool> = Vec::with_capacity(h.minuses.len());
        for (i, (m, p)) in h.minuses.iter().zip(h.pluses.iter()).enumerate() {
            let probe_before = used_rules.len();
            // WI-19c: item gate (now indent-tolerant) first, then the
            // self-ty `impl` single.
            let (body, is_impl) = match super::try_item_rename(
                m,
                p,
                path,
                file_old,
                &h.sites[i..i + 1],
                used_rules,
            ) {
                Some(full) => match full.split_once('\n') {
                    Some((_, b)) => (b.to_string(), false),
                    None => {
                        ok = false;
                        used_rules.truncate(probe_before);
                        break;
                    }
                },
                None => {
                    used_rules.truncate(probe_before);
                    match super::try_impl_header_single(
                        &[*m],
                        &[*p],
                        path,
                        file_old,
                        &h.sites[i..i + 1],
                        used_rules,
                    ) {
                        Some((full, _)) => match full.split_once('\n') {
                            Some((_, b)) => (b.to_string(), true),
                            None => {
                                ok = false;
                                used_rules.truncate(probe_before);
                                break;
                            }
                        },
                        None => {
                            ok = false;
                            used_rules.truncate(probe_before);
                            break;
                        }
                    }
                }
            };
            used_rules.truncate(probe_before);
            // One minus pattern per claim; a repeat would overlap-match.
            let mut minus_pat = None;
            let mut plus_pat = None;
            for line in body.lines() {
                if let Some(pat) = line.strip_prefix("- ") {
                    if minus_pat.is_none() {
                        minus_pat = Some(pat);
                    }
                } else if let Some(pat) = line.strip_prefix("+ ") {
                    if plus_pat.is_none() {
                        plus_pat = Some(pat);
                    }
                }
            }
            let (Some(mp), Some(pp)) = (minus_pat, plus_pat) else {
                ok = false;
                break;
            };
            if seen_pats.contains(&mp.to_string()) || pats.contains(&mp.to_string()) {
                ok = false;
                break;
            }
            // Pre-image totality per item pair (same posture as above).
            // WI-19c impl pairs skip this (scratch-apply probe already
            // passed; a same-name self-ty elsewhere is orphan evidence).
            if !is_impl && !crate::totality::template_pair_holds(path, file_old, mp, pp, 1) {
                ok = false;
                break;
            }
            pats.push(mp.to_string());
            impl_flags.push(is_impl);
        }
        if ok {
            seen_pats.extend(pats);
            hunk_impl_flags.push(impl_flags);
        } else {
            hunk_impl_flags.push(Vec::new());
        }
        hunk_claimed.push(ok);
    }
    let n_claimed: usize = parsed
        .iter()
        .zip(hunk_claimed.iter())
        .map(|(h, c)| if *c { h.minuses.len() } else { 0 })
        .sum();
    if n_claimed == 0 {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Second pass: reserve rules in order, rebuild claimed bodies
    // (rule names must be positional; the probe pass reserved nothing).
    let mut out = crate::git_header(path);
    out.push('\n');
    for (h, claimed) in parsed.iter().zip(hunk_claimed.iter()) {
        if *claimed {
            for (i, (m, p)) in h.minuses.iter().zip(h.pluses.iter()).enumerate() {
                let full = match super::try_item_rename(
                    m,
                    p,
                    path,
                    file_old,
                    &h.sites[i..i + 1],
                    used_rules,
                ) {
                    Some(f) => f,
                    None => match super::try_impl_header_single(
                        &[*m],
                        &[*p],
                        path,
                        file_old,
                        &h.sites[i..i + 1],
                        used_rules,
                    ) {
                        Some((f, _)) => f,
                        None => {
                            used_rules.truncate(checkpoint);
                            return None;
                        }
                    },
                };
                let Some((_, body)) = full.split_once('\n') else {
                    used_rules.truncate(checkpoint);
                    return None;
                };
                out.push_str(body);
            }
        } else {
            out.push_str(&h.native);
        }
    }
    Some((out, n_claimed))
}

/// WI-5 slice 2a: stranded-use veto for item rename + evolution
/// co-location (the item analogue of the WI-4a call-sweep suppression).
///
/// A section holding an item definition rename whose use sits in the
/// same native hunk as an evolution delta (alias/const/field/variant/
/// discriminant — the `definition_evolution` shapes) would otherwise
/// raise a bare rename single over the definition hunk while an
/// ordinary hunk silently rewrites the same rename at the stranded use
/// (probed: `- struct Old` sweep with `matches: 1` + orphan, the use
/// carried ordinary). Hunk atomicity cannot split the evolution hunk,
/// and no composed container-rename spelling exists yet (slice 2b owns
/// the firing shape), so the section stays wholly ordinary: `true`
/// asks the caller to decline.
///
/// Scope: balanced native hunks only (insertion shapes belong to slice
/// 2b — any unbalanced hunk answers `false`); `fn` defs belong to WI-4,
/// `mod`/`macro_rules!` decline like `item_intent`. A lone mixed hunk
/// answers `false` (already wholly ordinary, nothing to suppress); an
/// evolution with no stranded use answers `false` (independent singles
/// still partition).
pub(crate) fn section_has_stranded_item_joint(
    section: &str,
    path: &str,
    file_old: Option<&str>,
) -> bool {
    // Split native hunks; `rule=` hunks never count (already semantic).
    let mut hunks: Vec<String> = Vec::new();
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            hunks.push(String::new());
        }
        if let Some(h) = hunks.last_mut() {
            h.push_str(line);
            h.push('\n');
        }
    }
    if hunks.len() < 2 {
        return false;
    }
    struct Pairs {
        minuses: Vec<String>,
        pluses: Vec<String>,
        sites: Vec<usize>,
    }
    let mut parsed: Vec<Pairs> = Vec::with_capacity(hunks.len());
    for h in &hunks {
        if h.lines().next().is_some_and(|l| l.contains("rule=")) {
            return false;
        }
        let mut minuses: Vec<String> = Vec::new();
        let mut pluses: Vec<String> = Vec::new();
        for line in h.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
        if minuses.len() != pluses.len() || minuses.is_empty() {
            return false;
        }
        let sites = crate::totality::changed_old_lines(h);
        if sites.len() != minuses.len() {
            return false;
        }
        parsed.push(Pairs {
            minuses,
            pluses,
            sites,
        });
    }
    // Rename definitions in the section (item shapes only).
    let mut renames: Vec<(String, String)> = Vec::new();
    for h in &parsed {
        for (m, p) in h.minuses.iter().zip(h.pluses.iter()) {
            if let Some((_, _, old, new)) = split_rename_delta(m, p) {
                if !renames.iter().any(|(o, n)| o == &old && n == &new) {
                    renames.push((old, new));
                }
            }
        }
    }
    if renames.is_empty() {
        return false;
    }
    // A hunk is stranded iff it holds an evolution pair plus a use pair.
    for h in &parsed {
        let mut has_evo = false;
        let mut has_use = false;
        for ((m, p), site) in h.minuses.iter().zip(h.pluses.iter()).zip(h.sites.iter()) {
            if split_rename_delta(m, p).is_some() {
                continue;
            }
            if !has_evo {
                let mut scratch = Vec::new();
                let ms = [m.as_str()];
                let ps = [p.as_str()];
                let ss = [*site];
                if super::try_definition_evolution_sweep(
                    &ms,
                    &ps,
                    path,
                    file_old,
                    &ss,
                    &mut scratch,
                )
                .is_some()
                {
                    has_evo = true;
                }
            }
            if !has_use {
                for (old, new) in &renames {
                    if contains_ident(m, old) && contains_ident(p, new) {
                        has_use = true;
                        break;
                    }
                }
            }
            if has_evo && has_use {
                break;
            }
        }
        if has_evo && has_use {
            return true;
        }
    }
    false
}

/// WI-21 slice 4: cross-kind member suppression (the deletion analogue
/// of the slice-2a stranded-use veto). A section holding a `struct`
/// rename plus a variant-shaped deletion, or an `enum` rename plus a
/// field-shaped deletion, in unbalanced (pure-deletion or adjacent
/// shape-B-like) hunks would otherwise raise the bare rename single
/// over the def hunk while an ordinary hunk silently strands the
/// deletion (same forbidden middle as slice 2a: fires partially and
/// strands silently). The section stays wholly ordinary: `true` asks
/// the caller to decline.
///
/// Scope: unbalanced deletion hunks only (balanced evolution pairs
/// belong to slice 2a, already handled); `fn`/`mod`/`macro_rules!`
/// defs never count (same gate as `split_rename_delta`); same-kind
/// deletions never trigger (field/variant parses are disjoint, so a
/// struct+field or enum+variant section answers `false`); no carrier
/// proof — another container's cross-kind member strands the same way,
/// so it suppresses the same way.
pub(crate) fn section_has_cross_kind_deletion(section: &str) -> bool {
    // Split native hunks; `rule=` hunks never count (already semantic).
    let mut hunks: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if line.contains("rule=") {
                return false;
            }
            hunks.push((Vec::new(), Vec::new()));
        }
        if let Some((ms, ps)) = hunks.last_mut() {
            if line.starts_with('-') && !line.starts_with("---") {
                ms.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                ps.push(line[1..].to_string());
            }
        }
    }
    if hunks.is_empty() {
        return false;
    }
    // Def rename kinds present (balanced single-delta pairs only).
    let mut has_struct_rename = false;
    let mut has_enum_rename = false;
    for (ms, ps) in &hunks {
        if ms.len() != ps.len() || ms.is_empty() {
            continue;
        }
        for (m, p) in ms.iter().zip(ps.iter()) {
            if let Some((_, kind, _, _)) = split_rename_delta(m, p) {
                if kind == "struct" {
                    has_struct_rename = true;
                } else if kind == "enum" {
                    has_enum_rename = true;
                }
            }
        }
    }
    if !has_struct_rename && !has_enum_rename {
        return false;
    }
    // Deletion member kinds present (unbalanced hunks only).
    let mut has_field_deletion = false;
    let mut has_variant_deletion = false;
    for (ms, ps) in &hunks {
        if ms.len() == 1 && ps.is_empty() {
            if super::item_joint_field_add::parse_field_add(&ms[0]).is_some() {
                has_field_deletion = true;
            }
            if super::item_joint_field_add::parse_variant_add(&ms[0]).is_some() {
                has_variant_deletion = true;
            }
        } else if ms.len() == 2 && ps.len() == 1 {
            for m in ms {
                if super::item_joint_field_add::parse_field_add(m).is_some() {
                    has_field_deletion = true;
                }
                if super::item_joint_field_add::parse_variant_add(m).is_some() {
                    has_variant_deletion = true;
                }
            }
        }
    }
    (has_struct_rename && has_variant_deletion) || (has_enum_rename && has_field_deletion)
}

/// An item definition name delta, or `None` for use/evolution lines.
/// Mirrors the `item_intent` gate: `fn` belongs to WI-4, `mod` and
/// `macro_rules!` decline with it.
fn split_rename_delta(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
    let (o_pre, o_kind, o_id, o_suf) = super::split_item(minus.trim_end())?;
    let (n_pre, n_kind, n_id, n_suf) = super::split_item(plus.trim_end())?;
    if o_kind != n_kind || o_pre != n_pre || o_suf != n_suf || o_id == n_id {
        return None;
    }
    if o_kind == "fn" || o_kind == "mod" || o_kind == "macro_rules!" {
        return None;
    }
    Some((o_pre, o_kind, o_id, n_id))
}

/// Ident-boundary containment (never substring).
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn partial_partition_mixes_claimable_and_ordinary() {
        // WI-19: two claimable singleton hunks plus one unclaimable hunk
        // emit 2 semantic hunks + the native hunk verbatim, in order,
        // in one section. Round-trip must be byte-identical.
        let pre = "struct Old1 {\n    x: i32,\n}\nstruct Old2 {\n    x: i32,\n}\nfn m() {\n    let x = 1;\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old1 {\n+struct New1 {\n@@ -4 +4 @@\n-struct Old2 {\n+struct New2 {\n@@ -8 +8 @@\n-    let x = 1;\n+    let x = 2;\n";
        let mut used = Vec::new();
        let (out, n_claimed) = try_partial_partition_section(section, "f.rs", Some(pre), &mut used)
            .expect("partial must fire");
        assert_eq!(n_claimed, 2, "got:\n{out}");
        assert_eq!(out.matches("rule=").count(), 2, "got:\n{out}");
        assert!(
            out.contains("-    let x = 1;\n+    let x = 2;\n"),
            "ordinary remainder verbatim:\n{out}"
        );
        assert!(out.contains("@@ -8 +8 @@"), "native hunk kept:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = HashMap::from([("f.rs".to_string(), pre.to_string())]);
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "struct New1 {\n    x: i32,\n}\nstruct New2 {\n    x: i32,\n}\nfn m() {\n    let x = 2;\n}\n"
        );
    }

    #[test]
    fn partial_partition_vetoes_matrix() {
        // All-unclaimable section: None, rules untouched.
        let pre = "fn m() {\n    let x = 1;\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2 +2 @@\n-    let x = 1;\n+    let x = 2;\n@@ -3 +3 @@\n-}\n+}\n";
        let mut used = Vec::new();
        assert!(try_partial_partition_section(section, "f.rs", Some(pre), &mut used).is_none());
        assert!(used.is_empty());
        // Duplicate minus lines: verbatim territory, None.
        let pre2 = "struct Old1 {\n    x: i32,\n}\nstruct Old1b {\n    x: i32,\n}\n";
        let dup = "diff --git a/g.rs b/g.rs\n--- a/g.rs\n+++ b/g.rs\n@@ -1 +1 @@\n-struct Old1 {\n+struct New1 {\n@@ -4 +4 @@\n-struct Old1 {\n+struct New1 {\n";
        let mut used = Vec::new();
        assert!(try_partial_partition_section(dup, "g.rs", Some(pre2), &mut used).is_none());
        assert!(used.is_empty());
    }

    fn infer_section(section: &str, pre: &str) -> Option<String> {
        super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section.to_string()),
            &HashMap::from([("f.rs".to_string(), pre.to_string())]),
        )
    }

    #[test]
    fn stranded_item_joint_suppresses_rename_single() {
        // WI-5 slice 2a probe A: def rename + alias evolution + use, the
        // use sharing its hunk with the evolution. The bare def single
        // would strand the use — the section stays wholly ordinary.
        let pre = "struct Old {\n    x: u32,\n}\ntype A = Vec<u8>;\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old {\n+struct New {\n@@ -4,2 +4,2 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        assert!(section_has_stranded_item_joint(section, "f.rs", Some(pre)));
        let out = infer_section(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("rule="),
            "stranded section stays ordinary, got:\n{}",
            out.unwrap_or_default()
        );
    }

    #[test]
    fn independent_evo_without_stranded_use_still_partitions() {
        // Same def + evolution but no use anywhere: independent singles
        // still partition (no over-suppression).
        let pre = "struct Old {\n    x: u32,\n}\ntype A = Vec<u8>;\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old {\n+struct New {\n@@ -4 +4 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n";
        assert!(!section_has_stranded_item_joint(section, "f.rs", Some(pre)));
        let out = infer_section(section, pre).expect("def single must still fire");
        assert!(out.contains("rule="), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
    }

    #[test]
    fn use_with_def_plus_separate_evo_fires_intent() {
        // Use claimed alongside its def across pure hunks, evolution in
        // its own pure hunk: the follow intent still raises.
        let pre = "struct Old {\n    x: u32,\n}\n\ntype A = Vec<u8>;\n\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old {\n+struct New {\n@@ -5 +5 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n@@ -7 +7 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        assert!(!section_has_stranded_item_joint(section, "f.rs", Some(pre)));
        let out = infer_section(section, pre).expect("follow intent must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
    }

    #[test]
    fn fn_defs_never_strand() {
        // `fn` renames belong to WI-4: the item veto never claims them.
        let pre = "fn old() -> u32 {\n    1\n}\ntype A = Vec<u8>;\nfn f() -> u32 {\n    old()\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-fn old() -> u32 {\n+fn new() -> u32 {\n@@ -5,2 +5,2 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n-    old()\n+    new()\n";
        assert!(!section_has_stranded_item_joint(section, "f.rs", Some(pre)));
    }

    #[test]
    fn single_hunk_mixed_fires_field_evo_joint() {
        // WI-5 slice 2b flip target: rename + field-type evolution in one
        // hunk (same container) now raises as one joint intent hunk with
        // follow (previously wholly ordinary — the slice-2a pin).
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,5 +1,5 @@\n-struct Old {\n-    x: u32,\n+struct New {\n+    x: u64,\n }\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        assert!(!section_has_stranded_item_joint(section, "f.rs", Some(pre)));
        let out = infer_section(section, pre).expect("same-container joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        assert!(out.contains("+ struct New"), "got:\n{out}");
        assert!(out.contains("- x: u32"), "got:\n{out}");
        assert!(out.contains("+ x: u64"), "got:\n{out}");
        // Real-apply byte-compare: post equals the native post.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "struct New {\n    x: u64,\n}\nfn f(s: New) {\n}\n"
        );
    }

    #[test]
    fn cross_kind_deletion_matrix() {
        // WI-21 slice 4: struct+variant and enum+field suppress; same-kind
        // and balanced-evo shapes do not.
        let struct_variant = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -5 +4,0 @@\n-    B,\n@@ -7,1 +6,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        assert!(section_has_cross_kind_deletion(struct_variant));
        let enum_field = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -5 +4,0 @@\n-    y: u32,\n@@ -7,1 +6,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        assert!(section_has_cross_kind_deletion(enum_field));
        // Same-kind never suppresses.
        let struct_field = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -5,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        assert!(!section_has_cross_kind_deletion(struct_field));
        let enum_variant = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -5,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        assert!(!section_has_cross_kind_deletion(enum_variant));
        // Balanced evolution (no unbalanced deletion) never counts.
        let balanced = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old {\n+struct New {\n@@ -4 +4 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n";
        assert!(!section_has_cross_kind_deletion(balanced));
        // Lone def hunk (no deletion) never suppresses.
        let lone = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old {\n+struct New {\n";
        assert!(!section_has_cross_kind_deletion(lone));
    }

    #[test]
    fn two_same_kind_deletions_never_cross_kind() {
        // WI-21 slice 5: two same-kind deletions partition (slice-5 law)
        // and must never trip the slice-4 suppression gate — otherwise
        // the rename single they partition into would be wrongly
        // suppressed to wholly ordinary.
        let two_fields = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -4 +3,0 @@\n-    z: u32,\n@@ -6,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        assert!(!section_has_cross_kind_deletion(two_fields));
        let two_variants = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -4 +3,0 @@\n-    C,\n@@ -6,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        assert!(!section_has_cross_kind_deletion(two_variants));
    }

    #[test]
    fn cross_kind_deletion_suppresses_end_to_end() {
        // WI-21 slice 4 headline: the chain gate turns the section wholly
        // ordinary with the stranded delta named in declined provenance.
        let pre =
            "struct Old {\n    x: u32,\n}\nenum E {\n    A,\n    B,\n}\nfn f(s: Old, e: E) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -5 +4,0 @@\n-    B,\n@@ -7,1 +6,1 @@\n-fn f(s: Old, e: E) {\n+fn f(s: New, e: E) {\n";
        assert!(section_has_cross_kind_deletion(section));
        let out = infer_section(section, pre);
        assert!(
            out.is_none(),
            "cross-kind stays wholly ordinary, got:\n{}",
            out.unwrap_or_default()
        );
        // Provenance names the gate with the deletion line in sites.
        let mut prov = Vec::new();
        let opt = super::super::infer_semantic_with_provenance(
            std::slice::from_ref(&section.to_string()),
            &HashMap::from([("f.rs".to_string(), pre.to_string())]),
            &mut prov,
        );
        assert!(opt.is_none(), "must stay ordinary");
        assert_eq!(prov.len(), 1, "one ordinary entry, got: {prov:?}");
        assert_eq!(prov[0].detector, "ordinary");
        assert!(
            prov[0]
                .declined
                .iter()
                .any(|d| d.detector == "section" && d.reason == "cross-kind-member"),
            "gate named, got: {:?}",
            prov[0].declined
        );
        assert!(
            prov[0].sites.contains(&5),
            "stranded delta line named, got: {:?}",
            prov[0].sites
        );
    }
}
