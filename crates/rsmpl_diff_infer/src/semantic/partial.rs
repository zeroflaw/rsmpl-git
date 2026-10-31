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
}
