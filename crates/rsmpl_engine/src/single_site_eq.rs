//! WI-1s1 single-site instantiation equality (IMPROVE.md D17).
//!
//! Split from `canonical_tree` (D5: production code lands in a sibling
//! module once the file passes the 1600-line soft watch; unit tests
//! stay inline in the owning file and exercise this through
//! [`canonicalize_with_tree`](super::canonical_tree::canonicalize_with_tree)).
//!
//! A pure single-pair hunk that proves exactly one distinct line site
//! rewrites to its instantiated literal pair — trimmed pre/post lines
//! at that site (expression shapes terminator-stripped per the raiser
//! precedent) — with its range pinned to the site and locality + hole
//! declarations dropped, so the authored templated spelling and the
//! inferred literal diet share one tree-aware form. Multi-subsite
//! lines (two call sites, one line) unify at line granularity: one
//! distinct line span is one site. The caller keeps the rewrite only
//! when the unified patch re-proves the identical post-tree.

use std::collections::HashMap;

use rsmpl_patch::{Directive, Hunk, MetavarKind};

use super::canonical_tree::has_bare_token;

/// Deterministic key of a full file tree (sorted `path + NUL + text`
/// joined): key equality means byte-identical post-trees. The WI-1s0
/// helper and the WI-1s1 verification share this one function.
pub(crate) fn post_tree_key(files: &HashMap<String, String>) -> String {
    let mut keys: Vec<(&String, &String)> = files.iter().collect();
    keys.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::new();
    for (path, text) in keys {
        out.push_str(path);
        out.push('\0');
        out.push_str(text);
        out.push('\0');
    }
    out
}

/// WI-1s1 single-site instantiation equality, verbatim family
/// (IMPROVE.md D17): rewrite a pure single-pair hunk that proves
/// exactly one distinct line site to its instantiated literal pair,
/// pin its range to the site, and drop locality + hole declarations.
///
/// Site evidence comes from the normalized apply when it proves the
/// site, else from the raw apply (normalizations may drop a
/// declaration the engine needs in some positions): whichever proves
/// exactly one distinct line is the gate. The caller keeps the rewrite
/// only when the unified patch re-proves the identical post-tree.
///
/// Gates (anything else stays byte-identical): unique `(file, rule)`
/// key; exactly one distinct `(s, e)` line span with `s == e`;
/// exactly one minus + one plus line and zero context lines;
/// directives ⊆ locality + hole declarations (symbol / fallback /
/// follow / move / mode / repeat / optional / attrs / root shapes are
/// owned by other items and never enter); pre/post lines present and
/// different; no surviving declared hole name still a bare token in
/// the literal pair.
pub(crate) fn unify_single_site_verbatim_to_literal(
    patch: &mut rsmpl_patch::Patch,
    pre: &HashMap<String, String>,
    post: &HashMap<String, String>,
    spans: &mut HashMap<(String, String), Vec<(u32, u32)>>,
    raw_spans: &HashMap<(String, String), Vec<(u32, u32)>>,
) {
    let mut owners: HashMap<(String, String), Vec<(usize, usize)>> = HashMap::new();
    for (fi, sec) in patch.files.iter().enumerate() {
        for (hi, hunk) in sec.hunks.iter().enumerate() {
            if let Hunk::Rsmpl(r) = hunk {
                owners
                    .entry((sec.target_path.clone(), r.rule_name()))
                    .or_default()
                    .push((fi, hi));
            }
        }
    }
    // One distinct line span from the normalized evidence when it
    // proves the site, else from the raw evidence (never a mix: the
    // gate always describes a single apply).
    let keys: Vec<(String, String)> = owners.keys().cloned().collect();
    let mut keys = keys;
    keys.sort();
    for key in keys {
        let idxs = &owners[&key];
        if idxs.len() != 1 {
            continue;
        }
        let Some(s) = single_line_site(spans, raw_spans, &key) else {
            continue;
        };
        let (fi, hi) = idxs[0];
        // Read-only gate pass: compute the literal pair before any
        // mutation so a veto leaves the hunk byte-identical.
        let literal: Option<(String, String)> = {
            let sec = &patch.files[fi];
            if sec.is_rename() {
                None
            } else if let Hunk::Rsmpl(r) = &sec.hunks[hi] {
                gate_verbatim_single(r, &sec.target_path, s, pre, post)
            } else {
                None
            }
        };
        let Some((lit_minus, lit_plus)) = literal else {
            continue;
        };
        let Hunk::Rsmpl(r) = &mut patch.files[fi].hunks[hi] else {
            continue;
        };
        for l in r.lines.iter_mut() {
            // Body text keeps the post-marker space (`- foo` parses to
            // text ` foo`): the rewrite matches the raiser emission
            // (`- {lit}`) exactly, so both legs share one form.
            if l.is_removal() {
                l.text = format!(" {lit_minus}");
            } else if l.is_addition() {
                l.text = format!(" {lit_plus}");
            }
        }
        r.header.old_start = s;
        r.header.old_len = 1;
        // Diet single: no locality lines (the WI-8 range rule keeps the
        // single span as the tight actual) and no hole declarations
        // (the gate proved every declared name is gone from the pair).
        r.directives.retain(|d| {
            !matches!(
                d,
                Directive::Allow(_)
                    | Directive::Scope(_)
                    | Directive::MetaVar {
                        kind: MetavarKind::Pat | MetavarKind::Type | MetavarKind::Expr,
                        ..
                    }
            )
        });
        r.raw_directives.retain(|l| {
            let t = l.trim_start_matches('#').trim_start();
            !(t.starts_with("sweep")
                || t.starts_with("allow")
                || t.starts_with("scope")
                || t.starts_with("pat")
                || t.starts_with("type")
                || t.starts_with("expr"))
        });
        spans.insert(key, vec![(s, s)]);
    }
}

/// One distinct line span from the normalized evidence when it proves
/// the site, else from the raw evidence (never a mix: the gate always
/// describes a single apply). `None` for zero, multi-line, or
/// multi-site evidence.
fn single_line_site(
    spans: &HashMap<(String, String), Vec<(u32, u32)>>,
    raw_spans: &HashMap<(String, String), Vec<(u32, u32)>>,
    key: &(String, String),
) -> Option<u32> {
    for evidence in [spans, raw_spans] {
        if let Some(sites) = evidence.get(key) {
            let mut distinct = sites.clone();
            distinct.sort();
            distinct.dedup();
            if distinct.len() == 1 && distinct[0].0 == distinct[0].1 && distinct[0].0 != 0 {
                return Some(distinct[0].0);
            }
            return None;
        }
    }
    None
}

/// WI-1s1 gate: `Some((lit_minus, lit_plus))` when hunk `r` over
/// `target` is a pure single-pair verbatim shape proving the single
/// line site `s`, else `None` (byte-identical).
fn gate_verbatim_single(
    r: &rsmpl_patch::RsmplHunk,
    target: &str,
    s: u32,
    pre: &HashMap<String, String>,
    post: &HashMap<String, String>,
) -> Option<(String, String)> {
    if !r.directives.iter().all(|d| {
        matches!(
            d,
            Directive::Allow(_)
                | Directive::Scope(_)
                | Directive::LangRust
                | Directive::MetaVar {
                    kind: MetavarKind::Pat | MetavarKind::Type | MetavarKind::Expr,
                    ..
                }
        )
    }) {
        return None;
    }
    let mut minus = 0usize;
    let mut plus = 0usize;
    for l in &r.lines {
        if l.is_removal() {
            minus += 1;
        } else if l.is_addition() {
            plus += 1;
        } else {
            return None;
        }
    }
    if minus != 1 || plus != 1 {
        return None;
    }
    let pre_line = pre.get(target)?.lines().nth((s as usize).checked_sub(1)?)?;
    let post_line = post
        .get(target)?
        .lines()
        .nth((s as usize).checked_sub(1)?)?;
    let (lit_minus, lit_plus) = verbatim_literal_pair(pre_line, post_line)?;
    // A declared hole name surviving as a bare token in the literal
    // pair (e.g. a pre line that already names `E`) is not a hole
    // instantiation — leave the hunk pinned.
    let declared: Vec<&String> = r
        .directives
        .iter()
        .filter_map(|d| match d {
            Directive::MetaVar { names, .. } => Some(names),
            _ => None,
        })
        .flatten()
        .collect();
    if declared
        .iter()
        .any(|n| has_bare_token(&lit_minus, n) || has_bare_token(&lit_plus, n))
    {
        return None;
    }
    Some((lit_minus, lit_plus))
}

/// Literal pair for the WI-1s1 rewrite: the instantiated pre/post
/// line at the proven site, with the raiser's expression-shape
/// terminator rule (`try_verbatim_single` precedent: a trailing
/// statement terminator never reaches the pattern, so a `tasty + 1;`
/// line instantiates to the `tasty + 1` expression pair both legs
/// share; complete-statement shapes keep the whole trimmed line, `;`
/// included). `None` for empty, identical, or unclassifiable pairs —
/// the hunk stays pinned.
fn verbatim_literal_pair(pre_line: &str, post_line: &str) -> Option<(String, String)> {
    let m = pre_line.trim();
    let p = post_line.trim();
    if m.is_empty() || p.is_empty() || m == p {
        return None;
    }
    let is_expr = |t: &str| match syn::parse_str::<syn::Expr>(t) {
        Ok(e) => !matches!(e, syn::Expr::Let(_)),
        Err(_) => false,
    };
    let sm = strip_one_terminator(m)?.trim_end();
    let sp = strip_one_terminator(p)?.trim_end();
    if !sm.is_empty() && !sp.is_empty() && sm != sp && is_expr(sm) && is_expr(sp) {
        return Some((sm.to_string(), sp.to_string()));
    }
    if syn::parse_str::<syn::Stmt>(m).is_err() || syn::parse_str::<syn::Stmt>(p).is_err() {
        return None;
    }
    Some((m.to_string(), p.to_string()))
}

/// Strip one trailing statement/arg terminator (`;`, else `,`), the
/// `try_verbatim_single` precedent (kept local: the engine must not
/// depend on the inferrer). `None` on empty input only.
fn strip_one_terminator(text: &str) -> Option<&str> {
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        return Some(stripped.trim_end());
    }
    if let Some(stripped) = text.strip_suffix(',') {
        return Some(stripped.trim_end());
    }
    Some(text)
}
