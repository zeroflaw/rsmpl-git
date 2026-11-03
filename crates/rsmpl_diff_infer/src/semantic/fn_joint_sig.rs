//! WI-8 slice 1 split (D5 — `fn_joint.rs` was at the 2000-line gate):
//! `fn`-header signature-slot helpers: name-delta extraction,
//! rename-vs-joint slot comparison, the Joint-A/B1 header fire gates,
//! and the closed vis-set / ident helpers they share. Tests stay
//! inline in their owning detector modules per WI-19.

use super::fn_joint::call_parts;
use super::fn_joint_types::FlatPair;
use super::generalized::parse_sig_slots;

/// The `(old, new)` fn names when both lines parse as column-0 `fn`
/// headers with differing names, else `None`. Column-0 only: indented
/// headers are methods (WI-13 residue), and `split_item` def lines
/// (`struct`/`type`/…) never parse as `fn` headers.
pub(crate) fn header_name_delta(minus: &str, plus: &str) -> Option<(String, String)> {
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
pub(crate) fn slots_equal_except_name(minus: &str, plus: &str) -> bool {
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
/// (may evolve — WI-8 slice 1 two-slot); qual/generics/where
/// preserved; evolving ⊆ {vis, params, return} with params or return
/// evolving (vis-only rename stays the terse intent's domain), at most
/// two evolve (vis+params+return triple still suppresses).
pub(crate) fn header_fire_gates(minus: &str, plus: &str) -> bool {
    let (Some(m), Some(p)) = (parse_sig_slots(minus), parse_sig_slots(plus)) else {
        return false;
    };
    if !is_fn_vis_prefix(&m.vis) || !is_fn_vis_prefix(&p.vis) {
        return false;
    }
    // Self-referential vis (`pub(in foo)` renaming `foo`) is an
    // evolution, never a pure-slot joint (WI-1 posture, kept — both
    // sides and names now the vis may evolve with the rename).
    if contains_ident(&m.vis, &m.name)
        || contains_ident(&m.vis, &p.name)
        || contains_ident(&p.vis, &m.name)
        || contains_ident(&p.vis, &p.name)
    {
        return false;
    }
    if m.qual != p.qual || m.generics != p.generics || m.where_ != p.where_ {
        return false;
    }
    let params_or_ret = m.params != p.params || m.ret != p.ret;
    let n_evolve = (m.vis != p.vis) as u8 + (m.params != p.params) as u8 + (m.ret != p.ret) as u8;
    params_or_ret && n_evolve <= 2
}

/// WI-4b Joint-B1 header gates: vis in the WI-1 closed set and
/// preserved; qual/generics/where preserved; params evolve, return
/// preserved (a return delta plus an arg-add is two evolutions the
/// shared-rule pair cannot restate — suppressed, pinned).
pub(crate) fn header_argadd_gates(minus: &str, plus: &str) -> bool {
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
pub(crate) fn prefix_qual(qual: &str, callee: &str) -> String {
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
/// WI-4f reuses it for the workspace B1 gate over cross-file claims.
pub(crate) fn arg_add_template(
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

/// WI-1 closed vis set the terse `fn` rename admits (SPEC 14.1 diet
/// form — vis rides literally, never a `V` hole). Inputs here are
/// pretty-printed slot text (`pub ( crate )`, `pub ( in foo )`), so
/// the closed set compares on the token-compacted form (WI-8 slice 1:
/// without this the restricted arms below are dead on pretty input).
/// Compacting keeps a space only between adjacent word tokens, so
/// `in foo` never merges to `infoo`.
pub(crate) fn is_fn_vis_prefix(vis: &str) -> bool {
    let compact = compact_vis_tokens(vis);
    let vis = compact.as_str();
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

/// Token-compact pretty slot text: rejoin whitespace-separated tokens,
/// keeping one space only between adjacent word tokens (ident/keyword
/// adjacency is significant; every other adjacency re-joins bare).
fn compact_vis_tokens(vis: &str) -> String {
    let mut out = String::new();
    let mut prev_word = false;
    for tok in vis.split_whitespace() {
        let word = !tok.is_empty()
            && tok
                .chars()
                .all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric());
        if prev_word && word {
            out.push(' ');
        }
        out.push_str(tok);
        prev_word = word;
    }
    out
}

/// Ident-boundary containment (never substring).
pub(crate) fn contains_ident(haystack: &str, needle: &str) -> bool {
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
