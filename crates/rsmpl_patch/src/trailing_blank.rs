//! T431: trailing blank-context repair for ordinary hunks.
//!
//! Raw `git diff -U3` emits ` ` as the last context line of a hunk.
//! `parse_body_lines` (T212 lookahead) drops it because the next line is
//! `@@`/`diff --git`, not a body line, and the top-level loop then skips
//! it as a separator — so T367 fails with a spurious count mismatch.
//!
//! The repair is count-aware: blanks are only consumed while both sides
//! are short of the header counts (a genuine separator has need==0) and
//! the blank run leads to a header/diff/EOF. RSmPL hunks are exempt (no
//! count validation; blank separators stay separators).
//!
//! Lives in its own module to keep `parser.rs` under the 2000-line
//! hygiene limit (`no_rust_file_exceeds_2000_lines`).

use crate::model::BodyLine;

/// Consume trailing blank-context lines at `*next` into `body` when the
/// header counts show they belong to this hunk (see module docs).
pub(crate) fn repair_trailing_blank_context(
    body: &mut Vec<BodyLine>,
    next: &mut usize,
    lines: &[&str],
    old_len: u32,
    new_len: u32,
) {
    let mut old_c = body
        .iter()
        .filter(|b| b.prefix == ' ' || b.prefix == '-')
        .count() as u32;
    let mut new_c = body
        .iter()
        .filter(|b| b.prefix == ' ' || b.prefix == '+')
        .count() as u32;
    while old_c < old_len && new_c < new_len && *next < lines.len() {
        // Never consume the split artifact: `text.split('\n')` on a
        // newline-terminated patch always leaves a final `""`, which
        // is not a body line (real blank context from git is `" "`).
        if *next + 1 == lines.len() && lines[*next].is_empty() {
            break;
        }
        let l = lines[*next];
        let blank = if l.is_empty() {
            Some(BodyLine {
                prefix: ' ',
                text: String::new(),
            })
        } else if l.trim().is_empty() {
            Some(BodyLine {
                prefix: ' ',
                text: l.strip_prefix(' ').unwrap_or_default().to_string(),
            })
        } else {
            None
        };
        let Some(bl) = blank else { break };
        let mut j = *next + 1;
        while j < lines.len() {
            let lj = lines[j];
            if !(lj.is_empty() || lj.trim().is_empty()) {
                break;
            }
            j += 1;
        }
        let follows_header =
            j >= lines.len() || lines[j].starts_with("diff --git ") || lines[j].starts_with("@@");
        if !follows_header {
            break;
        }
        if old_len == old_c || new_len == new_c {
            break;
        }
        body.push(bl);
        old_c += 1;
        new_c += 1;
        *next += 1;
    }
}

#[cfg(test)]
mod tests {
    use crate::model::Hunk;
    use crate::parser::parse_patch;

    #[test]
    fn trailing_blank_context_before_header_is_hunk_body() {
        // T431: a trailing blank context line before a `@@` header (or at
        // EOF) belongs to the hunk; the T212 lookahead drops it and T367
        // then fails with a spurious count mismatch on raw `-U3` output.
        for blank in [" ", ""] {
            let text = format!(
                "diff --git f.txt\n@@ -1,5 +1,5 @@\n L1\n-L2\n+L2m\n L3\n L4\n{blank}\n@@ -15,8 +15,8 @@\n L14\n L15\n L16\n L17\n-L18\n+L18m\n L19\n L20\n L21\n"
            );
            let p = parse_patch(&text).unwrap();
            assert_eq!(p.files[0].hunks.len(), 2, "blank={blank:?}");
            match &p.files[0].hunks[0] {
                Hunk::Ordinary(oh) => {
                    assert_eq!(oh.lines.len(), 6, "blank={blank:?}");
                    assert_eq!(oh.lines[5].prefix, ' ');
                    assert_eq!(oh.lines[5].text, "");
                }
                _ => panic!("expected ordinary"),
            }
            let eof =
                format!("diff --git f.txt\n@@ -1,5 +1,5 @@\n L1\n-L2\n+L2m\n L3\n L4\n{blank}\n");
            let p = parse_patch(&eof).unwrap();
            match &p.files[0].hunks[0] {
                Hunk::Ordinary(oh) => assert_eq!(oh.lines.len(), 6, "blank={blank:?}"),
                _ => panic!("expected ordinary"),
            }
        }
        // A genuine separator blank with satisfied counts stays a separator.
        let sep = "diff --git f.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n\n@@ -10,3 +10,3 @@\n x\n-y\n+Y\n z\n";
        let p = parse_patch(sep).unwrap();
        assert_eq!(p.files[0].hunks.len(), 2);
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => assert_eq!(oh.lines.len(), 4),
            _ => panic!("expected ordinary"),
        }
    }
}
