#![allow(clippy::result_large_err)]
//! RSmPL-Git patch document model and parser.
//!
//! Implements SPEC sections 5-11 (patch structure, hunk detection,
//! header syntax, directive lines, range semantics).

mod canonical;
mod diet_detect;
mod directives;
mod errors;
mod gitpaths;
mod model;
mod parser;
mod sweep;
mod trailing_blank;
mod validate;

pub use canonical::{
    canonicalize, canonicalize_with_spans, canonicalize_with_spans_parsed, TIGHT_RANGE_MAX_LINES,
};
pub use directives::{closest_within, edit_distance, suggest_directive_key, suggest_value};
pub use errors::{ErrorCode, StructuredError};
pub use model::*;
pub use parser::{
    collapse_git_header, decode_git_path_token, parse_patch, quote_git_path, split_git_paths,
    ParseError,
};

/// WI-20: single source of truth for the JSON report schema version
/// (SPEC D.2). `applyrs`, `diffrs`, and the `gitrs` dispatcher all take
/// the number from here — never a per-binary literal — so the `gitrs`
/// version-skew handshake transitively pins schema parity. v4: the
/// WI-2 `follow_skipped` prose string is removed (structured
/// `follow_skipped_details` carry the signal); everything else is
/// additive over v3.
pub const REPORT_VERSION: u32 = 4;
