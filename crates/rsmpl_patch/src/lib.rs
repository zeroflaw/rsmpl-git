#![allow(clippy::result_large_err)]
//! RSmPL-Git patch document model and parser.
//!
//! Implements SPEC sections 5-11 (patch structure, hunk detection,
//! header syntax, directive lines, range semantics).

mod canonical;
mod diet_detect;
mod directives;
mod errors;
mod model;
mod parser;
mod sweep;
mod trailing_blank;

pub use canonical::{canonicalize, canonicalize_with_spans, TIGHT_RANGE_MAX_LINES};
pub use errors::{ErrorCode, StructuredError};
pub use model::*;
pub use parser::{
    collapse_git_header, decode_git_path_token, parse_patch, quote_git_path, split_git_paths,
    ParseError,
};
