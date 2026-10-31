//! WI-16 crate-symbol helpers for the planner (SPEC 19).
//!
//! Split from `plan.rs` per the D5 size gate: symbol-workspace detection,
//! pre-image graph construction, and the per-rule JSON `symbol` mode.
//! No matching logic lives here (see `crate_graph.rs`, `matcher_expr`).

use std::collections::HashMap;

use rsmpl_patch::{Hunk, Patch};

use crate::lines::FileState;

/// True when any hunk declares a bare `# symbol` (no `# fallback=syntax`).
/// Such patches verify seeds against the crate graph, which needs the
/// same workspace search as follow/orphans.
pub(crate) fn patch_has_symbol(patch: &Patch) -> bool {
    patch.files.iter().flat_map(|s| &s.hunks).any(|h| match h {
        Hunk::Rsmpl(rh) => {
            rh.directives
                .iter()
                .any(|d| matches!(d, rsmpl_patch::Directive::Symbol { .. }))
                && !rh.fallback_syntax()
        }
        Hunk::Ordinary(_) => false,
    })
}

/// Crate graph over the pre-image snapshot (call only when
/// `patch_has_symbol` holds; zero cost otherwise).
pub(crate) fn build_symbol_graph(states: &HashMap<String, FileState>) -> crate::CrateGraph {
    let snapshot: HashMap<String, String> = states
        .iter()
        .map(|(p, st)| (p.clone(), st.original.clone()))
        .collect();
    crate::CrateGraph::build(&snapshot)
}

/// Per-rule symbol resolution mode for the JSON report (`"resolved"`
/// when a no-fallback `# symbol` hunk verified against the crate graph,
/// `"syntax"` when `# fallback=syntax` carried it; absent otherwise).
/// Additive under v3 (WI-20 owns the schema bump).
pub(crate) fn symbol_mode(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
) -> Option<String> {
    if compiled.table.symbols.is_empty() {
        return None;
    }
    if rh.fallback_syntax() {
        Some("syntax".to_string())
    } else {
        Some("resolved".to_string())
    }
}
