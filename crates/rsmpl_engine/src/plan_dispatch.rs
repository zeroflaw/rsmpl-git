//! Single-hunk dispatch + range helper (SPEC 23.1 step 7).
//!
//! D5 sibling of `plan.rs`: the pass-2 loop and the T62/drift probes share
//! this dispatch so predicates cannot drift. No planner state lives here.

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};
use rsmpl_pattern::PatternKind;

use crate::capture::{paste_capture, CaptureStore, PasteTarget};
use crate::edits::TextEdit;
use crate::lines::FileState;
use crate::{
    matcher_assoc, matcher_expr, matcher_fn, matcher_general_apply, matcher_impl, matcher_item,
    matcher_item_joint, matcher_macro, matcher_member, matcher_member_del, matcher_member_evo,
    matcher_use, matcher_value,
};

/// Dispatch one pass-2 RSmPL hunk to its matcher (SPEC 23.1 step 7).
///
/// Extracted from the pass-2 loop so the T62 whole-file probe reuses the
/// exact same dispatch (no predicate duplication, no drift). File context
/// (`with_file`) is applied by the caller, not here, so real and probe
/// runs share error shapes.
pub(crate) fn apply_rsmpl_hunk(
    state: &FileState,
    path: &str,
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    captures: &CaptureStore,
    edits: &mut Vec<TextEdit>,
    symbol_graph: Option<&crate::CrateGraph>,
) -> Result<(usize, Vec<HashMap<String, String>>), StructuredError> {
    let rule = rh.rule_name();
    let parsed = state.parsed.as_ref().expect(
        "T137: RSmPL hunk on a file with no parsed AST (ordinary-only files carry no semantic hunks)",
    );
    // WI-2b: shapes without metavariables report empty per-site maps; call /
    // general-expr / macro shapes thread their per-site maps through.
    let empty = || HashMap::new();
    match &compiled.kind {
        // WI-6b1: definition matching only — `# follow=uses` runs as a
        // workspace step in the pass-2 loop (needs every file's state).
        PatternKind::Fn { .. } => matcher_fn::apply_fn_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::Expr {
            minus: rsmpl_pattern::ExprPatternKind::Macro { .. },
            ..
        } => matcher_macro::apply_macro_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Expr { .. } => matcher_expr::apply_expr_hunk_with_graph(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
            symbol_graph.map(|g| (g, path)),
        ),
        PatternKind::ItemRename { .. } => matcher_item::apply_item_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::FieldRename { .. } => matcher_member::apply_field_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::VariantRename { .. } => matcher_member::apply_variant_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::AssocRename { .. } => matcher_assoc::apply_assoc_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::AssocEvolution { .. } => matcher_assoc::apply_assoc_evolution_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::TypeAliasEvolution { .. } => matcher_value::apply_type_alias_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ConstEvolution { .. } => matcher_value::apply_const_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::FieldTypeEvolution { .. } => {
            matcher_member_evo::apply_field_type_hunk(
                parsed,
                &state.original,
                &state.index,
                rh,
                compiled,
                edits,
            )
        }
        PatternKind::StructRenameFieldEvo { .. } => {
            matcher_item_joint::apply_struct_rename_field_evo_hunk(
                parsed,
                &state.original,
                &state.index,
                rh,
                compiled,
                edits,
            )
        }
        PatternKind::EnumRenameVariantEvo { .. } => {
            matcher_item_joint::apply_enum_rename_variant_evo_hunk(
                parsed,
                &state.original,
                &state.index,
                rh,
                compiled,
                edits,
            )
        }
        PatternKind::VariantPayloadEvolution { .. } => {
            matcher_member_evo::apply_variant_payload_hunk(
                parsed,
                &state.original,
                &state.index,
                rh,
                compiled,
                edits,
            )
        }
        PatternKind::FieldAddition { .. } => matcher_member_evo::apply_field_addition_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::FieldDeletion { .. } => matcher_member_del::apply_field_deletion_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::VariantDeletion { .. } => matcher_member_del::apply_variant_deletion_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::VariantAddition { .. } => matcher_member_evo::apply_variant_addition_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ArmAddition { .. } => matcher_member_evo::apply_match_arm_addition_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::UseRename { .. } => matcher_use::apply_use_rename_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::UseRegroup { .. } => matcher_use::apply_use_regroup_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::General { .. } => matcher_general_apply::apply_general_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::ItemCapture { .. } => matcher_fn::apply_item_delete(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::Insertion { text, use_name } => match use_name {
            Some(name) => {
                paste_capture(
                    captures,
                    PasteTarget {
                        name,
                        template: Some(text),
                        anchor: rh.header.old_start,
                    },
                    &state.index,
                    &rule,
                    path,
                    edits,
                )?;
                Ok((1, vec![empty()]))
            }
            None => {
                matcher_fn::apply_insertion(&state.original, &state.index, rh, compiled, edits)
                    .map(|n| (n, vec![empty(); n]))
            }
        },
        PatternKind::ImplContext { .. } => matcher_impl::apply_impl_context_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        )
        .map(|n| (n, vec![empty(); n])),
        PatternKind::Macro { .. } => matcher_macro::apply_macro_hunk(
            parsed,
            &state.original,
            &state.index,
            rh,
            compiled,
            edits,
        ),
        PatternKind::Raw { .. } => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "unsupported pattern shape in v0.1 (only fn rename, struct/enum/trait/union/type/const/static/mod/impl item rename, type-alias/const-static evolution, `use`-path prefix rename, call rewrite, let-stmt/general-expr/type rewrite, item delete/insert, macro path rewrite)",
        )
        .with_rule(rule)
        .with_repair(StructuredError::fix_pattern_repair(
            "plan-dispatch-raw-template",
            "write a supported shape (fn/item rename, call/let/expr/type rewrite, evolution pair, `use` prefix, delete/insert, macro path; see SPEC 18.6) or use an ordinary hunk",
        ))),
    }
}

/// WI-10b drift retry + T62 probe live in `drift.rs` (D5 sibling); the
/// planner calls back in via `apply_rsmpl_hunk` above.
pub(crate) fn range_err(rule: &str, file: &str, line: u32) -> StructuredError {
    StructuredError::new(
        ErrorCode::RangeConflict,
        format!("span line {line} outside file"),
    )
    .with_rule(rule)
    .with_file(file)
    .with_repair(StructuredError::internal_repair())
}

#[cfg(test)]
mod wi18_slice12_tests {
    // WI-18 slice 12: the `Raw` dispatch arm carries the site-specific
    // `plan-dispatch-raw-template` repair. The pin runs the real route
    // (an empty `-`/`+` pair compiles to `Raw` naturally, no overwrite —
    // the `unsupported_shape_empty_pattern_verbatim` e2e shape).
    #[test]
    fn wi18_slice12_raw_dispatch_repair_is_specific() {
        let patch_text =
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# expr E\n-\n+\n";
        let patch = rsmpl_patch::parse_patch(patch_text).expect("empty patch parses");
        let mut files = std::collections::HashMap::new();
        files.insert("src/lib.rs".to_string(), "fn main() {}\n".to_string());
        let mut patch = patch;
        for f in &mut patch.files {
            f.target_path = "src/lib.rs".to_string();
        }
        let err = crate::apply_patch_to_tree(&mut files, &patch).expect_err("Raw must fail");
        assert_eq!(err.code, rsmpl_patch::ErrorCode::PatternConflict);
        assert!(
            err.message.starts_with("unsupported pattern shape in v0.1"),
            "got: {}",
            err.message
        );
        let repair = err.repair.expect("WI-18 slice 12: must carry repair");
        assert_eq!(
            repair.get("code").and_then(|v| v.as_str()),
            Some("plan-dispatch-raw-template")
        );
        let write_instead = repair
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str");
        assert_ne!(write_instead, err.message);
        assert!(write_instead.contains("SPEC 18.6"), "got: {write_instead}");
    }
}
