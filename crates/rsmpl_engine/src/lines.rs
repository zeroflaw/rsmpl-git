//! Original-source line/byte mapping (SPEC 10.2, 31.3).

/// 1-based line index over the original file text.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset at which each 1-based line starts. `starts[0]` == 0.
    starts: Vec<usize>,
    len_lines: usize,
    text_len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        // A trailing newline creates an empty final entry; drop it so
        // `line_count` equals the number of real lines.
        let mut len_lines = starts.len();
        if text.ends_with('\n') {
            len_lines -= 1;
        }
        Self {
            starts,
            len_lines,
            text_len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.len_lines
    }

    /// Byte offset of the start of 1-based `line`. One past the last line
    /// maps to EOF (insertion anchors, SPEC 10.5).
    pub fn line_start(&self, line: u32) -> Option<usize> {
        if line == 0 {
            return None;
        }
        let i = (line - 1) as usize;
        if i < self.starts.len() {
            Some(self.starts[i])
        } else if i == self.starts.len() {
            Some(self.text_len)
        } else {
            None
        }
    }

    /// Byte offset of (1-based line, 0-based byte column).
    /// Columns come from `proc-macro2` spans (byte-based). Values past the
    /// line end are clamped to the line end to stay total.
    pub fn offset(&self, line: u32, col: usize) -> Option<usize> {
        let start = self.line_start(line)?;
        let end = if (line as usize) < self.starts.len() {
            self.starts[line as usize].saturating_sub(1) // exclude '\n'
        } else {
            usize::MAX
        };
        let off = start.saturating_add(col);
        Some(off.min(end.min(self.text_len)))
    }

    pub fn eof(&self) -> usize {
        self.text_len
    }

    /// 1-based line containing byte `offset` (original coordinates).
    /// Offsets past EOF clamp to the last line; offset 0 is line 1.
    pub fn line_of(&self, offset: usize) -> u32 {
        let offset = offset.min(self.text_len);
        let mut lo = 0usize;
        let mut hi = self.starts.len();
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if self.starts[mid] <= offset {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo as u32) + 1
    }

    /// EOF byte length of the underlying text is not stored; callers needing
    /// exact EOF should use `text.len()`. This returns the last line start.
    pub fn last_line_start(&self) -> usize {
        *self.starts.last().unwrap_or(&0)
    }
}

/// Parsed original state for one file.
///
/// `parsed` is `None` for ordinary-only files (T137): a file touched by
/// no RSmPL hunk is plain text — no Rust parse of the original or the
/// rewritten file. Any file with >=1 RSmPL hunk keeps `Some` and the
/// current strict Rust behavior.
pub(crate) struct FileState {
    pub(crate) original: String,
    pub(crate) index: LineIndex,
    pub(crate) parsed: Option<syn::File>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lines() {
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.line_count(), 3);
        assert_eq!(idx.line_start(1), Some(0));
        assert_eq!(idx.line_start(2), Some(3));
        assert_eq!(idx.line_start(3), Some(6));
        assert_eq!(idx.line_start(4), Some(8)); // EOF anchor
        assert_eq!(idx.offset(2, 1), Some(4));
        let idx2 = LineIndex::new("ab\n");
        assert_eq!(idx2.line_count(), 1);
        assert_eq!(idx2.line_start(2), Some(3)); // EOF anchor
        assert_eq!(idx2.line_start(5), None);
    }

    #[test]
    fn line_of_maps_offsets() {
        // T27: byte offsets map back to 1-based lines; EOF clamps.
        let idx = LineIndex::new("ab\ncd\nef");
        assert_eq!(idx.line_of(0), 1);
        assert_eq!(idx.line_of(2), 1);
        assert_eq!(idx.line_of(3), 2); // line start: next line
        assert_eq!(idx.line_of(4), 2);
        assert_eq!(idx.line_of(6), 3);
        assert_eq!(idx.line_of(8), 3); // EOF clamps to last line
        assert_eq!(idx.line_of(999), 3);
    }
}
