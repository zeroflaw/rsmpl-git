#![allow(clippy::result_large_err)]
//! RSmPL-Git patch document model and parser.
//!
//! Implements SPEC sections 5-11 (patch structure, hunk detection,
//! header syntax, directive lines, range semantics).

mod errors;
mod model;
mod parser;

pub use errors::{ErrorCode, StructuredError};
pub use model::*;
pub use parser::{
    collapse_git_header, decode_git_path_token, parse_patch, split_git_paths, ParseError,
};
