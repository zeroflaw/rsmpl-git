//! N=1 macro-rename single plus its T361 byte-compare probe.
//! split off verbatim under the WI-19 1900-line warning gate (D5): pure move,
//! no behavior change. Tests stay inline in the origin file per WI-22.
use super::macro_parts::{parse_macro_pair, split_macro_path};
use super::unique_rule;

/// N=1 macro-rename single (WI-3 slice 3): one `old!(body)` →
/// `new!(body)` invocation pair with a shared prefix, a last-segment
/// name delta, and token-equal bodies compresses to the allow-one
/// verbatim pair (`- old!(...)` / `+ new!(...)` under
/// `# macro=token_tree`, no allow/scope directive — the WI-3 slice-1
/// use-prefix single precedent). Uniqueness is proved by
/// `macro_holds` / `qualified_macro_holds` at n=1 (exactly one `old`
/// invocation in the file, opaque macro/string interiors veto) plus
/// the `scratch_single_reproduces` byte-compare pre-reservation over
/// the actual emission. The holed diet is never emitted (the N≥2
/// `rename_hole_template` needs ≥2 sites by construction); a holed
/// authored leg (fixture 86) unifies via the D17 verbatim equality
/// arm, same posture as the WI-1s1 holed singles. Same-name interior
/// edits (fixture 75) decline here (`parse_macro_pair` returns equal
/// paths — the interior family owns them).
pub(crate) fn try_macro_rename_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    let Some((old_path, new_path)) = parse_macro_pair(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    if old_path == new_path {
        return super::prov::Counted::Miss;
    }
    let Some((o_pre, o_last)) = split_macro_path(&old_path) else {
        return super::prov::Counted::Miss;
    };
    let Some((n_pre, n_last)) = split_macro_path(&new_path) else {
        return super::prov::Counted::Miss;
    };
    if o_pre != n_pre || o_last == n_last {
        return super::prov::Counted::Miss;
    }
    if o_pre.is_empty() {
        match crate::totality_counted::macro_verdict(file_old, &o_last, false, minuses, changed) {
            crate::totality_counted::FamilyVerdict::Holds => {}
            crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality_counted::FamilyVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            crate::totality_counted::FamilyVerdict::Unproven => {
                return super::prov::Counted::Miss;
            }
        }
    } else {
        match crate::totality_counted::macro_verdict(file_old, &old_path, true, minuses, changed) {
            crate::totality_counted::FamilyVerdict::Holds => {}
            crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality_counted::FamilyVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            crate::totality_counted::FamilyVerdict::Unproven => {
                return super::prov::Counted::Miss;
            }
        }
    }
    let checkpoint = used_rules.len();
    let flat = |p: &str| p.replace("::", "_");
    let rule = unique_rule(
        used_rules,
        &format!("macros_{}_to_{}", flat(&old_path), flat(&new_path)),
    );
    // Allow-one: ranged hunk with no `# allow` directive (the
    // use-prefix single precedent); `# macro=token_tree` stays (the
    // engine needs it — paren `...` matches every delimiter, probed).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str("# macro=token_tree\n");
    hunk.push_str(&format!("- {old_path}!(...)\n+ {new_path}!(...)\n"));
    // Defensive: the probes above already applied equivalent
    // spellings, but the hunk header is new — never emit a guess.
    if rsmpl_patch::parse_patch(&hunk).is_err() {
        used_rules.truncate(checkpoint);
        return super::prov::Counted::Miss;
    }
    if !macro_single_reproduces(pre, path, &hunk, changed, pluses) {
        used_rules.truncate(checkpoint);
        return super::prov::Counted::Miss;
    }
    super::prov::Counted::Fire(hunk)
}

/// T361 byte-compare probe for the N=1 macro-rename single (the
/// slot.rs `scratch_single_reproduces` precedent, file-local like the
/// trait_methods.rs copy): the allow-one emission must real-apply to
/// the pre-image byte-identical to the native single-line swap.
/// `changed` cites the exact site; every other line stays verbatim.
fn macro_single_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let post = &files[path];
    let pre_lines: Vec<&str> = pre.lines().collect();
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() {
        return false;
    }
    let changed_set: std::collections::HashSet<usize> = changed.iter().copied().collect();
    for (i, &site) in changed.iter().enumerate() {
        let Some(post_line) = post_lines.get(site.saturating_sub(1)) else {
            return false;
        };
        if post_line.trim() != pluses[i].trim() {
            return false;
        }
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if changed_set.contains(&(idx + 1)) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
    true
}
