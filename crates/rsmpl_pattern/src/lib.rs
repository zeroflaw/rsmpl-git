//! Rust pattern language (SPEC sections 13-16).
//!
//! Compiles raw RSmPL hunk pattern text into structured patterns.
//! Actual matching against target files lives in `rsmpl_engine`.

mod expr_pattern;
mod fn_pattern;
mod metavar;
mod pattern;
mod quote_utils;

pub use expr_pattern::{ArgPat, ExprPattern, ExprPatternKind, PlaceholderMaps};
pub use fn_pattern::{parse_fn_pattern, FnPattern, GenericPat, NamePat, RetPat, VisPat, WherePat};
pub use metavar::{MetavarTable, TokenKind};
pub use pattern::{compile, CompiledPattern, PatternKind};
pub use quote_utils::{strip_literal_quotes, validate_quoting, LiteralToken};
