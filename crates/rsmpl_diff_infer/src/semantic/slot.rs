//! N-site heterogeneous fn/method rename slot sweep (SPEC 14).
//!
//! Split from `mod.rs` under the 2000-line gate (D5): pure move,
//! no behavior change.

use super::{ident_len, is_ident_char, item_emission_header, unique_rule};

/// N-site (N ≥ 2) heterogeneous fn/method rename with a
/// callee-only delta: every pair is a single-line `{`-terminated
/// fn header differing in exactly the fn ident (vis/qual/generics/
/// params/return/where byte-equal per site, heterogeneous across
/// sites). Column-0 sites emit the free-fn slot sweep (T329 shape);
/// uniformly indented sites emit the `impl Svc` advisory-context
/// sweep (T337 shape); mixed indent stays passthrough.
pub(crate) fn try_fn_slot_sweep(
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
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n) = fn_rename_pair(m, p)?;
        if old_name.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_name.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        old_name = Some(o);
        new_name = Some(n);
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let all_col0 = minuses
        .iter()
        .all(|m| !m.is_empty() && !m.starts_with([' ', '\t']));
    if !all_indented && !all_col0 {
        return None;
    }
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    let plus_col0 = pluses
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with([' ', '\t']));
    if all_indented != plus_indented || all_col0 != plus_col0 {
        return None;
    }
    if !crate::totality::slot_holds(file_old, &old_name, minuses, all_indented, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    let mut hunk = item_emission_header(path, &rule, file_old, changed);
    if all_indented {
        hunk.push_str(&format!(
            "  impl Svc {{\n      ...\n-     V Q fn {old_name} G (...) W {{\n+     V Q fn {new_name} G (...) W {{\n          ...\n      }}\n  }}\n"
        ));
    } else {
        hunk.push_str(&format!(
            "- V Q fn {old_name} G (...) W {{\n+ V Q fn {new_name} G (...) W {{\n      ...\n  }}\n"
        ));
    }
    Some(hunk)
}

/// A `-`/`+` line pair as a rename-only fn header: both trimmed
/// lines end with `{`, both parse as fn-shaped (`ItemFn` or
/// `ImplItemFn`), and they differ in exactly the fn ident.
fn fn_rename_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o = old_line.trim();
    let p = new_line.trim();
    if o.is_empty() || p.is_empty() {
        return None;
    }
    if !o.ends_with('{') || !p.ends_with('{') {
        return None;
    }
    if !is_fn_shaped(o) || !is_fn_shaped(p) {
        return None;
    }
    let (o_pre, o_ident, o_suf) = split_fn_ident(o)?;
    let (p_pre, p_ident, p_suf) = split_fn_ident(p)?;
    if o_pre != p_pre || o_suf != p_suf {
        return None;
    }
    if o_ident == p_ident || o_ident.is_empty() || p_ident.is_empty() {
        return None;
    }
    if o_suf.is_empty() {
        return None;
    }
    let tail = o_suf.trim_start();
    if !(tail.starts_with('(') || tail.starts_with('<')) {
        return None;
    }
    Some((o_ident, p_ident))
}

fn is_fn_shaped(trimmed: &str) -> bool {
    let probe = format!("{trimmed} }}");
    syn::parse_str::<syn::ItemFn>(&probe).is_ok()
        || syn::parse_str::<syn::ImplItemFn>(&probe).is_ok()
}

/// Split `... fn Ident<...>(...)` into (prefix-before-ident, ident,
/// suffix-after-ident). The `fn` keyword must be word-bounded.
fn split_fn_ident(text: &str) -> Option<(String, String, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..i + 2] == *b"fn"
            && (i == 0 || !is_ident_char(bytes[i - 1] as char))
            && (i + 2 < bytes.len() && bytes[i + 2].is_ascii_whitespace())
        {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            let len = ident_len(&text[j..])?;
            let end = start + len;
            return Some((
                text[..start].to_string(),
                text[start..end].to_string(),
                text[end..].to_string(),
            ));
        }
        i += 1;
    }
    None
}
