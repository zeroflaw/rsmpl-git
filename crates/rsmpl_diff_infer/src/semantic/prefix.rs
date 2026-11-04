//! `use`-path prefix rename fan-out (SPEC 14.9).
//!
//! Split from `mod.rs` under the 2000-line gate (D5): pure move,
//! no behavior change.

use super::prov::Counted;
use super::unique_rule;

/// N-site (N ≥ 2) use-path prefix rename fan-out (T363, SPEC 14.9):
/// every pair parses as `use <P_old>::<tail>` → `use <P_new>::<tail>`
/// with per-site textually identical non-empty tails and one shared
/// parent + one shared renamed last segment (the tight "one segment
/// renamed" formulation — the direct analog of the call sweep's
/// callee-only delta). Leading `pub…`/`::` and trailing `;` are
/// normalized away (the engine ignores them, so mixed `use`/`pub use`
/// sites share one hunk, probe-verified); tails may be heterogeneous
/// across sites (plain, grouped, glob, `as` — the engine preserves
/// them). Emits one `# allow=many` hunk (tight original range)
/// (`- use P_old` / `+ use P_new`, no metavars). Single-segment
/// shared prefixes (`use crate`), unequal prefix lengths, divergent
/// parents/tails, and lone pairs stay ordinary passthrough.
pub(crate) fn try_use_prefix_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() < 2 {
        return Counted::Miss;
    }
    let mut minus_paths: Vec<Vec<String>> = Vec::new();
    let mut plus_paths: Vec<Vec<String>> = Vec::new();
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some(__m) = parse_use_path(m) else {
            return Counted::Miss;
        };
        let Some(__p) = parse_use_path(p) else {
            return Counted::Miss;
        };
        minus_paths.push(__m);
        plus_paths.push(__p);
    }
    let old_prefix = common_prefix(&minus_paths);
    let new_prefix = common_prefix(&plus_paths);
    // Tight formulation: equal lengths, at least two segments (a lone
    // `use crate` matches too broadly to infer from N lines of
    // evidence), shared parent, renamed last segment.
    if old_prefix.len() != new_prefix.len() || old_prefix.len() < 2 {
        return Counted::Miss;
    }
    let k = old_prefix.len();
    if old_prefix[..k - 1] != new_prefix[..k - 1] {
        return Counted::Miss;
    }
    if old_prefix[k - 1] == new_prefix[k - 1] {
        return Counted::Miss;
    }
    for (m, p) in minus_paths.iter().zip(plus_paths.iter()) {
        if m.len() <= k || p.len() <= k {
            return Counted::Miss;
        }
        if m[k..] != p[k..] {
            return Counted::Miss;
        }
    }
    // WI-4 slice 2b-1: counted decline names `totality:`/`veto:`
    // instead of a bare `no-match`.
    match crate::totality_counted::use_verdict(file_old, &old_prefix, minuses, changed) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(
        used_rules,
        &format!("use_{}_to_{}", old_prefix[k - 1], new_prefix[k - 1]),
    );
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!(
        "- use {}\n+ use {}\n",
        old_prefix.join("::"),
        new_prefix.join("::")
    ));
    Counted::Fire(hunk)
}

/// N=1 use-path prefix single (WI-3 slice 1): one `use P_old::Tail`
/// → `use P_new::Tail` pair with a shared parent, one renamed
/// segment, and a textually equal non-empty tail compresses to the
/// allow-one bare-prefix pair (`- use P_old` / `+ use P_new`, no
/// directive — the `calls.rs`/`type_single.rs` allow-one precedent).
/// Uniqueness is proved by `use_holds` at n=1 (zero-or-two same
/// leading-prefix spans veto, fn-body pairs veto because fn-body
/// `use` spans never count) plus the T361 `single_reproduces`
/// byte-compare pre-reservation. Tail-empty leaf renames
/// (`use crate::old_mod;`) decline like the N>=2 sweep (which
/// requires `m.len() > k`); single-segment prefixes, parent
/// mismatches, and multi-rename pairs decline. Detector name
/// `use_prefix` (same family as the sweep; `guard_for` stays
/// `Strict` for multis — the singles blocks price their own
/// precision allowance).
pub(crate) fn try_use_prefix_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let minus_segs = parse_use_path(minuses[0])?;
    let plus_segs = parse_use_path(pluses[0])?;
    if minus_segs.len() != plus_segs.len() || minus_segs.len() < 3 {
        return None;
    }
    // Shared parent (longest common prefix), exactly one renamed
    // segment, textually equal non-empty tail.
    let mut parent = 0usize;
    while parent < minus_segs.len() && minus_segs[parent] == plus_segs[parent] {
        parent += 1;
    }
    // `parent` is the first diff index by construction (the
    // all-equal case exits here via `parent + 1 >= len`), so the
    // renamed segments differ and only the tail needs comparing.
    if parent < 1 || parent + 1 >= minus_segs.len() {
        return None;
    }
    if minus_segs[parent + 1..] != plus_segs[parent + 1..] {
        return None;
    }
    let old_prefix: Vec<String> = minus_segs[..parent + 1].to_vec();
    if !crate::totality::use_holds(Some(pre), &old_prefix, minuses, changed) {
        return None;
    }
    let new_prefix: Vec<String> = plus_segs[..parent + 1].to_vec();
    let minus_tpl = format!("use {}", old_prefix.join("::"));
    let plus_tpl = format!("use {}", new_prefix.join("::"));
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return None;
    }
    let checkpoint = used_rules.len();
    let rule = super::unique_rule(
        used_rules,
        &format!("use_{}_to_{}", minus_segs[parent], plus_segs[parent]),
    );
    // Allow-one: directive-free ranged hunk (singles-block precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    // Defensive: the probes above already applied equivalent spellings,
    // but the hunk header is new — never emit a guess.
    if rsmpl_patch::parse_patch(&hunk).is_err() {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some(hunk)
}

/// Parse one diff line as a `use` path: strip one trailing `;`,
/// leading visibility, the `use` keyword, and one leading `::`; split
/// the rest on `::`. The emitted pattern is the bare common prefix,
/// which is what the engine requires (SPEC 14.9 rejects braces, globs,
/// and `as` in the pattern — those may only appear in tails, where
/// per-site equality keeps them verbatim).
fn parse_use_path(line: &str) -> Option<Vec<String>> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    // Leading visibility (`pub`, `pub(…)`) is normalized away — the
    // pattern carries none and the engine matches both sides.
    if let Some(rest) = text.strip_prefix("pub") {
        if rest.is_empty() || rest.starts_with([' ', '\t']) {
            text = rest.trim_start();
        } else if rest.starts_with('(') {
            let end = rest.find(')')?;
            let after = &rest[end + 1..];
            if after.is_empty() || !after.starts_with([' ', '\t']) {
                return None;
            }
            text = after.trim_start();
        } else {
            return None;
        }
    }
    let mut text = text.strip_prefix("use")?;
    // `use` must be the keyword, not an ident prefix (`useful::x`).
    if text.is_empty() || !text.starts_with([' ', '\t']) {
        return None;
    }
    text = text.trim_start();
    // Tolerate one leading `::` (`use ::crate::old::Thing`).
    if let Some(rest) = text.strip_prefix("::") {
        text = rest.trim_start();
    }
    if text.is_empty() {
        return None;
    }
    let segs: Vec<String> = text.split("::").map(|s| s.trim().to_string()).collect();
    if segs.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(segs)
}

/// Longest common leading segments across all paths.
fn common_prefix(paths: &[Vec<String>]) -> Vec<String> {
    let mut out = paths.first().cloned().unwrap_or_default();
    for p in paths.iter().skip(1) {
        let n = out.iter().zip(p.iter()).take_while(|(a, b)| a == b).count();
        out.truncate(n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_prefix_single_fires_with_preserved_tail() {
        // WI-3 slice 1 (fixture 102 shape): one `use` pair with a
        // shared parent + renamed segment + preserved tail raises.
        let pre = "use crate::old::Thing;\nuse crate::other::Untouched;\n";
        let mut used = Vec::new();
        let out = try_use_prefix_single(
            &["use crate::old::Thing;"],
            &["use crate::new::Thing;"],
            "f.rs",
            Some(pre),
            &[1],
            &mut used,
        )
        .expect("single must fire");
        assert!(out.contains("- use crate::old\n"), "got:\n{out}");
        assert!(out.contains("+ use crate::new\n"), "got:\n{out}");
        assert!(
            !out.contains("allow"),
            "allow-one carries no directive, got:\n{out}"
        );
        // The emission real-applies to the exact native post-tree.
        let parsed = rsmpl_patch::parse_patch(&format!("{}\n{out}", crate::git_header("f.rs")))
            .expect("emission must parse with header");
        let mut files = std::collections::HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "use crate::new::Thing;\nuse crate::other::Untouched;\n"
        );
    }

    #[test]
    fn use_prefix_single_vetoes_twin_and_leaf_and_body() {
        // An untouched twin (count 2) vetoes — same posture as the sweep.
        let pre2 = "use crate::old::Thing;\nuse crate::old::Other;\n";
        let mut used = Vec::new();
        assert!(
            try_use_prefix_single(
                &["use crate::old::Thing;"],
                &["use crate::new::Thing;"],
                "f.rs",
                Some(pre2),
                &[1],
                &mut used,
            )
            .is_none(),
            "twin prefix must veto"
        );
        // Tail-empty leaf renames stay ordinary (sweep parity: `m.len() > k`).
        let pre_leaf = "use crate::old_mod;\n";
        assert!(
            try_use_prefix_single(
                &["use crate::old_mod;"],
                &["use crate::new_mod;"],
                "f.rs",
                Some(pre_leaf),
                &[1],
                &mut Vec::new(),
            )
            .is_none(),
            "leaf rename must stay ordinary"
        );
        // Single-segment prefixes never infer (too broad).
        assert!(
            try_use_prefix_single(
                &["use crate;"],
                &["use other;"],
                "f.rs",
                Some("use crate;\n"),
                &[1],
                &mut Vec::new(),
            )
            .is_none(),
            "single segment must stay ordinary"
        );
        // Fn-body `use` pairs stay ordinary (SPEC 14.9: only file-
        // and inline-mod-level `use` items match; `use_holds` never
        // counts them, so the count misses and vetoes).
        let pre_body = "fn m() {\n    use crate::old::Thing;\n}\n";
        assert!(
            try_use_prefix_single(
                &["    use crate::old::Thing;"],
                &["    use crate::new::Thing;"],
                "f.rs",
                Some(pre_body),
                &[2],
                &mut Vec::new(),
            )
            .is_none(),
            "fn-body use must stay ordinary"
        );
        // Parent mismatch (two renamed segments) stays ordinary.
        let pre_mm = "use crate::old::Thing;\n";
        assert!(
            try_use_prefix_single(
                &["use crate::old::Thing;"],
                &["use other::new::Thing;"],
                "f.rs",
                Some(pre_mm),
                &[1],
                &mut Vec::new(),
            )
            .is_none(),
            "parent mismatch must stay ordinary"
        );
    }

    #[test]
    fn use_prefix_single_fires_through_grouped_tail() {
        // Grouped/glob/`as` tails ride verbatim (target-side freedom,
        // SPEC 14.9) — the tail segment is textually equal.
        let pre = "use crate::old::{A, B};\n";
        let mut used = Vec::new();
        let out = try_use_prefix_single(
            &["use crate::old::{A, B};"],
            &["use crate::new::{A, B};"],
            "f.rs",
            Some(pre),
            &[1],
            &mut used,
        )
        .expect("grouped tail must fire");
        assert!(out.contains("- use crate::old\n"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&format!("{}\n{out}", crate::git_header("f.rs")))
            .expect("emission must parse with header");
        let mut files = std::collections::HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "use crate::new::{A, B};\n");
    }
}
