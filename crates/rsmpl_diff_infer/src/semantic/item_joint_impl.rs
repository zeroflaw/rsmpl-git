//! WI-3 joint double-delta intent: `impl Trait for Type` renames as one.
//!
//! A trait rename composed with its self-type rename
//! (`trait Printable {}` → `trait Labeled {}` plus `struct Old;` →
//! `struct New;` plus the composed header `impl Printable for Old {` →
//! `impl Labeled for New {`) re-diffs wholly ordinary: both header sides
//! change, so no single-delta detector starts, and the item-intent
//! pre-pass needs exactly one definition delta. This pre-pass claims the
//! triple as one shared-rule intent (D13): two `@@ -1,1 @@` + `# sweep` +
//! `# follow=uses` hunks sharing one rule id (trait pair first, self-type
//! pair second — SPEC 14.8 def-before-member law). The composed `impl`
//! header is never stated: the two follows rewrite it (probe-verified),
//! exactly like the WI-5 field-add two-hunk emission states only the
//! def pair plus the addition.
//!
//! Slice law (WI-3b): single section here, the triple (trait def +
//! `struct`/`enum`/`union`/`type`-alias self-ty def + composed `impl`
//! double-delta) plus zero or more `extra_claimable` follow-use lines,
//! same file. Any unbalanced hunk, any hybrid (pre-existing semantic
//! hunks), any def-shaped or double-delta extra (a third rename), or
//! any unclaimable changed line declines. Cross-file trait/type/impl
//! groups live in the `item_joint_impl_xfile` sibling (WI-12).
//!
//! Totality is the D13 joint scratch-apply probe: the composed emission
//! applied to a `pre` clone must reproduce the native plus lines at
//! exactly the claimed sites (triple plus extras) with every other line of every
//! pre-image file byte-identical (an off-diff same-name use rewritten by
//! either follow fails the unchanged leg — the uniqueness guard). The
//! T375 pure-section byte guard applies unchanged (diet emission must
//! beat the native section bytes).

use std::collections::HashMap;

use super::item_intent::{count_kind_defs, Pair};
use super::item_intent_sites::is_dedicated_use_site;
use super::{split_item, unique_rule};

/// Whole-patch joint-`impl` pre-pass: returns the rebuilt section list
/// (the claimed section replaced by the single shared-rule emission) plus
/// the definition path and rule for provenance, or `None` when the patch
/// holds no provable double-delta intent. Reserved rule names are
/// released on decline.
///
/// WI-12 slice 1: the cross-file path runs first and declines
/// single-section-pure shapes (the single-file arm owns those
/// byte-identically); otherwise the single-file arm runs unchanged.
/// WI-12 slice 3: the cross-file path lives in the
/// `item_joint_impl_xfile` sibling (D5 split).
pub(crate) fn try_joint_impl_rename(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    if let Some(out) =
        super::item_joint_impl_xfile::try_joint_impl_cross_file(sections, pre, used_rules)
    {
        return Some(out);
    }
    try_joint_impl_single_section(sections, pre, used_rules)
}

fn try_joint_impl_single_section(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    // Single-section slice: exactly one section holding the triple plus
    // zero or more follow-use lines (same file). Any unbalanced hunk or hybrid section
    // (pre-existing semantic hunks) declines the whole pre-pass.
    struct Cand {
        sec: usize,
        path: String,
        section: String,
        pairs: Vec<(String, String, usize)>,
    }
    let mut cand: Option<Cand> = None;
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
        let (_prelude, semantic, native_view) = super::split_semantic_native(sec);
        if !semantic.is_empty() {
            continue;
        }
        let Some(native_sec) = native_view else {
            continue;
        };
        let hunks = super::split_native_hunks(&native_sec);
        if hunks.iter().any(|h| !h.balanced) {
            used_rules.truncate(checkpoint);
            return None;
        }
        let mut pairs: Vec<(String, String, usize)> = Vec::new();
        for h in &hunks {
            if h.minuses.len() != h.sites.len() || h.minuses.len() != h.pluses.len() {
                used_rules.truncate(checkpoint);
                return None;
            }
            for ((m, p), s) in h.minuses.iter().zip(h.pluses.iter()).zip(h.sites.iter()) {
                pairs.push((m.clone(), p.clone(), *s));
            }
        }
        if pairs.is_empty() {
            continue;
        }
        if pairs.len() < 3 {
            continue;
        }
        if cand.is_some() {
            // Two 3-pair sections: each is a separate triple only if both
            // composed — this slice claims at most one triple per patch
            // (the second stays for a later pass; it cannot run twice in
            // one chain walk, so decline to keep the probe total).
            used_rules.truncate(checkpoint);
            return None;
        }
        cand = Some(Cand {
            sec: i,
            path: path.to_string(),
            section: sec.clone(),
            pairs,
        });
    }
    let cand = cand?;
    // Role assignment: exactly one trait def pair, one self-type def pair
    // (`struct`/`enum`/`union`/`type` alias, suffix-equal), one composed
    // impl double-delta — each role exactly once — plus zero or more
    // follow-use extras (WI-3b). A second def-shaped or double-delta pair
    // is a third rename, never a follow-use.
    let mut trait_leg: Option<(String, String, String, usize)> = None;
    let mut ty_leg: Option<(String, String, String, String, usize)> = None;
    let mut impl_leg: Option<(String, String, String, String, usize)> = None;
    let mut extras: Vec<(String, String, usize)> = Vec::new();
    for (m, p, site) in &cand.pairs {
        if let Some((prefix, kind, old, new)) = split_def_pair(m, p) {
            if kind == "trait" {
                if trait_leg.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                trait_leg = Some((prefix, old, new, *site));
                continue;
            }
            if matches!(kind.as_str(), "struct" | "enum" | "union" | "type") {
                if ty_leg.is_some() {
                    used_rules.truncate(checkpoint);
                    return None;
                }
                ty_leg = Some((prefix, kind, old, new, *site));
                continue;
            }
            // Any other def-shaped pair is a third rename, never a use.
            used_rules.truncate(checkpoint);
            return None;
        }
        if let Some((ot, nt, oy, ny)) = parse_impl_double_delta(m, p) {
            if impl_leg.is_some() {
                used_rules.truncate(checkpoint);
                return None;
            }
            impl_leg = Some((ot, nt, oy, ny, *site));
            continue;
        }
        extras.push((m.clone(), p.clone(), *site));
    }
    let Some((tr_prefix, ot, nt, tr_site)) = trait_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let Some((ty_prefix, ty_kind, oy, ny, _ty_site)) = ty_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let Some((iot, int_, ioy, iny, _impl_site)) = impl_leg else {
        used_rules.truncate(checkpoint);
        return None;
    };
    // Composition: the impl header must spell exactly the two renames.
    if iot != ot || int_ != nt || ioy != oy || iny != ny {
        used_rules.truncate(checkpoint);
        return None;
    }
    // WI-3b extras: every non-triple pair must be a follow-use line of
    // at least one rename half (ident-boundary + dedicated-position
    // gate; the D13 probe below stays authoritative).
    for (m, p, _) in &extras {
        // A same-kind definition delta naming either renamed ident is a
        // third definition delta (e.g. the self-ty with an evolved RHS),
        // never a follow-use line.
        if let (Some((_, ok, oi, _)), Some((_, nk, ni, _))) =
            (split_item(m.trim_end()), split_item(p.trim_end()))
        {
            if ok == nk && (oi == ot || oi == oy || ni == nt || ni == ny) {
                used_rules.truncate(checkpoint);
                return None;
            }
        }
        if !extra_claimable(m, p, &ot, &nt, &oy, &ny) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    // Twin-definition veto (unique-definition rule, SPEC 14.8): exactly
    // one `trait ot` and one `{kind} oy` definition across the claimed
    // file's pre-image.
    let Some(pre_text) = pre.get(&cand.path) else {
        used_rules.truncate(checkpoint);
        return None;
    };
    let probes = [Pair {
        sec: cand.sec,
        path: cand.path.clone(),
        minus: String::new(),
        plus: String::new(),
        site: tr_site,
    }];
    if count_kind_defs(pre, &probes, "trait", &ot) != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    if count_kind_defs(pre, &probes, &ty_kind, &oy) != 1 {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Pre-image alignment: each claimed site must show its minus line.
    let pre_lines: Vec<&str> = pre_text.lines().collect();
    let at = |site: usize| pre_lines.get(site.saturating_sub(1)).copied().unwrap_or("");
    // Map each role back to its raw minus text for the alignment check.
    let minus_of = |site: usize| {
        cand.pairs
            .iter()
            .find(|(_, _, s)| *s == site)
            .map(|(m, _, _)| m.as_str())
            .unwrap_or("")
    };
    for (_, _, site) in &cand.pairs {
        if at(*site).trim() != minus_of(*site).trim() {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    // Build the emission: trait follow hunk first, self-type second, one
    // shared rule. Concrete prefixes ride literally (WI-1 law).
    let rule = unique_rule(used_rules, &format!("rename_{ot}_to_{nt}"));
    let mut emission = crate::git_header(&cand.path);
    emission.push('\n');
    emission.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    emission.push_str("# sweep\n");
    emission.push_str("# follow=uses\n");
    emission.push_str(&format!(
        "- {tr_prefix}trait {ot}\n+ {tr_prefix}trait {nt}\n"
    ));
    emission.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    emission.push_str("# sweep\n");
    emission.push_str("# follow=uses\n");
    emission.push_str(&format!(
        "- {ty_prefix}{ty_kind} {oy}\n+ {ty_prefix}{ty_kind} {ny}\n"
    ));
    // D13 joint probe: composed emission on a `pre` clone must equal pre
    // with exactly the three minus lines replaced by the plus lines, in
    // every pre-image file (off-diff same-name uses rewritten by either
    // follow fail the unchanged leg).
    if !probe_joint(pre, &emission, &cand.pairs) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Strict T375 over the normalized span (diet must beat native).
    if !super::guard::within_strict(
        emission.len(),
        super::guard::normalized_section_len(&cand.section),
    ) {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(sections.len());
    for (i, sec) in sections.iter().enumerate() {
        if i == cand.sec {
            out.push(emission.clone());
        } else {
            out.push(sec.clone());
        }
    }
    Some((out, cand.path.clone(), rule))
}

/// A `split_item` definition delta, or `None` for use/impl lines. Mirrors
/// `item_intent`'s private `split_delta` (this module must not grow that
/// 1912-line file per D5).
pub(crate) fn split_def_pair(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
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
/// WI-3b follow-use gate for extras riding the triple: true when the
/// pair renames at least one full half (`old` in minus and `new` in
/// plus, ident-boundary) and no claimed half sits in a dedicated
/// detector position (call callee / macro head — those classes keep
/// their pinned shapes). A half mentioned on only one side (partial
/// mention) vetoes; overlap (both halves) is allowed — the D13 probe
/// owns disjoint-span rewriting. `use`-tail deltas pass (ident
/// containment, never dedicated).
pub(crate) fn extra_claimable(
    minus: &str,
    plus: &str,
    ot: &str,
    nt: &str,
    oy: &str,
    ny: &str,
) -> bool {
    if minus.trim() == plus.trim() {
        return false;
    }
    // A half mentioned on only one side is not a rename line.
    if contains_ident(minus, ot) != contains_ident(plus, nt) {
        return false;
    }
    if contains_ident(minus, oy) != contains_ident(plus, ny) {
        return false;
    }
    let t = contains_ident(minus, ot) && contains_ident(plus, nt);
    let y = contains_ident(minus, oy) && contains_ident(plus, ny);
    if !t && !y {
        return false;
    }
    if t && is_dedicated_use_site(minus, ot) {
        return false;
    }
    if y && is_dedicated_use_site(minus, oy) {
        return false;
    }
    true
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

/// Parse the composed double-delta header
/// `impl[<params>] OldTr for OldTy<sfx>` → `impl[<params>] NewTr for
/// NewTy<same sfx>`, or `None` for anything else (single-delta legs belong
/// to `impl_single`; `dyn`/`!` heads decline). A generic prefix (when
/// present) must compare byte-equal across the pair (proven, never
/// invented); the trait side may be bare or qualified with a byte-equal
/// prefix and a renamed tail.
pub(crate) fn parse_impl_double_delta(
    minus: &str,
    plus: &str,
) -> Option<(String, String, String, String)> {
    let old = minus.trim();
    let new = plus.trim();
    let old_rest = strip_impl_head(old)?;
    let new_rest = strip_impl_head(new)?;
    let (old_gen, old_rest) = strip_impl_generics(old_rest)?;
    let (new_gen, new_rest) = strip_impl_generics(new_rest)?;
    if old_gen != new_gen {
        return None;
    }
    let (old_tr, old_ty) = split_for(old_rest)?;
    let (new_tr, new_ty) = split_for(new_rest)?;
    let (old_tr, new_tr) = split_qualified_tail(old_tr, new_tr)?;
    if old_tr == new_tr {
        return None;
    }
    let (oy, old_sfx) = split_self_ty(old_ty)?;
    let (ny, new_sfx) = split_self_ty(new_ty)?;
    if oy == ny || old_sfx != new_sfx {
        return None;
    }
    Some((
        old_tr.to_string(),
        new_tr.to_string(),
        oy.to_string(),
        ny.to_string(),
    ))
}
/// Strip the `impl` head word, leaving the generic prefix (if any) or the
/// trait side. `impl Foo` (any single space/tab run) and `impl<T> Foo`
/// admit; anything else (`implements`, bare `impl`) declines.
pub(crate) fn strip_impl_head(s: &str) -> Option<&str> {
    let rest = s.strip_prefix("impl")?;
    if rest.starts_with('<') {
        return Some(rest);
    }
    let trimmed = rest.trim_start_matches([' ', '\t']);
    if trimmed.len() == rest.len() {
        return None;
    }
    Some(trimmed)
}

/// Split an `impl`-rest at the single `for` word: `Tr for Ty …` →
/// `Some(("Tr", "Ty …"))`. Same word-boundary discipline as
/// `impl_single` (its helper is private; `Format`/`before` never split).
fn split_for(rest: &str) -> Option<(&str, &str)> {
    let bytes = rest.as_bytes();
    let mut hits: Vec<usize> = Vec::new();
    let mut i = 0usize;
    while i + 3 <= bytes.len() {
        let boundary_before = i == 0 || {
            let c = bytes[i - 1] as char;
            !(c.is_ascii_alphanumeric() || c == '_')
        };
        let boundary_after = i + 3 >= bytes.len() || {
            let c = bytes[i + 3] as char;
            !(c.is_ascii_alphanumeric() || c == '_')
        };
        if rest[i..].starts_with("for") && boundary_before && boundary_after {
            hits.push(i);
            i += 3;
        } else {
            i += 1;
        }
    }
    if hits.len() != 1 {
        return None;
    }
    let at = hits[0];
    Some((rest[..at].trim_end(), rest[at + 3..].trim_start()))
}

/// True for a single unqualified ident (no paths, no `!`, no generics).
pub(crate) fn is_bare_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        && super::ident_len(s) == Some(s.len())
}

/// Strip an optional `impl<…>` generic prefix (balanced brackets) from an
/// `impl`-rest, returning (prefix, remainder). No generics → (`""`, rest).
/// A `<` that never closes to depth 0 followed by whitespace declines
/// (`None`) rather than guessing.
pub(crate) fn strip_impl_generics(rest: &str) -> Option<(&str, &str)> {
    if !rest.starts_with('<') {
        return Some(("", rest));
    }
    let bytes = rest.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] as char {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    let prefix = &rest[..=i];
                    let after = &rest[i + 1..];
                    if after.is_empty() || !(after.starts_with(' ') || after.starts_with('\t')) {
                        return None;
                    }
                    return Some((prefix, after.trim_start()));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a trait-side pair into (old_tail, new_tail) when both sides are
/// bare idents or qualified paths with byte-equal leading segments.
/// Returns the renamed tails, or `None` when the shapes differ.
pub(crate) fn split_qualified_tail(old_tr: &str, new_tr: &str) -> Option<(String, String)> {
    let (old_pre, old_last) = qualified_prefix_tail(old_tr)?;
    let (new_pre, new_last) = qualified_prefix_tail(new_tr)?;
    if old_pre != new_pre {
        return None;
    }
    if !is_bare_ident(old_last) || !is_bare_ident(new_last) {
        return None;
    }
    Some((old_last.to_string(), new_last.to_string()))
}

/// Split `path` into (prefix, tail) at the last `::`. Bare idents yield
/// (`""`, ident). Empty tails, empty segments, and generic tails decline.
pub(crate) fn qualified_prefix_tail(path: &str) -> Option<(&str, &str)> {
    match path.rfind("::") {
        None => Some(("", path)),
        Some(at) => {
            let (pre, tail) = (&path[..at], &path[at + 2..]);
            if pre.is_empty() || tail.is_empty() || pre.ends_with(':') {
                return None;
            }
            let segs = pre.strip_prefix("::").unwrap_or(pre);
            if segs.is_empty()
                || segs.split("::").any(|s| !is_bare_ident(s))
                || !is_bare_ident(tail)
            {
                return None;
            }
            Some((&path[..at], tail))
        }
    }
}

/// Split a self-ty side into (leading ident, byte suffix). The suffix
/// (generics tail, `where`, `{`) must compare byte-equal across the pair.
fn split_self_ty(s: &str) -> Option<(&str, &str)> {
    let len = super::ident_len(s)?;
    if len == 0 || len == s.len() {
        return None;
    }
    Some((&s[..len], &s[len..]))
}

/// D13 joint probe for the triple: the composed emission applied to a
/// `pre` clone must show each native plus line at its claimed site with
/// every other line of every pre-image file byte-identical.
fn probe_joint(
    pre: &HashMap<String, String>,
    emission: &str,
    pairs: &[(String, String, usize)],
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
    // The emission touches exactly one path (single-section slice).
    let path = &parsed.files[0].target_path;
    let Some(pre_text) = pre.get(path) else {
        return false;
    };
    let Some(post_text) = files.get(path) else {
        return false;
    };
    let pre_lines: Vec<&str> = pre_text.lines().collect();
    let post_lines: Vec<&str> = post_text.lines().collect();
    if pre_lines.len() != post_lines.len() {
        return false;
    }
    let mut claims: HashMap<usize, &str> = HashMap::new();
    for (_, plus, site) in pairs {
        claims.insert(*site, plus.trim());
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        let line_no = idx + 1;
        if let Some(want) = claims.get(&line_no) {
            if post_line.trim() != *want {
                return false;
            }
        } else if pre_line != post_line {
            return false;
        }
    }
    // Every other pre-image file byte-identical (workspace follow must
    // not leak into untouched files).
    for (p, pre_file) in pre {
        if p == path {
            continue;
        }
        if files.get(p) != Some(pre_file) {
            return false;
        }
    }
    true
}

/// WI-6(b) double-delta widening arm: one `impl`-double-delta pair
/// absorbed by two widened single-delta sweeps (SPEC D.4).
///
/// The authored `14` grouping covers the double-delta line
/// (`impl Printable for Old {` → `impl Labeled for New {`) with two
/// overlapping wide sweeps — `impl Old` over the self-ty head block
/// through the double-delta block, `impl Printable` over the
/// trait-side head block through the double-delta block — while the
/// trait/self-ty defs and every unrelated pair raise as tight singles.
/// The raiser's tight singles exclude the double-delta line, so it
/// stays ordinary. This arm replays the authored grouping when the
/// section holds exactly: one double-delta pair, its trait-def pair,
/// its self-ty-def pair (`struct`/`enum`/`union`/`type`), one inherent
/// self-ty head pair (no `for`), one trait-side head pair (with `for`,
/// unchanged self-ty), and any number of rest pairs that each fire
/// `try_item_rename` (bare defs). Anything else declines to today's
/// behavior (`Counted::Miss` — the 8 singles + ordinary remainder).
///
/// Emission: rest singles (their current tight bodies) plus the two
/// head bodies re-headered to the `syn` cover of head + double-delta
/// item spans (the authored `@@ -18,23 @@` / `@@ -30,11 @@` ranges by
/// construction), hunks ordered by start. Soundness is the D13 section
/// probe (`probe_joint` over every pair): the assembled emission
/// applied to a pre clone must show each native plus line at its site
/// with every other line byte-identical — range-cited hunks cannot
/// over-match, and the probe fails the arm on any mismatch. Pricing is
/// the caller's `pair_section_gate` (`item_rename`, Precision).
pub(crate) fn try_impl_double_delta_section(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    use super::prov::Counted;
    if minuses.len() != pluses.len() || minuses.len() < 5 || changed.len() != minuses.len() {
        return Counted::Miss;
    }
    let pre = match file_old {
        Some(p) => p,
        None => return Counted::Miss,
    };
    // Cheap prefilter: a double-delta needs an `impl … for …` minus.
    if !minuses
        .iter()
        .any(|m| m.trim_start().starts_with("impl ") && has_word(m, "for"))
    {
        return Counted::Miss;
    }
    let checkpoint = used_rules.len();
    let pairs: Vec<(String, String, usize)> = minuses
        .iter()
        .zip(pluses.iter())
        .zip(changed.iter())
        .map(|((m, p), s)| (m.to_string(), p.to_string(), *s))
        .collect();
    // Exactly one double-delta pair.
    let mut dd_idx: Option<usize> = None;
    let mut dd_names: Option<(String, String, String, String)> = None;
    for (i, (m, p, _)) in pairs.iter().enumerate() {
        if !m.trim_start().starts_with("impl ") {
            continue;
        }
        if let Some(names) = parse_impl_double_delta(m, p) {
            if dd_idx.is_some() {
                used_rules.truncate(checkpoint);
                return Counted::Miss;
            }
            dd_idx = Some(i);
            dd_names = Some(names);
        }
    }
    let (dd_at, (ot, nt, oy, ny)) = match (dd_idx, dd_names) {
        (Some(i), Some(n)) => (pairs[i].2, n),
        _ => {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        }
    };
    // Exactly one trait-def pair and one self-ty-def pair naming the
    // double-delta halves (cross-file defs are not in this section, so
    // the arm declines — residue).
    let mut trait_idx: Option<usize> = None;
    let mut ty_idx: Option<usize> = None;
    for (i, (m, p, _)) in pairs.iter().enumerate() {
        if Some(i) == dd_idx {
            continue;
        }
        let Some((_, kind, old_ident, new_ident)) = split_def_pair(m, p) else {
            continue;
        };
        if kind == "trait" && old_ident == ot && new_ident == nt {
            if trait_idx.is_some() {
                used_rules.truncate(checkpoint);
                return Counted::Miss;
            }
            trait_idx = Some(i);
        } else if matches!(kind.as_str(), "struct" | "enum" | "union" | "type")
            && old_ident == oy
            && new_ident == ny
        {
            if ty_idx.is_some() {
                used_rules.truncate(checkpoint);
                return Counted::Miss;
            }
            ty_idx = Some(i);
        }
    }
    if trait_idx.is_none() || ty_idx.is_none() {
        used_rules.truncate(checkpoint);
        return Counted::Miss;
    }
    // Exactly one inherent self-ty head (no `for`, delta oy→ny) and one
    // trait-side head (with `for`, delta ot→nt), each a valid tight
    // single today (the widened emission reuses their pair spelling).
    let mut head_ty_idx: Option<usize> = None;
    let mut head_tr_idx: Option<usize> = None;
    for (i, (m, p, _)) in pairs.iter().enumerate() {
        if Some(i) == dd_idx || Some(i) == trait_idx || Some(i) == ty_idx {
            continue;
        }
        let has_for = has_word(m, "for") || has_word(p, "for");
        let Some(single) = super::impl_single::parse_impl_pair(m, p) else {
            continue;
        };
        if !has_for && single.0 == oy && single.1 == ny {
            if head_ty_idx.is_some() {
                used_rules.truncate(checkpoint);
                return Counted::Miss;
            }
            head_ty_idx = Some(i);
        } else if has_for && single.0 == ot && single.1 == nt {
            if head_tr_idx.is_some() {
                used_rules.truncate(checkpoint);
                return Counted::Miss;
            }
            head_tr_idx = Some(i);
        }
    }
    let (head_ty_at, head_tr_at) = match (head_ty_idx, head_tr_idx) {
        (Some(a), Some(b)) => (pairs[a].2, pairs[b].2),
        _ => {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        }
    };
    // Every remaining pair must be a bare item rename (the defs found
    // above included — they raise as tight singles here, exactly like
    // the authored grouping); anything else declines the arm.
    let mut hunk_bodies: Vec<(usize, String)> = Vec::new();
    for (i, (m, p, site)) in pairs.iter().enumerate() {
        if Some(i) == dd_idx || Some(i) == head_ty_idx || Some(i) == head_tr_idx {
            continue;
        }
        let Some(sec) = super::try_item_rename(m, p, path, Some(pre), &[*site], used_rules) else {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        };
        let Some((_, body)) = sec.split_once('\n') else {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        };
        hunk_bodies.push((*site, body.to_string()));
    }
    // Widen the two head singles to cover the double-delta item span.
    // The tight bodies prove the pair shapes; the section probe below
    // proves the widened ranges.
    for (head_i, head_site) in [
        (head_ty_idx.unwrap(), head_ty_at),
        (head_tr_idx.unwrap(), head_tr_at),
    ] {
        let (m, p, _) = &pairs[head_i];
        let ms: &str = m;
        let ps: &str = p;
        let tight = match super::impl_single::try_impl_header_single(
            std::slice::from_ref(&ms),
            std::slice::from_ref(&ps),
            path,
            Some(pre),
            &[head_site],
            used_rules,
        ) {
            Counted::Fire((s, _)) => s,
            _ => {
                used_rules.truncate(checkpoint);
                return Counted::Miss;
            }
        };
        let Some((_, tight_hunk)) = tight.split_once('\n') else {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        };
        // Cover of head item + double-delta item (the authored ranges
        // by construction); a sweep dummy (span >64) declines.
        let cover = super::item_emission_header(path, "r0", Some(pre), &[head_site, dd_at]);
        if cover.contains("# sweep") {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        }
        let Some(wide_range) = cover.lines().nth(1) else {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        };
        let mut lines: Vec<String> = tight_hunk.lines().map(str::to_string).collect();
        if lines.is_empty() || !lines[0].starts_with("@@") {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        }
        // Keep the reserved rule name; swap in the widened range.
        let Some(old_rule) = lines[0]
            .split_whitespace()
            .find_map(|w| w.strip_prefix("rule="))
        else {
            used_rules.truncate(checkpoint);
            return Counted::Miss;
        };
        let new_head = wide_range.replacen("rule=r0", &format!("rule={old_rule}"), 1);
        lines[0] = new_head;
        let start: usize = wide_range
            .split_whitespace()
            .nth(1)
            .and_then(|r| r.strip_prefix('-'))
            .and_then(|r| r.split(',').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(head_site);
        hunk_bodies.push((start, lines.join("\n") + "\n"));
    }
    // File-ordered assembly (authored hunk order).
    hunk_bodies.sort_by_key(|(s, _)| *s);
    let mut out = crate::git_header(path);
    out.push('\n');
    for (_, body) in &hunk_bodies {
        out.push_str(body);
    }
    // D13 section probe over every pair (heads, defs, rest): the
    // assembled emission must reproduce each native plus line at its
    // site with every other line byte-identical.
    let mut pre_map = HashMap::new();
    pre_map.insert(path.to_string(), pre.to_string());
    if !probe_joint(&pre_map, &out, &pairs) {
        used_rules.truncate(checkpoint);
        return Counted::Miss;
    }
    Counted::Fire(out)
}

/// Word-boundary containment (never substring): the `for`-word gate
/// for the double-delta widening arm.
fn has_word(haystack: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    haystack
        .split(|c: char| c != '_' && !c.is_ascii_alphanumeric())
        .any(|w| w == word)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const PRE: &str = "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\n";

    fn triple_section() -> String {
        section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        )
    }

    fn fire(sections: &[String], pre: &HashMap<String, String>) -> Option<String> {
        let mut used = Vec::new();
        try_joint_impl_rename(sections, pre, &mut used).map(|(out, _, _)| out.join(""))
    }

    #[test]
    fn joint_impl_triple_fires_verified() {
        let pre = pre_map(&[("f.rs", PRE)]);
        let out = fire(&[triple_section()], &pre).expect("triple must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("- trait Printable\n"), "got: {out}");
        assert!(out.contains("+ trait Labeled\n"), "got: {out}");
        assert!(out.contains("- struct Old\n"), "got: {out}");
        assert!(out.contains("+ struct New\n"), "got: {out}");
        // One shared rule across both hunks, trait first.
        assert_eq!(out.matches("rule=r1\n").count(), 2, "got: {out}");
        assert!(
            out.find("- trait Printable").unwrap() < out.find("- struct Old").unwrap(),
            "got: {out}"
        );
        // Real-apply byte-identical.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "trait Labeled {}\nstruct New;\nimpl Labeled for New {\n}\n"
        );
    }

    #[test]
    fn joint_impl_enum_and_union_carriers_fire() {
        for (kind, pre, minus, plus, body) in [
            (
                "enum",
                "trait Printable {}\nenum Old {\n    A,\n}\nimpl Printable for Old {\n}\n",
                "enum Old {",
                "enum New {",
                "@@ -1,6 +1,6 @@\n-trait Printable {}\n-enum Old {\n+trait Labeled {}\n+enum New {\n     A,\n }\n-impl Printable for Old {\n+impl Labeled for New {\n }\n",
            ),
            (
                "union",
                "trait Printable {}\nunion Old {\n    x: i32,\n}\nimpl Printable for Old {\n}\n",
                "union Old {",
                "union New {",
                "@@ -1,6 +1,6 @@\n-trait Printable {}\n-union Old {\n+trait Labeled {}\n+union New {\n     x: i32,\n }\n-impl Printable for Old {\n+impl Labeled for New {\n }\n",
            ),
        ] {
            let _ = (minus, plus);
            let native = section("f.rs", body);
            let pre = pre_map(&[("f.rs", pre)]);
            let out = fire(&[native], &pre).expect("{kind} carrier must fire");
            assert!(out.contains(&format!("- {kind} Old\n")), "got: {out}");
            let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
            let mut files = pre.clone();
            rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
            assert!(files["f.rs"].contains("impl Labeled for New {"), "got: {}", files["f.rs"]);
        }
    }

    #[test]
    fn joint_impl_pub_prefix_fires_concrete() {
        let pre = "pub trait Printable {}\npub struct Old;\nimpl Printable for Old {\n}\n";
        let native = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-pub trait Printable {}\n-pub struct Old;\n-impl Printable for Old {\n+pub trait Labeled {}\n+pub struct New;\n+impl Labeled for New {\n",
        );
        let pre = pre_map(&[("f.rs", pre)]);
        let out = fire(&[native], &pre).expect("pub triple must fire");
        assert!(out.contains("- pub trait Printable\n"), "got: {out}");
        assert!(out.contains("- pub struct Old\n"), "got: {out}");
        assert!(!out.contains(" V "), "never a hole, got: {out}");
    }

    #[test]
    fn joint_impl_veto_matrix() {
        let pre = pre_map(&[("f.rs", PRE)]);
        // Third rename in the section stays ordinary.
        let four = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n-fn helper() {}\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n+fn ren_helper() {}\n }\n",
        );
        assert!(fire(&[four], &pre).is_none());
        // Mismatched composition (impl names a third ident) declines.
        let mismatch = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for Other {\n",
        );
        assert!(fire(&[mismatch], &pre).is_none());
        // Qualified trait path composes (prefix proven, tail renamed).
        let qpre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl crate::Printable for Old {\n}\n",
        )]);
        let qualified = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl crate::Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl crate::Labeled for New {\n",
        );
        let out = fire(&[qualified], &qpre).expect("qualified triple must fire");
        assert_eq!(out.matches("rule=r1\n").count(), 2, "got: {out}");
        assert!(
            !out.contains("impl crate::Printable"),
            "headers rewritten, never stated, got: {out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = qpre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "trait Labeled {}\nstruct New;\nimpl crate::Labeled for New {\n}\n"
        );
        // Generic `impl<…>` prefix composes (prefix proven byte-equal).
        let gpre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl<T> Printable for Old {\n}\n",
        )]);
        let generic = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl<T> Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl<T> Labeled for New {\n",
        );
        let out = fire(&[generic], &gpre).expect("generic triple must fire");
        assert_eq!(out.matches("rule=r1\n").count(), 2, "got: {out}");
        assert!(out.contains("- trait Printable\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = gpre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "trait Labeled {}\nstruct New;\nimpl<T> Labeled for New {\n}\n"
        );
        // Single-delta legs are not triples.
        let two = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-trait Printable {}\n-struct Old;\n+trait Labeled {}\n+struct New;\n",
        );
        assert!(fire(&[two], &pre).is_none());
        // Identical names decline.
        let same = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Printable {}\n+struct Old;\n+impl Printable for Old {\n",
        );
        assert!(fire(&[same], &pre).is_none());
    }

    #[test]
    fn joint_impl_header_prefix_mismatch_declines() {
        let pre = pre_map(&[("f.rs", PRE)]);
        // Generic prefix must compare byte-equal: `<T>` vs `<U>` declines.
        let gpre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl<T> Printable for Old {\n}\n",
        )]);
        let generic_mismatch = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl<T> Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl<U> Labeled for New {\n",
        );
        assert!(fire(&[generic_mismatch], &gpre).is_none());
        // Qualified prefix must compare byte-equal: `crate::` vs `other::` declines.
        let qpre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl crate::Printable for Old {\n}\n",
        )]);
        let qualified_mismatch = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl crate::Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl other::Labeled for New {\n",
        );
        assert!(fire(&[qualified_mismatch], &qpre).is_none());
        // Bare-vs-qualified shape mismatch declines.
        let shape_mismatch = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl crate::Labeled for New {\n",
        );
        assert!(fire(&[shape_mismatch], &pre).is_none());
    }

    #[test]
    fn joint_impl_twin_def_vetoes() {
        // Two `struct Old` definitions in the pre-image: the type rename
        // is no longer unique-definition, so the joint must decline.
        let twin_pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nmod m {\n    pub struct Old;\n}\n",
        )]);
        let native = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        );
        assert!(fire(&[native], &twin_pre).is_none());
    }

    #[test]
    fn joint_impl_misaligned_pre_image_declines() {
        let pre = pre_map(&[("f.rs", PRE)]);
        // Claimed site shows a different line in pre.
        let native = section(
            "f.rs",
            "@@ -2,4 +2,4 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n",
        );
        assert!(fire(&[native], &pre).is_none());
    }

    #[test]
    fn joint_impl_off_diff_use_vetoes() {
        // An off-diff `Old` use elsewhere in the file: the self-type
        // follow rewrites it, failing the unchanged leg.
        let pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nfn f(x: Old) -> Old {\n    x\n}\n",
        )]);
        assert!(fire(&[triple_section()], &pre).is_none());
    }

    #[test]
    fn joint_impl_parse_impl_double_delta_unit() {
        assert_eq!(
            parse_impl_double_delta("impl Printable for Old {", "impl Labeled for New {"),
            Some((
                "Printable".to_string(),
                "Labeled".to_string(),
                "Old".to_string(),
                "New".to_string()
            ))
        );
        // Suffix must be byte-equal.
        assert!(parse_impl_double_delta("impl A for B {", "impl C for D<T> {").is_none());
        // Single-delta legs decline here (owned by `impl_single`).
        assert!(parse_impl_double_delta("impl A for B {", "impl C for B {").is_none());
        assert!(parse_impl_double_delta("impl A for B {", "impl A for D {").is_none());
        assert!(parse_impl_double_delta("impl A for B {", "impl A for B {").is_none());
        // `Format` never splits a `for` word.
        assert!(parse_impl_double_delta("impl Format for B {", "impl Label for D {").is_some());
        assert!(parse_impl_double_delta("impl A for Before {", "impl C for D {").is_some());
        // Generic prefix: byte-equal composes, mismatch declines.
        assert!(parse_impl_double_delta("impl<T> A for B {", "impl<T> C for D {").is_some());
        assert!(parse_impl_double_delta("impl<T> A for B {", "impl<U> C for D {").is_none());
        assert!(
            parse_impl_double_delta("impl<T: Clone> A for B {", "impl<T: Clone> C for D {")
                .is_some()
        );
        // Qualified trait path: byte-equal prefix composes, mismatch declines.
        assert!(
            parse_impl_double_delta("impl crate::A for B {", "impl crate::C for D {").is_some()
        );
        assert!(
            parse_impl_double_delta("impl crate::A for B {", "impl other::C for D {").is_none()
        );
        assert!(parse_impl_double_delta("impl A for B {", "impl crate::C for D {").is_none());
    }

    #[test]
    fn joint_impl_extras_fire_verified() {
        // Triple plus a type-annotation use and a single-delta `impl`
        // header: both ride as follow-uses under the shared rule.
        let pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nfn f(x: Old) -> Old {\n    x\n}\nimpl Printable for Other {\n}\n",
        )]);
        let native = section(
            "f.rs",
            "@@ -1,9 +1,9 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n-fn f(x: Old) -> Old {\n+fn f(x: New) -> New {\n     x\n }\n-impl Printable for Other {\n+impl Labeled for Other {\n }\n",
        );
        let out = fire(&[native], &pre).expect("triple+extras must fire");
        assert_eq!(out.matches("rule=r1\n").count(), 2, "got: {out}");
        assert!(
            !out.contains("impl Printable"),
            "headers rewritten, never stated, got: {out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "trait Labeled {}\nstruct New;\nimpl Labeled for New {\n}\nfn f(x: New) -> New {\n    x\n}\nimpl Labeled for Other {\n}\n"
        );
    }

    #[test]
    fn joint_impl_type_alias_carrier_fires() {
        let pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Inner;\ntype Old = Inner;\nimpl Printable for Old {\n}\n",
        )]);
        let native = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n-trait Printable {}\n struct Inner;\n-type Old = Inner;\n-impl Printable for Old {\n+trait Labeled {}\n+type New = Inner;\n+impl Labeled for New {\n }\n",
        );
        let out = fire(&[native], &pre).expect("type-alias carrier must fire");
        assert!(out.contains("- type Old\n"), "got: {out}");
        assert!(out.contains("+ type New\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "trait Labeled {}\nstruct Inner;\ntype New = Inner;\nimpl Labeled for New {\n}\n"
        );
    }

    #[test]
    fn joint_impl_extra_veto_matrix() {
        // An unrelated fourth change is not a follow-use: ordinary.
        let pre = pre_map(&[("f.rs", PRE)]);
        let unrelated = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n-fn helper() {}\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n+fn ren_helper() {}\n }\n",
        );
        assert!(fire(&[unrelated], &pre).is_none());
        // A self-ty with an evolved RHS is a third definition delta,
        // never a follow-use.
        let alias_pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\ntype Old = Inner;\nimpl Printable for Old {\n}\n",
        )]);
        let evolved = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-trait Printable {}\n-type Old = Inner;\n-impl Printable for Old {\n+trait Labeled {}\n+type New = Other;\n+impl Labeled for New {\n",
        );
        assert!(fire(&[evolved], &alias_pre).is_none());
        // A call-callee extra keeps its dedicated shape: ordinary.
        let call_pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nfn g() -> u8 {\n    Old(1)\n}\n",
        )]);
        let call = section(
            "f.rs",
            "@@ -1,7 +1,7 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n fn g() -> u8 {\n-    Old(1)\n+    New(1)\n }\n",
        );
        assert!(fire(&[call], &call_pre).is_none());
        // A half mentioned on only one side is not a rename line.
        let partial_pre = pre_map(&[(
            "f.rs",
            "trait Printable {}\nstruct Old;\nimpl Printable for Old {\n}\nfn h() -> u8 {\n    let a = Old;\n    a\n}\n",
        )]);
        let partial = section(
            "f.rs",
            "@@ -1,8 +1,8 @@\n-trait Printable {}\n-struct Old;\n-impl Printable for Old {\n+trait Labeled {}\n+struct New;\n+impl Labeled for New {\n }\n fn h() -> u8 {\n-    let a = Old;\n+    let a = 0;\n     a\n }\n",
        );
        assert!(fire(&[partial], &partial_pre).is_none());
    }

    #[test]
    fn joint_impl_extra_claimable_unit() {
        assert!(extra_claimable(
            "fn f(x: Old) -> Old {",
            "fn f(x: New) -> New {",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(extra_claimable(
            "use crate::m::Old;",
            "use crate::m::New;",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(extra_claimable(
            "impl Printable for Other {",
            "impl Labeled for Other {",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        // Both halves at once is allowed (the probe owns the rewrite).
        assert!(extra_claimable(
            "fn f(x: Old) -> Printable {",
            "fn f(x: New) -> Labeled {",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(!extra_claimable(
            "fn helper() {}",
            "fn ren_helper() {}",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(!extra_claimable(
            "let a = Old;",
            "let a = 0;",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(!extra_claimable(
            "Old(1)",
            "New(1)",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(!extra_claimable(
            "Old!(x)",
            "New!(x)",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        assert!(!extra_claimable(
            "fn f(x: Old) {",
            "fn f(x: Old) {",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
        // Substring is never containment.
        assert!(!extra_claimable(
            "fn f(x: Older) {",
            "fn f(x: Newer) {",
            "Printable",
            "Labeled",
            "Old",
            "New"
        ));
    }

    const DD_PRE: &str = "struct Old {\n    x: i32,\n}\n\nimpl Old {\n    fn get(&self) -> i32 {\n        self.x\n    }\n}\n\ntrait Printable {\n    fn label(&self) -> String;\n}\n\nstruct Foo;\n\nimpl Printable for Foo {\n    fn label(&self) -> String {\n        \"foo\".to_string()\n    }\n}\n\nimpl Printable for Old {\n    fn label(&self) -> String {\n        \"old\".to_string()\n    }\n}\n";

    fn dd_pairs() -> (Vec<&'static str>, Vec<&'static str>, Vec<usize>) {
        (
            vec![
                "struct Old {",
                "impl Old {",
                "trait Printable {",
                "impl Printable for Foo {",
                "impl Printable for Old {",
            ],
            vec![
                "struct New {",
                "impl New {",
                "trait Labeled {",
                "impl Labeled for Foo {",
                "impl Labeled for New {",
            ],
            vec![1, 5, 11, 17, 23],
        )
    }

    #[test]
    fn dd_wide_section_fires_and_round_trips() {
        // WI-6(b) 14-shape: one double-delta + defs + two heads + bare
        // rest raises as tight singles plus two widened sweeps and
        // rebuilds the post tree byte-identical.
        let (m, p, c) = dd_pairs();
        let mut used = Vec::new();
        let out = super::try_impl_double_delta_section(&m, &p, "f.rs", Some(DD_PRE), &c, &mut used)
            .unwrap_fire("14 shape must fire");
        assert_eq!(out.matches("rule=").count(), 4, "got:\n{out}");
        assert!(
            out.contains("@@ -5,23 @@"),
            "ty wide covers head..dd, got:\n{out}"
        );
        assert!(
            out.contains("@@ -17,11 @@"),
            "tr wide covers head..dd, got:\n{out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map(&[("f.rs", DD_PRE)]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        let post = "struct New {\n    x: i32,\n}\n\nimpl New {\n    fn get(&self) -> i32 {\n        self.x\n    }\n}\n\ntrait Labeled {\n    fn label(&self) -> String;\n}\n\nstruct Foo;\n\nimpl Labeled for Foo {\n    fn label(&self) -> String {\n        \"foo\".to_string()\n    }\n}\n\nimpl Labeled for New {\n    fn label(&self) -> String {\n        \"old\".to_string()\n    }\n}\n";
        assert_eq!(files["f.rs"], post, "got:\n{}", files["f.rs"]);
    }

    #[test]
    fn dd_wide_section_veto_matrix() {
        // Missing trait def, missing head, non-item rest, and a second
        // double-delta all decline to today's behavior.
        let (m, p, c) = dd_pairs();
        let mut used = Vec::new();
        // No trait def (trait pair renamed to something else).
        let mut p2 = p.clone();
        p2[2] = "trait Other {";
        assert!(
            super::try_impl_double_delta_section(&m, &p2, "f.rs", Some(DD_PRE), &c, &mut used)
                .is_miss()
        );
        // No ty head (head pair left unchanged is not a pair at all —
        // drop it, leaving 4 pairs).
        let (mut m3, mut p3, mut c3) = (m.clone(), p.clone(), c.clone());
        m3.remove(1);
        p3.remove(1);
        c3.remove(1);
        assert!(super::try_impl_double_delta_section(
            &m3,
            &p3,
            "f.rs",
            Some(DD_PRE),
            &c3,
            &mut used
        )
        .is_miss());
        // Non-item rest (call pair) declines.
        let (mut m4, mut p4, mut c4) = (m.clone(), p.clone(), c.clone());
        m4.push("    Old(1)");
        p4.push("    New(1)");
        c4.push(24);
        assert!(super::try_impl_double_delta_section(
            &m4,
            &p4,
            "f.rs",
            Some(DD_PRE),
            &c4,
            &mut used
        )
        .is_miss());
        // Pre-image without the trait item (unparseable cover) declines.
        assert!(super::try_impl_double_delta_section(
            &m,
            &p,
            "f.rs",
            Some("garbage {{{"),
            &c,
            &mut used
        )
        .is_miss());
    }
}
