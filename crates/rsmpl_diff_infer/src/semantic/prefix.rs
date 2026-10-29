//! `use`-path prefix rename fan-out (SPEC 14.9).
//!
//! Split from `mod.rs` under the 2000-line gate (D5): pure move,
//! no behavior change.

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
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut minus_paths: Vec<Vec<String>> = Vec::new();
    let mut plus_paths: Vec<Vec<String>> = Vec::new();
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        minus_paths.push(parse_use_path(m)?);
        plus_paths.push(parse_use_path(p)?);
    }
    let old_prefix = common_prefix(&minus_paths);
    let new_prefix = common_prefix(&plus_paths);
    // Tight formulation: equal lengths, at least two segments (a lone
    // `use crate` matches too broadly to infer from N lines of
    // evidence), shared parent, renamed last segment.
    if old_prefix.len() != new_prefix.len() || old_prefix.len() < 2 {
        return None;
    }
    let k = old_prefix.len();
    if old_prefix[..k - 1] != new_prefix[..k - 1] {
        return None;
    }
    if old_prefix[k - 1] == new_prefix[k - 1] {
        return None;
    }
    for (m, p) in minus_paths.iter().zip(plus_paths.iter()) {
        if m.len() <= k || p.len() <= k {
            return None;
        }
        if m[k..] != p[k..] {
            return None;
        }
    }
    if !crate::totality::use_holds(file_old, &old_prefix, minuses, changed) {
        return None;
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
