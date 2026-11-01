//! WI-4f-next split: Joint-B2 attr-joint helpers (D5 — `fn_joint.rs`
//! stays under the 2000-line gate; tests stay inline in `fn_joint.rs`
//! per WI-19).

use std::collections::HashMap;

use super::fn_joint::prefix_qual;
use super::fn_joint::{call_parts, contains_ident, header_name_delta, is_fn_vis_prefix, Fire};
use super::fn_joint::{FlatPair, Joint};
use super::generalized::parse_sig_slots;

fn is_attr_line(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("#[") && t.ends_with(']') && t.len() > 3 && line.starts_with('#')
}

/// WI-4b Joint-B2 attr shape: the single unbalanced hunk of the group
/// holds exactly one `fn`-header pair with a name delta plus exactly
/// one extra `#[…]` attr line (add: 1 minus / 2 pluses; remove: 2
/// minus / 1 plus). Returns `(attr_line, attr_add, def_minus,
/// def_plus)`.
pub(crate) fn attr_def_shape(
    minuses: &[String],
    pluses: &[String],
) -> Option<(String, bool, String, String)> {
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
pub(crate) fn attr_joint_fire(
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
        leftover: HashMap::new(),
        claimed_hunk_bytes: 0,
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
pub(crate) fn probe_attr_joint(
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
