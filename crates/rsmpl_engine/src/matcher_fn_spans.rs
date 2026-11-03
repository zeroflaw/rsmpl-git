//! WI-22 fn-signature span axis (D5 split from `matcher_fn`):
//! byte-range slicers over the original text (`span_bytes`,
//! qualifier/params/generics/return/where ranges, one-space
//! swallowing). Re-exported through `matcher_fn`; no behavior change.

use crate::lines::LineIndex;
use syn::spanned::Spanned;

/// Byte range of `span` in the original text, or `None` when the span
/// falls outside the file (never guesses).
pub(crate) fn span_bytes(
    original: &str,
    index: &LineIndex,
    span: proc_macro2::Span,
) -> Option<(usize, usize)> {
    let s = span.start();
    let e = span.end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

/// Slice original text covered by `span`.
pub(crate) fn span_text(
    original: &str,
    index: &LineIndex,
    span: proc_macro2::Span,
) -> Option<String> {
    let (a, b) = span_bytes(original, index, span)?;
    original.get(a..b).map(|t| t.to_string())
}

/// Start line/column and end line/column of the qualifier token sequence
/// (`const`/`async`/`unsafe`/`extern ["ABI"]`), or `None` when the
/// signature has no qualifiers.
pub(crate) fn qualifier_span_ends(
    sig: &syn::Signature,
) -> Option<(proc_macro2::Span, proc_macro2::Span)> {
    let mut starts: Vec<proc_macro2::Span> = Vec::new();
    let mut ends: Vec<proc_macro2::Span> = Vec::new();
    if let Some(c) = &sig.constness {
        starts.push(c.span());
        ends.push(c.span());
    }
    if let Some(a) = &sig.asyncness {
        starts.push(a.span());
        ends.push(a.span());
    }
    if let Some(u) = &sig.unsafety {
        starts.push(u.span());
        ends.push(u.span());
    }
    if let Some(abi) = &sig.abi {
        starts.push(abi.span());
        ends.push(abi.span());
    }
    if starts.is_empty() {
        return None;
    }
    // Signatures store qualifiers in fixed order; first start / last end
    // covers the whole qualifier run.
    let first = starts.into_iter().next()?;
    let last = ends.into_iter().next_back()?;
    Some((first, last))
}

/// Byte range covering the qualifier run, or `None` when absent.
pub(crate) fn qualifier_bytes(
    original: &str,
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    let (first, last) = qualifier_span_ends(sig)?;
    let s = first.start();
    let e = last.end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

/// Byte range inside the parameter parentheses (empty for `()`).
pub(crate) fn params_inner_bytes(
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    let open_end = sig.paren_token.span.open().end();
    let close_start = sig.paren_token.span.close().start();
    let a = index.offset(open_end.line as u32, open_end.column)?;
    let b = index.offset(close_start.line as u32, close_start.column)?;
    if a <= b {
        Some((a, b))
    } else {
        None
    }
}

/// Byte range of `<...>` (inclusive), or `None` when no generics.
pub(crate) fn generics_bytes(
    original: &str,
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    let (lt, gt) = (
        sig.generics.lt_token.as_ref()?,
        sig.generics.gt_token.as_ref()?,
    );
    let s = lt.span().start();
    let e = gt.span().end();
    let a = index.offset(s.line as u32, s.column)?;
    let b = index.offset(e.line as u32, e.column)?;
    if a <= b && b <= original.len() {
        Some((a, b))
    } else {
        None
    }
}

/// Byte range of `-> Ty` (inclusive), or `None` when no return type.
pub(crate) fn return_bytes(
    original: &str,
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    match &sig.output {
        syn::ReturnType::Type(arrow, ty) => {
            let s = arrow.span().start();
            let e = ty.span().end();
            let a = index.offset(s.line as u32, s.column)?;
            let b = index.offset(e.line as u32, e.column)?;
            if a <= b && b <= original.len() {
                Some((a, b))
            } else {
                None
            }
        }
        syn::ReturnType::Default => None,
    }
}

/// Byte range of the `where ...` clause, or `None` when absent.
pub(crate) fn where_bytes(
    original: &str,
    index: &LineIndex,
    sig: &syn::Signature,
) -> Option<(usize, usize)> {
    let w = sig.generics.where_clause.as_ref()?;
    span_bytes(original, index, w.span())
}

/// Extend `end` over one following ASCII space so slot deletions do not
/// leave doubled spacing (`fn foo()  {`).
pub(crate) fn swallow_one_space(original: &str, end: usize) -> usize {
    if original.as_bytes().get(end) == Some(&b' ') {
        end + 1
    } else {
        end
    }
}
