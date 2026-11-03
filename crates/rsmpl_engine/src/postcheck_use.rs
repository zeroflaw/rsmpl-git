//! `use`-regroup merge postcondition (SPEC 14.9, WI-14 slice 2).
//!
//! Split from `postcheck` per the D5 hygiene gate (`postcheck.rs` sits
//! near 2000 lines): pure move, no behavior change.

use rsmpl_pattern::PatternKind;

/// Verify a `UseSplit` hunk in the post-image: the minus grouped line must
/// be gone as a full line and every plus line must be present as a full line.
pub(crate) fn verify_use_split(kind: &PatternKind, post_text: &str) -> Option<bool> {
    let (prefix, members, vis, minus_line, plus_lines) = match kind {
        PatternKind::UseSplit {
            prefix,
            members,
            vis,
            minus_line,
            plus_lines,
        } => (prefix, members, vis, minus_line, plus_lines),
        _ => return None,
    };
    let _ = (prefix, members);
    let minus_gone = !post_text.lines().any(|l| l.trim() == minus_line.trim());
    let plus_ok = plus_lines
        .iter()
        .all(|pl| post_text.lines().any(|l| l.trim() == pl.trim()));
    let _ = vis;
    Some(minus_gone && plus_ok && !minus_line.trim().is_empty() && !plus_lines.is_empty())
}

/// Verify a `UseRegroup` hunk in the post-image: the grouped line the
/// matcher wrote must be present and every minus line must be gone as a
/// full line. Merge+delete edits translate awkwardly to post spans (a
/// deleted line has no post coordinate), so the check is global, not
/// span-local — both halves fail on real input (a clobbered write, a
/// surviving minus).
pub(crate) fn verify_use_regroup(kind: &PatternKind, post_text: &str) -> Option<bool> {
    let (prefix, members, vis, plus_line) = match kind {
        PatternKind::UseRegroup {
            prefix,
            members,
            vis,
            plus_line,
        } => (prefix, members, vis, plus_line),
        _ => return None,
    };
    let needle = plus_line.trim();
    let vis_prefix = if vis.trim().is_empty() {
        String::new()
    } else {
        format!("{} ", vis.trim())
    };
    let plus_ok = !needle.is_empty() && post_text.contains(needle);
    let minus_gone = members.iter().all(|m| {
        let full = format!("{vis_prefix}use {}::{m};", prefix.join("::"));
        !post_text.lines().any(|l| l.trim() == full.trim())
    });
    Some(plus_ok && minus_gone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind() -> PatternKind {
        PatternKind::UseRegroup {
            prefix: vec!["a".to_string(), "b".to_string()],
            members: vec!["X".to_string(), "Y".to_string()],
            vis: String::new(),
            plus_line: "use a::b::{X, Y};".to_string(),
        }
    }

    #[test]
    fn merged_post_verifies() {
        assert_eq!(
            verify_use_regroup(&kind(), "use a::b::{X, Y};\nfn m() {}\n"),
            Some(true)
        );
    }

    #[test]
    fn surviving_minus_fails() {
        assert_eq!(
            verify_use_regroup(&kind(), "use a::b::{X, Y};\nuse a::b::X;\nfn m() {}\n"),
            Some(false)
        );
    }

    #[test]
    fn missing_group_fails() {
        assert_eq!(
            verify_use_regroup(&kind(), "use a::b::X;\nuse a::b::Y;\n"),
            Some(false)
        );
    }

    #[test]
    fn wrong_kind_returns_none() {
        assert_eq!(
            verify_use_regroup(
                &PatternKind::Raw {
                    minus: String::new(),
                    plus: String::new(),
                },
                "use a::b::{X, Y};\n"
            ),
            None
        );
    }

    fn split_kind() -> PatternKind {
        PatternKind::UseSplit {
            prefix: vec!["a".to_string(), "b".to_string()],
            members: vec!["X".to_string(), "Y".to_string()],
            vis: String::new(),
            minus_line: "use a::b::{X, Y};".to_string(),
            plus_lines: vec!["use a::b::X;".to_string(), "use a::b::Y;".to_string()],
        }
    }

    #[test]
    fn split_post_verifies() {
        assert_eq!(
            verify_use_split(&split_kind(), "use a::b::X;\nuse a::b::Y;\nfn m() {}\n"),
            Some(true)
        );
    }

    #[test]
    fn split_surviving_minus_fails() {
        assert_eq!(
            verify_use_split(
                &split_kind(),
                "use a::b::{X, Y};\nuse a::b::X;\nuse a::b::Y;\n"
            ),
            Some(false)
        );
    }

    #[test]
    fn split_missing_plus_fails() {
        assert_eq!(
            verify_use_split(&split_kind(), "use a::b::X;\nfn m() {}\n"),
            Some(false)
        );
    }
}
