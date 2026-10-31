//! WI-10a: shared attribute-delta helpers (SPEC 14.6).
//!
//! Written attr lines are the delta under `# attrs=preserve`: minus-empty /
//! plus-N adds, minus-N / plus-empty removes, same-length differing replaces
//! (1-to-1, written order). Other policies keep the C11/C12 gates.
//! Both the fn and item matchers share this module (D5: no second copy of
//! the matching/normalization laws).

use crate::lines::LineIndex;

/// Whitespace-insensitive normalization for pattern-vs-site comparison.
/// `///`/`#[doc]` mirrors compare equal (normalized at parse).
pub(crate) fn norm_pat(s: &str) -> String {
    s.split_whitespace().collect()
}

pub(crate) fn norm_pat_list(xs: &[String]) -> Vec<String> {
    xs.iter().map(|a| norm_pat(a)).collect()
}

/// Rendered site attrs (`quote!`) normalized for comparison.
pub(crate) fn site_attr_texts(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .map(|a| norm_pat(&quote::quote!(#a).to_string()))
        .collect()
}

/// Indices of each `want` line inside `have` (subsequence, in order).
/// `None` when any want line is absent.
pub(crate) fn subsequence_positions(have: &[String], want: &[String]) -> Option<Vec<usize>> {
    let mut out = Vec::with_capacity(want.len());
    let mut cursor = 0;
    for w in want {
        let mut found = None;
        for (i, h) in have.iter().enumerate().skip(cursor) {
            if h == w {
                found = Some(i);
                cursor = i + 1;
                break;
            }
        }
        out.push(found?);
    }
    Some(out)
}

/// Compact a token-joined pattern attr (`# [ inline ]`) to valid Rust
/// (`#[inline]`).
pub(crate) fn compact_attr(s: &str) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    t.replace("# [", "#[")
        .replace("[ ", "[")
        .replace(" ]", "]")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace(" ,", ",")
        .replace(" ::", "::")
        .replace(":: ", "::")
}

/// WI-10a delta mode for one hunk (Preserve only; other policies keep C11/C12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttrDelta {
    Mirror,
    Add,
    Remove,
    Replace,
}

pub(crate) fn attr_delta(minus: &[String], plus: &[String]) -> AttrDelta {
    let m = norm_pat_list(minus);
    let p = norm_pat_list(plus);
    if m == p {
        return AttrDelta::Mirror;
    }
    if minus.is_empty() && !plus.is_empty() {
        return AttrDelta::Add;
    }
    if !minus.is_empty() && plus.is_empty() {
        return AttrDelta::Remove;
    }
    if minus.len() == plus.len() {
        return AttrDelta::Replace;
    }
    AttrDelta::Mirror
}

/// Leading whitespace of 1-based `line` (insert indent).
pub(crate) fn line_indent(original: &str, index: &LineIndex, line: u32) -> String {
    let Some(start) = index.line_start(line) else {
        return String::new();
    };
    let rest = &original[start..];
    let cur = rest.split('\n').next().unwrap_or("");
    cur.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}
