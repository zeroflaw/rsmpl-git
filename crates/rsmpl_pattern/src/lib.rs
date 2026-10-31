#![allow(clippy::result_large_err)]
//! Rust pattern language (SPEC sections 13-16).
//!
//! Compiles raw RSmPL hunk pattern text into structured patterns.
//! Actual matching against target files lives in `rsmpl_engine`.

mod expr_pattern;
mod fn_pattern;
mod infer;
mod infer_control;
mod item_rename;
mod metavar;
mod pattern;
mod pattern_classify;
mod pattern_ellipsis;
mod pattern_evolution;
mod pattern_general;
mod quote_utils;
mod terse_fn;
mod use_rename;

pub use expr_pattern::{try_parse_expr, ArgPat, ExprPattern, ExprPatternKind, PlaceholderMaps};
pub use fn_pattern::{parse_fn_pattern, FnPattern, GenericPat, NamePat, RetPat, VisPat, WherePat};
pub use item_rename::{parse_side as parse_item_rename_side, ItemKind, ItemRenameSide};
pub use metavar::{MetavarTable, TokenKind};
pub use pattern::{
    compile, impl_block_matches, is_evolution_template, is_trait_header,
    parse_const_evolution_template, parse_enum_evolution_template, parse_evolution_template,
    parse_fn_full_header_template, parse_impl_header, parse_mod_evolution_template,
    parse_static_evolution_template, parse_struct_evolution_template,
    parse_trait_evolution_template, parse_type_evolution_template, parse_union_evolution_template,
    split_leading_attrs, trait_header_ident, CompiledPattern, GeneralShape, ImplHeaderFilter,
    PatternKind,
};
pub use quote_utils::{
    char_col_to_byte, char_lit_len, count_ellipsis_markers, has_ellipsis_marker,
    is_ellipsis_marker_ident, mask_literal_spans, qualifier_leads_to_fn, string_end,
    strip_literal_quotes, unmask_literal_spans, validate_quoting, LiteralToken,
};
pub use terse_fn::is_terse_fn_line;
pub use use_rename::{classify_use_rename, parse_use_prefix, UsePrefix};
