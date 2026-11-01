//! Token-aware placeholder normalization (WI-2d gap 3 follow-up).
//!
//! Split from [`crate::use_graph_local`] (D5: that file sits on the 1600-line
//! watch line). A single-hole placeholder (`__RSMPL_E`) is a string prefix
//! of the `...` block marker (`__RSMPL_ELLIPSIS_0`), so plain
//! `str::replace` mangles markers. The helper below fires only on whole
//! ident-token boundaries, longest placeholder first.

use std::collections::HashMap;

/// Map `__RSMPL_*` placeholder idents back to their declared names, one
/// whole ident token at a time. Both sides of every binder-equality check
/// go through this helper so `...` markers compare intact.
pub(crate) fn normalize_placeholders(s: &str, ph_map: &HashMap<&str, &str>) -> String {
    if ph_map.is_empty() || !s.contains("__RSMPL_") {
        return s.to_string();
    }
    let mut keys: Vec<(&str, &str)> = ph_map.iter().map(|(k, v)| (*k, *v)).collect();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let mut matched: Option<(usize, &str)> = None;
        for (ph, name) in &keys {
            let pb = ph.as_bytes();
            if i + pb.len() > bytes.len() {
                continue;
            }
            if &bytes[i..i + pb.len()] != pb {
                continue;
            }
            let prev_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let next_ok = i + pb.len() == bytes.len() || !is_ident_byte(bytes[i + pb.len()]);
            if prev_ok && next_ok {
                matched = Some((pb.len(), *name));
                break;
            }
        }
        if let Some((len, name)) = matched {
            out.push_str(name);
            i += len;
        } else {
            // Emit one UTF-8 char to keep indices valid on non-ASCII.
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn ellipsis_marker_survives_single_hole_normalization() {
        // Real single-hole naming: `__RSMPL_E` is a string prefix of the
        // `...` block marker. Whole-token replacement must leave the
        // marker intact on both sides.
        let mut map = HashMap::new();
        map.insert("__RSMPL_E", "E");
        assert_eq!(
            normalize_placeholders("__RSMPL_ELLIPSIS_0", &map),
            "__RSMPL_ELLIPSIS_0"
        );
        assert_eq!(
            normalize_placeholders("for total in __RSMPL_E { __RSMPL_ELLIPSIS_0 }", &map),
            "for total in E { __RSMPL_ELLIPSIS_0 }"
        );
        assert_eq!(normalize_placeholders("__RSMPL_E", &map), "E");
    }

    #[test]
    fn placeholder_prefix_does_not_fire_inside_longer_ident() {
        let mut map = HashMap::new();
        map.insert("__RSMPL_E", "E");
        assert_eq!(normalize_placeholders("__RSMPL_E1", &map), "__RSMPL_E1");
        assert_eq!(normalize_placeholders("x__RSMPL_E", &map), "x__RSMPL_E");
    }

    #[test]
    fn overlapping_placeholders_longest_first() {
        let mut map = HashMap::new();
        map.insert("__RSMPL_E", "E");
        map.insert("__RSMPL_E1", "B");
        assert_eq!(normalize_placeholders("__RSMPL_E1", &map), "B");
        assert_eq!(normalize_placeholders("__RSMPL_E", &map), "E");
    }
}
