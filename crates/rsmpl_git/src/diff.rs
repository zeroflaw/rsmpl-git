//! Concrete unified-diff generation (SPEC 26.1-26.2).
//!
//! The lowered patch must contain no RSmPL directives (guaranteed: it is
//! generated from materialized file text, never from the input patch).

/// Build a `diff --git` section for one file, or `None` when unchanged.
pub fn unified_diff_for_file(path: &str, old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!("diff --git a/{path} b/{path}\n"));
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));
    let diff = similar::TextDiff::from_lines(old, new);
    // Portable `@@ -a,b +c,d @@` hunks with 3 context lines (unified format).
    // `---`/`+++` headers are emitted above (a/ b/ style, git-compatible).
    let hunks = diff.unified_diff().context_radius(3).to_string();
    out.push_str(&hunks);
    Some(out)
}

/// Assemble per-file sections into one patch document.
pub fn assemble_patch(sections: Vec<String>) -> String {
    let mut out = String::new();
    for s in sections {
        out.push_str(&s);
        if !s.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_git_sections_without_directives() {
        let d = unified_diff_for_file("src/lib.rs", "fn foo() {}\n", "fn bar() {}\n").unwrap();
        assert!(d.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        assert!(d.contains("--- a/src/lib.rs"));
        assert!(d.contains("+++ b/src/lib.rs"));
        assert!(d.contains("@@"));
        assert!(!d.contains("# lang=rust"));
    }

    #[test]
    fn unchanged_returns_none() {
        assert!(unified_diff_for_file("x", "a\n", "a\n").is_none());
    }
}
