//! Ordinary unified-diff hunk application over original coordinates.
//!
//! NOTE (T5): this will move to `rsmpl_git` once the CLI/lowering crate is
//! built. It lives here temporarily so the engine can plan ordinary and
//! semantic edits in one coordinate system (SPEC 10.6, 23.5).

use rsmpl_patch::{ErrorCode, OrdinaryHunk, StructuredError};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// Convert an ordinary hunk into byte-range edits against the original file.
/// Context and removal lines are strictly verified at `old_start`.
pub fn ordinary_hunk_edits(
    index: &LineIndex,
    original: &str,
    hunk: &OrdinaryHunk,
) -> Result<Vec<TextEdit>, StructuredError> {
    let rule = hunk.header.rule.clone().unwrap_or_default();
    let fail = |msg: String| {
        StructuredError::new(ErrorCode::GitHunkConflict, msg)
            .with_rule(&rule)
            .with_range(format!(
                "{}..{}",
                hunk.header.old_start,
                hunk.header.old_start + hunk.header.old_len
            ))
    };
    let lines: Vec<&str> = original.split('\n').collect();
    // `split('\n')` on trailing-newline text yields a final ""; hunk line
    // numbers address real lines only.
    let n = if original.ends_with('\n') {
        lines.len() - 1
    } else {
        lines.len()
    };

    let mut edits = Vec::new();
    // Byte cursor over original; line cursor is 1-based.
    let mut cur_line: u32 = if hunk.header.old_len == 0 {
        hunk.header.old_start + 1
    } else {
        hunk.header.old_start
    };
    // Pending deletion span: consecutive `-` lines merged into one edit.
    let mut del_start: Option<usize> = None;
    let mut del_end: usize = 0;
    let mut pending_insert = String::new();

    let flush = |edits: &mut Vec<TextEdit>,
                 del_start: &mut Option<usize>,
                 del_end: &mut usize,
                 pending_insert: &mut String| {
        if del_start.is_some() || !pending_insert.is_empty() {
            edits.push(TextEdit::new(
                del_start.unwrap_or(*del_end),
                *del_end,
                std::mem::take(pending_insert),
            ));
            *del_start = None;
        }
    };

    let mut i = 0;
    let body = &hunk.lines;
    while i < body.len() {
        let bl = &body[i];
        match bl.prefix {
            ' ' => {
                flush(
                    &mut edits,
                    &mut del_start,
                    &mut del_end,
                    &mut pending_insert,
                );
                if cur_line == 0 || cur_line as usize > n {
                    return Err(fail(format!("context line beyond EOF (line {cur_line})")));
                }
                let actual = lines[(cur_line - 1) as usize];
                if actual != bl.text {
                    return Err(fail(format!(
                        "context mismatch at line {cur_line}: expected `{}`",
                        bl.text
                    )));
                }
                cur_line += 1;
            }
            '-' => {
                // A `-` line after a pure insertion: emit the insertion first.
                if del_start.is_none() && !pending_insert.is_empty() {
                    edits.push(TextEdit::new(
                        del_end,
                        del_end,
                        std::mem::take(&mut pending_insert),
                    ));
                }
                if cur_line == 0 || cur_line as usize > n {
                    return Err(fail(format!("removal beyond EOF (line {cur_line})")));
                }
                let actual = lines[(cur_line - 1) as usize];
                if actual != bl.text {
                    return Err(fail(format!(
                        "removal mismatch at line {cur_line}: expected `{}`",
                        bl.text
                    )));
                }
                let ls = index
                    .line_start(cur_line)
                    .ok_or_else(|| fail("bad range".into()))?;
                let le = index.line_start(cur_line + 1).unwrap_or(original.len());
                if del_start.is_none() {
                    del_start = Some(ls);
                }
                del_end = le;
                cur_line += 1;
            }
            '+' => {
                if del_start.is_none() {
                    // Pure insertion at the cursor line.
                    let pos = if cur_line == 0 {
                        0
                    } else {
                        index.line_start(cur_line).unwrap_or(original.len())
                    };
                    del_end = pos;
                }
                pending_insert.push_str(&bl.text);
                pending_insert.push('\n');
            }
            '\\' => {
                // `\ No newline at end of file`: tolerated; the missing
                // trailing newline is preserved as-is by the merger.
            }
            _ => return Err(fail(format!("invalid diff prefix `{}`", bl.prefix))),
        }
        i += 1;
    }
    // `\ No newline at end of file` on the new side (T355): a trailing
    // marker after insertions means the new content ends without `\n`
    // (symlink targets are newline-free link strings). The `+` arm
    // above always appends `\n`, so strip the one it added. Pending
    // content here implies the hunk tail is insertions (context lines
    // flush), so the marker annotates new-side content.
    if body.last().is_some_and(|bl| bl.prefix == '\\') && pending_insert.ends_with('\n') {
        pending_insert.pop();
    }
    flush(
        &mut edits,
        &mut del_start,
        &mut del_end,
        &mut pending_insert,
    );
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_patch::parse_patch;
    use rsmpl_patch::{BodyLine, Hunk, HunkHeader};

    fn ordinary_of(patch: &str) -> OrdinaryHunk {
        let p = parse_patch(patch).unwrap();
        match p.files[0].hunks[0].clone() {
            Hunk::Ordinary(o) => o,
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn applies_simple_hunk() {
        let src = "mod old;\n\nfn main() {}\n";
        let h = ordinary_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n mod old;\n+mod new;\n\n fn main() {}\n",
        );
        let idx = LineIndex::new(src);
        let edits = ordinary_hunk_edits(&idx, src, &h).unwrap();
        let out = crate::edits::apply_edits(src, edits).unwrap();
        assert!(out.contains("mod new;"));
    }

    #[test]
    fn rejects_context_mismatch() {
        let src = "mod other;\n\nfn main() {}\n";
        let h = ordinary_of(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n mod old;\n+mod new;\n\n fn main() {}\n",
        );
        let idx = LineIndex::new(src);
        assert!(ordinary_hunk_edits(&idx, src, &h).is_err());
    }

    #[test]
    fn preserves_missing_trailing_newline_on_create() {
        // T355: symlink creates are newline-free link strings; the
        // `\ No newline` marker must survive instead of gaining `\n`.
        let src = "";
        let h = ordinary_of(
            "diff --git a/link b/link\n--- /dev/null\n+++ b/link\n@@ -0,0 +1,1 @@\n+real.txt\n\\ No newline at end of file\n",
        );
        let idx = LineIndex::new(src);
        let edits = ordinary_hunk_edits(&idx, src, &h).unwrap();
        let out = crate::edits::apply_edits(src, edits).unwrap();
        assert_eq!(out, "real.txt");
    }

    #[test]
    fn preserves_missing_trailing_newline_on_retarget() {
        // T355: retargeting a symlink must not append `\n` to the target.
        let src = "real.txt";
        let h = ordinary_of(
            "diff --git a/link b/link\n--- a/link\n+++ b/link\n@@ -1,1 +1,1 @@\n-real.txt\n\\ No newline at end of file\n+other.txt\n\\ No newline at end of file\n",
        );
        let idx = LineIndex::new(src);
        let edits = ordinary_hunk_edits(&idx, src, &h).unwrap();
        let out = crate::edits::apply_edits(src, edits).unwrap();
        assert_eq!(out, "other.txt");
    }

    #[test]
    fn context_line_beyond_eof_reason() {
        // T367: truthful ordinary headers always fit the file (Pass 0
        // range check), so the beyond-EOF arms are unreachable via the
        // CLI parser; pin the engine-verbatim reasons here with a
        // hand-built hunk that skips parse-time count validation.
        let h = OrdinaryHunk {
            header: HunkHeader {
                old_start: 1,
                old_len: 1,
                new_start: 1,
                new_len: 2,
                new_elided: false,
                rule: None,
                raw_tail: String::new(),
            },
            lines: vec![
                BodyLine {
                    prefix: ' ',
                    text: "fn main() {}".to_string(),
                },
                BodyLine {
                    prefix: ' ',
                    text: "EXTRA-CTX".to_string(),
                },
            ],
        };
        let src = "fn main() {}\n";
        let idx = LineIndex::new(src);
        let err = ordinary_hunk_edits(&idx, src, &h).unwrap_err();
        assert_eq!(err.code, ErrorCode::GitHunkConflict);
        assert_eq!(err.message, "context line beyond EOF (line 2)");
    }

    #[test]
    fn removal_beyond_eof_reason() {
        // Same T367 note as above: engine-level pin.
        let h = OrdinaryHunk {
            header: HunkHeader {
                old_start: 1,
                old_len: 1,
                new_start: 1,
                new_len: 0,
                new_elided: false,
                rule: None,
                raw_tail: String::new(),
            },
            lines: vec![
                BodyLine {
                    prefix: '-',
                    text: "fn main() {}".to_string(),
                },
                BodyLine {
                    prefix: '-',
                    text: "EXTRA-DEL".to_string(),
                },
            ],
        };
        let src = "fn main() {}\n";
        let idx = LineIndex::new(src);
        let err = ordinary_hunk_edits(&idx, src, &h).unwrap_err();
        assert_eq!(err.code, ErrorCode::GitHunkConflict);
        assert_eq!(err.message, "removal beyond EOF (line 2)");
    }
}
