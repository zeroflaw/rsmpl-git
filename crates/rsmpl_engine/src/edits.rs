//! Byte-range text edits over original coordinates.

use rsmpl_patch::{ErrorCode, StructuredError};

/// A replacement of `original[start..end]` with `replacement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

impl TextEdit {
    pub fn new(start: usize, end: usize, replacement: String) -> Self {
        debug_assert!(start <= end);
        Self {
            start,
            end,
            replacement,
        }
    }

    pub fn is_insertion(&self) -> bool {
        self.start == self.end
    }

    pub fn overlaps(&self, other: &TextEdit) -> bool {
        // Insertions overlap only if strictly inside another edit's range.
        if self.is_insertion() && other.is_insertion() {
            return false; // same-point insertions are tie-broken by order
        }
        if self.is_insertion() {
            return other.start < self.start && self.start < other.end;
        }
        if other.is_insertion() {
            return self.start < other.start && other.start < self.end;
        }
        self.start < other.end && other.start < self.end
    }

    pub fn contains(&self, other: &TextEdit) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

/// Apply edits (original coordinates) to `text`. Edits are sorted descending
/// so earlier offsets stay valid. Overlapping non-nested edits are rejected.
pub fn apply_edits(text: &str, mut edits: Vec<TextEdit>) -> Result<String, StructuredError> {
    edits.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
    let mut out = text.to_string();
    let mut applied: Vec<TextEdit> = Vec::new();
    for e in edits {
        if e.end > out.len() || e.start > e.end {
            return Err(StructuredError::new(
                ErrorCode::RangeConflict,
                format!("edit range {}..{} outside file", e.start, e.end),
            ));
        }
        for prev in &applied {
            // `applied` holds edits at higher offsets already applied; `e`
            // is at a lower offset and cannot overlap them unless ranges
            // cross. A crossing check on original coordinates:
            if e.start < prev.start && prev.start < e.end {
                return Err(StructuredError::new(
                    ErrorCode::GitHunkConflict,
                    "overlapping edits cannot be merged",
                ));
            }
        }
        out.replace_range(e.start..e.end, &e.replacement);
        applied.push(e);
    }
    Ok(out)
}

/// Token-normalized equality for code fragments: parse both as expressions
/// (fallback: types, then whitespace-squashed strings) and compare the
/// `quote` rendering. Implements repeated-metavar structural equality
/// (SPEC 15.7) and trailing-comma insensitivity (SPEC 17.7).
pub fn fragments_equal(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(ea), Ok(eb)) = (
        rsmpl_pattern::try_parse_expr(a),
        rsmpl_pattern::try_parse_expr(b),
    ) {
        return norm(&quote::quote!(#ea).to_string()) == norm(&quote::quote!(#eb).to_string());
    }
    if let (Ok(ta), Ok(tb)) = (
        syn::parse_str::<syn::Type>(a),
        syn::parse_str::<syn::Type>(b),
    ) {
        return norm(&quote::quote!(#ta).to_string()) == norm(&quote::quote!(#tb).to_string());
    }
    norm(a) == norm(b)
}

/// Normalize rendered code: drop whitespace and insignificant trailing commas
/// (`(a, b,)` == `(a, b)`, SPEC 17.7).
fn norm(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    s.replace(",)", ")").replace(",]", "]").replace(",}", "}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_descending() {
        let out = apply_edits(
            "aaa bbb",
            vec![
                TextEdit::new(0, 3, "x".into()),
                TextEdit::new(4, 7, "y".into()),
            ],
        )
        .unwrap();
        assert_eq!(out, "x y");
    }

    #[test]
    fn trailing_comma_insensitive() {
        assert!(fragments_equal("foo(1, 2)", "foo(1, 2,)"));
        assert!(!fragments_equal("foo()", "bar()"));
    }
}
