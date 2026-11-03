//! WI-22 item-intent probe axis (D5 split from `item_intent`):
//! the native `Pair`, the `split_item` definition delta, and the
//! scratch-apply uniqueness probe. Re-exported through
//! `item_intent` so existing `super::item_intent::{Pair, ...}`
//! paths keep resolving; no behavior change.

use std::collections::{HashMap, HashSet};

use super::split_item;

/// One native pair at its old-side site.
pub(crate) struct Pair {
    pub(crate) sec: usize,
    pub(crate) path: String,
    pub(crate) minus: String,
    pub(crate) plus: String,
    pub(crate) site: usize,
}

/// A `split_item` definition delta, or `None` for use lines.
pub(crate) fn split_delta(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(plus.trim_end())?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    Some((old_prefix, old_kind, old_ident, new_ident))
}

/// Count `split_item` definitions of (`kind`, `ident`) across the
/// involved pre-images (one line per definition).
pub(crate) fn count_kind_defs(
    pre: &HashMap<String, String>,
    pairs: &[Pair],
    kind: &str,
    ident: &str,
) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut count = 0usize;
    for pair in pairs {
        if !seen.insert(pair.path.clone()) {
            continue;
        }
        if let Some(text) = pre.get(&pair.path) {
            for line in text.lines() {
                if let Some((_, k, id, _)) = split_item(line) {
                    if k == kind && id == ident {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Scratch-apply probe: the emission applied to a `pre` clone must show
/// each claimed plus line at its site with every other line identical.
pub(crate) fn probe_reproduces(
    pre: &HashMap<String, String>,
    emission: &str,
    pairs: &[Pair],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    // Per-file claim index: site -> plus trim.
    let mut claims: HashMap<&str, HashMap<usize, &str>> = HashMap::new();
    for pair in pairs {
        claims
            .entry(pair.path.as_str())
            .or_default()
            .insert(pair.site, pair.plus.trim());
    }
    for (path, pre_text) in pre {
        let Some(post_text) = files.get(path) else {
            return false;
        };
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            return false;
        }
        let empty: HashMap<usize, &str> = HashMap::new();
        let claim = claims.get(path.as_str()).unwrap_or(&empty);
        for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            let line_no = idx + 1;
            if let Some(want) = claim.get(&line_no) {
                if post_line.trim() != *want {
                    return false;
                }
            } else if pre_line != post_line {
                return false;
            }
        }
    }
    true
}
