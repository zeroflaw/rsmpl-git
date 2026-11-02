//! WI-5-R2 vis-evolving item-rename probe (SPEC D.4 bare-item joint).
//!
//! Sibling of the intent pre-pass (D5 line gate): the vis-evolving twin of
//! `try_item_rename` plus its closed-vis gate. The classifier lives in
//! `item_joint_visqual`; the emission rides the same module. This file owns
//! only the probe the pre-pass runs before emitting.
//!
//! Bare-item kinds only (`fn` rides the fn joint, `mod` rides WI-15c,
//! `impl` / `macro_rules!` take no visibility).

use super::{contains_word, ident_len, item_emission_header, split_item, unique_rule};

/// WI-5-R2 vis-evolving `try_item_rename` twin: the vis slot may step once
/// within the WI-1 closed set while every other slot stays token-equal
/// and the names differ. Bare-item kinds only (`fn` rides the fn joint,
/// `mod` rides WI-15c, `impl` / `macro_rules!` take no visibility).
/// Emits the concrete evolved terse pair the engine vis arm applies.
pub(crate) fn try_item_rename_vis_joint(
    old_line: &str,
    new_line: &str,
    path: &str,
    file_old: Option<&str>,
    sites: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let old = old_line.trim_end();
    let new = new_line.trim_end();
    if old == new {
        return None;
    }
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
    if old_kind == "fn" || old_kind != new_kind {
        return None;
    }
    if matches!(old_kind.as_str(), "impl" | "macro_rules!" | "mod") {
        return None;
    }
    if old_ident.is_empty() || new_ident.is_empty() || old_ident == new_ident {
        return None;
    }
    if old_prefix == new_prefix || old_suffix != new_suffix {
        return None;
    }
    if !is_closed_vis_prefix(&old_prefix) || !is_closed_vis_prefix(&new_prefix) {
        return None;
    }
    // Self-referential vis is an evolution, never a joint.
    if contains_word(&old_prefix, &old_ident)
        || contains_word(&old_prefix, &new_ident)
        || contains_word(&new_prefix, &old_ident)
        || contains_word(&new_prefix, &new_ident)
    {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let mut hunk = item_emission_header(path, &rule, file_old, sites);
    hunk.push_str(&format!(
        "- {old_prefix}{old_kind} {old_ident}\n+ {new_prefix}{new_kind} {new_ident}\n"
    ));
    Some(hunk)
}

/// WI-1 closed vis set for terse renames (`""`, `pub`, `pub(crate)`,
/// `pub(super)`, `pub(in <path>)`). Mirrors the item-intent gate (local
/// copy per the per-module convention).
fn is_closed_vis_prefix(prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "pub " {
        return true;
    }
    if prefix == "pub(crate) " || prefix == "pub(super) " {
        return true;
    }
    let Some(inner) = prefix
        .strip_prefix("pub(in ")
        .and_then(|r| r.strip_suffix(") "))
    else {
        return false;
    };
    let mut segs = inner.split("::").peekable();
    let mut first = true;
    while let Some(seg) = segs.next() {
        if seg.is_empty() {
            return false;
        }
        let head_ok =
            first && matches!(seg, "crate" | "self" | "super" | "$crate") && segs.peek().is_some();
        if !head_ok && ident_len(seg) != Some(seg.len()) {
            return false;
        }
        first = false;
    }
    !first
}
