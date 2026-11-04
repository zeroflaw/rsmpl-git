//! Whole-section detector chain (WI-4 prerequisite / WI-10 split).
//!
//! Split from `semantic/mod.rs` under the 2000-line gate: pure move, no
//! behavior change. Owns the detector registration — the ordered
//! pair-sweep chain shared by the whole-section path and the per-hunk
//! fallback (`crate::compress_balanced_pair_body` delegates to
//! [`run_pair_chain`]) — plus `compress_section` and its per-hunk
//! fallback. `mod.rs` keeps the facade (`infer_semantic*`), the item
//! helpers, and the unit tests.

/// WI-8 pair partition driver, split to `partition.rs` under the
/// 2000-line gate (D5): pure move, call sites unchanged.
pub(crate) use super::partition::try_partition_section;
use super::prov::{
    attempt, attempt_counted, commit_prov, declined_guard_with_numbers, declined_precondition,
    declined_veto, push_section_prov, reheader_rename,
};
use super::{HunkDeclined, HunkProvenance};

/// Ordered pair-sweep chain, tightest first (existing order,
/// unchanged; WI-4a appends one shape at the end, WI-4b one more, WI-4c one more, WI-5 one more, WI-9 one more): slot → sig-evolution → header-evolution → trait-method → method-to-fn → method-to-method →
/// same-literal → `&`-strip → qualified-call → turbofish →
/// macro-rename → use-prefix → bare call → call-perm →
/// call-arg-add →
/// call-literal → call-arg-template → method-arg-template →
/// general-expr → control-template → call-peel → let-type-add → type-sweep → let-template → untyped-let + pat-swap → sequence → repeat-fold → partial-strip → single-block → literal-frame. Returns the winning semantic section
/// plus its WI-2 detector name. No T375 byte guard inside — callers
/// keep their existing guards so reservation/truncation semantics
/// stay byte-identical to the pre-split `mod.rs`.
pub(crate) fn run_pair_chain(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
    declined: &mut Vec<HunkDeclined>,
) -> Option<(String, &'static str)> {
    // WI-6(b): double-delta `impl` widening (one double-delta pair
    // absorbed by two widened sweeps + tight singles for the rest) —
    // first: the gate requires a double-delta pair plus its defs and
    // heads, so unrelated sections skip fast and nothing else is
    // stolen.
    if let Some(wide) = attempt_counted(
        declined,
        "item_rename",
        super::item_joint_impl::try_impl_double_delta_section(
            minuses, pluses, path, file_old, changed, used_rules,
        ),
    ) {
        return Some((wide, "item_rename"));
    }
    // WI-7b + WI-16: `macro_rules!` definition + invocation rename (one
    // def plus N uses, hetero by construction; WI-16 emits the def with
    // `# follow=uses` when the scratch probe proves the site set, else
    // the WI-7b two-hunk verbatim fallback) — before the slot sweep, which owns
    // homogeneous header classes and never claims a `macro_rules!` line.
    if let Some(combined) = attempt_counted(
        declined,
        "macro_rules_rename",
        super::try_macro_rules_rename(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((combined, "macro_rules_rename"));
    }
    if let Some(slot) = attempt_counted(
        declined,
        "fn_slot_sweep",
        super::try_fn_slot_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((slot, "fn_slot_sweep"));
    }
    // WI-4c: signature evolution (name plus any vis/qual/generics/
    // params/return/where delta, bodies untouched) — immediately after
    // the rename-only slot sweep whose zero-delta classes it
    // generalizes (disjoint by construction: slot keeps rename-only,
    // this requires an evolving non-name slot).
    if let Some(evo) = attempt_counted(
        declined,
        "sig_evolution",
        super::try_sig_evolution_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((evo, "sig_evolution"));
    }
    // WI-9: header slot-only evolution (hetero preserved names, one
    // uniform slot delta, `N` emission) — immediately after its
    // shared-name sibling (disjoint by construction: that one owns
    // uniform names, this one requires hetero preserved names),
    // before the trait-method detector (those own `;` decls).
    if let Some(header) = attempt_counted(
        declined,
        "header_evolution",
        super::try_header_evolution_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((header, "header_evolution"));
    }
    // N≥2 bare-item vis sweep (uniform closed-set vis step over
    // same-kind items, N-hole emission) — immediately after its
    // fn-header sibling (disjoint by construction: that one owns
    // `{`-terminated fn headers and declines bare items; this one
    // declines `fn`), before the trait-method detector (those own
    // `;` decls, never bare vis pairs).
    if let Some(vis) = attempt(
        declined,
        "item_rename",
        super::try_item_vis_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((vis, "item_rename"));
    }
    // WI-10b: attribute-delta sweep (uniform attr-line add/remove/replace
    // over hetero items, N-hole emission) — immediately after its slot
    // sibling (those own header shapes; this owns pure-attr lines, which
    // no earlier detector parses).
    if let Some(attr) = attempt_counted(
        declined,
        "attr_sweep",
        super::try_attr_replace_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some(attr);
    }
    // WI-4b: trait+impl method rename (mixed `;` decl / `{` def
    // classes the slot and sig-evolution sweeps decline by
    // construction) — immediately after sig-evolution, before the
    // method sweeps (those own same-name call shapes, never header
    // pairs).
    if let Some(tm) = attempt_counted(
        declined,
        "trait_method_rename",
        super::try_trait_method_rename(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((tm, "trait_method_rename"));
    }
    // WI-6: inherent-method rename + anchored calls (def-only classes
    // stay with the slot sweep above; unproven call sets stay
    // ordinary). Disjoint from the trait half (that one requires a
    // trait side; this one requires none).
    if let Some(im) = attempt_counted(
        declined,
        "inherent_method_rename",
        super::try_inherent_method_rename(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((im, "inherent_method_rename"));
    }
    // WI-9b: field/variant member rename (one definition pair plus
    // use-site lines sharing the ident delta, follow emission with a
    // scratch-apply line-equality probe). Disjoint from the trait
    // detector (those own `;`/`{` fn headers; this owns `:`/bare-ident
    // pairs) — immediately after it, before the method sweeps.
    if let Some(member) = attempt_counted(
        declined,
        "member_rename",
        super::try_member_rename(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((member, "member_rename"));
    }
    // WI-13 remainder 3: associated-item rename (trait/impl `type`/
    // `const` defs plus `::Old` use tails, one hunk per block with
    // `# follow=uses` on the first). Disjoint from the member
    // detector (those own field/variant lines outside trait/impl
    // blocks or without a `type`/`const` head; this requires both)
    // and from definition evolution (those own same-name RHS
    // deltas; this requires a name delta).
    if let Some(assoc) = attempt_counted(
        declined,
        "assoc_rename",
        super::try_assoc_rename(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((assoc, "assoc_rename"));
    }
    // WI-12 slice 1: definition-value evolution (alias/const/field/
    // payload/discriminant, uniform delta) — immediately after the
    // member rename (those own ident deltas; this owns same-identity
    // RHS deltas), before the method sweeps (those own call shapes,
    // never `:`/payload/alias lines).
    if let Some((evo, _)) = attempt_counted(
        declined,
        "definition_evolution",
        super::try_definition_evolution_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((evo, "definition_evolution"));
    }
    if let Some(m2f) = attempt_counted(
        declined,
        "method_to_fn",
        super::try_method_to_fn_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((m2f, "method_to_fn"));
    }
    if let Some(m2m) = attempt_counted(
        declined,
        "method_to_method",
        super::try_method_to_method_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((m2m, "method_to_method"));
    }
    if let Some(same) = attempt_counted(
        declined,
        "method_same_literal",
        super::try_method_same_literal_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((same, "method_same_literal"));
    }
    if let Some(strip) = attempt_counted(
        declined,
        "ref_strip",
        super::try_ref_method_strip_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((strip, "ref_strip"));
    }
    // WI-3: qualified-path, turbofish, and macro-rename sweeps — after
    // the method sweeps, before the bare-call sweep (tightest first).
    // WI-4 slice 2b-1: counted declines.
    if let Some(qual) = attempt_counted(
        declined,
        "qualified_call",
        super::try_qualified_call_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((qual, "qualified_call"));
    }
    if let Some(turbo) = attempt_counted(
        declined,
        "turbofish",
        super::try_turbofish_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((turbo, "turbofish"));
    }
    if let Some(mac) = attempt_counted(
        declined,
        "macro_rename",
        super::try_macro_rename_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((mac, "macro_rename"));
    }
    if let Some(uses) = attempt_counted(
        declined,
        "use_prefix",
        super::try_use_prefix_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((uses, "use_prefix"));
    }
    // N >= 2 bare-call rewrites (callee-only delta), use-path prefix
    // renames above included; fall through to the trailing-arg sweeps
    // otherwise.
    // WI-17 slice 4: counted decline — a shape match with a twin on an
    // unchanged line records `totality:found:want` (or `veto:opaque-hit`)
    // instead of `no-match`.
    if let Some(sweep) = attempt_counted(
        declined,
        "call_sweep",
        super::try_call_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((sweep, "call_sweep"));
    }
    // WI-6 slice 2: whole-arg permutation sub-line sweep (same-callee
    // same-arity calls with one uniform arg permutation across all
    // subterms; the sub-line form of the arg-template permute promise).
    // After the callee-only sweep (those own token-equal-args classes;
    // this owns permuted-args classes), before the trailing-arg
    // sweeps. Pure whole-line classes decline here (they belong to
    // `call_arg_template` below).
    if let Some(perm) = attempt_counted(
        declined,
        "call_perm",
        super::try_call_perm_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((perm, "call_perm"));
    }
    // T447: same-callee trailing-arg addition (homo concrete tails
    // compress; hetero tails like 09d3446's `symlink` vs `false`
    // veto via minus-arity totality).
    // WI-17 slice 4: counted decline, like `call_sweep` above.
    if let Some(add) = attempt_counted(
        declined,
        "call_arg_add",
        super::try_call_arg_add_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((add, "call_arg_add"));
    }
    // T448: same-callee same-arity trailing-literal delta (homo concrete
    // trailing pair, hetero leading bound as E-vars; different-trailing
    // same-arity sites never veto via tuple-keyed totality).
    if let Some(lit) = attempt_counted(
        declined,
        "call_literal",
        super::try_call_literal_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((lit, "call_literal"));
    }
    // WI-5: hetero call/method arg templates (wrap/transform migrations
    // the engine applies but no detector raised) — after the concrete
    // literal sweeps (those own uniform-concrete classes; this owns
    // hole/wrapper classes), before the peel sweep.
    if let Some(arg) = attempt_counted(
        declined,
        "call_arg_template",
        super::try_call_arg_template_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((arg, "call_arg_template"));
    }
    if let Some(marg) = attempt_counted(
        declined,
        "method_arg_template",
        super::try_method_arg_template_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((marg, "method_arg_template"));
    }
    // WI-6: general-expression sweep (bare-expression anti-unification
    // with at most one hole) — after the call/method arg templates
    // (those own call shapes; this declines them), before the peel
    // sweep (which owns array-head shapes, never bare expressions).
    if let Some(general) = attempt_counted(
        declined,
        "general_expr",
        super::try_general_expr_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((general, "general_expr"));
    }
    // WI-8b: control-position sweep (guard/arm-body/closure/condition/
    // iterable/diverge/await-`?` subterms through the WI-6 core, bare
    // emission + whole-file totality) — after the whole-line general
    // sweep (those own whole-line expressions; this owns control
    // fragments no earlier detector parses), before the peel sweep.
    if let Some(control) = attempt_counted(
        declined,
        "control_template",
        super::try_control_template_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((control, "control_template"));
    }
    // T441: hetero-arg array-head-peel call migration
    // (`f(D, &[H, M, T…])` → `g(D, M, &[T…])` per-tail-arity groups);
    // all-identical classes stay for the tighter verbatim sweep below.
    if let Some(peel) = attempt_counted(
        declined,
        "call_peel",
        super::try_call_peel_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((peel, "call_peel"));
    }
    // WI-4a: let type-addition (`let P = E` → `let P: T = E`, P/E
    // hetero across sites, uniform concrete T) — after the call-shape
    // chain (those ignore the `let` wrapper and cannot express an
    // ascription delta), before the verbatim stage in the caller.
    if let Some(let_sweep) = attempt_counted(
        declined,
        "let_type_add",
        super::try_let_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((let_sweep, "let_type_add"));
    }
    // WI-4b: type rewrite (`Vec<T>` → `Box<[T]>`, T hetero across
    // sites) — after the let-shape chain (those own whole-statement
    // templates with init/pattern holes; this owns sub-line type
    // templates), before the verbatim stage in the caller.
    if let Some(type_sweep) = attempt_counted(
        declined,
        "type_sweep",
        super::try_type_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((type_sweep, "type_sweep"));
    }
    // WI-5 slice 1: joint let template (typed→typed `let` pairs with
    // per-site preserved pat/ty plus one uniform init delta) — after
    // the type sweep (that owns per-site-preserved-init classes; this
    // owns per-site init deltas), before the verbatim stage. Disjoint
    // from both let slices by construction.
    if let Some(joint) = attempt_counted(
        declined,
        "let_template",
        super::try_let_template_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((joint, "let_template"));
    }
    // WI-8 slice 1: untyped joint let template (same as above without
    // ascriptions — hetero `P0`, uniform or `E0`-hole init incl.
    // const-block/`_` inits via the shape parity gate) plus the
    // tuple-position swap (`let (P0, P1) = E0;` →
    // `let (P1, P0) = E0;`). After the typed template (those own
    // ascribed classes; this declines any ascribed side), before the
    // sequence sweep (which owns multi-line windows, never singles).
    if let Some(untyped) = attempt_counted(
        declined,
        "let_template",
        super::try_untyped_let_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((untyped, "let_template"));
    }
    if let Some(swap) = attempt_counted(
        declined,
        "let_template",
        super::try_pat_swap_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((swap, "let_template"));
    }
    // WI-5: scoped `let`-binder rename (`let count = E;` → `let n = E;`
    // plus its in-scope use lines, the engine follow covering the uses)
    // — after the let-template trio (those own per-site-preserved-pat
    // classes and decline binder-changing pairs by construction) and
    // before the window fusers (a `[binder-def + uses]` run is one
    // binder intent under follow, never a verbatim-ish window): the
    // shape gate admits only binder-def-first runs, so no other family
    // can be stolen.
    if let Some(binder) = attempt_counted(
        declined,
        "let_binder",
        super::try_let_binder_window(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((binder, "let_binder"));
    }
    // WI-5c: repeated-block `let` sequences (N ≥ 2 blocks × 2–4 stmts,
    // hetero positions ride holes) — after the single-statement let
    // sweeps (those own whole-class templates; this owns multi-line
    // windows the singles cannot express), before the fold (which owns
    // call-spine strips, never statement windows).
    if let Some(seq) = attempt_counted(
        declined,
        "sequence",
        super::try_sequence_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((seq, "sequence"));
    }
    // WI-1c: nested same-callee full-strip fold (`Box::new(Box::new(x))`
    // → `x`, `# repeat`) — after the type sweep (those own whole-statement
    // templates; this owns call-spine strips), before the verbatim stage
    // in the caller. No earlier detector claims a call→non-call pair.
    if let Some(fold) = attempt_counted(
        declined,
        "repeat_fold",
        super::try_repeat_fold_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((fold, "repeat_fold"));
    }
    // WI-1d: uniform one-layer partial strip (`C(C(x))` → `C(x)`,
    // plain hunk, no `# repeat` — a repeat emission would over-strip
    // the kept layers). After the fold (full strips stay folded;
    // disjoint by construction: the fold declines partial pairs and
    // the strip declines full strips), before the verbatim stage.
    if let Some(strip) = attempt_counted(
        declined,
        "partial_strip",
        super::try_partial_strip_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((strip, "partial_strip"));
    }
    // WI-5d: single-block hetero `let` run (one contiguous K = 2–4
    // window, within-pair abstraction) — every earlier detector keeps
    // its classes (this owns only runs declined by all of them).
    if let Some(single) = attempt_counted(
        declined,
        "single_block",
        super::try_single_block_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((single, "single_block"));
    }
    // WI-7 slice-2: hetero pair window (exactly two consecutive plain
    // lets: a pure type-add plus a typed method-wrap, fixture 107) as
    // one allow-one window intent. A narrow arm, not a single_block
    // gate widening — the one-sided decline pin above stands; any
    // other delta combination declines and the section falls through
    // to the partition literals. Strict T375 downstream owns the byte
    // guard, like every chain arm.
    if let Some(window) = attempt_counted(
        declined,
        "single_block",
        super::try_hetero_pair_window_single(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((window, "single_block"));
    }
    // WI-7: lone-literal sweep (`"old"` → `"new"`, `10` → `20`,
    // `false` → `true` inside hetero frames) — last: the bare-literal
    // template is the broadest still-semantic sweep, so tighter
    // detectors (calls, lets, windows) keep their classes first; the
    // verbatim stage in the caller owns all-identical lines (T422,
    // declined inside the detector).
    if let Some(frame) = attempt_counted(
        declined,
        "literal_frame",
        super::try_literal_frame_sweep(minuses, pluses, path, file_old, changed, used_rules),
    ) {
        return Some((frame, "literal_frame"));
    }
    None
}

pub(crate) fn compress_section(
    section: &str,
    used_rules: &mut Vec<String>,
    pre: &std::collections::HashMap<String, String>,
    prov: &mut Vec<HunkProvenance>,
    suppressed: &std::collections::HashSet<(String, String)>,
    declined: &mut Vec<HunkDeclined>,
) -> Option<String> {
    let header = section.lines().next()?;
    // T385: CRLF content stays ordinary passthrough — detectors strip
    // `\r` via `str::lines` and would emit hunks that cannot match
    // the worktree file.
    if !header.starts_with("diff --git ") {
        return None;
    }
    if section.contains('\r') {
        // WI-15c: the gate itself is the explanation (WI-17 slice 3:
        // LF-only input premise, `precondition:` form).
        declined_precondition(declined, "section", "crlf-content");
        return None;
    }
    // WI-7a: rename sections compress their content hunks instead of
    // staying ordinary passthrough. The engine seeds a rename section's
    // new-side entry from the OLD file's content, so semantic hunks
    // citing old-side coordinates apply transparently; the lowering
    // rebuilds `rename from/to` around the new content. The original
    // rename prelude (`similarity index`, `rename from/to`, modes,
    // `---`/`+++`) is preserved byte-exact — detectors rebuild only
    // `diff --git`, which would drop it. Pre-image totality reads the
    // old path (the new path does not exist at base; `rsmpl_diff`
    // maps it, with section-visible fallback here).
    let rename_new: Option<String> = if super::is_rename_header(header) {
        let rest = header.strip_prefix("diff --git ")?;
        let parts = rsmpl_patch::split_git_paths(rest)?;
        if parts.len() != 2 {
            return None;
        }
        let _old = parts[0].strip_prefix("a/").unwrap_or(&parts[0]);
        let new = parts[1].strip_prefix("b/").unwrap_or(&parts[1]);
        if new == "/dev/null" || new == "dev/null" {
            return None;
        }
        if !new.ends_with(".rs") {
            return None;
        }
        // Headers-only renames (no hunks) stay passthrough below via
        // the `saw_hunk` gate; nothing else is rename-specific here —
        // the shared machinery runs on the original section (whose
        // prelude splitters already preserve) with `path` = new side.
        // Pre-image totality reads `pre[new]` (mapped from the old path
        // by `rsmpl_diff`; section-visible fallback otherwise).
        Some(new.to_string())
    } else {
        None
    };
    let path_owned;
    let path = if let Some(n) = &rename_new {
        path_owned = n.clone();
        &path_owned
    } else {
        let p = crate::b_side_path(header.strip_prefix("diff --git ")?)?;
        if !p.ends_with(".rs") {
            return None;
        }
        path_owned = p;
        &path_owned
    };
    // Rename prelude (everything before the first `@@`): preserved
    // byte-exact around winning semantic bodies.
    let rename_prelude: Option<String> = rename_new.as_ref().and_then(|_| {
        let at = section.find("@@")?;
        Some(section[..at].to_string())
    });
    // T464: whole-file item-deletion sweep (minus-only deletion sections
    // compress to one many-delete hunk + ordinary leftovers). Renames
    // always have both sides real, so the delete sweep cannot fire —
    // skip it on the rename path.
    let delete_before = used_rules.len();
    if rename_new.is_none() {
        if let Some(deleted) = attempt(
            declined,
            "delete",
            super::deletes::try_delete_sweep(section, used_rules),
        ) {
            push_section_prov(
                prov,
                &deleted,
                section,
                path,
                "delete",
                &crate::totality::changed_old_lines(section),
                pre.get(path.as_str()).map(String::as_str),
                delete_before,
                used_rules,
            );
            return Some(deleted);
        }
    }
    // T403 totality input: the pre-image text for this path, if the
    // caller supplied it (None = section-visible evidence only).
    let file_old = pre.get(path.as_str()).map(String::as_str);
    // Group changed lines per `@@` hunk: unified diff lists each
    // hunk's `-` block before its `+` block, so global order pairing
    // WI-4d: pre-pass emissions bypass whole-section detectors and go
    // straight to the per-hunk fallback (which skips `rule=` hunks below).
    // Whole-section pair sweeps would re-harvest the call-template hunk
    // of a joint emission as a second rule; per-hunk leftovers (e.g. a
    // delete sweep in a move section) still compress.
    if super::is_semantic_section(section) {
        return compress_section_per_hunk(
            section, path, file_old, used_rules, prov, suppressed, declined,
        );
    }
    // is only sound when every hunk is a pure modification.
    let mut minuses: Vec<&str> = Vec::new();
    let mut pluses: Vec<&str> = Vec::new();
    let mut hunk_minus = 0usize;
    let mut hunk_plus = 0usize;
    let mut balanced = true;
    let mut in_hunk = false;
    let mut saw_hunk = false;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if in_hunk && hunk_minus != hunk_plus {
                balanced = false;
            }
            in_hunk = true;
            saw_hunk = true;
            hunk_minus = 0;
            hunk_plus = 0;
            continue;
        }
        if !in_hunk {
            balanced &= !(line.starts_with("old mode ") || line.starts_with("new mode ")); // T401
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(&line[1..]);
            hunk_minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(&line[1..]);
            hunk_plus += 1;
        }
    }
    if in_hunk && hunk_minus != hunk_plus {
        balanced = false;
    }
    // Whole-section attempt first (preserves cross-hunk sweeps): it
    // needs every hunk balanced and equal non-zero global counts, so
    // global order pairing stays sound. Gate failures skip straight
    // to the per-hunk fallback (T349).
    let whole = balanced && saw_hunk && minuses.len() == pluses.len() && !minuses.is_empty();
    // T375: every whole-section candidate clears the token-win guard
    // (byte-smaller than the ordinary section, same prelude either
    // way); a losing shape truncates its reserved rule name and falls
    // through to the per-hunk retry so one losing shape never vetoes
    // winning runs in other hunks.
    let checkpoint = used_rules.len();
    // T411: old-side changed lines for the line-aware opaque veto.
    let changed = crate::totality::changed_old_lines(section);
    // WI-5 slice 2a: a rename def stranded from its use by an evolution
    // co-resident suppresses the bare rename single (WI-4a posture for
    // items) — the section stays wholly ordinary.
    if super::partial::section_has_stranded_item_joint(section, path, file_old) {
        // WI-15c: the gate itself is the explanation (WI-17 slice 3:
        // joint-territory veto form).
        declined_veto(declined, "section", "stranded-joint");
        return None;
    }
    // WI-21 slice 4: a cross-kind member deletion (field delta under an
    // enum rename, variant delta under a struct rename) suppresses the
    // bare rename single like the stranded joint above — the section
    // stays wholly ordinary with the stranded delta in sites.
    if super::partial::section_has_cross_kind_deletion(section) {
        // WI-15c: the gate itself is the explanation (WI-17 slice 3:
        // joint-territory veto form).
        declined_veto(declined, "section", "cross-kind-member");
        return None;
    }
    // WI-4c: a vetoed fn joint suppresses its old callee name
    // workspace-wide — a section of pure call-callee sites of a
    // suppressed `(path, old)` stays ordinary instead of emitting a
    // rename-claiming sweep over a definition delta it cannot see. The
    // per-hunk fallback below carries the same gate per hunk; this
    // whole-section gate stops the pure-call section (e.g. the call
    // file of a cross-file veto) from sweeping before it ever reaches
    // the per-hunk path. Mixed sections fall through (their pure-call
    // hunks still hit the per-hunk gate; other hunks compress alone).
    if !suppressed.is_empty()
        && !minuses.is_empty()
        && minuses.len() == pluses.len()
        && minuses.iter().all(|m| {
            suppressed
                .iter()
                .any(|(p, o)| p == path && super::fn_joint::is_call_callee_of(m, o))
        })
    {
        return compress_section_per_hunk(
            section, path, file_old, used_rules, prov, suppressed, declined,
        );
    }
    if whole && minuses.len() == 1 {
        // WI-7 slice 1: the seven single-pair arms live in
        // `whole_single.rs` (one shared guard decision + span-priced
        // ordinary); the per-hunk fallback below still owns leftovers.
        if let Some(single) = super::whole_single::try_whole_single_section(
            section,
            path,
            file_old,
            &changed,
            &minuses,
            &pluses,
            &rename_prelude,
            checkpoint,
            used_rules,
            prov,
            declined,
        ) {
            return Some(single);
        }
        return compress_section_per_hunk(
            section, path, file_old, used_rules, prov, suppressed, declined,
        );
    }
    if !whole {
        // T441: hetero-arg peel cross-hunk sweep before verbatim
        // (claims hetero classes; pure-identical classes stay for the
        // tighter verbatim sweep, which ignores the single-pair peel
        // bodies — pinned by `peel_body_verbatim_still_fires`). T449:
        // same-callee trailing-literal cross-hunk sweep between peel
        // and verbatim (claims hetero literal classes split across
        // singleton hunks, e.g. 7025a14's spaced `let` prize pair;
        // all-identical classes stay for verbatim, T422 precedent).
        // Each rewrite composes on the previous one and is kept even
        // when later stages find nothing (returning `None` here
        // would discard it); downstream stages group the single-pair
        // bodies at n=1 (skipped) and pass them through unchanged.
        let mut rewritten: Option<String> = None;
        // WI-2: the section drivers below record every rule they reserve
        // into `fired` with its exact detector (verbatim also records its
        // leftover-retry rules with theirs); commit keeps only the rules
        // present in the final composed text.
        let mut fired: Vec<(String, &'static str)> = Vec::new();
        if let Some(peeled) = attempt(
            declined,
            "call_peel",
            super::peel::try_call_peel_section(section, path, used_rules, file_old, &mut fired),
        ) {
            rewritten = Some(peeled);
        }
        let after_peel = rewritten.as_deref().unwrap_or(section);
        if let Some(lit) = attempt(
            declined,
            "call_literal",
            super::literal::try_call_literal_section(
                after_peel, path, used_rules, file_old, &mut fired,
            ),
        ) {
            rewritten = Some(lit);
        }
        let after_lit = rewritten.as_deref().unwrap_or(section);
        // WI-9a: same-name concrete macro interiors (whole-line pairs and
        // stranded joins) before verbatim (which vetoes every macro, so no
        // double-claim); emitted single-pattern hunks group at n=1 below.
        if let Some(interior) = attempt(
            declined,
            "macro_interior",
            super::try_macro_interior_section(after_lit, path, used_rules, file_old, &mut fired),
        ) {
            rewritten = Some(interior);
        }
        // WI-17b: rule-body interiors (inner delta inside
        // `macro_rules!` arms) over the interior output: already
        // semantic (`rule=`) hunks are never re-harvested, so claimed
        // pairs cannot double-fire.
        let after_rule_body = if let Some(rule_body) = attempt(
            declined,
            "macro_rule_body",
            super::macro_rule_body::try_macro_rule_body_section(
                rewritten.as_deref().unwrap_or(section),
                path,
                used_rules,
                file_old,
                &mut fired,
            ),
        ) {
            rewritten = Some(rule_body);
            rewritten.as_deref().unwrap_or(section)
        } else {
            rewritten.as_deref().unwrap_or(section)
        };
        // WI-7a: enclosing-macro interiors (changed lines inside one
        // shared macro invocation) over the interior output: already
        // semantic (`rule=`) hunks are never re-harvested, so claimed
        // pairs cannot double-fire.
        let after_interior = after_rule_body;
        if let Some(enclosing) = attempt(
            declined,
            "macro_enclosing",
            super::macro_enclosing::try_macro_enclosing_section(
                after_interior,
                path,
                used_rules,
                file_old,
                &mut fired,
            ),
        ) {
            rewritten = Some(enclosing);
        }
        let after_mac = rewritten.as_deref().unwrap_or(section);
        // WI-10b: attribute-delta groups (pure-insertion/deletion/replace
        // attr hunks) before verbatim (which vetoes bare attr lines, so no
        // double-claim); claimed hunks rewrite in place, the rest passes
        // through byte-identical for verbatim + per-hunk below.
        if let Some(attr) = attempt(
            declined,
            "attr_sweep",
            super::try_attr_sweep_section(after_mac, path, used_rules, file_old, &mut fired),
        ) {
            rewritten = Some(attr);
        }
        let after_attr = rewritten.as_deref().unwrap_or(section);
        if let Some(verbatim) = attempt(
            declined,
            "verbatim",
            super::verbatim::try_verbatim_sweep(after_attr, path, used_rules, file_old, &mut fired),
        ) {
            commit_prov(prov, &verbatim, section, path, &fired, &changed, file_old);
            return Some(verbatim);
        }
        if let Some(r) = rewritten {
            commit_prov(prov, &r, section, path, &fired, &changed, file_old);
            return Some(r);
        }
        // T403: non-whole sections try the cross-hunk verbatim sweep
        // before per-hunk (whole-section detectors need balance) —
        // already attempted above on the unmodified section.
        return compress_section_per_hunk(
            section, path, file_old, used_rules, prov, suppressed, declined,
        );
    }
    // N >= 2 whole-section pair sweeps in chain order; a losing shape
    // truncates its reserved rule and falls through to verbatim, so
    // one losing shape never vetoes the rest.
    if let Some((sweep, detector)) = run_pair_chain(
        &minuses, &pluses, path, file_old, &changed, used_rules, declined,
    ) {
        // WI-7a: rename sections re-emit under the preserved prelude.
        // WI-9 slice 1: the gate consults `guard_for` (one decision
        // point in `guard::pair_section_gate`) — Precision detectors
        // price the WI-7 allowance over the claimed pair count,
        // Strict keeps T375 over the normalized span.
        let ordinary = super::guard::normalized_section_len(section);
        let n_claimed = minuses.len();
        // WI-17 slice 1: pair-chain gate vetoes carry the WI-7 numbers
        // in the closed `guard:` form (SPEC D.2). `sem` prices the exact
        // losing emission, `ord` the normalized ordinary span, `allow`
        // the precision allowance (0 under strict T375).
        let guard_numbers = |emission: &str| -> (&'static str, usize, usize, usize) {
            let sem = emission.len();
            if super::guard::guard_for(detector) == super::guard::Guard::Precision {
                let allow = super::guard::precision_allowance(
                    n_claimed,
                    super::block_headers::filter_header_excess(emission),
                );
                ("precision", sem, ordinary, allow)
            } else {
                ("byte", sem, ordinary, 0)
            }
        };
        if let Some(prelude) = &rename_prelude {
            if let Some(final_text) = reheader_rename(&sweep, prelude) {
                if super::guard::pair_section_gate(detector, &final_text, ordinary, n_claimed) {
                    push_section_prov(
                        prov,
                        &final_text,
                        section,
                        path,
                        detector,
                        &changed,
                        file_old,
                        checkpoint,
                        used_rules,
                    );
                    return Some(final_text);
                }
            }
            let (kind, sem, ord, allow) = guard_numbers(&sweep);
            declined_guard_with_numbers(declined, detector, kind, sem, ord, allow);
            used_rules.truncate(checkpoint);
        } else if super::guard::pair_section_gate(detector, &sweep, ordinary, n_claimed) {
            push_section_prov(
                prov, &sweep, section, path, detector, &changed, file_old, checkpoint, used_rules,
            );
            return Some(sweep);
        } else {
            let (kind, sem, ord, allow) = guard_numbers(&sweep);
            declined_guard_with_numbers(declined, detector, kind, sem, ord, allow);
            used_rules.truncate(checkpoint);
        }
    }
    // WI-8 pair partition: the chain declines hetero multi-rename
    // classes (distinct deltas share no template); when every pair is
    // a single-pair item rename the section still compresses pair by
    // pair (one hunk per pair, positional rules). Verbatim owns
    // identical pairs (the duplicate-minus veto), so at most one of
    // the two fires; the per-hunk fallback below keeps every other
    // class. Same T375 + provenance shape as the chain block.
    let part_checkpoint = used_rules.len();
    if let Some((part, part_detector)) = attempt(
        declined,
        "partition",
        try_partition_section(&minuses, &pluses, path, file_old, &changed, used_rules),
    ) {
        // WI-7a: rename sections re-emit under the preserved prelude;
        // Strict T375 over the normalized span compares the final bytes.
        if let Some(prelude) = &rename_prelude {
            if let Some(final_text) = reheader_rename(&part, prelude) {
                if super::guard::within_strict(
                    final_text.len(),
                    super::guard::normalized_section_len(section),
                ) {
                    push_section_prov(
                        prov,
                        &final_text,
                        section,
                        path,
                        part_detector,
                        &changed,
                        file_old,
                        part_checkpoint,
                        used_rules,
                    );
                    return Some(final_text);
                }
                // WI-17 slice 2: veto carries the WI-7 numbers (strict: allow 0).
                declined_guard_with_numbers(
                    declined,
                    part_detector,
                    "byte",
                    final_text.len(),
                    super::guard::normalized_section_len(section),
                    0,
                );
            }
            used_rules.truncate(part_checkpoint);
        } else if super::guard::within_strict(
            part.len(),
            super::guard::normalized_section_len(section),
        ) {
            push_section_prov(
                prov,
                &part,
                section,
                path,
                part_detector,
                &changed,
                file_old,
                part_checkpoint,
                used_rules,
            );
            return Some(part);
        } else {
            // WI-17 slice 2: veto carries the WI-7 numbers (strict: allow 0).
            declined_guard_with_numbers(
                declined,
                part_detector,
                "byte",
                part.len(),
                super::guard::normalized_section_len(section),
                0,
            );
            used_rules.truncate(part_checkpoint);
        }
    }
    // WI-19 partial partition: the all-or-nothing path above declines
    // mixed sections (one unclaimable pair vetoes); when ≥1 hunk is
    // fully claimable the section still compresses hunk by hunk with
    // the rest left ordinary verbatim in the same section (per-hunk
    // ranges are disjoint, SPEC 10.6). Hunk atomicity is kept: a mixed
    // native hunk stays ordinary whole (the per-hunk fallback below
    // owns it). Scaled precision guard (`semantic <= ordinary + 16 *
    // claimed`, `guard_for` Precision for `item_rename`); provenance
    // shape as above.
    let partial_checkpoint = used_rules.len();
    if let Some((partial, n_claimed)) = attempt(
        declined,
        "partial",
        super::partial::try_partial_partition_section(section, path, file_old, used_rules),
    ) {
        let ordinary = super::guard::normalized_section_len(section);
        if let Some(prelude) = &rename_prelude {
            if let Some(final_text) = reheader_rename(&partial, prelude) {
                if super::guard::within_precision(final_text.len(), ordinary, n_claimed, 0) {
                    push_section_prov(
                        prov,
                        &final_text,
                        section,
                        path,
                        "item_rename",
                        &changed,
                        file_old,
                        partial_checkpoint,
                        used_rules,
                    );
                    return Some(final_text);
                }
                // WI-17 slice 2: veto carries the WI-7 numbers.
                declined_guard_with_numbers(
                    declined,
                    "partial",
                    "precision",
                    final_text.len(),
                    ordinary,
                    super::guard::precision_allowance(n_claimed, 0),
                );
            }
            used_rules.truncate(partial_checkpoint);
        } else if super::guard::within_precision(partial.len(), ordinary, n_claimed, 0) {
            push_section_prov(
                prov,
                &partial,
                section,
                path,
                "item_rename",
                &changed,
                file_old,
                partial_checkpoint,
                used_rules,
            );
            return Some(partial);
        } else {
            // WI-17 slice 2: veto carries the WI-7 numbers.
            declined_guard_with_numbers(
                declined,
                "partial",
                "precision",
                partial.len(),
                ordinary,
                super::guard::precision_allowance(n_claimed, 0),
            );
            used_rules.truncate(partial_checkpoint);
        }
    }
    let mut verb_fired: Vec<(String, &'static str)> = Vec::new();
    // WI-9a: same-name concrete macro interiors before verbatim (which
    // vetoes every macro, so no double-claim).
    if let Some(interior) = attempt(
        declined,
        "macro_interior",
        super::try_macro_interior_section(section, path, used_rules, file_old, &mut verb_fired),
    ) {
        // WI-17b: rule-body interiors over the interior output, then
        // WI-7a enclosing over the rule-body output (already-semantic
        // hunks are never re-harvested).
        let after_rule = attempt(
            declined,
            "macro_rule_body",
            super::macro_rule_body::try_macro_rule_body_section(
                &interior,
                path,
                used_rules,
                file_old,
                &mut verb_fired,
            ),
        )
        .unwrap_or_else(|| interior.clone());
        if let Some(enclosing) = attempt(
            declined,
            "macro_enclosing",
            super::macro_enclosing::try_macro_enclosing_section(
                &after_rule,
                path,
                used_rules,
                file_old,
                &mut verb_fired,
            ),
        ) {
            commit_prov(
                prov,
                &enclosing,
                section,
                path,
                &verb_fired,
                &changed,
                file_old,
            );
            return Some(enclosing);
        }
        commit_prov(
            prov,
            &after_rule,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(after_rule);
    }
    // WI-17b: the interior sweep declined the section whole — the
    // rule-body sweep still sees the original ordinary hunks.
    if let Some(rule_body) = attempt(
        declined,
        "macro_rule_body",
        super::macro_rule_body::try_macro_rule_body_section(
            section,
            path,
            used_rules,
            file_old,
            &mut verb_fired,
        ),
    ) {
        // WI-7a enclosing over the rule-body output.
        if let Some(enclosing) = attempt(
            declined,
            "macro_enclosing",
            super::macro_enclosing::try_macro_enclosing_section(
                &rule_body,
                path,
                used_rules,
                file_old,
                &mut verb_fired,
            ),
        ) {
            commit_prov(
                prov,
                &enclosing,
                section,
                path,
                &verb_fired,
                &changed,
                file_old,
            );
            return Some(enclosing);
        }
        commit_prov(
            prov,
            &rule_body,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(rule_body);
    }
    // WI-7a: the interior sweep declined the section whole — the
    // enclosing sweep still sees the original ordinary hunks.
    if let Some(enclosing) = attempt(
        declined,
        "macro_enclosing",
        super::macro_enclosing::try_macro_enclosing_section(
            section,
            path,
            used_rules,
            file_old,
            &mut verb_fired,
        ),
    ) {
        commit_prov(
            prov,
            &enclosing,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(enclosing);
    }
    if let Some(verbatim) = attempt(
        declined,
        "verbatim",
        super::verbatim::try_verbatim_sweep(section, path, used_rules, file_old, &mut verb_fired),
    ) {
        commit_prov(
            prov,
            &verbatim,
            section,
            path,
            &verb_fired,
            &changed,
            file_old,
        );
        return Some(verbatim);
    }
    // T403: verbatim sweep after the whole-section detectors fail.
    // Per-hunk fallback (T349): pairwise-disjoint hunks merge by
    // construction (SPEC 10.6 option-1).
    compress_section_per_hunk(
        section, path, file_old, used_rules, prov, suppressed, declined,
    )
}

/// Per-hunk fallback for `compress_section`: compress each balanced
/// native hunk independently with the same detectors, keep the rest as
/// ordinary hunks in original order. Returns `None` when no hunk
/// compressed (caller keeps the ordinary passthrough).
fn compress_section_per_hunk(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
    prov: &mut Vec<HunkProvenance>,
    suppressed: &std::collections::HashSet<(String, String)>,
    declined: &mut Vec<HunkDeclined>,
) -> Option<String> {
    // Split the prelude (`diff --git`/`index`/`---`/`+++`) from the
    // native hunks; a section without hunks has nothing to compress.
    let mut prelude = String::new();
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        match current.as_mut() {
            Some(h) => {
                h.push_str(line);
                h.push('\n');
            }
            None => {
                prelude.push_str(line);
                prelude.push('\n');
            }
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return None;
    }
    // WI-12c slice 1: definition-addition + struct-literal construction
    // follow-through (all-or-nothing group over the section's hunks).
    // `None` falls through to the per-hunk path below, where the
    // definition insertion still fires alone.
    if let Some(group) = attempt(
        declined,
        "definition_addition_follow",
        super::try_definition_addition_follow(section, path, file_old, used_rules),
    ) {
        let mut out = prelude;
        for h in &group {
            commit_prov(
                prov,
                &h.emitted,
                &h.native,
                path,
                &[(h.rule.clone(), h.detector)],
                &h.changed,
                file_old,
            );
            out.push_str(&h.emitted);
        }
        return Some(out);
    }
    // WI-12d: variant-addition + match-arm follow-through (disjoint
    // from the field group above, which declines variant defs).
    // `None` falls through byte-identically below.
    if let Some(group) = attempt(
        declined,
        "variant_addition_follow",
        super::try_variant_addition_follow(section, path, file_old, used_rules),
    ) {
        let mut out = prelude;
        for h in &group {
            commit_prov(
                prov,
                &h.emitted,
                &h.native,
                path,
                &[(h.rule.clone(), h.detector)],
                &h.changed,
                file_old,
            );
            out.push_str(&h.emitted);
        }
        return Some(out);
    }
    let mut out = prelude;
    let mut compressed_any = false;
    for hunk in &hunks {
        let mut minuses: Vec<&str> = Vec::new();
        // WI-4d: already-semantic (`rule=`) hunks are never re-harvested,
        // so pre-pass emissions keep their rule and leftovers still compress.
        if hunk.lines().next().is_some_and(|h| h.contains("rule=")) {
            out.push_str(hunk);
            continue;
        }
        let mut pluses: Vec<&str> = Vec::new();
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(&line[1..]);
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(&line[1..]);
            }
        }
        // WI-4a: a vetoed fn joint suppresses its old callee name for
        // this file — a hunk of pure call-callee sites of a suppressed
        // `(path, old)` stays ordinary instead of emitting a
        // rename-claiming sweep over a definition delta it cannot see.
        // The def hunk itself never matches (headers are not calls),
        // and mixed hunks fall through to the chain below (which
        // declines hetero shapes on its own).
        if !suppressed.is_empty()
            && !minuses.is_empty()
            && minuses.len() == pluses.len()
            && minuses.iter().all(|m| {
                suppressed
                    .iter()
                    .any(|(p, o)| p == path && super::fn_joint::is_call_callee_of(m, o))
            })
        {
            out.push_str(hunk);
            continue;
        }
        // T465: partial item-deletion (whole-item minus-heavy hunks only).
        let partial_before = used_rules.len();
        if let Some(d) = attempt(
            declined,
            "delete",
            super::deletes::try_partial_delete_hunk(hunk, file_old, used_rules),
        ) {
            push_section_prov(
                prov,
                &d,
                hunk,
                path,
                "delete",
                &crate::totality::changed_old_lines(hunk),
                file_old,
                partial_before,
                used_rules,
            );
            compressed_any = true;
            out.push_str(&d);
            continue;
        }
        // WI-12b slice 1: single pure-insertion member addition
        // (`+ b: bool,` / `+ C` inside a proven container) under the
        // precision guard. Balanced hunks keep the shared chain below.
        if minuses.is_empty() && pluses.len() == 1 {
            let add_before = used_rules.len();
            if let Some((add, detector)) = attempt(
                declined,
                "definition_addition",
                super::try_definition_addition(&minuses, &pluses, path, file_old, hunk, used_rules),
            ) {
                let body = add.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
                // WI-2b: the span prices rendering-normalized (git elides
                // `,1`, unit convention writes it), never the raw header.
                if super::guard::within_precision(
                    body.len(),
                    super::normalized_hunk_len(hunk),
                    1,
                    0,
                ) {
                    push_section_prov(
                        prov,
                        body,
                        hunk,
                        path,
                        detector,
                        &crate::totality::changed_old_lines(hunk),
                        file_old,
                        add_before,
                        used_rules,
                    );
                    compressed_any = true;
                    out.push_str(body);
                    continue;
                }
                used_rules.truncate(add_before);
            } else {
                used_rules.truncate(add_before);
            }
        }
        // WI-14/WI-18: `use`-regroup merge (N→1) + split (1→N); see sibling.
        if (minuses.len() >= 2 && pluses.len() == 1) || (minuses.len() == 1 && pluses.len() >= 2) {
            let regroup_before = used_rules.len();
            let probe_name: &'static str = if minuses.len() == 1 {
                "use_split"
            } else {
                "use_regroup"
            };
            if let Some((body, detector)) = attempt(
                declined,
                probe_name,
                super::try_use_regroup_or_split_hunk(
                    &minuses, &pluses, path, file_old, hunk, used_rules,
                ),
            ) {
                push_section_prov(
                    prov,
                    &body,
                    hunk,
                    path,
                    detector,
                    &crate::totality::changed_old_lines(hunk),
                    file_old,
                    regroup_before,
                    used_rules,
                );
                compressed_any = true;
                out.push_str(&body);
                continue;
            } else {
                used_rules.truncate(regroup_before);
            }
        }
        // WI-6(c): rename + full attr-strip joint (unbalanced Nv1
        // header hunk: N-1 attr/doc deletions + one header rename pair).
        // Balanced hunks keep the shared chain below.
        if minuses.len() >= 2 && pluses.len() == 1 {
            let strip_before = used_rules.len();
            let strip_changed = crate::totality::changed_old_lines(hunk);
            if let Some(stripped) = attempt_counted(
                declined,
                "attr_sweep",
                super::attr_strip::try_attr_strip_joint(
                    &minuses,
                    &pluses,
                    path,
                    file_old,
                    &strip_changed,
                    used_rules,
                ),
            ) {
                let body = stripped
                    .split_once('\n')
                    .map(|(_, rest)| rest)
                    .unwrap_or("");
                if super::guard::within_precision(
                    body.len(),
                    super::normalized_hunk_len(hunk),
                    1,
                    0,
                ) {
                    push_section_prov(
                        prov,
                        body,
                        hunk,
                        path,
                        "attr_sweep",
                        &strip_changed,
                        file_old,
                        strip_before,
                        used_rules,
                    );
                    compressed_any = true;
                    out.push_str(body);
                    continue;
                }
                declined_guard_with_numbers(
                    declined,
                    "attr_sweep",
                    "precision",
                    body.len(),
                    super::normalized_hunk_len(hunk),
                    super::guard::precision_allowance(1, 0),
                );
                used_rules.truncate(strip_before);
            } else {
                used_rules.truncate(strip_before);
            }
        }
        // One balanced hunk through the shared detector chain
        // (`compress_balanced_pair_body` — same shapes the verbatim
        // sweep retries leftovers with); ordinary hunks pass through.
        let changed = crate::totality::changed_old_lines(hunk);
        let pair_before = used_rules.len();
        match crate::compress_balanced_pair_body(
            &minuses,
            &pluses,
            path,
            file_old,
            &changed,
            super::normalized_hunk_len(hunk),
            used_rules,
            declined,
        ) {
            Some((body, detector)) => {
                push_section_prov(
                    prov,
                    &body,
                    hunk,
                    path,
                    detector,
                    &changed,
                    file_old,
                    pair_before,
                    used_rules,
                );
                compressed_any = true;
                out.push_str(&body);
            }
            // WI-4 sub-hunk partition: the whole hunk declines but its
            // contiguous same-class runs may still raise per-group, the
            // rest staying ordinary verbatim in place (hunk atomicity is
            // kept per run, never per line reordered).
            None => {
                if let Some(frag) = attempt(
                    declined,
                    "subhunk",
                    super::subhunk::try_subhunk_partition(
                        hunk, path, file_old, used_rules, prov, suppressed,
                    ),
                ) {
                    compressed_any = true;
                    out.push_str(&frag);
                } else {
                    out.push_str(hunk);
                }
            }
        }
    }
    if compressed_any {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // WI-8: two consts interleaved with two plain fns (long suffixes so
    // the per-pair semantic hunks beat ordinary bytes under T375).
    const PRE: &str = "const OLD_A: usize = 1001;\nfn old_a(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nconst OLD_B: usize = 1002;\nfn old_b(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\n";
    const POST: &str = "const NEW_A: usize = 1001;\nfn new_a(first_argument: FirstLongTypeArgument) {\n    first_argument\n}\nconst NEW_B: usize = 1002;\nfn new_b(second_argument: SecondLongTypeArgument) {\n    second_argument\n}\n";

    fn pairs() -> (Vec<&'static str>, Vec<&'static str>) {
        (
            vec![
                "const OLD_A: usize = 1001;",
                "fn old_a(first_argument: FirstLongTypeArgument) {",
                "const OLD_B: usize = 1002;",
                "fn old_b(second_argument: SecondLongTypeArgument) {",
            ],
            vec![
                "const NEW_A: usize = 1001;",
                "fn new_a(first_argument: FirstLongTypeArgument) {",
                "const NEW_B: usize = 1002;",
                "fn new_b(second_argument: SecondLongTypeArgument) {",
            ],
        )
    }

    fn pre_map() -> HashMap<String, String> {
        HashMap::from([("f.rs".to_string(), PRE.to_string())])
    }

    fn applied(pre: &HashMap<String, String>, patch: &str) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("partition emission must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("partition must apply cleanly");
        files
    }

    #[test]
    fn partition_fires_and_round_trips() {
        let (minuses, pluses) = pairs();
        let mut used = Vec::new();
        let (out, detector) = try_partition_section(
            &minuses,
            &pluses,
            "f.rs",
            Some(PRE),
            &[1, 2, 5, 6],
            &mut used,
        )
        .expect("all-item section must partition");
        assert_eq!(detector, "item_rename");
        assert_eq!(used.len(), 4);
        assert_eq!(out.matches("rule=").count(), 4, "one hunk per pair:\n{out}");
        assert!(
            out.contains("rule=r1") && out.contains("rule=r4"),
            "positional rules:\n{out}"
        );
        let files = applied(&pre_map(), &out);
        assert_eq!(files["f.rs"], POST, "round-trip must be byte-identical");
    }

    #[test]
    fn partition_vetoes_matrix() {
        let (minuses, pluses) = pairs();
        // Single pair: owned by the single-pair path, never partition.
        let mut used = Vec::new();
        assert!(try_partition_section(
            &minuses[..1],
            &pluses[..1],
            "f.rs",
            Some(PRE),
            &[1],
            &mut used
        )
        .is_none());
        assert!(used.is_empty());
        // Duplicate minus lines: identical pairs belong to verbatim.
        let mut used = Vec::new();
        let dup_m = vec![minuses[0], minuses[0]];
        let dup_p = vec![pluses[0], pluses[0]];
        assert!(
            try_partition_section(&dup_m, &dup_p, "f.rs", Some(PRE), &[1, 1], &mut used).is_none()
        );
        assert!(used.is_empty());
        // One unclaimable pair vetoes the whole section (atomicity) and
        // releases every reservation.
        let mut used = Vec::new();
        let mut bad_m = minuses.clone();
        let mut bad_p = pluses.clone();
        bad_m.push("    indented = true;");
        bad_p.push("    indented = false;");
        assert!(try_partition_section(
            &bad_m,
            &bad_p,
            "f.rs",
            Some(PRE),
            &[1, 2, 5, 6, 3],
            &mut used
        )
        .is_none());
        assert!(used.is_empty());
        // Pair-to-site mapping must be exact.
        let mut used = Vec::new();
        assert!(
            try_partition_section(&minuses, &pluses, "f.rs", Some(PRE), &[1, 2, 4], &mut used)
                .is_none()
        );
        assert!(used.is_empty());
        // The plain-fn gate applies per pair: visibility declines.
        let mut used = Vec::new();
        let mut vis_m = minuses.clone();
        let mut vis_p = pluses.clone();
        vis_m[1] = "pub fn old_a(first_argument: FirstLongTypeArgument) {";
        vis_p[1] = "pub fn new_a(first_argument: FirstLongTypeArgument) {";
        assert!(
            try_partition_section(&vis_m, &vis_p, "f.rs", Some(PRE), &[1, 2, 5, 6], &mut used)
                .is_none()
        );
        assert!(used.is_empty());
        // Return-type headers decline (today's single-pair gate).
        let mut used = Vec::new();
        let mut ret_m = minuses.clone();
        let mut ret_p = pluses.clone();
        ret_m[1] = "fn old_a(first_argument: FirstLongTypeArgument) -> i32 {";
        ret_p[1] = "fn new_a(first_argument: FirstLongTypeArgument) -> i32 {";
        assert!(
            try_partition_section(&ret_m, &ret_p, "f.rs", Some(PRE), &[1, 2, 5, 6], &mut used)
                .is_none()
        );
        assert!(used.is_empty());
    }

    #[test]
    fn partition_section_fires_end_to_end() {
        // Two -U0 hunks (adjacent const+fn pairs merge per hunk), whole
        // balanced: chain and verbatim decline, partition emits 4/4.
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n-const OLD_A: usize = 1001;\n-fn old_a(first_argument: FirstLongTypeArgument) {\n+const NEW_A: usize = 1001;\n+fn new_a(first_argument: FirstLongTypeArgument) {\n@@ -5,2 +5,2 @@\n-const OLD_B: usize = 1002;\n-fn old_b(second_argument: SecondLongTypeArgument) {\n+const NEW_B: usize = 1002;\n+fn new_b(second_argument: SecondLongTypeArgument) {\n";
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let pre = pre_map();
        let out = compress_section(
            section,
            &mut used,
            &pre,
            &mut prov,
            &std::collections::HashSet::new(),
            &mut Vec::new(),
        )
        .expect("partition must fire");
        assert_eq!(out.matches("rule=").count(), 4, "got:\n{out}");
        assert_eq!(prov.len(), 4, "one provenance entry per hunk: {prov:?}");
        assert!(prov.iter().all(|p| p.detector == "item_rename"), "{prov:?}");
        assert!(
            prov.iter().all(|p| p.group.is_empty()),
            "distinct deltas never group: {prov:?}"
        );
        let files = applied(&pre, &out);
        assert_eq!(files["f.rs"], POST);
    }

    #[test]
    fn partition_section_fires_tiny_structs_under_precision() {
        // WI-19: -U0 tiny structs no longer decline under strict T375 —
        // definition-shape singles clear the scaled precision guard
        // (`semantic <= ordinary + 16 * claimed`). Full partition still
        // declines (strict), partial fires with the same bytes.
        let tiny = "struct Old1 {\n    x: i32,\n}\nstruct Old2 {\n    x: i32,\n}\nstruct Old3 {\n    x: i32,\n}\n";
        let section = "diff --git a/g.rs b/g.rs\n--- a/g.rs\n+++ b/g.rs\n@@ -1 +1 @@\n-struct Old1 {\n+struct New1 {\n@@ -4 +4 @@\n-struct Old2 {\n+struct New2 {\n@@ -7 +7 @@\n-struct Old3 {\n+struct New3 {\n";
        let mut pre = HashMap::new();
        pre.insert("g.rs".to_string(), tiny.to_string());
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = compress_section(
            section,
            &mut used,
            &pre,
            &mut prov,
            &std::collections::HashSet::new(),
            &mut Vec::new(),
        )
        .expect("precision guard must fire");
        assert_eq!(out.matches("rule=").count(), 3, "got:\n{out}");
        assert!(prov.iter().all(|p| p.detector == "item_rename"), "{prov:?}");
        let mut files = pre.clone();
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["g.rs"],
            "struct New1 {\n    x: i32,\n}\nstruct New2 {\n    x: i32,\n}\nstruct New3 {\n    x: i32,\n}\n"
        );
    }

    #[test]
    fn partition_section_atomicity_stays_ordinary() {
        // One hunk, three pairs: two renames plus one ordinary edit — the
        // edit vetoes partition, the chain declines hetero pairs, and the
        // per-hunk fallback declines the mixed hunk: fully ordinary.
        // WI-4 sub-hunk keeps this pin only via the pre-image: the
        // top-level `let` makes it unparseable, so every single declines
        // and no run proves. Valid-Rust mixed hunks now split per run
        // (see the `subhunk` tests and diffrs fixture `129`).
        // WI-19 partial keeps this posture: a single mixed hunk never
        // splits (pinned at the function level below too).
        let pre = "const OLD_A: usize = 1001;\nconst OLD_B: usize = 1002;\nlet x = 1;\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,3 @@\n-const OLD_A: usize = 1001;\n-const OLD_B: usize = 1002;\n-let x = 1;\n+const NEW_A: usize = 1001;\n+const NEW_B: usize = 1002;\n+let x = 2;\n";
        let mut files = HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(compress_section(
            section,
            &mut used,
            &files,
            &mut prov,
            &std::collections::HashSet::new(),
            &mut Vec::new()
        )
        .is_none());
        assert!(used.is_empty());
        assert!(prov.is_empty());
        // Function level: the same single mixed hunk declines partial too.
        let mut used = Vec::new();
        assert!(super::super::partial::try_partial_partition_section(
            section,
            "f.rs",
            Some(pre),
            &mut used
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn suppressed_joint_callee_stays_ordinary() {
        // WI-4a: a vetoed joint suppresses its old callee name for the
        // file — the per-hunk fallback leaves the pure call hunk
        // ordinary instead of emitting a rename-claiming sweep.
        let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n";
        let mut files = HashMap::new();
        files.insert("f.rs".to_string(), pre.to_string());
        let suppressed: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::from([("f.rs".to_string(), "foo".to_string())]);
        let mut used = Vec::new();
        let mut prov = Vec::new();
        assert!(compress_section(
            section,
            &mut used,
            &files,
            &mut prov,
            &suppressed,
            &mut Vec::new()
        )
        .is_none());
        assert!(used.is_empty());
        assert!(prov.is_empty());
        // Without the suppression the same call hunk still sweeps (the
        // veto is what makes it ordinary, never a blanket call decline).
        let mut used = Vec::new();
        let mut prov = Vec::new();
        let out = compress_section(
            section,
            &mut used,
            &files,
            &mut prov,
            &std::collections::HashSet::new(),
            &mut Vec::new(),
        )
        .expect("unsuppressed call hunk must sweep");
        assert!(out.contains("- foo(E0)"), "got:\n{out}");
    }
}
