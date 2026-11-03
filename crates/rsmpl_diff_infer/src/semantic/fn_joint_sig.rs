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
/// (may evolve — WI-8 slice 1 two-slot, slice 4 qual-run);
/// generics/where preserved; evolving ⊆ {vis, qual, params, return}
/// with params or return evolving (vis-only/qual-only renames stay
/// their terse intents' domain — item_intent claims them first), at
/// most two evolve (triples still suppress).
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
    if m.generics != p.generics || m.where_ != p.where_ {
        return false;
    }
    let params_or_ret = m.params != p.params || m.ret != p.ret;
    let n_evolve = (m.vis != p.vis) as u8
        + (m.qual != p.qual) as u8
        + (m.params != p.params) as u8
        + (m.ret != p.ret) as u8;
    params_or_ret && n_evolve <= 2
}

/// WI-4b Joint-B1 header gates: vis in the WI-1 closed set and
/// preserved; qual may evolve alongside params (WI-8 slice 6);
/// generics/where preserved; params evolve, return preserved
/// (a return delta plus an arg-add is two evolutions the
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
    if m.generics != p.generics || m.where_ != p.where_ {
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

/// WI-4b uniform arg-evolution template over the non-def pairs
/// (indices `!= def_idx`): every call renames old→new with an equal
/// qualifier and equal turbofish, with uniform arity on both sides,
/// in one of three shapes — trailing-add (every plus side extends the
/// token-identical minus prefix by the same non-empty concrete
/// suffix), trailing-drop (WI-8 slice 7; mirror — every plus side is
/// the token-identical minus prefix, the same non-empty concrete
/// suffix rides every minus side), or trailing-retype (WI-8 slice 3;
/// same arity, leading args token-identical per call and hole-bound,
/// trailing suffix uniform concrete on both sides and differing).
/// Returns the `(call_minus, call_plus)` pattern pair, or `None` on
/// any hetero shape (prefix rewrite, hetero suffix/arity/qual/turbo,
/// non-trailing or hetero drop, varying-value retype — suppressed,
/// pinned). WI-4f reuses it for the workspace B1 gate over
/// cross-file claims.
pub(crate) fn arg_add_template(
    flat: &[FlatPair],
    def_idx: usize,
    old: &str,
    new: &str,
) -> Option<(String, String)> {
    struct EvolveCall {
        qual: String,
        turbo: String,
        minus: Vec<String>,
        plus: Vec<String>,
    }
    let mut calls: Vec<EvolveCall> = Vec::new();
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        let (qm, cm, tm, am) = call_parts(&fp.minus)?;
        let (qp, cp, tp, ap) = call_parts(&fp.plus)?;
        if cm != old || cp != new || qm != qp || tm != tp {
            return None;
        }
        calls.push(EvolveCall {
            qual: qm,
            turbo: tm,
            minus: am,
            plus: ap,
        });
    }
    let first = calls.first()?;
    if !calls
        .iter()
        .all(|c| c.qual == first.qual && c.turbo == first.turbo)
    {
        return None;
    }
    let (minus_len, plus_len) = (first.minus.len(), first.plus.len());
    if !calls
        .iter()
        .all(|c| c.minus.len() == minus_len && c.plus.len() == plus_len)
    {
        return None;
    }
    if plus_len > minus_len {
        // Trailing-add (WI-4b law, unchanged): the minus prefix is
        // token-identical per call and hole-bound; one shared concrete
        // grown suffix rides every plus side.
        for c in &calls {
            if !c.minus.iter().zip(c.plus.iter()).all(|(a, b)| a == b) {
                return None;
            }
        }
        let grown: Vec<String> = first.plus[minus_len..].to_vec();
        if grown.is_empty() {
            return None;
        }
        if !calls.iter().all(|c| c.plus[minus_len..] == grown) {
            return None;
        }
        return Some(render_call_template(
            &first.qual,
            &first.turbo,
            old,
            new,
            minus_len,
            &[],
            &grown,
        ));
    }
    if plus_len == minus_len {
        // Trailing-retype (WI-8 slice 3): the largest leading run
        // preserved per call binds holes; the trailing suffix is
        // uniform concrete on both sides and differs.
        for k in (0..minus_len).rev() {
            if !calls.iter().all(|c| c.minus[..k] == c.plus[..k]) {
                continue;
            }
            let (minus_suffix, plus_suffix) = (&first.minus[k..], &first.plus[k..]);
            if minus_suffix == plus_suffix {
                continue;
            }
            if !calls
                .iter()
                .all(|c| c.minus[k..] == *minus_suffix && c.plus[k..] == *plus_suffix)
            {
                continue;
            }
            return Some(render_call_template(
                &first.qual,
                &first.turbo,
                old,
                new,
                k,
                minus_suffix,
                plus_suffix,
            ));
        }
        return None;
    }
    if minus_len > plus_len {
        // Trailing-drop (WI-8 slice 7): mirror of trailing-add — the
        // plus prefix is token-identical per call and hole-bound; one
        // shared concrete dropped suffix rides every minus side.
        for c in &calls {
            if !c.plus.iter().zip(c.minus.iter()).all(|(a, b)| a == b) {
                return None;
            }
        }
        let dropped: Vec<String> = first.minus[plus_len..].to_vec();
        if dropped.is_empty() {
            return None;
        }
        if !calls.iter().all(|c| c.minus[plus_len..] == dropped) {
            return None;
        }
        return Some(render_call_template(
            &first.qual,
            &first.turbo,
            old,
            new,
            plus_len,
            &dropped,
            &[],
        ));
    }
    None
}

/// Render one call-template pair: `holes` leading `E{i}` holes plus a
/// concrete suffix on each side (`- foo(E0, 2)` /
/// `+ bar(E0, 2u32)`; the trailing-add shape passes an empty minus
/// suffix). Zero holes renders the bare concrete pair.
fn render_call_template(
    qual: &str,
    turbo: &str,
    old: &str,
    new: &str,
    holes: usize,
    minus_suffix: &[String],
    plus_suffix: &[String],
) -> (String, String) {
    let hole_names: Vec<String> = (0..holes).map(|i| format!("E{i}")).collect();
    let mut minus_parts = hole_names.clone();
    minus_parts.extend(minus_suffix.iter().cloned());
    let mut plus_parts = hole_names;
    plus_parts.extend(plus_suffix.iter().cloned());
    let old_path = super::shape::literal_path_last(&prefix_qual(qual, old));
    let new_path = super::shape::literal_path_last(&prefix_qual(qual, new));
    (
        format!("{old_path}{turbo}({})", minus_parts.join(", ")),
        format!("{new_path}{turbo}({})", plus_parts.join(", ")),
    )
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

#[cfg(test)]
mod tests {
    use super::arg_add_template;
    use super::FlatPair;

    fn pair(minus: &str, plus: &str) -> FlatPair {
        FlatPair {
            sec: 0,
            hunk: 0,
            minus: minus.to_string(),
            plus: plus.to_string(),
            site: 1,
        }
    }

    fn def_pair() -> FlatPair {
        pair("fn foo(a: i32, b: i32) {", "fn bar(a: i32, b: u32) {")
    }

    #[test]
    fn add_template_law_unchanged() {
        // WI-4b add law pinned byte-exact: hole-bound prefix plus the
        // shared concrete grown suffix.
        let flat = vec![
            def_pair(),
            pair("foo(1);", "bar(1, 0);"),
            pair("foo(2);", "bar(2, 0);"),
        ];
        assert_eq!(
            arg_add_template(&flat, 0, "foo", "bar"),
            Some(("foo(E0)".to_string(), "bar(E0, 0)".to_string()))
        );
    }

    #[test]
    fn retype_template_fires() {
        // WI-8 slice 3: same arity, leading prefix preserved per call
        // and hole-bound, trailing suffix uniform concrete and
        // differing on both sides.
        let flat = vec![
            def_pair(),
            pair("foo(1, 2);", "bar(1, 2u32);"),
            pair("foo(3, 2);", "bar(3, 2u32);"),
        ];
        assert_eq!(
            arg_add_template(&flat, 0, "foo", "bar"),
            Some(("foo(E0, 2)".to_string(), "bar(E0, 2u32)".to_string()))
        );
        // Fully concrete pair (no hole) fires too.
        let flat = vec![
            def_pair(),
            pair("foo(0);", "bar(1);"),
            pair("foo(0);", "bar(1);"),
        ];
        assert_eq!(
            arg_add_template(&flat, 0, "foo", "bar"),
            Some(("foo(0)".to_string(), "bar(1)".to_string()))
        );
    }

    #[test]
    fn retype_hetero_vetoes() {
        // Non-uniform suffixes, varying-value retypes (no uniform
        // concrete to state), and mixed arities each decline —
        // suppressed downstream, pinned. (WI-8 slice 7: the uniform
        // trailing-drop twin now fires, pinned by drop_template_fires.)
        for (minus0, plus0, minus1, plus1) in [
            ("foo(1, 2);", "bar(1, 2u32);", "foo(3, 2);", "bar(3, 3u32);"),
            ("foo(1);", "bar(1u32);", "foo(2);", "bar(2u32);"),
            ("foo(1);", "bar(1, 0);", "foo(2, 3);", "bar(2, 3);"),
        ] {
            let flat = vec![def_pair(), pair(minus0, plus0), pair(minus1, plus1)];
            assert_eq!(arg_add_template(&flat, 0, "foo", "bar"), None);
        }
    }

    #[test]
    fn drop_template_fires() {
        // WI-8 slice 7: mirror of the add law — hole-bound prefix plus
        // the shared concrete dropped suffix.
        let flat = vec![
            def_pair(),
            pair("foo(1, 0);", "bar(1);"),
            pair("foo(2, 0);", "bar(2);"),
        ];
        assert_eq!(
            arg_add_template(&flat, 0, "foo", "bar"),
            Some(("foo(E0, 0)".to_string(), "bar(E0)".to_string()))
        );
        // Fully concrete drop (no hole) fires too.
        let flat = vec![
            def_pair(),
            pair("foo(0);", "bar();"),
            pair("foo(0);", "bar();"),
        ];
        assert_eq!(
            arg_add_template(&flat, 0, "foo", "bar"),
            Some(("foo(0)".to_string(), "bar()".to_string()))
        );
    }

    #[test]
    fn drop_hetero_vetoes() {
        // Hetero dropped suffixes, prefix rewrites under a shrink, and
        // mixed shrink arities each decline — suppressed, pinned.
        for (minus0, plus0, minus1, plus1) in [
            ("foo(1, 0);", "bar(1);", "foo(2, 1);", "bar(2);"),
            ("foo(1, 0);", "bar(9);", "foo(2, 0);", "bar(2);"),
            ("foo(1, 0);", "bar(1);", "foo(2, 0, 0);", "bar(2);"),
        ] {
            let flat = vec![def_pair(), pair(minus0, plus0), pair(minus1, plus1)];
            assert_eq!(arg_add_template(&flat, 0, "foo", "bar"), None);
        }
    }
}
