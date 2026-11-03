#![allow(clippy::result_large_err)]
//! Rust pattern language (SPEC sections 13-16).
//!
//! Compiles raw RSmPL hunk pattern text into structured patterns.
//! Actual matching against target files lives in `rsmpl_engine`.

pub mod binder;
mod expr_pattern;
mod fn_pattern;
mod infer;
mod infer_call;
mod infer_control;
mod infer_def;
mod item_rename;
mod macro_bracket;
mod metavar;
mod pattern;
mod pattern_assoc;
mod pattern_classify;
mod pattern_classify_item;
mod pattern_def_evo;
mod pattern_ellipsis;
mod pattern_evolution;
mod pattern_evolution_container;
mod pattern_evolution_impl;
mod pattern_general;
mod pattern_item_joint;
mod pattern_member_evo;
mod quote_utils;
mod terse_fn;
mod use_rename;

pub use expr_pattern::{try_parse_expr, ArgPat, ExprPattern, ExprPatternKind, PlaceholderMaps};
pub use fn_pattern::{
    parse_fn_pattern, try_parse_fn_body_joint, FnBodyJoint, FnPattern, GenericPat, NamePat, RetPat,
    VisPat, WherePat,
};
pub use item_rename::{
    canonical_vis_prefix, parse_side as parse_item_rename_side, ItemKind, ItemRenameSide,
};
pub use macro_bracket::macro_bracket_args;
pub use metavar::{MetavarTable, TokenKind};
pub use pattern::{
    body_evolution_veto, compile, impl_block_matches, is_evolution_template, is_trait_header,
    parse_const_evolution_template, parse_enum_evolution_template, parse_evolution_template,
    parse_fn_body_evolution_template, parse_fn_full_header_template, parse_fn_sig_body_template,
    parse_impl_header, parse_mod_evolution_template, parse_static_evolution_template,
    parse_struct_evolution_template, parse_trait_evolution_template, parse_type_evolution_template,
    parse_union_evolution_template, rename_ident_in_code, split_leading_attrs, trait_header_ident,
    BodyVeto, CompiledPattern, GeneralShape, ImplHeaderFilter, PatternKind, VariantPayload,
};
pub use pattern_evolution_container::{
    container_body_evolution_template, item_body_evolution_veto,
    parse_enum_body_evolution_template, parse_struct_body_evolution_template,
    parse_trait_body_evolution_template, parse_union_body_evolution_template,
};
pub use pattern_evolution_impl::{
    impl_body_evolution_template, impl_body_evolution_veto, parse_impl_body_evolution_template,
};
pub use pattern_member_evo::norm_arm_expr;
pub use quote_utils::{
    char_col_to_byte, char_lit_len, count_ellipsis_markers, has_ellipsis_marker,
    is_ellipsis_marker_ident, mask_literal_spans, qualifier_leads_to_fn, string_end,
    strip_literal_quotes, unmask_literal_spans, validate_quoting, LiteralToken,
};
pub use terse_fn::is_terse_fn_line;
pub use use_rename::{
    classify_use_regroup, classify_use_rename, parse_use_prefix, UsePrefix, UseRegroup,
};
