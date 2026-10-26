//! Git lowering and application (SPEC section 26).
//!
//! After the engine has materialized the modified tree in memory, this crate
//! turns old/new file contents into a concrete ordinary Git patch and
//! applies it with `git apply`. The lowered patch contains no RSmPL
//! directives and is acceptable to vanilla Git tooling.

#![allow(clippy::result_large_err)]

mod apply;
mod diff;

pub use apply::{apply_with_git, check_with_git};
pub use diff::{
    assemble_patch, file_line_stats, unified_diff_for_file_with_context,
    unified_diff_for_new_file_with_context,
};
