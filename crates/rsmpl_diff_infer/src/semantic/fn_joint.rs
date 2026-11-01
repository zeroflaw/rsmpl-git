//! WI-4a fn rename+slot joint intent (SPEC D.4 `fn_joint`).
//!
//! A `fn` definition renamed *while* its header evolves (params or
//! return slot — SPEC 14.7 header evolution) joined with N-1 call-callee
//! use lines of the old name compresses to the single definition hunk
//! *with* `# follow=uses` + `# sweep`, the header pair carrying both
//! deltas. The engine follow rewrites the callee names at apply time,
//! so the emission is one hunk — but only when the call pairs are
//! callee-only (arguments token-identical): the follow pass never adds
//! arguments, and a single hunk claiming otherwise would fail its own
//! probe (D13).
//!
//! Claim-then-veto suppresses instead of fragmenting: when the joint
//! candidate is present but any fire gate fails (body/attr delta,
//! arg-delta calls, vis/qual/generics/where delta, twin defs, probe or
//! T375 fail), the old callee name is suppressed for its file — the
//! per-hunk fallback leaves those call hunks ordinary instead of
//! emitting a rename-claiming sweep over a definition delta it cannot
//! see (the F4b/F4d false-completeness shape). Suppression is keyed per
//! `(path, old)`: only sections that took part in the vetoed candidate.
//!
//! Scope (WI-4b landed, WI-4c residue): single-file groups only
//! (multi-file candidate groups decline silently); Joint-A fires on
//! evolving ⊆ {params, return} with callee-only calls; Joint-B1 fires
//! on a params-only evolution with a uniform arg-add call template;
//! Joint-B2 fires on an attr add/remove + rename with a uniform
//! callee-only call template (each a shared-rule two-hunk composition,
//! no follow — a follow would overlap the call hunk's sites). Body
//! deltas, return-plus-arg-add doubles, slot-plus-attr doubles, and
//! hetero call shapes suppress without raising (their composition has
//! no probe-sound two-hunk form).

use std::collections::{HashMap, HashSet};

use super::generalized::parse_sig_slots;
use super::item_intent::{count_kind_defs, probe_reproduces, Pair};
use super::unique_rule;

/// One paired hunk inside a section.
struct HunkPairs {
    minuses: Vec<String>,
    pluses: Vec<String>,
    balanced: bool,
    sites: Vec<usize>,
}

/// One flattened native pair at its old-side site.
#[derive(Clone)]
struct FlatPair {
    sec: usize,
    hunk: usize,
    minus: String,
    plus: String,
    site: usize,
}

/// The whole-patch pre-pass outcome: rebuilt sections, the definition
/// path and rule for provenance (both empty on suppress-only), and the
/// suppressed `(path, old-name)` set.
type JointOutcome = (Vec<String>, String, String, HashSet<(String, String)>);

/// Which joint shape fired.
enum Joint {
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
}

/// A validated fire payload: the def pair plus its call claims, all in
/// one file. Joint-A claims ride one follow hunk; Joint-B claims ride
/// the shared-rule two-hunk composition (def hunk + call-template hunk
/// under one rule name).
struct Fire {
    old: String,
    new: String,
    def_minus: String,
    def_plus: String,
    claims: Vec<FlatPair>,
    claimed_secs: Vec<usize>,
    joint: Joint,
}

/// Whole-patch fn-joint pre-pass: returns the rebuilt section list plus
/// the definition path and rule for provenance, and the suppressed
/// `(path, old-name)` set. `None` when no file holds a joint
/// candidate. A suppress-only outcome returns the sections
/// byte-identical with an empty rule — the caller keeps the sections
/// and threads only the suppression set. Reserved rule names are
/// released on decline.
pub(crate) fn try_fn_joint_intent(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<JointOutcome> {
    let checkpoint = used_rules.len();
    // Parse eligible sections (`.rs` non-rename sections only).
    let mut parsed: Vec<(usize, String, Vec<HunkPairs>)> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        // WI-4d: pre-pass emissions never re-enter later pre-passes.
        if super::is_semantic_section(sec) {
            continue;
        }
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        parsed.push((i, path, split_hunks(sec)));
    }
    if parsed.is_empty() {
        return None;
    }
    // Group parsed-section keys by path (single-file groups only).
    let mut by_path: HashMap<&str, Vec<usize>> = HashMap::new();
    for (k, (_, path, _)) in parsed.iter().enumerate() {
        by_path.entry(path.as_str()).or_default().push(k);
    }
    let mut suppressed: HashSet<(String, String)> = HashSet::new();
    let mut fire: Option<(Box<Fire>, String, String)> = None;
    for (path, keys) in &by_path {
        if pre.get(*path).is_none() {
            continue;
        }
        match joint_candidate(&parsed, keys, path, pre) {
            Candidate::None => {}
            Candidate::Suppress(old) => {
                suppressed.insert((path.to_string(), old));
            }
            Candidate::Ready(f) => {
                if fire.is_some() {
                    // Two fireable joints in one patch: the second is
                    // suppressed instead (one joint intent per pre-pass
                    // keeps rule provenance unambiguous).
                    suppressed.insert((path.to_string(), f.old));
                    continue;
                }
                let rule = unique_rule(used_rules, &format!("rename_{}_to_{}", f.old, f.new));
                let emission = emit(path, &rule, &f);
                let probed = match &f.joint {
                    Joint::A | Joint::B1 { .. } => {
                        let pairs: Vec<Pair> = f
                            .claims
                            .iter()
                            .map(|c| Pair {
                                sec: 0,
                                path: path.to_string(),
                                minus: c.minus.clone(),
                                plus: c.plus.clone(),
                                site: c.site,
                            })
                            .collect();
                        probe_reproduces(pre, &emission, &pairs)
                    }
                    Joint::B2 { .. } => {
                        let calls: Vec<(usize, String)> =
                            f.claims.iter().map(|c| (c.site, c.plus.clone())).collect();
                        probe_attr_joint(pre, &emission, path, &f, &calls)
                    }
                };
                if !probed {
                    used_rules.truncate(checkpoint);
                    suppressed.insert((path.to_string(), f.old));
                    continue;
                }
                let claimed_bytes: usize = keys.iter().map(|k| sections[parsed[*k].0].len()).sum();
                if emission.len() >= claimed_bytes {
                    used_rules.truncate(checkpoint);
                    suppressed.insert((path.to_string(), f.old));
                    continue;
                }
                fire = Some((f, path.to_string(), rule));
            }
        }
    }
    match fire {
        Some((f, def_path, rule)) => {
            let emission = emit(&def_path, &rule, &f);
            let claimed: HashSet<usize> = f.claimed_secs.iter().copied().collect();
            let first = *claimed.iter().min().unwrap_or(&0);
            let mut out: Vec<String> = Vec::new();
            for (i, sec) in sections.iter().enumerate() {
                if !claimed.contains(&i) {
                    out.push(sec.clone());
                    continue;
                }
                if i == first {
                    out.push(emission.clone());
                }
            }
            Some((out, def_path, rule, suppressed))
        }
        None => {
            if suppressed.is_empty() {
                used_rules.truncate(checkpoint);
                None
            } else {
                Some((sections.to_vec(), String::new(), String::new(), suppressed))
            }
        }
    }
}

/// Candidate verdict for one single-file group.
enum Candidate {
    /// No joint shape here (pure rename, no def pair, twin defs, no
    /// pre-image, …) — existing behavior untouched, no suppression.
    None,
    /// Joint shape present but vetoed — suppress the old callee name.
    Suppress(String),
    /// Fireable joint payload (boxed: the B2 shape carries two
    /// templates plus the attr line).
    Ready(Box<Fire>),
}

/// Classify one single-file group (sections `keys` of `parsed`, all the
/// same `path`).
fn joint_candidate(
    parsed: &[(usize, String, Vec<HunkPairs>)],
    keys: &[usize],
    path: &str,
    pre: &HashMap<String, String>,
) -> Candidate {
    // Positional pairs from balanced hunks, plus header-pair candidates
    // scanned from unbalanced hunks (an unbalanced attr-shaped def hunk
    // fires as Joint-B2; any other unbalanced def hunk is a joint shape
    // that must suppress the fragmented fallback).
    let mut unbalanced = false;
    let mut flat: Vec<FlatPair> = Vec::new();
    // (sec, hunk, minus, plus) header pairs found only via the
    // unbalanced scan (no sites — never fireable as Joint-A/B1).
    // Full minus/plus runs of every unbalanced hunk (Joint-B2 shapes).
    let mut combo_defs: Vec<(usize, usize, String, String)> = Vec::new();
    let mut unbalanced_hunks: Vec<(usize, usize, Vec<String>, Vec<String>)> = Vec::new();
    for k in keys {
        let (sec, _, hunks) = &parsed[*k];
        for (h, hp) in hunks.iter().enumerate() {
            if !hp.balanced {
                unbalanced = true;
                unbalanced_hunks.push((*sec, h, hp.minuses.clone(), hp.pluses.clone()));
                for m in &hp.minuses {
                    for p in &hp.pluses {
                        if header_name_delta(m, p).is_some() {
                            combo_defs.push((*sec, h, m.clone(), p.clone()));
                        }
                    }
                }
                continue;
            }
            for ((m, p), s) in hp.minuses.iter().zip(hp.pluses.iter()).zip(hp.sites.iter()) {
                flat.push(FlatPair {
                    sec: *sec,
                    hunk: h,
                    minus: m.clone(),
                    plus: p.clone(),
                    site: *s,
                });
            }
        }
    }
    // Exactly one `fn`-header pair with a name delta (column-0 only —
    // indented headers are methods, WI-13 residue).
    let mut def: Option<usize> = None;
    for (i, fp) in flat.iter().enumerate() {
        if header_name_delta(&fp.minus, &fp.plus).is_some() {
            if def.is_some() {
                return Candidate::None;
            }
            def = Some(i);
        }
    }
    // A combo-scanned def plus a positional one (or two) is ambiguity,
    // never a joint.
    if def.is_some() && !combo_defs.is_empty() {
        return Candidate::None;
    }
    if def.is_none() && combo_defs.len() != 1 {
        return Candidate::None;
    }
    let (old, new, def_single) = match def {
        Some(def_idx) => {
            let (old, new) = header_name_delta(&flat[def_idx].minus, &flat[def_idx].plus)
                .expect("def pair re-parses");
            // The def hunk holding exactly the header pair is the only
            // fireable shape; extra pairs in the same hunk are body/attr
            // lines of the definition (still a joint shape, suppress).
            let h = flat[def_idx].hunk;
            let s = flat[def_idx].sec;
            let len = flat.iter().filter(|fp| fp.sec == s && fp.hunk == h).count();
            (old, new, len == 1)
        }
        None => {
            let (_, _, m, p) = &combo_defs[0];
            let (old, new) = header_name_delta(m, p).expect("combo def re-parses");
            (old, new, false)
        }
    };
    // A pure rename (header slots equal in a single-pair def hunk) is
    // item_intent's business — never a joint candidate, never
    // suppressed. A header-identical pair with extra def-hunk lines
    // (F4b body-literal shape) IS a joint shape: the definition delta
    // lives outside the header pair.
    if def_single && def.is_some_and(|d| slots_equal_except_name(&flat[d].minus, &flat[d].plus)) {
        return Candidate::None;
    }
    // At least one call-callee pair of old→new must exist.
    let mut call_count = 0usize;
    for (i, fp) in flat.iter().enumerate() {
        if def.is_some_and(|d| d == i) {
            continue;
        }
        if is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            call_count += 1;
        }
    }
    if call_count == 0 {
        return Candidate::None;
    }
    // Unique-definition guard (same posture as item_intent): twin `fn
    // old` defs decline silently (no suppression — a twin file may
    // hold a legitimate independent sweep).
    let probe_pair = Pair {
        sec: 0,
        path: path.to_string(),
        minus: String::new(),
        plus: String::new(),
        site: 0,
    };
    if count_kind_defs(pre, std::slice::from_ref(&probe_pair), "fn", &old) != 1 {
        return Candidate::None;
    }
    // Any veto from here on suppresses: the file holds a joint shape
    // the raiser cannot prove, and the fragmented fallback (ordinary
    // def + rename-claiming sweep) must not stand in for it.
    //
    // WI-4b Joint-B2 (no positional def): an unbalanced attr-shaped
    // def hunk plus uniform callee-only calls composes to the
    // shared-rule two-hunk form. Anything else unbalanced suppresses.
    if !def_single {
        if unbalanced {
            // Lone call sites (N=1) never pay the second hunk: the
            // unique-single detector would re-harvest the call-template
            // hunk on the emission's second pass and fragment the
            // intent (pinned) — suppress like every other veto.
            if call_count >= 2 {
                if let Some(mut joint_fire) =
                    attr_joint_fire(path, pre, &unbalanced_hunks, &flat, &old, &new)
                {
                    joint_fire.claimed_secs = keys.iter().map(|k| parsed[*k].0).collect();
                    return Candidate::Ready(Box::new(joint_fire));
                }
            }
        }
        return Candidate::Suppress(old);
    }
    if unbalanced {
        return Candidate::Suppress(old);
    }
    let def_idx = def.expect("def_single implies a positional def");
    // Every other pair must be a call-callee site of old→new.
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if !is_call_site_of(&fp.minus, &fp.plus, &old, &new) {
            return Candidate::Suppress(old);
        }
    }
    let callee_only = flat
        .iter()
        .enumerate()
        .all(|(i, fp)| i == def_idx || is_callee_only(&fp.minus, &fp.plus, &old, &new));
    let joint = if callee_only {
        // Joint-A (WI-4a) fire gates on the header: vis in the WI-1
        // closed set and preserved; qual/generics/where preserved;
        // evolving ⊆ {params, return}, nonempty.
        if !header_fire_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
            return Candidate::Suppress(old);
        }
        Joint::A
    } else {
        // WI-4b Joint-B1: every call adds the same concrete trailing
        // args (uniform template below); the header must evolve params
        // only with the return preserved — a return delta plus an
        // arg-add is two evolutions the shared-rule pair cannot
        // restate. Lone additions (N=1) never pay the second hunk.
        if flat.len() < 3 {
            return Candidate::Suppress(old);
        }
        if !header_argadd_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
            return Candidate::Suppress(old);
        }
        match arg_add_template(&flat, def_idx, &old, &new) {
            Some((call_minus, call_plus)) => Joint::B1 {
                call_minus,
                call_plus,
            },
            None => return Candidate::Suppress(old),
        }
    };
    let def_minus = flat[def_idx].minus.clone();
    let def_plus = flat[def_idx].plus.clone();
    let mut claimed_secs: Vec<usize> = Vec::new();
    for k in keys {
        claimed_secs.push(parsed[*k].0);
    }
    Candidate::Ready(Box::new(Fire {
        old,
        new,
        def_minus,
        def_plus,
        claims: flat,
        claimed_secs,
        joint,
    }))
}

/// Split one native section into per-hunk minus/plus runs with their
/// old-side sites (0-context `-U0` hunks hold pure `-`/`+` runs, but
/// context lines are skipped soundly when present).
fn split_hunks(section: &str) -> Vec<HunkPairs> {
    let mut out: Vec<HunkPairs> = Vec::new();
    let mut current: Option<(Vec<String>, Vec<String>, Vec<String>)> = None;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if let Some((raw, minuses, pluses)) = current.take() {
                out.push(finish_hunk(raw, minuses, pluses));
            }
            current = Some((vec![line.to_string()], Vec::new(), Vec::new()));
            continue;
        }
        if let Some((raw, minuses, pluses)) = current.as_mut() {
            raw.push(line.to_string());
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
    }
    if let Some((raw, minuses, pluses)) = current.take() {
        out.push(finish_hunk(raw, minuses, pluses));
    }
    out
}

/// Pair one hunk positionally (the whole-section balanced gate in
/// `section_pairs` style, per hunk): balanced iff every `-` has its
/// `+` in global order.
fn finish_hunk(raw: Vec<String>, minuses: Vec<String>, pluses: Vec<String>) -> HunkPairs {
    let balanced = minuses.len() == pluses.len() && !minuses.is_empty();
    let sites = if balanced {
        crate::totality::changed_old_lines(&raw.join("\n"))
    } else {
        Vec::new()
    };
    HunkPairs {
        minuses,
        pluses,
        balanced,
        sites,
    }
}

/// The `(old, new)` fn names when both lines parse as column-0 `fn`
/// headers with differing names, else `None`. Column-0 only: indented
/// headers are methods (WI-13 residue), and `split_item` def lines
/// (`struct`/`type`/…) never parse as `fn` headers.
fn header_name_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    if minus.is_empty()
        || plus.is_empty()
        || minus.starts_with([' ', '\t'])
        || plus.starts_with([' ', '\t'])
    {
        return None;
    }
    let m = parse_sig_slots(minus)?;
    let p = parse_sig_slots(plus)?;
    if m.name == p.name || m.name.is_empty() || p.name.is_empty() {
        return None;
    }
    Some((m.name, p.name))
}

/// True when both lines parse as `fn` headers whose every slot matches
/// (a pure rename — item_intent's domain, never joint).
fn slots_equal_except_name(minus: &str, plus: &str) -> bool {
    let (Some(m), Some(p)) = (parse_sig_slots(minus), parse_sig_slots(plus)) else {
        return false;
    };
    m.vis == p.vis
        && m.qual == p.qual
        && m.generics == p.generics
        && m.params == p.params
        && m.ret == p.ret
        && m.where_ == p.where_
}

/// Fire gates on the def header pair: vis in the WI-1 closed set and
/// preserved; qual/generics/where preserved; evolving ⊆ {params,
/// return} and nonempty.
fn header_fire_gates(minus: &str, plus: &str) -> bool {
    let (Some(m), Some(p)) = (parse_sig_slots(minus), parse_sig_slots(plus)) else {
        return false;
    };
    if !is_fn_vis_prefix(&m.vis) || m.vis != p.vis {
        return false;
    }
    // Self-referential vis (`pub(in foo)` renaming `foo`) is an
    // evolution, never a pure-slot joint (WI-1 posture, kept).
    if contains_ident(&m.vis, &m.name) {
        return false;
    }
    if m.qual != p.qual || m.generics != p.generics || m.where_ != p.where_ {
        return false;
    }
    m.params != p.params || m.ret != p.ret
}

/// WI-4b Joint-B1 header gates: vis in the WI-1 closed set and
/// preserved; qual/generics/where preserved; params evolve, return
/// preserved (a return delta plus an arg-add is two evolutions the
/// shared-rule pair cannot restate — suppressed, pinned).
fn header_argadd_gates(minus: &str, plus: &str) -> bool {
    let (Some(m), Some(p)) = (parse_sig_slots(minus), parse_sig_slots(plus)) else {
        return false;
    };
    if !is_fn_vis_prefix(&m.vis) || m.vis != p.vis {
        return false;
    }
    if contains_ident(&m.vis, &m.name) {
        return false;
    }
    if m.qual != p.qual || m.generics != p.generics || m.where_ != p.where_ {
        return false;
    }
    if m.ret != p.ret {
        return false;
    }
    m.params != p.params
}

/// Qualifier prefix for a call-template path: `""`, `"::"`, or
/// `"{q}::"`.
fn prefix_qual(qual: &str, callee: &str) -> String {
    if qual.is_empty() {
        callee.to_string()
    } else if qual.ends_with("::") {
        format!("{qual}{callee}")
    } else {
        format!("{qual}::{callee}")
    }
}

/// WI-4b: uniform arg-add template over the non-def pairs (indices
/// `!= def_idx`): every call renames old→new with an equal qualifier,
/// and every plus side extends the token-identical minus prefix by the
/// same non-empty concrete suffix. Returns the `(call_minus,
/// call_plus)` pattern pair, or `None` on any hetero shape (prefix
/// rewrite, hetero suffix/arity/qual/turbo — suppressed, pinned).
fn arg_add_template(
    flat: &[FlatPair],
    def_idx: usize,
    old: &str,
    new: &str,
) -> Option<(String, String)> {
    let mut qual: Option<String> = None;
    let mut turbo: Option<String> = None;
    let mut arity: Option<usize> = None;
    let mut suffix: Option<Vec<String>> = None;
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        let (qm, cm, tm, am) = call_parts(&fp.minus)?;
        let (qp, cp, tp, ap) = call_parts(&fp.plus)?;
        if cm != old || cp != new || qm != qp {
            return None;
        }
        if ap.len() <= am.len() {
            return None;
        }
        if !am.iter().zip(ap.iter()).all(|(a, b)| a == b) {
            return None;
        }
        if tm != tp {
            return None;
        }
        let grown: Vec<String> = ap[am.len()..].to_vec();
        if grown.is_empty() {
            return None;
        }
        if qual.as_ref().is_some_and(|q| *q != qm)
            || turbo.as_ref().is_some_and(|t| *t != tm)
            || arity.is_some_and(|k| k != am.len())
            || suffix.as_ref().is_some_and(|s| *s != grown)
        {
            return None;
        }
        qual = Some(qm);
        turbo = Some(tm);
        arity = Some(am.len());
        suffix = Some(grown);
    }
    let (q, t, k, s) = (qual?, turbo?, arity?, suffix?);
    let holes: Vec<String> = (0..k).map(|i| format!("E{i}")).collect();
    let minus_args = holes.join(", ");
    let plus_args = if holes.is_empty() {
        s.join(", ")
    } else {
        format!("{}, {}", holes.join(", "), s.join(", "))
    };
    let old_path = super::shape::literal_path_last(&prefix_qual(&q, old));
    let new_path = super::shape::literal_path_last(&prefix_qual(&q, new));
    Some((
        format!("{old_path}{t}({minus_args})"),
        format!("{new_path}{t}({plus_args})"),
    ))
}

/// One single-line `#[…]` attribute (doc comments and multi-attr lines
/// are not joint shapes — suppressed, pinned).
fn is_attr_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("#[") && t.ends_with(']') && t.len() > 3 && line.starts_with('#')
}

/// WI-4b Joint-B2 attr shape: the single unbalanced hunk of the group
/// holds exactly one `fn`-header pair with a name delta plus exactly
/// one extra `#[…]` attr line (add: 1 minus / 2 pluses; remove: 2
/// minus / 1 plus). Returns `(attr_line, attr_add, def_minus,
/// def_plus)`.
fn attr_def_shape(minuses: &[String], pluses: &[String]) -> Option<(String, bool, String, String)> {
    let (attr_line, attr_add, def_minus, def_plus) = match (minuses, pluses) {
        ([m], [a, p]) => (a.clone(), true, m.clone(), p.clone()),
        ([a, m], [p]) => (a.clone(), false, m.clone(), p.clone()),
        _ => return None,
    };
    if !is_attr_line(&attr_line) {
        return None;
    }
    let (old, new) = header_name_delta(&def_minus, &def_plus)?;
    let (Some(m), Some(p)) = (parse_sig_slots(&def_minus), parse_sig_slots(&def_plus)) else {
        return None;
    };
    if m.name != old || p.name != new {
        return None;
    }
    // Rename-only header: every slot preserved (a slot evolution plus
    // an attr delta is two evolutions — suppressed, pinned).
    if m.vis != p.vis
        || m.qual != p.qual
        || m.generics != p.generics
        || m.params != p.params
        || m.ret != p.ret
        || m.where_ != p.where_
    {
        return None;
    }
    if !is_fn_vis_prefix(&m.vis) || contains_ident(&m.vis, &m.name) {
        return None;
    }
    Some((attr_line.trim().to_string(), attr_add, def_minus, def_plus))
}

/// WI-4b Joint-B2 fire attempt for a group with no positional def:
/// exactly one unbalanced hunk (attr-shaped) plus balanced uniform
/// callee-only call pairs. `None` keeps the caller's suppress-only
/// posture.
fn attr_joint_fire(
    path: &str,
    pre: &HashMap<String, String>,
    unbalanced: &[(usize, usize, Vec<String>, Vec<String>)],
    flat: &[FlatPair],
    old: &str,
    new: &str,
) -> Option<Fire> {
    if unbalanced.len() != 1 {
        return None;
    }
    let (_, _, minuses, pluses) = &unbalanced[0];
    let (attr_line, attr_add, def_minus, def_plus) = attr_def_shape(minuses, pluses)?;
    // Every other pair is a uniform callee-only call site of old→new
    // (one template must state them all — hetero arity/qual/turbo
    // suppresses, pinned).
    if flat.is_empty() {
        return None;
    }
    let mut arity: Option<usize> = None;
    let mut qual: Option<String> = None;
    let mut turbo: Option<String> = None;
    for fp in flat {
        let (qm, cm, tm, am) = call_parts(&fp.minus)?;
        let (qp, cp, tp, ap) = call_parts(&fp.plus)?;
        if cm != old || cp != new || qm != qp {
            return None;
        }
        if tm != tp || am != ap {
            return None;
        }
        if arity.is_some_and(|k| k != am.len())
            || qual.as_ref().is_some_and(|q| *q != qm)
            || turbo.as_ref().is_some_and(|t| *t != tm)
        {
            return None;
        }
        arity = Some(am.len());
        qual = Some(qm);
        turbo = Some(tm);
    }
    let (q, t, k) = (qual?, turbo?, arity?);
    // Locate the header site in the pre-image (unique by the def
    // guard); an attr-remove additionally proves the attr line sits
    // directly above it.
    let pre_text = pre.get(path)?;
    let head_site = pre_text
        .lines()
        .enumerate()
        .find(|(_, l)| l.trim() == def_minus.trim())
        .map(|(i, _)| i + 1)?;
    if !attr_add {
        if head_site < 2 {
            return None;
        }
        let above = pre_text.lines().nth(head_site - 2)?;
        if above.trim() != attr_line {
            return None;
        }
    }
    let holes: Vec<String> = (0..k).map(|i| format!("E{i}")).collect();
    let args = holes.join(", ");
    let old_path = super::shape::literal_path_last(&prefix_qual(&q, old));
    let new_path = super::shape::literal_path_last(&prefix_qual(&q, new));
    Some(Fire {
        old: old.to_string(),
        new: new.to_string(),
        def_minus,
        def_plus,
        claims: flat.to_vec(),
        claimed_secs: Vec::new(),
        joint: Joint::B2 {
            attr_line,
            attr_add,
            head_site,
            call_minus: format!("{old_path}{t}({args})"),
            call_plus: format!("{new_path}{t}({args})"),
        },
    })
}

/// Leading whitespace of a line (the probe re-indents claimed plus
/// lines with the native line's own indent).
fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// WI-4b Joint-B2 scratch-apply probe (D13): the two-hunk emission
/// applied to the pre-image must equal the splice-expected post text —
/// the attr line inserted/removed at the header site, the header
/// swapped, each call site showing its native plus line, every other
/// line byte-identical (the insertion/removal shifts lines after the
/// header site by one). The shared `probe_reproduces` cannot state
/// this (it requires equal line counts), so the joint owns its probe.
/// The whole-composition comparison is authoritative; the def hunk's
/// per-hunk orphans under `--check` are sibling-covered by
/// construction (SPEC D.4).
fn probe_attr_joint(
    pre: &HashMap<String, String>,
    emission: &str,
    path: &str,
    fire: &Fire,
    calls: &[(usize, String)],
) -> bool {
    let Joint::B2 {
        attr_line,
        attr_add,
        head_site,
        ..
    } = &fire.joint
    else {
        return false;
    };
    let (head_site, attr_add) = (*head_site, *attr_add);
    let (def_minus, def_plus) = (&fire.def_minus, &fire.def_plus);
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    let (Some(post), Some(pre_text)) = (files.get(path), pre.get(path)) else {
        return false;
    };
    let pre_lines: Vec<&str> = pre_text.lines().collect();
    if head_site == 0 || head_site > pre_lines.len() {
        return false;
    }
    if !attr_add && head_site < 2 {
        return false;
    }
    if pre_lines[head_site - 1].trim() != def_minus.trim() {
        return false;
    }
    let call_map: HashMap<usize, String> = calls
        .iter()
        .map(|(s, p)| (*s, p.trim().to_string()))
        .collect();
    let mut expected: Vec<String> = Vec::new();
    for (idx, line) in pre_lines.iter().enumerate() {
        let no = idx + 1;
        if attr_add && no == head_site {
            expected.push(attr_line.trim().to_string());
            expected.push(def_plus.trim().to_string());
            continue;
        }
        if !attr_add && no == head_site - 1 {
            continue;
        }
        if !attr_add && no == head_site {
            expected.push(def_plus.trim().to_string());
            continue;
        }
        if let Some(want) = call_map.get(&no) {
            expected.push(format!("{}{}", indent_of(line), want));
        } else {
            expected.push(line.to_string());
        }
    }
    let post_lines: Vec<&str> = post.lines().collect();
    post_lines.len() == expected.len()
        && post_lines.iter().zip(expected.iter()).all(|(p, e)| *p == e)
}
/// WI-1 closed vis set the terse `fn` rename admits (SPEC 14.1 diet
/// form — vis rides literally, never a `V` hole).
fn is_fn_vis_prefix(vis: &str) -> bool {
    if vis.is_empty() {
        return true;
    }
    if vis == "pub" || vis == "pub(crate)" || vis == "pub(super)" {
        return true;
    }
    let Some(inner) = vis
        .strip_prefix("pub(in ")
        .and_then(|r| r.strip_suffix(')'))
    else {
        return false;
    };
    let mut segs = inner.split("::").peekable();
    let mut first = true;
    while let Some(seg) = segs.next() {
        if seg.is_empty() {
            return false;
        }
        let head_ok =
            first && matches!(seg, "crate" | "self" | "super" | "$crate") && segs.peek().is_some();
        if !head_ok && super::ident_len(seg) != Some(seg.len()) {
            return false;
        }
        first = false;
    }
    !first
}

/// Ident-boundary containment (never substring).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                return true;
            }
            token.clear();
        }
    }
    false
}

/// One call-callee site of `old`→`new`: both sides are the same call
/// shape (bare, qualified, turbofish, `let`-wrapped) whose callee head
/// swaps `old`→`new` with an equal qualifier. Argument deltas are
/// allowed here (they force suppress-only downstream); use lines,
/// method calls, fields, and macro heads return false.
fn is_call_site_of(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    let (Some((qm, cm, _, _)), Some((qp, cp, _, _))) = (call_parts(minus), call_parts(plus)) else {
        return false;
    };
    qm == qp && cm == old && cp == new
}

/// True when the pair is a callee-only rename of `old`→`new`:
/// qualifier, turbofish, and every argument token-identical.
fn is_callee_only(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    if !is_call_site_of(minus, plus, old, new) {
        return false;
    }
    let (Some((_, _, tm, am)), Some((_, _, tp, ap))) = (call_parts(minus), call_parts(plus)) else {
        return false;
    };
    tm == tp && am == ap
}

/// `(qualifier, callee ident, turbofish, args)` of a call expression,
/// unwrapping one `let` init. `None` for method calls, macros,
/// non-calls, and qualified-self paths.
fn call_parts(line: &str) -> Option<(String, String, String, Vec<String>)> {
    let mut text = line.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return None;
    }
    // Two separate attempts (item_intent posture): a `let` init
    // stripped of its `;` parses as `Expr::Let`, which must not shadow
    // the `Stmt::Local` path — each attempt must yield a call itself.
    if let Ok(syn::Expr::Call(call)) = syn::parse_str::<syn::Expr>(text) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            if path.qself.is_none() {
                return call_parts_of(path, &call.args);
            }
        }
        return None;
    }
    let syn::Stmt::Local(local) = syn::parse_str::<syn::Stmt>(line.trim()).ok()? else {
        return None;
    };
    let init = local.init?;
    let syn::Expr::Call(call) = init.expr.as_ref() else {
        return None;
    };
    let syn::Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    call_parts_of(path, &call.args)
}

/// `(qualifier, callee ident, turbofish, args)` from a proven call path.
fn call_parts_of(
    path: &syn::ExprPath,
    args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
) -> Option<(String, String, String, Vec<String>)> {
    let mut segs = path.path.segments.iter();
    let mut qual: Vec<String> = Vec::new();
    let mut last = segs.next()?;
    for seg in segs {
        qual.push(quote::quote!(#last).to_string());
        last = seg;
    }
    // Method calls (`recv.method()`) parse as `ExprMethodCall`, never
    // here — but a path callee with a `self`-style receiver cannot
    // occur; no extra gate needed.
    // Canonical `::<…>` spelling (the turbofish-sweep precedent):
    // `quote!` renders `:: < u8 >` with spaces, which the probe's
    // trim comparison would read as a rewrite of the native
    // `::<u8>` line — the template must spell what the tree holds.
    let turbo = match &last.arguments {
        syn::PathArguments::None => String::new(),
        syn::PathArguments::AngleBracketed(g) => {
            let inner: Vec<String> = g
                .args
                .iter()
                .map(|a| quote::quote!(#a).to_string())
                .collect();
            format!("::<{}>", inner.join(", "))
        }
        args => quote::quote!(#args).to_string(),
    };
    let mut qualifier = String::new();
    if path.path.leading_colon.is_some() {
        qualifier.push_str("::");
    }
    qualifier.push_str(&qual.join("::"));
    let rendered: Vec<String> = args.iter().map(|a| quote::quote!(#a).to_string()).collect();
    Some((qualifier, last.ident.to_string(), turbo, rendered))
}

/// True when `line` is a call-callee site of `old` (any new spelling) —
/// the per-hunk suppression probe (plus side unconstrained).
pub(crate) fn is_call_callee_of(line: &str, old: &str) -> bool {
    let Some((_, callee, _, _)) = call_parts(line) else {
        return false;
    };
    callee == old
}

/// Emit the joint emission under `rule`: the header pair rebuilt from
/// the native def lines (`{` → `{ ... }`, native spacing preserved),
/// plus — for Joint-B — the call-template hunk under the same rule.
/// Joint-A rides one hunk with `# sweep` + `# follow=uses`; Joint-B1/B2
/// ride two hunks with `# sweep` and no follow (a follow would claim
/// the call hunk's sites and the lowering would refuse the overlap).
fn emit(path: &str, rule: &str, fire: &Fire) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    match &fire.joint {
        Joint::A => {
            out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            out.push_str("# sweep\n");
            out.push_str("# follow=uses\n");
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
        }
        Joint::B1 {
            call_minus,
            call_plus,
        } => {
            out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            out.push_str("# sweep\n");
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
            out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            out.push_str("# sweep\n");
            out.push_str(&format!("- {call_minus}\n+ {call_plus}\n"));
        }
        Joint::B2 {
            attr_line,
            attr_add,
            call_minus,
            call_plus,
            ..
        } => {
            out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            out.push_str("# sweep\n");
            if !*attr_add {
                out.push_str(&format!("- {attr_line}\n"));
            }
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            if *attr_add {
                out.push_str(&format!("+ {attr_line}\n"));
            }
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
            out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
            out.push_str("# sweep\n");
            out.push_str(&format!("- {call_minus}\n+ {call_plus}\n"));
        }
    }
    out
}

/// `fn foo(x: i32) {` → `fn foo(x: i32) { ... }` (the engine's fn
/// pattern needs the `{ ... }` body hole; the header text stays
/// native so the lowering re-renders byte-identically on formatted
/// code and the probe vetoes otherwise).
fn with_body_hole(header_line: &str) -> String {
    let trimmed = header_line.trim_end();
    let no_brace = trimmed.strip_suffix('{').unwrap_or(trimmed).trim_end();
    format!("{no_brace} {{ ... }}")
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, split_item};
    use super::*;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    const PRE: &str = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";

    fn params_sections() -> Vec<String> {
        vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )]
    }

    #[test]
    fn joint_params_fires_with_follow() {
        let pre = pre_map(&[("src.rs", PRE)]);
        let out =
            infer_semantic_with_pre_images(&params_sections(), &pre).expect("joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("# sweep"), "got:\n{out}");
        assert!(out.contains("- fn foo(x: i32) { ... }"), "got:\n{out}");
        assert!(out.contains("+ fn bar(x: u32) { ... }"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"], "fn bar(x: u32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "joint must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_return_fires_with_follow() {
        let pre = "fn foo(x: i32) -> i32 {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> u32 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("return joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(
            out.contains("+ fn bar(x: i32) -> u32 { ... }"),
            "got:\n{out}"
        );
        validated_live(&out, &pre);
    }

    #[test]
    fn joint_qualified_and_let_calls_fire() {
        let pre =
            "fn foo(x: i32) {\n    x\n}\nfn m() {\n    crate::m::foo(1);\n    let y = foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,2 +5,2 @@\n-    crate::m::foo(1);\n-    let y = foo(2);\n+    crate::m::bar(1);\n+    let y = bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out =
            infer_semantic_with_pre_images(&sections, &pre).expect("qualified/let joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["src.rs"].contains("crate::m::bar(1);"),
            "got: {}",
            files["src.rs"]
        );
        assert!(
            files["src.rs"].contains("let y = bar(2);"),
            "got: {}",
            files["src.rs"]
        );
    }

    #[test]
    fn joint_single_call_site_fires() {
        // N=1 call site still joins (the probe is the uniqueness guard).
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("N=1 joint must fire");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    #[test]
    fn joint_b2_attr_add_single_call_stays_ordinary() {
        // N=1 never pays the second hunk (the unique-single detector
        // would re-harvest the call template and fragment the intent)
        // — the vetoed shape stays wholly ordinary, pinned.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,1 +6,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "B2 N=1 must stay wholly ordinary, got: {full:?}"
        );
    }
    #[test]
    fn body_delta_vetoes_to_wholly_ordinary() {
        // F4b: rename+body-literal must not fragment into ordinary def +
        // rename-claiming sweep — the whole section stays ordinary.
        let pre = "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32) -> i32 {\n-    x + 1\n+fn bar(x: i32) -> i32 {\n+    x + 2\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let mut used = Vec::new();
        let out = try_fn_joint_intent(&sections, &pre, &mut used);
        let Some((rewritten, _, rule, suppressed)) = out else {
            panic!("body-delta joint must suppress, not decline silently");
        };
        assert!(rule.is_empty(), "suppress-only carries no rule");
        assert_eq!(rewritten, sections, "suppress-only leaves bytes identical");
        assert_eq!(
            suppressed,
            HashSet::from([("src.rs".to_string(), "foo".to_string())])
        );
        // End to end: no semantic hunk at all (the sweep is suppressed).
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "body veto must stay wholly ordinary"
        );
    }

    #[test]
    fn joint_b2_attr_add_fires_shared_rule() {
        // WI-4b B2 (F4d): rename+attr-add with callee-only calls raises
        // to the shared-rule two-hunk composition (def hunk carrying the
        // attr line + call-template hunk, no follow). Real-apply is
        // byte-identical.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B2 attr joint must fire");
        assert!(
            out.matches("rule=r1").count() >= 2,
            "both hunks share one rule, got:\n{out}"
        );
        assert!(
            !out.contains("# follow=uses"),
            "no follow in a joint, got:\n{out}"
        );
        assert!(
            out.contains("+ #[must_use]\n"),
            "attr line must raise, got:\n{out}"
        );
        assert!(
            out.contains("- foo(E0)\n"),
            "call template minus, got:\n{out}"
        );
        assert!(
            out.contains("+ bar(E0)\n"),
            "call template plus, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "#[must_use]\nfn bar(x: i32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "B2 attr-add must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b2_attr_remove_fires() {
        // Attr-remove mirror: the minus-side attr line drops, the header
        // renames, calls ride the template.
        let pre =
            "#[must_use]\nfn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,2 +1,1 @@\n-#[must_use]\n-fn foo(x: i32) {\n+fn bar(x: i32) {\n@@ -6,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B2 remove must fire");
        assert!(out.matches("rule=r1").count() >= 2, "got:\n{out}");
        assert!(
            out.contains("- #[must_use]\n"),
            "minus attr must raise, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"], "fn bar(x: i32) {\n    x\n}\nfn m() {\n    bar(1);\n    bar(2);\n}\n",
            "B2 attr-remove must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b2_double_evolution_vetoes_stay_ordinary() {
        // Two evolutions in one commit stay wholly ordinary: params
        // delta under the attr, double attr, arg-delta calls under an
        // attr def, and hetero-arity calls under an attr def.
        for body in [
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: u32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            "@@ -1,1 +1,3 @@\n-fn foo(x: i32) {\n+#[must_use]\n+#[inline]\n+fn bar(x: i32) {\n@@ -5,2 +7,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(1, 2);\n+    bar(1);\n+    bar(1, 2);\n",
        ] {
            let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre)]);
            let full = infer_semantic_with_pre_images(&sections, &pre);
            assert!(
                full.as_ref().is_none_or(|s| !s.contains("rule=")),
                "B2 double-evolution must stay wholly ordinary for {body}, got: {full:?}"
            );
        }
    }

    #[test]
    fn joint_b2_without_calls_is_not_a_candidate() {
        // Attr+rename with no call sites: no joint shape, no suppression.
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn joint_b1_arg_add_fires_shared_rule() {
        // WI-4b B1 (F4a): rename+param-add with uniform arg-adding calls
        // raises to the shared-rule two-hunk composition (def hunk +
        // call-template hunk, no follow — a follow would overlap the
        // call hunk's sites). Real-apply is byte-identical.
        let pre = PRE;
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("B1 arg-add must fire");
        assert!(
            out.matches("rule=r1").count() >= 2,
            "both hunks share one rule, got:\n{out}"
        );
        assert!(
            !out.contains("# follow=uses"),
            "no follow in a joint, got:\n{out}"
        );
        assert!(
            out.contains("- fn foo(x: i32) { ... }"),
            "def minus must raise, got:\n{out}"
        );
        assert!(
            out.contains("+ fn bar(x: i32, y: i32) { ... }"),
            "def plus must raise, got:\n{out}"
        );
        assert!(
            out.contains("- foo(E0)\n"),
            "call template minus, got:\n{out}"
        );
        assert!(
            out.contains("+ bar(E0, 0)\n"),
            "call template plus, got:\n{out}"
        );
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["src.rs"],
            "fn bar(x: i32, y: i32) {\n    x\n}\nfn m() {\n    bar(1, 0);\n    bar(2, 0);\n}\n",
            "B1 must round-trip byte-identical"
        );
    }

    #[test]
    fn joint_b1_zero_prefix_and_qualified_fire() {
        // Zero-prefix addition (`foo()` → `bar(false)`) and qualified
        // uniform calls (`crate::m::foo(1)` → `crate::m::bar(1, 0)`)
        // each compose the same way.
        for (pre_text, body, minus_tpl, plus_tpl) in [
            (
                "fn foo() {\n    1\n}\nfn m() {\n    foo();\n    foo();\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo() {\n+fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo();\n-    foo();\n+    bar(false);\n+    bar(false);\n",
                "- foo()\n",
                "+ bar(false)\n",
            ),
            (
                "fn foo(x: i32) {\n    x\n}\nfn m() {\n    crate::m::foo(1);\n    crate::m::foo(2);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    crate::m::foo(1);\n-    crate::m::foo(2);\n+    crate::m::bar(1, 0);\n+    crate::m::bar(2, 0);\n",
                "- crate::m::foo(E0)\n",
                "+ crate::m::bar(E0, 0)\n",
            ),
            (
                "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo::<u8>(1);\n    foo::<u8>(2);\n}\n",
                "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo::<u8>(1);\n-    foo::<u8>(2);\n+    bar::<u8>(1, 0);\n+    bar::<u8>(2, 0);\n",
                "- foo::<u8>(E0)\n",
                "+ bar::<u8>(E0, 0)\n",
            ),
        ] {
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre_text)]);
            let out = infer_semantic_with_pre_images(&sections, &pre)
                .unwrap_or_else(|| panic!("B1 twin must fire for {body}"));
            assert!(
                out.matches("rule=r1").count() >= 2,
                "shared rule for {body}, got:\n{out}"
            );
            assert!(out.contains(minus_tpl), "template minus for {body}, got:\n{out}");
            assert!(out.contains(plus_tpl), "template plus for {body}, got:\n{out}");
            validated_live(&out, &pre);
        }
    }

    #[test]
    fn joint_b1_hetero_vetoes_stay_ordinary() {
        // Lone additions (N=1), hetero suffixes, prefix rewrites,
        // hetero arities, return deltas, and vis/qual deltas each stay
        // wholly ordinary — no uniform template states them.
        let pre = PRE;
        for body in [
            // N=1 lone addition never pays the second hunk.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1, 0);\n",
            // Hetero trailing suffix.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 1);\n",
            // Leading-arg rewrite is not an addition.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(9, 0);\n+    bar(2, 0);\n",
            // Hetero minus arity.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2, 3);\n+    bar(1, 0);\n+    bar(2, 3, 0);\n",
            // Return delta plus arg-add is two evolutions.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32, y: i32) -> u32 {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
            // Qualifier delta plus arg-add is two evolutions.
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+async fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
        ] {
            let sections = vec![section("src.rs", body)];
            let pre = pre_map(&[("src.rs", pre)]);
            let full = infer_semantic_with_pre_images(&sections, &pre);
            assert!(
                full.as_ref().is_none_or(|s| !s.contains("rule=")),
                "B1 hetero shape must stay wholly ordinary for {body}, got: {full:?}"
            );
        }
    }

    #[test]
    fn pure_rename_is_not_a_candidate() {
        // Suffix-equal renames belong to item_intent: no suppression, and
        // the terse follow emission is untouched.
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pure rename fires");
        assert!(
            out.contains("- fn foo\n+ fn bar\n"),
            "terse intent kept, got:\n{out}"
        );
    }

    #[test]
    fn twin_defs_decline_silently() {
        // Two `fn foo` defs in one file: the unique-definition guard
        // declines with no suppression (a twin file may hold a
        // legitimate independent sweep).
        let pre =
            "fn foo(x: i32) {\n    x\n}\nfn foo(y: i32) {\n    y\n}\nfn m() {\n    foo(1);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -8,1 +8,1 @@\n-    foo(1);\n+    bar(1);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn def_without_calls_is_not_a_candidate() {
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        assert!(try_fn_joint_intent(&sections, &pre, &mut used).is_none());
    }

    #[test]
    fn vis_and_qual_deltas_suppress() {
        // `pub fn` → `pub(crate) fn` + same-name rename is WI-14's vis
        // evolution, not a slot joint; `async` gain likewise.
        for (minus, plus) in [
            ("pub fn foo(x: i32) {", "pub(crate) fn bar(x: u32) {"),
            ("fn foo(x: i32) {", "async fn bar(x: u32) {"),
        ] {
            let sections = vec![section(
                "src.rs",
                &format!(
                    "@@ -1,1 +1,1 @@\n-{minus}\n+{plus}\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n"
                ),
            )];
            let pre = pre_map(&[("src.rs", PRE)]);
            let full = infer_semantic_with_pre_images(&sections, &pre);
            assert!(
                full.as_ref().is_none_or(|s| !s.contains("rule=")),
                "vis/qual veto must stay wholly ordinary for {minus} → {plus}, got: {full:?}"
            );
        }
    }

    #[test]
    fn method_calls_are_not_joint_sites() {
        // `s.foo()` is a different namespace: not a joint use line, and
        // the joint shape around it suppresses rather than firing.
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: u32) {\n@@ -5,1 +5,1 @@\n-    s.foo();\n+    s.bar();\n",
        )];
        let pre = pre_map(&[("src.rs", PRE)]);
        let mut used = Vec::new();
        let out = try_fn_joint_intent(&sections, &pre, &mut used);
        // `s.foo()` is not a call-callee site of `foo`, so the joint
        // shape is absent entirely (no def+call group): silent decline.
        assert!(out.is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn header_helpers_behave() {
        assert_eq!(
            header_name_delta("fn foo(x: i32) {", "fn bar(x: u32) {"),
            Some(("foo".to_string(), "bar".to_string()))
        );
        assert!(header_name_delta("fn foo(x: i32) {", "fn foo(x: u32) {").is_none());
        assert!(header_name_delta("    fn foo() {", "    fn bar() {").is_none());
        assert!(header_name_delta("struct Old {", "struct New {").is_none());
        assert!(slots_equal_except_name(
            "fn foo(x: i32) {",
            "fn bar(x: i32) {"
        ));
        assert!(!slots_equal_except_name(
            "fn foo(x: i32) {",
            "fn bar(x: u32) {"
        ));
        assert!(is_call_site_of("    foo(1);", "    bar(1);", "foo", "bar"));
        assert!(is_call_site_of(
            "    let y = foo(2);",
            "    let y = bar(2);",
            "foo",
            "bar"
        ));
        assert!(!is_call_site_of("    foo(1);", "    baz(1);", "foo", "bar"));
        assert!(!is_call_site_of(
            "    s.foo();",
            "    s.bar();",
            "foo",
            "bar"
        ));
        assert!(is_callee_only("    foo(1);", "    bar(1);", "foo", "bar"));
        assert!(!is_callee_only(
            "    foo(1);",
            "    bar(1, 0);",
            "foo",
            "bar"
        ));
        assert_eq!(with_body_hole("fn foo(x: i32) {"), "fn foo(x: i32) { ... }");
        // `split_item` still sees the joint def as a plain `fn` item.
        assert!(split_item("fn foo(x: i32) {").is_some());
    }
}
