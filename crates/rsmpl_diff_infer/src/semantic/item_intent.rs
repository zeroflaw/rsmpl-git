//! WI-15a item-intent raising (SPEC D.4 `item_intent`).
//!
//! One definition pair (`struct`/`enum`/`trait`/`type`/`mod`/`const`/
//! `static`/terse-`fn`, via [`super::split_item`]) joined with N-1
//! same-delta use lines (`impl Old`, `s: Old`, `-> Old`, `Old { … }`,
//! …) across one or more files compresses to the single definition
//! hunk *with* `# follow=uses` + `# sweep`. The engine follow rewrites
//! the rest at apply time (workspace-wide, WI-7a), so the emission is
//! one hunk.
//!
//! WI-9 `fn` arm (widened WI-1, WI-2): a top-level `fn old` pure-rename
//! definition with a concrete vis from the closed set (`""`, `pub`,
//! `pub(crate)`, `pub(super)`, `pub(in <path>)`), equal
//! qualifier/signature suffixes (any signature incl. `->`, generics,
//! `where`), joined with N-1 *call-callee* use lines of `old` (bare,
//! qualified, turbofish; `let`-wrapped included) *or* `use`-tail deltas
//! (`use crate::a::old;` → `use crate::a::new;`, grouped members,
//! `as`-preserving, `pub use` included) emits the same single hunk in
//! the terse `- [pub …] fn old` / `+ [pub …] fn new` diet form with
//! `# sweep` + `# follow=uses` — never a `V` hole, because the intent
//! proved one concrete vis. One-line definitions join the same way
//! (the `split_delta` suffix covers the identical body). `pub(…)`
//! paths naming the old ident itself, vis changes, method calls,
//! fields, bare value paths, macro heads, glob/suffix `use` shapes, and
//! second-hop re-exports veto the whole intent (never a partial follow
//! claim); the scratch-apply probe below is the uniqueness guard (an
//! off-diff same-name call *or* same-tail `use` fails the
//! unchanged-leg).
//!
//! Totality is a cross-file scratch-apply probe (member-rename
//! posture, WI-9b): the authored emission applied to an in-memory
//! clone of the pre-image must reproduce exactly the native plus lines
//! at every claimed changed site with every other line byte-identical
//! (a missed site fails the plus-leg, an overwritten twin fails the
//! unchanged-leg). `None` pre-image declines; twin definitions decline;
//! call-callee and macro-head uses decline to their dedicated detectors
//! (WI-7 grouped shapes keep their pins); `macro_rules!` and `mod` defs
//! decline to theirs.

use std::collections::{HashMap, HashSet};

use super::{split_item, try_item_rename, unique_rule};

/// One native pair at its old-side site.
pub(crate) struct Pair {
    pub(crate) sec: usize,
    pub(crate) path: String,
    pub(crate) minus: String,
    pub(crate) plus: String,
    pub(crate) site: usize,
}

/// Whole-patch item-intent pre-pass: returns the rebuilt section list
/// (claimed `.rs` sections replaced by the single semantic section at
/// the definition section's slot) plus the definition path and rule
/// for provenance, or `None` when the patch holds no provable intent.
/// Reserved rule names are released on decline.
pub(crate) fn try_item_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    // Eligible: `.rs` non-rename sections (non-`.rs` passes through;
    // rename sections own the file move and stay out of the intent).
    // WI-4f-next: hunk-level claiming — per-hunk pairs with verbatim
    // text; unclaimable hunks stay ordinary in place.
    struct HunkEntry {
        sec: usize,
        text: String,
        minuses: Vec<String>,
        pluses: Vec<String>,
        sites: Vec<usize>,
        balanced: bool,
        path: String,
    }
    let mut hunks: Vec<HunkEntry> = Vec::new();
    // WI-4e: preserved pre-existing semantic hunks + native byte lens
    // per claimed section (pure-native sections hold empty vecs, so
    // emission stays byte-identical to the pre-4e path).
    let mut preserved: HashMap<usize, (String, Vec<String>)> = HashMap::new();
    let mut native_lens: HashMap<usize, usize> = HashMap::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        // WI-4e: hunk-level pre-pass inputs — skip `rule=` hunks, not
        // whole sections, so leftover native pairs in hybrid sections
        // (e.g. move leftovers) can still form intents. Pure-semantic
        // sections stay skipped.
        let (prelude, semantic_hunks, native_view) = super::split_semantic_native(sec);
        let Some(native_sec) = native_view else {
            continue;
        };
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        preserved.insert(i, (prelude, semantic_hunks));
        native_lens.insert(i, native_sec.len());
        for nh in super::split_native_hunks(&native_sec).into_iter() {
            hunks.push(HunkEntry {
                sec: i,
                text: nh.text,
                minuses: nh.minuses,
                pluses: nh.pluses,
                sites: nh.sites,
                balanced: nh.balanced,
                path: path.clone(),
            });
        }
    }
    if hunks.is_empty() {
        return None;
    }
    // Flatten balanced hunks to one pair per changed line, in section
    // order. Unbalanced hunks are always leftovers (never pairs).
    let mut pairs: Vec<Pair> = Vec::new();
    // (pair index -> hunk entry index)
    let mut pair_hunk: Vec<usize> = Vec::new();
    for (hi, he) in hunks.iter().enumerate() {
        if !he.balanced {
            continue;
        }
        if he.minuses.len() != he.sites.len() {
            used_rules.truncate(checkpoint);
            return None;
        }
        for ((m, p), s) in he.minuses.iter().zip(he.pluses.iter()).zip(he.sites.iter()) {
            pairs.push(Pair {
                sec: he.sec,
                path: he.path.clone(),
                minus: m.clone(),
                plus: p.clone(),
                site: *s,
            });
            pair_hunk.push(hi);
        }
    }
    if pairs.len() < 2 {
        return None;
    }
    // Exactly one definition pair (a `split_item` delta): every other
    // pair is a use line sharing the delta.
    let mut def_idx: Option<usize> = None;
    let mut def_kind = String::new();
    let mut old_ident = String::new();
    let mut new_ident = String::new();
    let mut def_prefix = String::new();
    for (i, pair) in pairs.iter().enumerate() {
        if let Some((prefix, kind, old, new)) = split_delta(&pair.minus, &pair.plus) {
            if def_idx.is_some() {
                used_rules.truncate(checkpoint);
                return None;
            }
            def_idx = Some(i);
            def_kind = kind;
            old_ident = old;
            new_ident = new;
            def_prefix = prefix;
        }
    }
    let def_idx = def_idx?;
    if old_ident == new_ident || old_ident.is_empty() || new_ident.is_empty() {
        return None;
    }
    // WI-15a slice 1: `macro_rules!` intents belong to the dedicated
    // `macro_rules_rename` detector (per-arg token-tree holes); `mod`
    // renames join their file move in WI-15c. Both decline here.
    if def_kind == "macro_rules!" || def_kind == "mod" {
        return None;
    }
    // WI-9: `fn` intents claim call-callee uses (the follow domain)
    // plus `use`-tail deltas (WI-2: the cross-file import is part of
    // the definition set, D9); every other shape keeps the item gate
    // (any same-delta use line).
    let is_fn_intent = def_kind == "fn";
    // WI-4f-next: per-pair claimability (cheap, ident-boundary); the
    // probe below is authoritative. Hunks with any unclaimable pair
    // become leftovers instead of vetoing the whole patch.
    let pair_claimable = |pair: &Pair| -> bool {
        if pair.minus.trim() == pair.plus.trim() {
            return false;
        }
        if !contains_ident(&pair.minus, &old_ident) || !contains_ident(&pair.plus, &new_ident) {
            return false;
        }
        if is_fn_intent {
            if !is_fn_call_use_site(&pair.minus, &old_ident)
                && !is_fn_use_tail_site(&pair.minus, &pair.plus, &old_ident, &new_ident)
            {
                return false;
            }
        } else if is_dedicated_use_site(&pair.minus, &old_ident) {
            return false;
        }
        true
    };
    // Group pair indices per hunk entry.
    let mut hunk_pairs: Vec<Vec<usize>> = vec![Vec::new(); hunks.len()];
    for (pi, hi) in pair_hunk.iter().enumerate() {
        hunk_pairs[*hi].push(pi);
    }
    let def_hunk = pair_hunk[def_idx];
    // No partial def hunk: the def hunk must hold exactly the def pair.
    if hunk_pairs[def_hunk].len() != 1 {
        return None;
    }
    // Partition balanced hunks into claimable vs leftover.
    let mut claimed_hunks: HashSet<usize> = HashSet::new();
    claimed_hunks.insert(def_hunk);
    for (hi, pis) in hunk_pairs.iter().enumerate() {
        if pis.is_empty() {
            continue;
        }
        if hi == def_hunk {
            continue;
        }
        let mut ok = true;
        for pi in pis {
            if *pi == def_idx {
                continue;
            }
            if !pair_claimable(&pairs[*pi]) {
                ok = false;
                break;
            }
        }
        if ok {
            claimed_hunks.insert(hi);
        }
    }
    // Claimed pairs = def + pairs in claimable hunks.
    let mut claimed_idx: Vec<usize> = Vec::new();
    for hi in claimed_hunks.iter() {
        for pi in &hunk_pairs[*hi] {
            claimed_idx.push(*pi);
        }
    }
    if claimed_idx.len() < 2 {
        return None;
    }
    // No claimable use hunk beyond the def → nothing to join.
    if claimed_hunks.len() < 2 {
        return None;
    }
    claimed_idx.sort_unstable();
    let claimed_pairs: Vec<Pair> = claimed_idx
        .iter()
        .map(|pi| Pair {
            sec: pairs[*pi].sec,
            path: pairs[*pi].path.clone(),
            minus: pairs[*pi].minus.clone(),
            plus: pairs[*pi].plus.clone(),
            site: pairs[*pi].site,
        })
        .collect();
    let def = &pairs[def_idx];
    // Pre-image for every involved file (precision-first, claimed only).
    for pair in &claimed_pairs {
        if !pre.contains_key(&pair.path) {
            return None;
        }
    }
    // Twin-definition veto (unique-definition rule, SPEC 14.8): exactly
    // one `split_item` definition of `old` with the def kind across
    // the involved pre-images (claimed set).
    if count_kind_defs(pre, &claimed_pairs, &def_kind, &old_ident) != 1 {
        return None;
    }
    // The def pair must be emittable as a lone item rename (terse-`fn`
    // gates, column-0, suffix laws); otherwise the follow emission
    // would carry a shape the engine rejects. WI-9 `fn` arm (WI-1
    // widened): vis from the closed set the terse form accepts —
    // empty, `pub`, `pub(crate)`, `pub(super)`, `pub(in <path>)` —
    // with equal suffixes (any signature); `try_item_rename` suffix
    // laws are skipped (`->` fns carry them; the scratch probe below
    // subsumes the check since the concrete-vis terse emission always
    // parses).
    if is_fn_intent {
        if !is_fn_vis_prefix(&def_prefix) {
            return None;
        }
        // Self-referential vis (`pub(in foo)` renaming `foo`) is an
        // evolution, never a pure rename (WI-4's domain).
        if contains_ident(&def_prefix, &old_ident) {
            return None;
        }
    } else {
        let mut probe_used = vec![format!("probe{}", used_rules.len())];
        let def_pre = pre.get(&def.path).map(String::as_str);
        try_item_rename(
            &def.minus,
            &def.plus,
            &def.path,
            def_pre,
            std::slice::from_ref(&def.site),
            &mut probe_used,
        )?;
    }
    // Build the emission: sweep dummy + follow + def pair.
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let emission = emit(
        &def.path,
        &rule,
        &def_kind,
        &def_prefix,
        &old_ident,
        &new_ident,
    );
    // Scratch-apply probe: every claimed site must show its native plus
    // line, every other line byte-identical (per file, claimed subset).
    if !probe_reproduces(pre, &emission, &claimed_pairs) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // T375: pure sections keep today's whole-native-view comparison
    // (pins byte-identical); mixed sections compare emission hunks vs
    // claimed native hunk bytes (shared header stays in place).
    let has_leftover = hunks.iter().enumerate().any(|(hi, _)| {
        if claimed_hunks.contains(&hi) {
            false
        } else {
            // Unbalanced hunks and unclaimed balanced hunks in touched
            // sections are leftovers; untouched sections are not leftovers
            // (they pass through, never counted).
            let sec_touched = hunks
                .iter()
                .enumerate()
                .any(|(hj, _)| claimed_hunks.contains(&hj) && hunks[hj].sec == hunks[hi].sec);
            sec_touched && preserved.contains_key(&hunks[hi].sec)
        }
    });
    let claimed_bytes: usize = if has_leftover {
        claimed_hunks.iter().map(|hi| hunks[*hi].text.len()).sum()
    } else {
        let claimed_secs: HashSet<usize> = claimed_hunks.iter().map(|hi| hunks[*hi].sec).collect();
        claimed_secs
            .iter()
            .map(|s| native_lens.get(s).copied().unwrap_or(0))
            .sum()
    };
    let emission_bytes: usize = if has_leftover {
        super::emission_hunks(&emission)
            .iter()
            .map(|h| h.len())
            .sum()
    } else {
        emission.len()
    };
    if emission_bytes >= claimed_bytes {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Rebuild: claimed hunks collapse to the emission at the def
    // section; leftover native hunks stay verbatim in place (WI-4f-next).
    // WI-4e: hybrid sections keep pre-existing semantic hunks ahead.
    let claimed_secs: HashSet<usize> = claimed_hunks.iter().map(|hi| hunks[*hi].sec).collect();
    // Leftover native hunk texts per section, in original hunk order.
    let mut leftovers: HashMap<usize, Vec<String>> = HashMap::new();
    for (hi, he) in hunks.iter().enumerate() {
        if claimed_hunks.contains(&hi) {
            continue;
        }
        if !claimed_secs.contains(&he.sec) {
            continue;
        }
        leftovers.entry(he.sec).or_default().push(he.text.clone());
    }
    let new_hunks = super::emission_hunks(&emission);
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if !claimed_secs.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        let (prelude, keep) = preserved.get(&i).cloned().unwrap_or_default();
        let rest = leftovers.get(&i).cloned().unwrap_or_default();
        if i == def.sec {
            if keep.is_empty() && rest.is_empty() {
                out.push(emission.clone());
            } else {
                let mut rebuilt = prelude;
                for h in &keep {
                    rebuilt.push_str(h);
                }
                for h in &new_hunks {
                    rebuilt.push_str(h);
                }
                for h in &rest {
                    rebuilt.push_str(h);
                }
                out.push(rebuilt);
            }
        } else if keep.is_empty() && rest.is_empty() {
            // Fully claimed non-def section collapses away.
        } else {
            let mut rebuilt = prelude;
            for h in &keep {
                rebuilt.push_str(h);
            }
            for h in &rest {
                rebuilt.push_str(h);
            }
            out.push(rebuilt);
        }
    }
    Some((out, def.path.clone(), rule))
}

/// A `split_item` definition delta, or `None` for use lines.
fn split_delta(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(plus.trim_end())?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    Some((old_prefix, old_kind, old_ident, new_ident))
}

/// WI-1: closed vis set the terse `fn` rename accepts (SPEC 14.1
/// diet form — vis rides literally, never a `V` hole). `pub(in
/// <path>)` paths are `::`-separated ident runs with an optional
/// leading `crate`/`self`/`super`/`$crate` head.
fn is_fn_vis_prefix(prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "pub " {
        return true;
    }
    if prefix == "pub(crate) " || prefix == "pub(super) " {
        return true;
    }
    let Some(inner) = prefix
        .strip_prefix("pub(in ")
        .and_then(|r| r.strip_suffix(") "))
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

/// True when the use line's changed subterm sits in a dedicated
/// detector's position: a call callee or macro head naming `old`
/// (`Old(1)`, `m::Old::<T>(x)`, `old!(…)` incl. `let`-wrapped). Those
/// classes keep their pinned shapes; everything else (impl/fn headers,
/// type positions, struct literals/patterns, `use` lines, bare value
/// paths) stays claimable for the probe.
fn is_dedicated_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_dedicated_expr(&expr, old) {
            return true;
        }
    }
    // `let`-wrapped sites (`let b = old!(…);`) parse as statements.
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_dedicated_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

/// WI-9: true when the use line's changed subterm is a *call callee*
/// naming `old` — the plain-`fn` follow domain (bare, qualified,
/// turbofish; `let`-wrapped included). Method calls, fields, bare
/// value paths, macro heads, and `use` lines all return false and
/// veto the `fn` intent. Arguments never count — only the head.
fn is_fn_call_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_fn_call_expr(&expr, old) {
            return true;
        }
    }
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_fn_call_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

/// WI-2: true when the pair is a `use`-tail delta renaming exactly one
/// leaf ident `old` → `new` at a tail-or-direct-group-member position:
/// plain tails (`use crate::a::foo;`), grouped members
/// (`use crate::a::{foo, Other};`), `as`-preserving
/// (`use crate::a::foo as f;`), and `pub use` re-export forms. Both
/// lines must parse as `syn::ItemUse` with equal vis/attrs; glob trees,
/// nested member paths (`crate::{x::foo}` suffix), intermediate-segment
/// changes (`crate::foo::Bar`), and vis/attr deltas all return false.
/// The workspace scratch-apply probe below stays authoritative
/// (second-hop re-exports and off-diff same-tail uses veto there).
fn is_fn_use_tail_site(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    let minus_item: syn::ItemUse = match syn::parse_str(minus.trim()) {
        Ok(item) => item,
        Err(_) => return false,
    };
    let plus_item: syn::ItemUse = match syn::parse_str(plus.trim()) {
        Ok(item) => item,
        Err(_) => return false,
    };
    let minus_vis = &minus_item.vis;
    let plus_vis = &plus_item.vis;
    if quote::quote!(#minus_vis).to_string() != quote::quote!(#plus_vis).to_string() {
        return false;
    }
    if minus_item.leading_colon != plus_item.leading_colon {
        return false;
    }
    if minus_item.attrs.len() != plus_item.attrs.len() {
        return false;
    }
    for (a, b) in minus_item.attrs.iter().zip(plus_item.attrs.iter()) {
        if quote::quote!(#a).to_string() != quote::quote!(#b).to_string() {
            return false;
        }
    }
    match use_tree_single_tail_delta(&minus_item.tree, &plus_item.tree, old, new, false) {
        Some(count) => count == 1,
        None => false,
    }
}

/// Single-tail delta walk over two `use` trees: `Some(n)` when the trees
/// are structurally identical except for `n` leaf `old` → `new` swaps at
/// allowed tail positions, `None` on any other divergence (glob, vis
/// already checked above, mismatched variants, intermediate changes).
/// `in_group` is true inside a `{…}` member list: a change under a
/// nested `Path` there (`{x::foo}`) is a suffix and vetoes.
fn use_tree_single_tail_delta(
    minus: &syn::UseTree,
    plus: &syn::UseTree,
    old: &str,
    new: &str,
    in_group: bool,
) -> Option<u32> {
    match (minus, plus) {
        (syn::UseTree::Path(m), syn::UseTree::Path(p)) => {
            if m.ident != p.ident {
                return None;
            }
            let inner = use_tree_single_tail_delta(&m.tree, &p.tree, old, new, in_group)?;
            if inner > 0 && in_group {
                return None;
            }
            Some(inner)
        }
        (syn::UseTree::Name(m), syn::UseTree::Name(p)) => {
            if m.ident == p.ident {
                Some(0)
            } else if m.ident == old && p.ident == new {
                Some(1)
            } else {
                None
            }
        }
        (syn::UseTree::Rename(m), syn::UseTree::Rename(p)) => {
            if m.rename != p.rename {
                return None;
            }
            if m.ident == p.ident {
                Some(0)
            } else if m.ident == old && p.ident == new {
                Some(1)
            } else {
                None
            }
        }
        (syn::UseTree::Group(m), syn::UseTree::Group(p)) => {
            if m.items.len() != p.items.len() {
                return None;
            }
            let mut total = 0u32;
            for (mm, pp) in m.items.iter().zip(p.items.iter()) {
                total += use_tree_single_tail_delta(mm, pp, old, new, true)?;
            }
            Some(total)
        }
        _ => None,
    }
}

/// True for call callees naming `old` (qualified and turbofish
/// spellings included — the engine's scalar follow rewrites them;
/// method calls never count: different namespace).
fn is_fn_call_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// True for call callees and macro heads naming `old` (qualified and
/// turbofish spellings included; arguments never count — only the
/// head position).
fn is_dedicated_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                if path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
                {
                    return true;
                }
            }
            false
        }
        syn::Expr::Macro(m) => m.mac.path.segments.last().is_some_and(|s| s.ident == old),
        _ => false,
    }
}

/// Count `split_item` definitions of (`kind`, `ident`) across the
/// involved pre-images (one line per definition).
pub(crate) fn count_kind_defs(
    pre: &HashMap<String, String>,
    pairs: &[Pair],
    kind: &str,
    ident: &str,
) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut count = 0usize;
    for pair in pairs {
        if !seen.insert(pair.path.clone()) {
            continue;
        }
        if let Some(text) = pre.get(&pair.path) {
            for line in text.lines() {
                if let Some((_, k, id, _)) = split_item(line) {
                    if k == kind && id == ident {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Emit the single-hunk follow emission under `rule`.
fn emit(path: &str, rule: &str, kind: &str, prefix: &str, old: &str, new: &str) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    if kind == "fn" {
        // WI-1: concrete vis rides the terse pair (never a `V` hole —
        // the intent proved one vis).
        out.push_str(&format!("- {prefix}fn {old}\n+ {prefix}fn {new}\n"));
    } else {
        out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    }
    out
}

/// Scratch-apply probe: the emission applied to a `pre` clone must show
/// each claimed plus line at its site with every other line identical.
pub(crate) fn probe_reproduces(
    pre: &HashMap<String, String>,
    emission: &str,
    pairs: &[Pair],
) -> bool {
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
    // Per-file claim index: site -> plus trim.
    let mut claims: HashMap<&str, HashMap<usize, &str>> = HashMap::new();
    for pair in pairs {
        claims
            .entry(pair.path.as_str())
            .or_default()
            .insert(pair.site, pair.plus.trim());
    }
    for (path, pre_text) in pre {
        let Some(post_text) = files.get(path) else {
            return false;
        };
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            return false;
        }
        let empty: HashMap<usize, &str> = HashMap::new();
        let claim = claims.get(path.as_str()).unwrap_or(&empty);
        for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            let line_no = idx + 1;
            if let Some(want) = claim.get(&line_no) {
                if post_line.trim() != *want {
                    return false;
                }
            } else if pre_line != post_line {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use super::*;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
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

    const A_PRE: &str = "struct Old {\n    x: i32,\n}\n";
    const B_PRE: &str = "fn f(s: Old) -> i32 {\n    0\n}\nimpl Old {\n    fn m(&self) {}\n}\n";

    fn sections_two_file() -> Vec<String> {
        vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-fn f(s: Old) -> i32 {\n+fn f(s: New) -> i32 {\n@@ -4,1 +4,1 @@\n-impl Old {\n+impl New {\n"
                .to_string(),
        ]
    }

    #[test]
    fn def_delta_extracts() {
        assert!(split_delta("struct Old {", "struct New {").is_some());
        assert!(split_delta("fn f(s: Old) -> i32 {", "fn f(s: New) -> i32 {").is_none());
        assert!(split_delta("impl Old {", "impl New {").is_none());
    }

    #[test]
    fn dedicated_positions_decline() {
        // Call callees and macro heads naming `old` belong to the
        // call/macro detectors — the intent never claims them.
        assert!(is_dedicated_use_site("    Old(1);", "Old"));
        assert!(is_dedicated_use_site("    m::Old(1);", "Old"));
        assert!(!is_dedicated_use_site("    Some(Old);", "Old"));
        assert!(is_dedicated_use_site("    old!(1);", "old"));
        assert!(is_dedicated_use_site("    let b = old!(2);", "old"));
        assert!(!is_dedicated_use_site("impl Old {", "Old"));
        assert!(!is_dedicated_use_site("fn f(s: Old) -> i32 {", "Old"));
        assert!(!is_dedicated_use_site("use m::Old;", "Old"));
        // A bare method ident is not a dedicated head (different
        // namespace); the probe declines it instead.
        assert!(!is_dedicated_use_site("    s.old();", "old"));
    }

    #[test]
    fn call_site_uses_decline_to_grouped_shape() {
        // Tuple-struct def + constructor call sites: the WI-7 grouped
        // shape (def + `call_sweep`, shared rule) keeps its pin — the
        // intent declines on the call-callee carve.
        let pre = "struct Old(i32);\n";
        let use_pre = "fn m() {\n    Old(1);\n    Old(2);\n}\n";
        let sections = vec![
            "diff --git a/z.rs b/z.rs\n--- a/z.rs\n+++ b/z.rs\n@@ -1,1 +1,1 @@\n-struct Old(i32);\n+struct New(i32);\n"
                .to_string(),
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,2 +2,2 @@\n-    Old(1);\n-    Old(2);\n+    New(1);\n+    New(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[("z.rs", pre), ("a.rs", use_pre)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn method_use_fails_the_probe() {
        // `w.s.Old()` is a method ident (different namespace): the
        // follow leaves it, so the probe fails and the section stays
        // ordinary. (Capitals keep the ident gate green so the probe
        // is what declines.)
        let pre_a = "struct Old {\n    x: i32,\n}\n";
        let pre_b = "fn f(w: Wrap) -> i32 {\n    w.s.Old()\n}\n";
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,1 +2,1 @@\n-    w.s.Old()\n+    w.s.New()\n"
                .to_string(),
        ];
        let pre = pre_map(&[("a.rs", pre_a), ("b.rs", pre_b)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn twin_definitions_decline() {
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
        ];
        let pre = pre_map(&[("a.rs", A_PRE), ("b.rs", A_PRE)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn single_pair_declines() {
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
        ];
        let pre = pre_map(&[("a.rs", A_PRE)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
    }

    #[test]
    fn two_file_intent_fires_and_round_trips() {
        let sections = sections_two_file();
        let pre = pre_map(&[("a.rs", A_PRE), ("b.rs", B_PRE)]);
        let mut used = Vec::new();
        let (out, path, rule) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("intent must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "one intent section, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("# sweep"), "got:\n{}", out[0]);
        assert!(out[0].contains("- struct Old"), "got:\n{}", out[0]);
        assert!(!rule.is_empty());
        validated_live(&out[0], &pre);
        // Full post trees equal the native post lines at claimed sites.
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["a.rs"].contains("struct New {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["b.rs"].contains("s: New"), "got: {}", files["b.rs"]);
        assert!(
            files["b.rs"].contains("impl New {"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["b.rs"].contains("Old"), "got: {}", files["b.rs"]);
    }

    #[test]
    fn full_pipeline_two_file_compresses() {
        let sections = sections_two_file();
        let pre = pre_map(&[("a.rs", A_PRE), ("b.rs", B_PRE)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    // WI-9: plain-`fn` intents raise with follow.

    const FN_PRE: &str =
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";

    fn fn_sections_two_use() -> Vec<String> {
        vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1);\n+    let b = bar(2);\n"
                .to_string(),
        ]
    }

    fn assert_fn_follow_emission(out: &[String], pre: &HashMap<String, String>) {
        assert_eq!(out.len(), 1, "one intent section, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("# sweep"), "got:\n{}", out[0]);
        assert!(out[0].contains("- fn foo"), "got:\n{}", out[0]);
        assert!(out[0].contains("+ fn bar"), "got:\n{}", out[0]);
        validated_live(&out[0], pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["src/lib.rs"].contains("fn bar(x: i32) -> i32 {"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert!(
            !files["src/lib.rs"].contains("foo"),
            "got: {}",
            files["src/lib.rs"]
        );
    }

    #[test]
    fn fn_call_head_gate() {
        // The `fn` call domain is call callees only (bare,
        // qualified, `let`-wrapped); methods, fields, value paths,
        // macro heads, and `use` lines are not calls (WI-2 `use`
        // tails join through `is_fn_use_tail_site` instead).
        assert!(is_fn_call_use_site("    foo(1);", "foo"));
        assert!(is_fn_call_use_site("    m::foo(2);", "foo"));
        assert!(is_fn_call_use_site("    let a = foo(1);", "foo"));
        assert!(!is_fn_call_use_site("    s.foo();", "foo"));
        assert!(!is_fn_call_use_site("    s.foo;", "foo"));
        assert!(!is_fn_call_use_site("    let x = foo;", "foo"));
        assert!(!is_fn_call_use_site("    foo!(2);", "foo"));
        assert!(!is_fn_call_use_site("    let b = foo!(2);", "foo"));
        assert!(!is_fn_call_use_site("use m::foo;", "foo"));
        assert!(!is_fn_call_use_site("    Some(foo);", "foo"));
    }

    #[test]
    fn fn_use_tail_gate() {
        // WI-2: plain, grouped-member, `as`, and `pub use` tails join;
        // globs, suffix members, intermediate changes, and vis deltas
        // veto.
        assert!(is_fn_use_tail_site(
            "use crate::a::foo;",
            "use crate::a::bar;",
            "foo",
            "bar"
        ));
        assert!(is_fn_use_tail_site(
            "use crate::a::{foo, Other};",
            "use crate::a::{bar, Other};",
            "foo",
            "bar"
        ));
        assert!(is_fn_use_tail_site(
            "use crate::a::foo as f;",
            "use crate::a::bar as f;",
            "foo",
            "bar"
        ));
        assert!(is_fn_use_tail_site(
            "pub use crate::a::foo;",
            "pub use crate::a::bar;",
            "foo",
            "bar"
        ));
        assert!(is_fn_use_tail_site("use foo;", "use bar;", "foo", "bar"));
        assert!(!is_fn_use_tail_site(
            "use crate::a::*;",
            "use crate::a::*;",
            "foo",
            "bar"
        ));
        assert!(!is_fn_use_tail_site(
            "use crate::{x::foo};",
            "use crate::{x::bar};",
            "foo",
            "bar"
        ));
        assert!(!is_fn_use_tail_site(
            "use crate::foo::Bar;",
            "use crate::bar::Bar;",
            "foo",
            "bar"
        ));
        assert!(!is_fn_use_tail_site(
            "use crate::a::foo;",
            "pub use crate::a::bar;",
            "foo",
            "bar"
        ));
        assert!(!is_fn_use_tail_site(
            "use crate::a::foo;",
            "use crate::a::foo;",
            "foo",
            "bar"
        ));
        assert!(!is_fn_use_tail_site(
            "use crate::a::foo;",
            "fn bar() {}",
            "foo",
            "bar"
        ));
    }

    #[test]
    fn fn_intent_use_tail_cross_file_fires() {
        // WI-2 F2: def + `use` tail + call across files → one hunk.
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) -> i32 {\n+pub fn bar(x: i32) -> i32 {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,3 @@\n-use crate::a::foo;\n+use crate::a::bar;\n@@ -3,1 +3,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[
            ("a.rs", "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\n"),
            (
                "b.rs",
                "use crate::a::foo;\nfn m() {\n    let a = foo(1);\n}\n",
            ),
        ]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("use-tail must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert!(out[0].contains("- pub fn foo"), "got:\n{}", out[0]);
        assert!(out[0].contains("+ pub fn bar"), "got:\n{}", out[0]);
        validated_live(&out[0], &pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["b.rs"].contains("use crate::a::bar;"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["b.rs"].contains("foo"), "got: {}", files["b.rs"]);
    }

    #[test]
    fn fn_intent_use_tail_grouped_and_as_fire() {
        for (use_minus, use_plus) in [
            ("use crate::a::{foo, Other};", "use crate::a::{bar, Other};"),
            ("use crate::a::foo as f;", "use crate::a::bar as f;"),
            ("pub use crate::a::foo;", "pub use crate::a::bar;"),
            (
                "pub(crate) use crate::a::foo;",
                "pub(crate) use crate::a::bar;",
            ),
        ] {
            let sections = vec![format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {{\n+fn bar(x: i32) -> i32 {{\n@@ -5,2 +5,2 @@\n-{use_minus}\n-    let a = foo(1);\n+{use_plus}\n+    let a = bar(1);\n"
            )];
            let pre_text = format!(
                "fn foo(x: i32) -> i32 {{\n    x + 1\n}}\nfn m() {{\n{use_minus}\n    let a = foo(1);\n}}\n"
            );
            let pre = pre_map(&[("src/lib.rs", pre_text.as_str())]);
            let mut used = Vec::new();
            let (out, _, _) = try_item_intent_sweep(&sections, &pre, &mut used)
                .unwrap_or_else(|| panic!("{use_minus} must fire"));
            assert!(out[0].contains("- fn foo"), "got:\n{}", out[0]);
            validated_live(&out[0], &pre);
        }
    }

    #[test]
    fn fn_intent_use_tail_vetoes() {
        // Glob, suffix-member, and intermediate-segment deltas never join.
        for (use_minus, use_plus) in [
            ("use crate::a::*;", "use crate::a::*;"),
            ("use crate::{x::foo};", "use crate::{x::bar};"),
            ("use crate::foo::Bar;", "use crate::bar::Bar;"),
        ] {
            let sections = vec![format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {{\n+fn bar(x: i32) -> i32 {{\n@@ -5,1 +5,1 @@\n-{use_minus}\n+{use_plus}\n"
            )];
            let pre = pre_map(&[(
                "src/lib.rs",
                "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n}\n",
            )]);
            let mut used = Vec::new();
            assert!(
                try_item_intent_sweep(&sections, &pre, &mut used).is_none(),
                "{use_minus} must veto"
            );
            assert!(used.is_empty());
        }
    }

    #[test]
    fn fn_intent_second_hop_vetoes() {
        // A re-export chain through two hops is not a proven one-hop
        // tail: the engine's follow vetoes, so the scratch probe fails
        // and the whole intent stays ordinary (never a partial claim).
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) -> i32 {\n+pub fn bar(x: i32) -> i32 {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-pub use crate::a::foo;\n+pub use crate::a::bar;\n"
                .to_string(),
            "diff --git a/c.rs b/c.rs\n--- a/c.rs\n+++ b/c.rs\n@@ -1,1 +1,1 @@\n-pub use crate::b::foo;\n+pub use crate::b::bar;\n"
                .to_string(),
            "diff --git a/d.rs b/d.rs\n--- a/d.rs\n+++ b/d.rs\n@@ -1,1 +1,1 @@\n-use crate::c::foo;\n+use crate::c::bar;\n@@ -3,1 +3,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[
            ("a.rs", "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\n"),
            ("b.rs", "pub use crate::a::foo;\n"),
            ("c.rs", "pub use crate::b::foo;\n"),
            (
                "d.rs",
                "use crate::c::foo;\nfn m() {\n    let a = foo(1);\n}\n",
            ),
        ]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_off_diff_use_tail_vetoes() {
        // An untouched same-tail `use` elsewhere fails the unchanged-leg.
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-use crate::a::foo;\n+use crate::a::bar;\n"
                .to_string(),
        ];
        let pre = pre_map(&[
            ("a.rs", "fn foo(x: i32) -> i32 {\n    x + 1\n}\n"),
            ("b.rs", "use crate::a::foo;\n"),
            ("c.rs", "use crate::a::foo;\n"),
        ]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_fires_with_return_type() {
        // 46 shape: `->` return in the def suffix skips the
        // `try_item_rename` laws; the scratch probe proves the bare
        // terse emission instead.
        let sections = fn_sections_two_use();
        let pre = pre_map(&[("src/lib.rs", FN_PRE)]);
        let mut used = Vec::new();
        let (out, path, rule) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("fn intent must fire");
        assert_eq!(path, "src/lib.rs");
        assert!(!rule.is_empty());
        assert_fn_follow_emission(&out, &pre);
    }

    #[test]
    fn fn_intent_single_use_fires() {
        // 89 shape: def + one call collapses to the same hunk.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,1 +5,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let _ = a;\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("single-use must fire");
        assert_fn_follow_emission(&out, &pre);
    }

    #[test]
    fn fn_intent_qualified_call_fires() {
        // `m::foo` is in the scalar follow domain (round-4 WI-6a pin).
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = m::foo(2);\n+    let a = bar(1);\n+    let b = m::bar(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = m::foo(2);\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("qualified must fire");
        assert_fn_follow_emission(&out, &pre);
    }

    #[test]
    fn fn_intent_cross_file_fires() {
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,1 +2,1 @@\n-    foo(1);\n+    bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[
            ("a.rs", "fn foo(x: i32) -> i32 {\n    x + 1\n}\n"),
            ("b.rs", "fn m() {\n    foo(1);\n}\n"),
        ]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("cross-file must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert!(out[0].contains("- fn foo"), "got:\n{}", out[0]);
        validated_live(&out[0], &pre);
    }

    #[test]
    fn fn_intent_method_use_vetoes() {
        // A use pair whose head is a method ident is outside the
        // scalar follow domain — never a partial claim.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = s.foo();\n+    let a = bar(1);\n+    let b = s.bar();\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = s.foo();\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_macro_head_vetoes() {
        // `foo!(…)` is a macro head (different namespace) — the call
        // carve declines it.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    foo!(2);\n+    let a = bar(1);\n+    bar!(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    foo!(2);\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_off_diff_twin_fails_the_probe() {
        // An untouched `foo(3)` elsewhere fails the unchanged-leg:
        // the follow would rewrite it, so the intent declines instead
        // of under-claiming.
        let sections = fn_sections_two_use();
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n    let c = foo(3);\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_pub_prefix_fires() {
        // WI-1: `pub fn` carries its vis on the terse pair.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) -> i32 {\n+pub fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1);\n+    let b = bar(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("pub fn must fire");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert!(out[0].contains("- pub fn foo"), "got:\n{}", out[0]);
        assert!(out[0].contains("+ pub fn bar"), "got:\n{}", out[0]);
        validated_live(&out[0], &pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            !files["src/lib.rs"].contains("foo"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert!(
            files["src/lib.rs"].contains("pub fn bar(x: i32) -> i32 {"),
            "got: {}",
            files["src/lib.rs"]
        );
    }

    #[test]
    fn fn_intent_vis_matrix_fires() {
        // WI-1: every closed-set vis fires at N=1 (single use keeps the
        // vis on the pair).
        for vis in ["pub(crate) ", "pub(super) ", "pub(in crate::x) "] {
            let def_minus = format!("{vis}fn foo(x: i32) -> i32 {{");
            let def_plus = format!("{vis}fn bar(x: i32) -> i32 {{");
            let sections = vec![format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-{def_minus}\n+{def_plus}\n@@ -5,1 +5,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
            )];
            let pre_minus =
                format!("{def_minus}\n    x + 1\n}}\nfn main() {{\n    let a = foo(1);\n}}\n");
            let pre = pre_map(&[("src/lib.rs", pre_minus.as_str())]);
            let mut used = Vec::new();
            let (out, _, _) = try_item_intent_sweep(&sections, &pre, &mut used)
                .unwrap_or_else(|| panic!("{vis} must fire"));
            assert!(
                out[0].contains(&format!("- {vis}fn foo")),
                "got:\n{}",
                out[0]
            );
            validated_live(&out[0], &pre);
        }
    }

    #[test]
    fn fn_intent_vis_change_vetoes() {
        // A vis delta is an evolution (WI-14's domain), never this arm.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) -> i32 {\n+pub(crate) fn bar(x: i32) -> i32 {\n@@ -5,1 +5,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_self_referential_vis_vetoes() {
        // `pub(in foo)` renaming `foo` is an evolution, not a rename.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-pub(in foo) fn foo(x: i32) -> i32 {\n+pub(in foo) fn bar(x: i32) -> i32 {\n@@ -5,1 +5,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "pub(in foo) fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n}\n",
        )]);
        let mut used = Vec::new();
        // `split_item` cannot parse `pub(in foo)` vis (parens hold a
        // path, not an ident) — either decline path keeps the veto.
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
        // The gate itself fires on the parsed prefix shape.
        assert!(!is_fn_vis_prefix("pub(in foo-bar) "));
        assert!(is_fn_vis_prefix("pub(in crate::foo) "));
        assert!(contains_ident("pub(in crate::foo) ", "foo"));
    }

    #[test]
    fn fn_intent_one_line_def_fires() {
        // WI-1: one-line headers join — the `split_delta` suffix covers
        // the token-identical body.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 { x }\n+fn bar(x: i32) -> i32 { x }\n@@ -3,2 +3,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1);\n+    let b = bar(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 { x }\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("one-line must fire");
        assert!(out[0].contains("- fn foo"), "got:\n{}", out[0]);
        validated_live(&out[0], &pre);
    }

    #[test]
    fn fn_intent_evolution_declines() {
        // Signature deltas are evolutions (C17), never renames.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32, y: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1, 2);\n+    let b = bar(2, 3);\n"
                .to_string(),
        ];
        let mut used = Vec::new();
        let pre = pre_map(&[("src/lib.rs", FN_PRE)]);
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_full_pipeline_compresses() {
        let sections = fn_sections_two_use();
        let pre = pre_map(&[("src/lib.rs", FN_PRE)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- fn foo"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    #[test]
    fn mixed_section_claims_subset_leaves_leftover() {
        // WI-4f-next: one section holds def + use hunks plus an unrelated
        // hunk — the intent fires on the subset, leftover stays verbatim.
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -4,1 +4,1 @@\n-fn f(s: Old) -> i32 {\n+fn f(s: New) -> i32 {\n@@ -8,1 +8,1 @@\n-    let z = 1;\n+    let z = 2;\n"
            .to_string();
        let pre = pre_map(&[(
            "f.rs",
            "struct Old {\n    x: i32,\n}\nfn f(s: Old) -> i32 {\n    0\n}\nfn u() {\n    let z = 1;\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) = try_item_intent_sweep(std::slice::from_ref(&section), &pre, &mut used)
            .expect("subset must fire");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("- struct Old"), "got:\n{}", out[0]);
        assert!(
            out[0].contains("-    let z = 1;"),
            "leftover verbatim:\n{}",
            out[0]
        );
        assert!(
            out[0].contains("+    let z = 2;"),
            "leftover verbatim:\n{}",
            out[0]
        );
        validated_live(&out[0], &pre);
    }

    #[test]
    fn def_hunk_with_extra_pair_still_declines() {
        // No partial def hunk: def + extra pair in the SAME hunk declines.
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n-struct Old {\n-    x: i32,\n+struct New {\n+    x: u32,\n"
            .to_string();
        let pre = pre_map(&[("f.rs", "struct Old {\n    x: i32,\n}\n")]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(std::slice::from_ref(&section), &pre, &mut used).is_none());
    }

    #[test]
    fn hybrid_section_native_pairs_still_fire() {
        // WI-4e: a hybrid section (pre-existing `rule=` hunk + native
        // struct-rename pairs) still raises the intent on the native
        // hunks, preserving the semantic hunk verbatim.
        let hybrid = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -10,0 +10,0 @@ rule=r1\n# bind=M1\n- I\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -4,1 +4,1 @@\n-fn f(s: Old) -> i32 {\n+fn f(s: New) -> i32 {\n"
            .to_string();
        let pre = pre_map(&[(
            "f.rs",
            "struct Old {\n    x: i32,\n}\nfn f(s: Old) -> i32 {\n    0\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) = try_item_intent_sweep(std::slice::from_ref(&hybrid), &pre, &mut used)
            .expect("native pairs in hybrid must fire");
        assert_eq!(out.len(), 1, "one section, hybrid preserved: {out:?}");
        assert!(
            out[0].contains("rule=r1"),
            "preserved hunk kept:\n{}",
            out[0]
        );
        assert!(
            out[0].contains("# follow=uses"),
            "new intent present:\n{}",
            out[0]
        );
        assert!(out[0].contains("- struct Old"), "got:\n{}", out[0]);
        let parsed = rsmpl_patch::parse_patch(&out[0]).expect("hybrid output must parse");
        assert_eq!(parsed.files.len(), 1);
    }
}
