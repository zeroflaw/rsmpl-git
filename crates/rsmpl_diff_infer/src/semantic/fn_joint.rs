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
//! Scope (WI-4b/WI-4c residue, pinned): single-file groups only
//! (multi-file candidate groups decline silently); evolving slots
//! ⊆ {params, return}; attr/body deltas suppress without raising;
//! arg-delta calls suppress without raising (their composition needs a
//! two-hunk shared-rule emission with its own evidence design).

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

/// A validated fire payload (Joint-A): the def pair plus its callee-only
/// call claims, all in one file.
struct Fire {
    old: String,
    new: String,
    def_minus: String,
    def_plus: String,
    claims: Vec<FlatPair>,
    claimed_secs: Vec<usize>,
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
    let mut fire: Option<(Fire, String, String)> = None;
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
                if !probe_reproduces(pre, &emission, &pairs) {
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
    /// Fireable Joint-A payload.
    Ready(Fire),
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
    // scanned from unbalanced hunks (an unbalanced def hunk is itself
    // an attr/insertion delta — never fireable, but still a joint
    // shape that must suppress the fragmented fallback).
    let mut unbalanced = false;
    let mut flat: Vec<FlatPair> = Vec::new();
    // (sec, hunk, minus, plus) header pairs found only via the
    // unbalanced scan (no sites — never fireable).
    let mut combo_defs: Vec<(usize, usize, String, String)> = Vec::new();
    for k in keys {
        let (sec, _, hunks) = &parsed[*k];
        for (h, hp) in hunks.iter().enumerate() {
            if !hp.balanced {
                unbalanced = true;
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
    if unbalanced || !def_single {
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
    // Fire gates on the header: vis in the WI-1 closed set and
    // preserved; qual/generics/where preserved; evolving ⊆
    // {params, return}, nonempty.
    if !header_fire_gates(&flat[def_idx].minus, &flat[def_idx].plus) {
        return Candidate::Suppress(old);
    }
    // Callee-only calls: arguments token-identical on every pair.
    for (i, fp) in flat.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if !is_callee_only(&fp.minus, &fp.plus, &old, &new) {
            return Candidate::Suppress(old);
        }
    }
    let def_minus = flat[def_idx].minus.clone();
    let def_plus = flat[def_idx].plus.clone();
    let mut claimed_secs: Vec<usize> = Vec::new();
    for k in keys {
        claimed_secs.push(parsed[*k].0);
    }
    Candidate::Ready(Fire {
        old,
        new,
        def_minus,
        def_plus,
        claims: flat,
        claimed_secs,
    })
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
    let turbo = match &last.arguments {
        syn::PathArguments::None => String::new(),
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

/// Emit the single-hunk follow emission under `rule`: the header pair
/// rebuilt from the native def lines (`{` → `{ ... }`, native spacing
/// preserved) with `# sweep` + `# follow=uses`.
fn emit(path: &str, rule: &str, fire: &Fire) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
    out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
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
    fn attr_delta_vetoes_to_wholly_ordinary() {
        // F4d: rename+attr must not fragment either.
        let pre = "fn foo(x: i32) {\n    x\n}\nfn m() {\n    foo(1);\n    foo(2);\n}\n";
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,2 @@\n-fn foo(x: i32) {\n+#[must_use]\n+fn bar(x: i32) {\n@@ -5,2 +6,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "attr veto must stay wholly ordinary, got: {full:?}"
        );
    }

    #[test]
    fn arg_delta_calls_veto_to_wholly_ordinary() {
        // F4a: rename+arg-add has no single-hunk form (the follow pass
        // never adds arguments) — suppress until WI-4b states it.
        let pre = PRE;
        let sections = vec![section(
            "src.rs",
            "@@ -1,1 +1,1 @@\n-fn foo(x: i32) {\n+fn bar(x: i32, y: i32) {\n@@ -5,2 +5,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1, 0);\n+    bar(2, 0);\n",
        )];
        let pre = pre_map(&[("src.rs", pre)]);
        let full = infer_semantic_with_pre_images(&sections, &pre);
        assert!(
            full.as_ref().is_none_or(|s| !s.contains("rule=")),
            "arg-add veto must stay wholly ordinary, got: {full:?}"
        );
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
