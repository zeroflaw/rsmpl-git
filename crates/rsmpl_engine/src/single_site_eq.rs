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
//! precedent; bare-type hunks rewrite to the concrete type delta
//! between the site lines, since a lone header line is not an
//! applicable pattern) — with its range pinned to the site and
//! locality + hole declarations dropped, so the authored templated
//! spelling and the inferred literal diet share one tree-aware form. Multi-subsite
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
/// WI-2 slice 2 (WI-6 slice 1) filtered fn-single equality lives in
/// `unify_fn_filter_single_to_concrete` below (same D17 discipline,
/// disjoint gate: that arm owns filter-carrying hunks, this one owns
/// context-free pairs).
pub(crate) fn unify_fn_filter_single_to_concrete(
    patch: &mut rsmpl_patch::Patch,
    _pre: &std::collections::HashMap<String, String>,
    _post: &std::collections::HashMap<String, String>,
    spans: &mut std::collections::HashMap<(String, String), Vec<(u32, u32)>>,
    raw_spans: &std::collections::HashMap<(String, String), Vec<(u32, u32)>>,
) {
    use std::collections::HashMap;
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
    let mut keys: Vec<(String, String)> = owners.keys().cloned().collect();
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
        // Read-only gate pass: compute the rewrite before any
        // mutation so a veto leaves the hunk byte-identical.
        let rewritten: Option<Vec<rsmpl_patch::BodyLine>> = {
            let sec = &patch.files[fi];
            if sec.is_rename() {
                None
            } else if let Hunk::Rsmpl(r) = &sec.hunks[hi] {
                gate_fn_filter_single(r)
            } else {
                None
            }
        };
        let Some(lines) = rewritten else {
            continue;
        };
        let Hunk::Rsmpl(r) = &mut patch.files[fi].hunks[hi] else {
            continue;
        };
        r.lines = lines;
        r.header.old_start = s;
        r.header.old_len = 1;
        // Diet single: no locality lines (the WI-8 range rule keeps the
        // single span as the tight actual) and no hole declarations.
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

/// WI-2 slice 2 gate: `Some(new lines)` when hunk `r` is a filtered
/// fn-rename single, else `None` (byte-identical). The target derives
/// from the hunk pair itself; the caller verifies by re-applying.
fn gate_fn_filter_single(r: &rsmpl_patch::RsmplHunk) -> Option<Vec<rsmpl_patch::BodyLine>> {
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
    let mut minus: Vec<usize> = Vec::new();
    let mut plus: Vec<usize> = Vec::new();
    let mut saw_filter = false;
    for (i, l) in r.lines.iter().enumerate() {
        if l.is_removal() {
            minus.push(i);
        } else if l.is_addition() {
            plus.push(i);
        } else {
            let t = l.text.trim();
            if t == "..." || t == "{" || t == "}" {
                continue;
            }
            if is_filter_header_line(t) {
                saw_filter = true;
                continue;
            }
            return None;
        }
    }
    if minus.len() != 1 || plus.len() != 1 || !saw_filter {
        return None;
    }
    let (m_indent, m_name, m_rest) = fn_diet_erase(&r.lines[minus[0]].text)?;
    let (p_indent, p_name, p_rest) = fn_diet_erase(&r.lines[plus[0]].text)?;
    if m_rest != p_rest || m_rest != "(...) { ... }" || m_name == p_name {
        return None;
    }
    // The `N` fn-header name hole is a template, never a rename.
    if m_name == "N" || p_name == "N" {
        return None;
    }
    // Declared hole names surviving as bare tokens are not hole
    // instantiations (same law as the verbatim gate).
    let declared: Vec<&String> = r
        .directives
        .iter()
        .filter_map(|d| match d {
            Directive::MetaVar { names, .. } => Some(names),
            _ => None,
        })
        .flatten()
        .collect();
    for n in &declared {
        if super::canonical_tree::has_bare_token(&format!("fn {m_name}"), n)
            || super::canonical_tree::has_bare_token(&format!("fn {p_name}"), n)
        {
            return None;
        }
    }
    let mut out: Vec<rsmpl_patch::BodyLine> = Vec::new();
    for (i, l) in r.lines.iter().enumerate() {
        if i == minus[0] {
            let mut nl = l.clone();
            nl.text = format!("{m_indent}fn {m_name}(...) {{ ... }}");
            out.push(nl);
        } else if i == plus[0] {
            let mut nl = l.clone();
            nl.text = format!("{p_indent}fn {p_name}(...) {{ ... }}");
            out.push(nl);
        } else if is_filter_header_line(l.text.trim()) {
            out.push(l.clone());
        }
        // Pure-frame context lines drop (both legs share the reduced
        // frame); anything else already vetoed above.
    }
    Some(out)
}

/// `{`-terminated `impl`/`trait` filter header (trimmed): mirrors the
/// `block_headers.rs` law (cross-crate, so mirrored, not shared —
/// `unsafe` prefix, word-bounded keyword, `{` terminus; attribute and
/// comment lines never match).
fn is_filter_header_line(trimmed: &str) -> bool {
    if !trimmed.ends_with('{') {
        return false;
    }
    let rest = trimmed.strip_prefix("unsafe ").unwrap_or(trimmed);
    let body = rest
        .strip_prefix("impl")
        .or_else(|| rest.strip_prefix("trait"));
    let Some(body) = body else {
        return false;
    };
    body.is_empty() || body.starts_with([' ', '\t', '<', '{'])
}

/// Split a pair line into (indent, fn name, diet-erased rest):
/// `V Q fn foo G (...) W { ... }` → `("     ", "foo", "(...) { ... }")`,
/// `fn foo(...) {` → `("", "foo", "(...) { ... }")`. The `(...)`
/// family only — full concrete signatures stay divergent (a future
/// slice may widen). `None` for non-header shapes.
fn fn_diet_erase(line_text: &str) -> Option<(String, String, String)> {
    let trimmed = line_text.trim_start();
    let indent_len = line_text.len() - trimmed.len();
    let indent = line_text[..indent_len].to_string();
    let (vq, t) = match trimmed.strip_prefix("V Q ") {
        Some(r) => (true, r),
        None => (false, trimmed),
    };
    let t = t.strip_prefix("fn ")?;
    let name_end = t.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    let (name, mut rest) = (t[..name_end].to_string(), t[name_end..].to_string());
    if name.is_empty() || name == "N" {
        return None;
    }
    if vq {
        rest = rest.replace("G (...) W", "(...)");
    }
    rest = rest.trim().to_string();
    // Concrete `(...) {` and inline `(...) { ... }` both erase to the
    // shared inline target.
    if rest == "(...) {" || rest == "(...) { ... }" {
        rest = "(...) { ... }".to_string();
    } else {
        return None;
    }
    Some((indent, name, rest))
}
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

/// Item premise for the return-slot arm: exactly one distinct matched
/// span (an item match, not a line match) with a nonzero header line.
/// Returns the span; the header line fills the omitted return. Two
/// spans — e.g. an omitted-return template proving two same-name
/// items — decline by construction (never a mix: the gate always
/// describes a single apply, like `single_line_site`).
fn single_item_site(
    spans: &HashMap<(String, String), Vec<(u32, u32)>>,
    raw_spans: &HashMap<(String, String), Vec<(u32, u32)>>,
    key: &(String, String),
) -> Option<(u32, u32)> {
    for evidence in [spans, raw_spans] {
        if let Some(sites) = evidence.get(key) {
            let mut distinct = sites.clone();
            distinct.sort();
            distinct.dedup();
            if distinct.len() == 1 && distinct[0].0 != 0 {
                return Some(distinct[0]);
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
    let (lit_minus, lit_plus) = if is_bare_type_hunk(r) {
        concrete_type_delta(pre_line, post_line)?
    } else {
        verbatim_literal_pair(pre_line, post_line)?
    };
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

/// A bare-type hunk body: exactly one minus + one plus line, both
/// parsing as `syn::Type` (the WI-2 type-single shape, holed or
/// literal).
fn is_bare_type_hunk(r: &rsmpl_patch::RsmplHunk) -> bool {
    let mut bodies = Vec::new();
    for l in &r.lines {
        if l.is_removal() || l.is_addition() {
            // Body text keeps the post-marker space (`- foo` parses to
            // text ` foo`): trim before the shape probe.
            bodies.push(l.text.trim().to_string());
        } else {
            return false;
        }
    }
    if bodies.len() != 2 {
        return false;
    }
    syn::parse_str::<syn::Type>(&bodies[0]).is_ok()
        && syn::parse_str::<syn::Type>(&bodies[1]).is_ok()
}

/// Concrete type delta between two site lines: the longest-shared-frame
/// middle on chars, scanned from the longest shared suffix down to the
/// first split whose middles both parse as one `syn::Type` (the shared
/// suffix may start inside the delta — `Vec<i32>` vs `Box<[i32]>`
/// share the trailing `>` before `) -> i32 {`). `None` when no split
/// yields a type pair (multi-delta lines never parse as one `Type`).
fn concrete_type_delta(pre_line: &str, post_line: &str) -> Option<(String, String)> {
    let m: Vec<char> = pre_line.trim().chars().collect();
    let p: Vec<char> = post_line.trim().chars().collect();
    if m.is_empty() || p.is_empty() {
        return None;
    }
    let mut pre = 0usize;
    while pre < m.len() && pre < p.len() && m[pre] == p[pre] {
        pre += 1;
    }
    let mut max_suf = 0usize;
    while max_suf < m.len() - pre
        && max_suf < p.len() - pre
        && m[m.len() - 1 - max_suf] == p[p.len() - 1 - max_suf]
    {
        max_suf += 1;
    }
    let mut suf = max_suf;
    loop {
        let m_mid: String = m[pre..m.len() - suf].iter().collect();
        let p_mid: String = p[pre..p.len() - suf].iter().collect();
        let m_mid = m_mid.trim().to_string();
        let p_mid = p_mid.trim().to_string();
        if !m_mid.is_empty()
            && !p_mid.is_empty()
            && m_mid != p_mid
            && syn::parse_str::<syn::Type>(&m_mid).is_ok()
            && syn::parse_str::<syn::Type>(&p_mid).is_ok()
        {
            return Some((m_mid, p_mid));
        }
        if suf == 0 {
            return None;
        }
        suf -= 1;
    }
}
/// Literal pair for the WI-1s1 rewrite: the instantiated pre/post
/// line at the proven site, with the raiser's expression-shape
/// terminator rule (`try_verbatim_single` precedent: a trailing
/// statement terminator never reaches the pattern, so a `tasty + 1;`
/// line instantiates to the `tasty + 1` expression pair both legs
/// share; complete-statement shapes keep the whole trimmed line, `;`
/// included). Bare-type hunks never reach this (the caller routes
/// them to `concrete_type_delta`). `None` for empty, identical, or
/// unclassifiable pairs — the hunk stays pinned.
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
    if syn::parse_str::<syn::Stmt>(m).is_ok() && syn::parse_str::<syn::Stmt>(p).is_ok() {
        return Some((m.to_string(), p.to_string()));
    }
    None
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

/// WI-2 slice 3 single-site instantiation equality, return-slot family
/// (IMPROVE.md D17): a single-pair fn evolution single whose minus
/// omits the return (`- fn helper(...)`) rewrites the minus to carry
/// the single site's concrete return (`- fn helper(...) -> i32`), so
/// the authored bare-`...` spelling and the inferred concrete-minus
/// spelling share one tree-aware form. Same D17 discipline as the
/// verbatim/filter arms: unique (file, rule) owner, exactly one item
/// site, read-only gate, caller re-proves the identical post-tree.
///
/// Disjoint gates: the verbatim arm owns context-free pairs, the
/// filter arm owns filter-carrying renames, this one owns fn pairs
/// whose minus omits a single-token return the plus side carries
/// concretely. Omitted-return twins (two same-name sites) fail the
/// single-site gate and composite site returns fail the stability
/// bound — both stay divergent by construction.
pub(crate) fn unify_fn_evo_single_to_concrete(
    patch: &mut rsmpl_patch::Patch,
    pre: &HashMap<String, String>,
    _post: &HashMap<String, String>,
    spans: &mut HashMap<(String, String), Vec<(u32, u32)>>,
    raw_spans: &HashMap<(String, String), Vec<(u32, u32)>>,
) {
    use std::collections::HashMap;
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
    let mut keys: Vec<(String, String)> = owners.keys().cloned().collect();
    keys.sort();
    for key in keys {
        let idxs = &owners[&key];
        if idxs.len() != 1 {
            continue;
        }
        // Item premise (not the line premise of the verbatim/filter
        // arms): exactly one matched item span — an fn evolution
        // single matches its whole item, so the evidence span is the
        // item range and the return fill comes from its header line.
        let Some((s, _e)) = single_item_site(spans, raw_spans, &key) else {
            continue;
        };
        let (fi, hi) = idxs[0];
        // Read-only gate pass: compute the rewritten minus before any
        // mutation so a veto leaves the hunk byte-identical.
        let rewritten: Option<String> = {
            let sec = &patch.files[fi];
            if sec.is_rename() {
                None
            } else if let Hunk::Rsmpl(r) = &sec.hunks[hi] {
                match pre
                    .get(&sec.target_path)
                    .and_then(|text| text.lines().nth((s as usize).checked_sub(1)?))
                {
                    Some(site_line) => gate_fn_evo_single(r, site_line),
                    None => None,
                }
            } else {
                None
            }
        };
        let Some(new_minus) = rewritten else {
            continue;
        };
        let Hunk::Rsmpl(r) = &mut patch.files[fi].hunks[hi] else {
            continue;
        };
        for l in r.lines.iter_mut() {
            if l.is_removal() {
                l.text = new_minus.clone();
            }
        }
        // Range and spans stay untouched: the evidence spans are line
        // granularity while the rewritten form keeps the item frame, so
        // only the minus text converges (both legs already share the
        // item range on SPEC-conformant patches).
        // Diet single: no locality lines and no hole declarations
        // (same posture as the verbatim/filter arms).
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
    }
}

/// WI-2 slice 3 gate: `Some(rewritten minus text)` when hunk `r` is a
/// single fn evolution pair whose minus omits a return the plus side
/// carries concretely and the pre-image site line `site_line` carries
/// as one stable token, else `None` (byte-identical). The rewritten
/// minus keeps the hunk's own indent and vis/qual run and drops a
/// trailing `W` (a concrete return plus `W` never parses — the
/// `-> T W` law); the caller verifies by re-applying.
fn gate_fn_evo_single(r: &rsmpl_patch::RsmplHunk, site_line: &str) -> Option<String> {
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
    let mut minus: Vec<&str> = Vec::new();
    let mut plus: Vec<&str> = Vec::new();
    for l in &r.lines {
        if l.is_removal() {
            minus.push(&l.text);
        } else if l.is_addition() {
            plus.push(&l.text);
        } else {
            let t = l.text.trim();
            if t == "..." || t == "{" || t == "}" || is_filter_header_line(t) {
                continue;
            }
            return None;
        }
    }
    if minus.len() != 1 || plus.len() != 1 {
        return None;
    }
    let m = fn_evo_head(minus[0])?;
    let p = fn_evo_head(plus[0])?;
    // Same diet header, same indent; the minus omits what the plus
    // carries. The `N` name hole is a template, never a rewrite (same
    // law as the filter arm).
    if m.name != p.name || m.name == "N" || m.indent != p.indent {
        return None;
    }
    if m.ret.is_some() || p.ret.is_none() {
        return None;
    }
    // No other slot may differ: the vis/qual run is the evolution and
    // the return is the fill — generics, params, and `where` ride the
    // diet identically on both sides by construction of `fn_evo_head`
    // (only the leading run and the return clause are erased).
    if m.core != p.core {
        return None;
    }
    let site_ret = site_fn_return(site_line)?;
    Some(format!(
        "{}{}fn {}(...) -> {} {{",
        m.indent, m.prefix, m.name, site_ret
    ))
}

/// One diet fn-header pair line: `PREFIX fn NAME(...) [W] [-> RET] {`
/// with literal `(...)` params and no generics slot. `core` is the
/// slot-erased header (`fn NAME(...) {`) the gate compares; `prefix`
/// is the vis/qual run verbatim. `None` for any other shape.
struct FnEvoHead {
    indent: String,
    prefix: String,
    name: String,
    core: String,
    ret: Option<String>,
}

/// Closed vis/qual run vocabulary for the return-slot gate: concrete
/// `pub` (incl. `pub(...)`), qualifier keywords, the `V`/`Q` wildcards,
/// and quoted ABIs (`extern "C"` lexes as two tokens). Anything else
/// before `fn` declines — the re-prove is the soundness net, this set
/// is the legibility bound.
fn fn_evo_prefix_ok(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    prefix.split_whitespace().all(|tok| {
        tok == "V"
            || tok == "Q"
            || tok == "pub"
            || tok.starts_with("pub(")
            || tok == "async"
            || tok == "unsafe"
            || tok == "const"
            || tok == "extern"
            || (tok.starts_with('"') && tok.ends_with('"') && tok.len() >= 2)
    })
}

fn fn_evo_head(line_text: &str) -> Option<FnEvoHead> {
    // BodyLine.text keeps the post-marker space (`- fn f` → ` fn f`).
    let stripped = line_text.trim_start();
    let indent = line_text[..line_text.len() - stripped.len()].to_string();
    let head = stripped.strip_suffix('{')?.trim_end();
    // Literal `(...)` params only — concrete signatures and missing
    // parens decline (a different spelling class).
    let open = head.find("(...)")?;
    let before = head[..open].trim_end();
    let after = head[open + "(...)".len()..].trim();
    // `before` is `PREFIX fn NAME`: the name is the trailing ident.
    let name_end = before.len();
    let name_start = before[..name_end]
        .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
        .map(|i| i + 1)
        .unwrap_or(0);
    let name = before[name_start..].to_string();
    if name.is_empty() || name == "N" {
        return None;
    }
    let prefix = before[..name_start].trim_end();
    let fn_word = prefix.split_whitespace().next_back()?;
    if fn_word != "fn" {
        return None;
    }
    let prefix_run = prefix[..prefix.len() - "fn".len()].trim().to_string();
    if !fn_evo_prefix_ok(&prefix_run) {
        return None;
    }
    // `after` is ``, `W`, or `-> RET` — anything else (including the
    // unparsable `-> T W` order and `where` clauses) declines.
    let after = match after.strip_prefix('W') {
        Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => rest.trim_start(),
        _ => after,
    };
    let ret = match after.strip_prefix("->") {
        Some(ty) => {
            let ty = ty.trim();
            if ty.is_empty() || ty.contains(char::is_whitespace) {
                return None;
            }
            // Single-token types re-render byte-identically (the same
            // stability bound the detector enforces); anything wider
            // stays divergent rather than emitting spaced pretty text.
            if syn::parse_str::<syn::Type>(ty).is_err() {
                return None;
            }
            Some(ty.to_string())
        }
        None => {
            if !after.is_empty() {
                return None;
            }
            None
        }
    };
    let core = format!("fn {name}(...) {{");
    let prefix = if prefix_run.is_empty() {
        String::new()
    } else {
        format!("{prefix_run} ")
    };
    Some(FnEvoHead {
        indent,
        prefix,
        name,
        core,
        ret,
    })
}

/// Concrete single-token return of a pre-image site line (`i32` for
/// `fn helper(x: i32) -> i32 {`), pretty-printed via `syn` so both
/// legs compute the identical fill deterministically. `None` when the
/// line is not a `{`-terminated fn header with a concrete
/// single-token return (removals, decls, and composite returns
/// decline — nothing to fill, or nothing stable to fill with).
fn site_fn_return(site_line: &str) -> Option<String> {
    // The site line is `{`-terminated (a def header); appending a
    // close yields an empty-body `ItemFn` probe (the `parse_sig_probe`
    // posture — stripping the brace first leaves an unparsable lone
    // `}`).
    let probe = format!("{} }}", site_line.trim_end());
    let sig = syn::parse_str::<syn::ItemFn>(&probe).ok().map(|f| f.sig)?;
    let ty = match sig.output {
        syn::ReturnType::Type(_, ty) => ty,
        syn::ReturnType::Default => return None,
    };
    let pretty = quote::ToTokens::to_token_stream(&ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if pretty.is_empty() || pretty.contains(char::is_whitespace) {
        return None;
    }
    if syn::parse_str::<syn::Type>(&pretty).is_err() {
        return None;
    }
    Some(pretty)
}

#[cfg(test)]
mod tests {
    use crate::canonical_tree::{canonicalize_with_tree, single_site_post_key};
    use std::collections::HashMap;

    fn pre(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn wi2_evo_ret_single_unifies_to_concrete_minus() {
        // WI-2 slice 3: the return-slot equality arm. The authored
        // bare-`...` minus (`- fn helper(...)`) and the inferred
        // concrete minus (`- fn helper(...) -> i32`) prove the same
        // one item site; both canonicalize to the concrete-minus form.
        let p = pre(&[(
            "src/lib.rs",
            "fn helper(x: i32) -> i32 {\n    x + 1\n}\nfn compute(values: Vec<i32>) -> i32 {\n    values.iter().sum()\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r1\n- fn helper(...) {\n+ pub fn helper(...) -> i64 {\n      ...\n  }\n";
        let inferred = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r1\n- fn helper(...) -> i32 {\n+ pub fn helper(...) -> i64 {\n      ...\n  }\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- fn helper(...) -> i32 {\n+ pub fn helper(...) -> i64 {\n"),
            "concrete-minus form:\n{a}"
        );
        // Tree legs close: one site, one post-tree on both legs.
        let ka = single_site_post_key(&p, authored).expect("authored applies");
        let kb = single_site_post_key(&p, inferred).expect("inferred applies");
        assert_eq!(ka, kb, "same single site, same post-tree");
    }

    #[test]
    fn wi2_evo_ret_twin_stays_divergent() {
        // Negative control (D17): the bare minus ranged over both
        // twins proves two sites (the wide range fails the single-site
        // premise — the apply itself reports two matches), so the arm
        // declines and the legs stay canonically distinct, never healed.
        // (A tight range scoping to one twin is a different, sound
        // single-site claim the arm unifies — ranges are the author's.)
        let p = pre(&[(
            "src/lib.rs",
            "fn helper(x: i32) -> i32 {\n    x\n}\nfn helper(y: i32) -> i64 {\n    y\n}\n",
        )]);
        let bare = "diff --git src/lib.rs\n@@ -1,6 @@ rule=r1\n- fn helper(...) {\n+ pub fn helper(...) -> i64 {\n      ...\n  }\n";
        let concrete = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r1\n- fn helper(...) -> i32 {\n+ pub fn helper(...) -> i64 {\n      ...\n  }\n";
        let a = canonicalize_with_tree(bare, &p).unwrap();
        let b = canonicalize_with_tree(concrete, &p).unwrap();
        assert_ne!(a, b, "twin-site bare minus never unifies:\n{a}\nvs\n{b}");
        assert!(
            a.contains("- fn helper(...) {\n"),
            "bare minus untouched:\n{a}"
        );
    }

    #[test]
    fn wi2_evo_ret_complex_site_stays_divergent() {
        // Negative control: the site return is composite
        // (`Option<String>` pretty-prints spaced), so no stable fill
        // exists — the arm declines and the bare minus stays bare.
        let p = pre(&[(
            "src/lib.rs",
            "fn helper(x: i32) -> Option<String> {\n    x\n}\n",
        )]);
        let bare = "diff --git src/lib.rs\n@@ -1,3 @@ rule=r1\n- fn helper(...) {\n+ pub fn helper(...) -> Vec<String> {\n      ...\n  }\n";
        let a = canonicalize_with_tree(bare, &p).unwrap();
        assert!(
            a.contains("- fn helper(...) {\n"),
            "composite site return never fills:\n{a}"
        );
    }
}
