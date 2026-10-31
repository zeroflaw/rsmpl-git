//! WI-1a: `# sweep` fused header (SPEC 9.3).
//!
//! Bare `# sweep` is exactly `# allow=many` + `# scope=file`, expanded
//! in place during directive parsing so a later explicit directive wins.

use crate::errors::{ErrorCode, StructuredError};
use crate::model::{Allow, Directive, Scope};
use crate::parser::ParseError;

/// True for a bare `# sweep` directive line (surrounding whitespace tolerated).
pub(crate) fn is_sweep_line(line: &str) -> bool {
    matches!(line.strip_prefix('#').map(str::trim), Some("sweep"))
}

/// Expansion of one `# sweep` line: `[Allow(Many), Scope(File)]`.
pub(crate) fn expansion() -> [Directive; 2] {
    [Directive::Allow(Allow::Many), Directive::Scope(Scope::File)]
}

/// Reject `# sweep=<value>` (mirrors the bare-`# repeat` precedent).
pub(crate) fn reject_value(v: &str, line: &str) -> Result<Directive, ParseError> {
    Err(StructuredError::new(
        ErrorCode::PatternConflict,
        format!("invalid sweep value `{v}` (write bare `# sweep`): {line}"),
    )
    .with_suggestion("write bare `# sweep` with no value".to_string())
    .into())
}

#[cfg(test)]
mod tests {
    use super::is_sweep_line;
    use crate::model::{Allow, Directive, Hunk, Scope};
    use crate::parser::{parse_directives, parse_patch};

    #[test]
    fn parses_sweep_flag() {
        // WI-1a: bare `# sweep` expands to `# allow=many` + `# scope=file`.
        assert!(is_sweep_line("# sweep"));
        assert!(!is_sweep_line("# sweep=true"));
        let d = parse_directives(&["# sweep".to_string()]).unwrap();
        assert_eq!(d.len(), 2);
        assert!(matches!(d[0], Directive::Allow(Allow::Many)));
        assert!(matches!(d[1], Directive::Scope(Scope::File)));
        assert!(parse_directives(&["# sweep=true".to_string()]).is_err());
    }

    #[test]
    fn sweep_is_many_plus_file_scope() {
        // WI-1a: end-to-end through parse_patch.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# expr E\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => {
                assert_eq!(rh.allow(), Allow::Many);
                assert_eq!(rh.scope(), Scope::File);
            }
            _ => panic!("expected rsmpl hunk"),
        }
    }

    #[test]
    fn sweep_yields_to_later_explicit_allow() {
        // WI-1a: expansion is positional — a later explicit allow wins.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# allow=one\n# expr E\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => {
                assert_eq!(rh.allow(), Allow::One);
                assert_eq!(rh.scope(), Scope::File);
            }
            _ => panic!("expected rsmpl hunk"),
        }
    }

    #[test]
    fn sweep_with_bind_parses_as_collection() {
        // WI-6: `# sweep` implies allow=many, so `# bind=` on an item cut
        // declares a whole-file collection (the kind gate lives in the
        // engine, not the parser).
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=cut\n# sweep\n# item I\n# bind=H\n- I\n";
        let p = parse_patch(text).expect("bind+many must parse");
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => {
                assert_eq!(rh.allow(), Allow::Many);
                assert_eq!(rh.binds(), vec!["H"]);
            }
            _ => panic!("expected rsmpl hunk"),
        }
    }
}
