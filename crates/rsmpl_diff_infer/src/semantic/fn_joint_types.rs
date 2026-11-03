//! WI-20 slice 1 (D5): fn-joint shared types (`HunkPairs`,
//! `FlatPair`, `Joint`, `Fire`, `JointOutcome`) — `fn_joint.rs` crossed
//! the 2000-line gate; tests stay inline in their owners per WI-22.

use std::collections::{HashMap, HashSet};

/// One paired hunk inside a section.
pub(crate) struct HunkPairs {
    pub(crate) minuses: Vec<String>,
    pub(crate) pluses: Vec<String>,
    pub(crate) balanced: bool,
    pub(crate) sites: Vec<usize>,
}

/// One flattened native pair at its old-side site.
#[derive(Clone)]
pub(crate) struct FlatPair {
    pub(crate) sec: usize,
    pub(crate) hunk: usize,
    pub(crate) minus: String,
    pub(crate) plus: String,
    pub(crate) site: usize,
}

/// The whole-patch pre-pass outcome: rebuilt sections, the definition
/// path and rule for provenance (both empty on suppress-only), and the
/// suppressed `(path, old-name)` set.
pub(crate) type JointOutcome = (Vec<String>, String, String, HashSet<(String, String)>);

/// Which joint shape fired.
pub(crate) enum Joint {
    /// WI-4a: header pair + callee-only calls → one follow hunk.
    A,
    /// WI-4b B1: header pair (params evolve, return preserved) +
    /// uniform arg-add call template → shared-rule two-hunk, no follow
    /// (a follow would overlap the call hunk's sites).
    B1 {
        call_minus: String,
        call_plus: String,
    },
    /// WI-4b B2: attr add/remove + rename header + uniform callee-only
    /// call template → shared-rule two-hunk, no follow.
    B2 {
        attr_line: String,
        attr_add: bool,
        head_site: usize,
        call_minus: String,
        call_plus: String,
    },
    /// WI-20 slice 1 + WI-13 slices 1–2: rename-only header + one or two
    /// body line pairs + callee-only calls → one follow hunk carrying
    /// the header `...` pair plus the raw body pair lines (the engine
    /// applies each body pair span-scoped inside the renamed body,
    /// `body`-labelled per pair).
    Body { bodies: Vec<(String, String)> },
}

/// A validated fire payload: the def pair plus its call claims, all in
/// one file. Joint-A claims ride one follow hunk; Joint-B claims ride
/// the shared-rule two-hunk composition (def hunk + call-template hunk
/// under one rule name).
pub(crate) struct Fire {
    pub(crate) old: String,
    pub(crate) new: String,
    pub(crate) def_minus: String,
    pub(crate) def_plus: String,
    pub(crate) claims: Vec<FlatPair>,
    pub(crate) claimed_secs: Vec<usize>,
    pub(crate) joint: Joint,
    /// WI-4f-next leftover native hunk texts per section (verbatim, in
    /// original hunk order). Empty on pure-section fires (byte-identical
    /// path).
    pub(crate) leftover: HashMap<usize, Vec<String>>,
    /// WI-4f-next claimed native hunk bytes (hunk texts only, no `diff`
    /// header). Used for T375 only when `leftover` is nonempty.
    pub(crate) claimed_hunk_bytes: usize,
    /// WI-15 slice 1: extra emission-side bytes for a split def hunk
    /// (the synthesized tight `@@` headers of parked body hunks).
    /// Zero on every non-split fire; the fire gate prices it on the
    /// emission side under the precision allowance.
    pub(crate) split_extra: usize,
}
