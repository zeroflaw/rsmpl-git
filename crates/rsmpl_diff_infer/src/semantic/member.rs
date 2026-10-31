//! Field/variant member rename raising (WI-9b + WI-15b, SPEC D.4).
//!
//! Single-file def+uses intent (`try_member_rename`): exactly one
//! definition pair (a field `old: T` → `new: T` line or a bare `Old` →
//! `New` variant line) plus N-1 use-site lines sharing the ident delta
//! compress to the authored definition hunk *with* `# follow=uses`.
//! Cross-file intent (`try_member_intent_sweep`, WI-15b): the same one-def
//! shape joined across files, file/section-pure partial (proven files
//! collapse, unproven files stay ordinary).
//!
//! Totality is a scratch-apply line-equality probe (template-probe
//! posture): the authored emission is applied to an in-memory clone of
//! the pre-image and the post tree must show exactly the native plus
//! lines at the changed sites with every other line byte-identical. An
//! untouched twin the follow would rewrite fails the unchanged-leg; a
//! missed site fails the plus-leg; an opaque site (macro, qself) fails
//! at apply time without fallback (WI-14: sibling definitions and
//! unanchored uses never veto — the probe reproduces exactly the
//! anchored follow). `None` pre-image declines (the container is unprovable
//! without it).

use std::collections::{HashMap, HashSet};

use super::{item_emission_header, unique_rule};

/// Whole-section field/variant member rename: exactly one definition
/// pair plus N-1 use-site lines sharing its ident delta. Returns the
/// single-hunk follow emission, or `None` when the section is not
/// exactly this shape.
pub(crate) fn try_member_rename(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if changed.len() != minuses.len() {
        return None;
    }
    let pre = file_old?;
    // Exactly one definition pair: field first (colon shape), else
    // variant (bare ident). A second def pair stays ordinary (the
    // engine would veto the ambiguity — raising never splits it).
    let mut def_idx: Option<usize> = None;
    let mut def_kind: Option<MemberDef> = None;
    for (i, (m, p)) in minuses.iter().zip(pluses.iter()).enumerate() {
        if let Some(def) = parse_field_pair(m, p) {
            if def_idx.is_some() {
                return None;
            }
            def_idx = Some(i);
            def_kind = Some(MemberDef::Field(def));
        } else if let Some((old, new)) = parse_variant_pair(m, p) {
            if def_idx.is_some() {
                return None;
            }
            def_idx = Some(i);
            def_kind = Some(MemberDef::Variant { old, new });
        }
    }
    let def_idx = def_idx?;
    let def = def_kind?;
    // Use-site gate per non-def pair: the minus names `old`, the plus
    // names `new` (ident-boundary, never substring), and the pair is
    // not itself a second definition. The scratch probe below is
    // authoritative; this only rejects cheap non-shares.
    let (old, new) = def.idents();
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    for (i, (m, p)) in minuses.iter().zip(pluses.iter()).enumerate() {
        if i == def_idx {
            continue;
        }
        if m.trim() == p.trim() {
            return None;
        }
        if !contains_ident(m, &old) || !contains_ident(p, &new) {
            return None;
        }
    }
    // Container from the pre-image at the def site, with the shadow
    // census: a second same-named member anywhere vetoes (the authored
    // follow would veto loud — raising stays ordinary instead).
    let container = match &def {
        MemberDef::Field(f) => struct_of_field(pre, &f.old, changed[def_idx])?,
        MemberDef::Variant { old, .. } => enum_of_variant(pre, old, changed[def_idx])?,
    };
    // Pre-image minus alignment (stripped trailing `,` tolerated on
    // the def pair): the probe below re-verifies the post side.
    let pre_lines: Vec<&str> = pre.lines().collect();
    for (i, &site) in changed.iter().enumerate() {
        let pre_line = pre_lines.get(site.saturating_sub(1))?;
        let want = minuses[i].trim();
        let got = pre_line.trim();
        if got != want {
            // The def pair carries no trailing comma in the pattern
            // but does in the file; tolerate exactly that.
            if i != def_idx
                || got != format!("{want},").as_str()
                    && got.trim_end_matches(',').trim() != want.trim_end_matches(',').trim()
            {
                return None;
            }
        }
    }
    let def_site = changed[def_idx];
    // Scratch-apply probe with line equality (template-probe totality):
    // the authored emission must reproduce exactly the native plus
    // lines at the changed sites and touch nothing else.
    let probe = emit(path, "probe", Some(pre), def_site, &container, &def);
    if !scratch_reproduces(pre, path, &probe, &pre_lines, changed, pluses) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    Some(emit(path, &rule, Some(pre), def_site, &container, &def))
}

/// WI-15b whole-patch member-intent pre-pass: one member definition pair
/// (field or variant, same `parse_*` as the single-file path) joined with
/// same-delta use lines across files into the single definition hunk with
/// `# follow=uses`. File/section-pure partial: proven sections collapse to
/// the emission at the def section's slot, unproven sections pass through
/// byte-identical. Returns the rebuilt section list plus the def path and
/// rule, or `None` when the patch holds no provable cross-file intent.
/// Reserved rule names are released on decline.
pub(crate) fn try_member_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    struct SecPairs {
        idx: usize,
        path: String,
        minuses: Vec<String>,
        pluses: Vec<String>,
        changed: Vec<usize>,
    }
    struct Pair {
        sec: usize,
        path: String,
        minus: String,
        plus: String,
        site: usize,
    }
    let mut parsed: Vec<SecPairs> = Vec::new();
    let mut eligible: Vec<usize> = Vec::new();
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
        eligible.push(i);
        let (minuses, pluses, balanced) = section_pairs(sec);
        if !balanced || minuses.is_empty() {
            return None;
        }
        let changed = crate::totality::changed_old_lines(sec);
        if changed.len() != minuses.len() {
            return None;
        }
        parsed.push(SecPairs {
            idx: i,
            minuses,
            pluses,
            changed,
            path,
        });
    }
    if parsed.is_empty() {
        return None;
    }
    let mut pairs: Vec<Pair> = Vec::new();
    for sp in &parsed {
        for ((m, p), s) in sp
            .minuses
            .iter()
            .zip(sp.pluses.iter())
            .zip(sp.changed.iter())
        {
            pairs.push(Pair {
                sec: sp.idx,
                path: sp.path.clone(),
                minus: m.clone(),
                plus: p.clone(),
                site: *s,
            });
        }
    }
    if pairs.len() < 2 {
        return None;
    }
    // Exactly one definition pair.
    let mut def_idx: Option<usize> = None;
    let mut def_kind: Option<MemberDef> = None;
    for (i, pair) in pairs.iter().enumerate() {
        let found = if let Some(f) = parse_field_pair(&pair.minus, &pair.plus) {
            Some(MemberDef::Field(f))
        } else if let Some((old, new)) = parse_variant_pair(&pair.minus, &pair.plus) {
            Some(MemberDef::Variant { old, new })
        } else {
            None
        };
        if let Some(d) = found {
            if def_idx.is_some() {
                return None;
            }
            def_idx = Some(i);
            def_kind = Some(d);
        }
    }
    let def_idx = def_idx?;
    let def = def_kind?;
    let (old, new) = def.idents();
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    for (i, pair) in pairs.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if pair.minus.trim() == pair.plus.trim() {
            return None;
        }
        if !contains_ident(&pair.minus, &old) || !contains_ident(&pair.plus, &new) {
            return None;
        }
    }
    for pair in &pairs {
        if !pre.contains_key(&pair.path) {
            return None;
        }
    }
    let def_pair = &pairs[def_idx];
    let def_pre = pre.get(&def_pair.path).map(String::as_str)?;
    let container = match &def {
        MemberDef::Field(f) => struct_of_field(def_pre, &f.old, def_pair.site)?,
        MemberDef::Variant { old: o, .. } => enum_of_variant(def_pre, o, def_pair.site)?,
    };
    // Single-file two-pair shapes stay with `try_member_rename` + T375
    // (that path owns the one-section win; the intent needs ≥2 sections
    // or ≥3 pairs to beat ordinary — enforced by the T375 below, but a
    // lone section with exactly the single-file shape must not
    // double-claim it).
    if parsed.len() == 1 {
        let sp = &parsed[0];
        if sp.minuses.len() == pairs.len() {
            // Same shape the per-section detector owns: let it fire (or
            // decline) alone so canonical bytes stay stable.
            return None;
        }
    }
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    let emission = emit(
        &def_pair.path,
        &rule,
        Some(def_pre),
        def_pair.site,
        &container,
        &def,
    );
    // Cross-file scratch probe: apply emission to a `pre` clone, then
    // classify each pair as proven (post trim == plus trim).
    let parsed_patch = rsmpl_patch::parse_patch(&emission).ok()?;
    let mut files = pre.clone();
    for section in &parsed_patch.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed_patch).is_err() {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut proven = vec![false; pairs.len()];
    for (i, pair) in pairs.iter().enumerate() {
        let Some(post) = files.get(&pair.path) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        let post_lines: Vec<&str> = post.lines().collect();
        let Some(got) = post_lines.get(pair.site.saturating_sub(1)) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        proven[i] = got.trim() == pair.plus.trim();
    }
    if !proven[def_idx] {
        used_rules.truncate(checkpoint);
        return None;
    }
    if proven.iter().filter(|&&p| p).count() < 2 {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Section purity: every section holding a proven pair must be fully
    // proven (else the emission would rewrite a line the ordinary
    // remainder also rewrites). Mixed sections veto the whole intent.
    let mut sec_proven_all: HashMap<usize, bool> = HashMap::new();
    let mut sec_has_proven: HashMap<usize, bool> = HashMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        let e = sec_proven_all.entry(pair.sec).or_insert(true);
        if !proven[i] {
            *e = false;
        }
        if proven[i] {
            sec_has_proven.insert(pair.sec, true);
        }
    }
    for (sec, has) in &sec_has_proven {
        if *has && !sec_proven_all.get(sec).copied().unwrap_or(false) {
            used_rules.truncate(checkpoint);
            return None;
        }
    }
    let claimed_secs: HashSet<usize> = sec_has_proven
        .iter()
        .filter(|(_, h)| **h)
        .map(|(s, _)| *s)
        .collect();
    // Unclaimed files must be byte-identical under the emission (the
    // follow touches nothing there); claimed files must be clean outside
    // claimed sites.
    let claimed_files: HashSet<&str> = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| proven[*i])
        .map(|(_, p)| p.path.as_str())
        .collect();
    for (path, pre_text) in pre.iter() {
        let Some(post_text) = files.get(path) else {
            used_rules.truncate(checkpoint);
            return None;
        };
        if !claimed_files.contains(path.as_str()) {
            if post_text != pre_text {
                used_rules.truncate(checkpoint);
                return None;
            }
            continue;
        }
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            used_rules.truncate(checkpoint);
            return None;
        }
        let mut claimed_sites: HashSet<usize> = HashSet::new();
        for (i, pair) in pairs.iter().enumerate() {
            if proven[i] && pair.path == *path {
                claimed_sites.insert(pair.site);
            }
        }
        for (idx, (a, b)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            if claimed_sites.contains(&(idx + 1)) {
                continue;
            }
            if a != b {
                used_rules.truncate(checkpoint);
                return None;
            }
        }
    }
    let claimed_bytes: usize = parsed
        .iter()
        .filter(|sp| claimed_secs.contains(&sp.idx))
        .map(|sp| sections[sp.idx].len())
        .sum();
    if emission.len() >= claimed_bytes {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if !eligible.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        if !claimed_secs.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        if i == def_pair.sec {
            out.push(emission.clone());
        }
    }
    Some((out, def_pair.path.clone(), rule))
}

/// One section's minus/plus lines plus the balanced gate (same walk as
/// `chain.rs` / `item_intent.rs`: every hunk balanced, global counts
/// equal, non-empty).
fn section_pairs(section: &str) -> (Vec<String>, Vec<String>, bool) {
    let mut minuses: Vec<String> = Vec::new();
    let mut pluses: Vec<String> = Vec::new();
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
            if line.starts_with("old mode ") || line.starts_with("new mode ") {
                balanced = false;
            }
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(line[1..].to_string());
            hunk_minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(line[1..].to_string());
            hunk_plus += 1;
        }
    }
    if in_hunk && hunk_minus != hunk_plus {
        balanced = false;
    }
    let ok = balanced && saw_hunk && minuses.len() == pluses.len() && !minuses.is_empty();
    (minuses, pluses, ok)
}

/// A classified definition pair.
enum MemberDef {
    Field(FieldDef),
    Variant { old: String, new: String },
}

struct FieldDef {
    old: String,
    new: String,
    /// Minus-side type spelling (the engine compares `quote!`-rendered,
    /// so any one spelling of the equal type emits fine).
    ty: String,
}

impl MemberDef {
    fn idents(&self) -> (String, String) {
        match self {
            MemberDef::Field(f) => (f.old.clone(), f.new.clone()),
            MemberDef::Variant { old, new } => (old.clone(), new.clone()),
        }
    }
}

/// A `-`/`+` line pair as a field definition rename: `old: T` →
/// `new: T` with one single colon, an optional equal `pub…` prefix,
/// one ident each side, and `quote!`-equal types. Trailing `,`/`;`
/// tolerated (native lines carry the comma the pattern omits).
fn parse_field_pair(minus: &str, plus: &str) -> Option<FieldDef> {
    let (m_name, m_ty) = split_field_line(minus)?;
    let (p_name, p_ty) = split_field_line(plus)?;
    if m_name.0 != p_name.0 {
        // Vis prefixes must be equal (item-rename parity).
        return None;
    }
    let old = m_name.1.clone();
    let new = p_name.1.clone();
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    if super::ident_len(&old)? != old.len() || super::ident_len(&new)? != new.len() {
        return None;
    }
    let m_ty_parsed: syn::Type = syn::parse_str(&m_ty).ok()?;
    let p_ty_parsed: syn::Type = syn::parse_str(&p_ty).ok()?;
    if quote::quote!(#m_ty_parsed)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        != quote::quote!(#p_ty_parsed)
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    {
        return None;
    }
    Some(FieldDef {
        old,
        new,
        ty: m_ty.trim().to_string(),
    })
}

/// Split a field line into ((prefix, ident), type): one single colon,
/// no braces/parens/semicolons on the name side (those are struct
/// literals, calls, or item headers the member shape never owns).
fn split_field_line(line: &str) -> Option<((String, String), String)> {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
    // Exactly one single colon (a `::` path neighbor never counts).
    let chars: Vec<char> = t.chars().collect();
    let mut colons = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        if c == ':' {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j).copied());
            let next = chars.get(i + 1).copied();
            if prev != Some(':') && next != Some(':') {
                colons.push(i);
            }
        }
    }
    if colons.len() != 1 {
        return None;
    }
    let byte: usize = chars[..colons[0]].iter().collect::<String>().len();
    let (left, right) = t.split_at(byte);
    let right = right.strip_prefix(':')?.trim();
    if right.is_empty() {
        return None;
    }
    let left = left.trim();
    if left.is_empty() || left.contains(['{', '}', '(', ')', ';', '=', '[', ']']) {
        return None;
    }
    // Optional `pub…` prefix (equal on both sides, checked by caller
    // via the returned prefix): the last whitespace-separated token is
    // the ident.
    let mut parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let ident = parts.pop()?.to_string();
    let prefix = if parts.is_empty() {
        String::new()
    } else {
        parts.join(" ") + " "
    };
    if !prefix.is_empty() && !prefix.starts_with("pub") {
        return None;
    }
    Some(((prefix, ident), right.to_string()))
}

/// A `-`/`+` line pair as a variant definition rename: bare `Old` →
/// `New` with an optional equal suffix (discriminant, tuple fields,
/// struct fields). Trailing `,` tolerated.
fn parse_variant_pair(minus: &str, plus: &str) -> Option<(String, String)> {
    let (m_ident, m_suffix) = split_variant_line(minus)?;
    let (p_ident, p_suffix) = split_variant_line(plus)?;
    if m_suffix != p_suffix {
        return None;
    }
    if m_ident == p_ident || m_ident.is_empty() || p_ident.is_empty() {
        return None;
    }
    if super::ident_len(&m_ident)? != m_ident.len() || super::ident_len(&p_ident)? != p_ident.len()
    {
        return None;
    }
    Some((m_ident, p_ident))
}

fn split_variant_line(line: &str) -> Option<(String, String)> {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
    if t.is_empty() || t.contains(':') {
        // `:` belongs to field pairs (struct-variant payloads carry
        // their colon inside the suffix braces — but a top-level colon
        // means this is a field line, not a variant line). Suffix
        // braces make the colon non-top-level; check naively: a colon
        // outside any bracket pair vetoes.
        let mut depth = 0i32;
        for c in t.chars() {
            match c {
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' => depth -= 1,
                ':' if depth == 0 => return None,
                _ => {}
            }
        }
    }
    let len = super::ident_len(t)?;
    let (ident, suffix) = t.split_at(len);
    Some((ident.to_string(), suffix.trim().to_string()))
}

/// Ident-boundary containment (never substring: `old` must not match
/// `older`).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    let mut hit = false;
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                hit = true;
                break;
            }
            token.clear();
        }
    }
    hit
}

/// The `struct S` whose field `old` spans 1-based `site`, or `None`
/// when the site names no field. WI-14: sibling same-named fields no
/// longer veto (the anchored follow classifies per-site); the def site
/// must still resolve to exactly one container.
fn struct_of_field(pre: &str, old: &str, site: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut at_site: Vec<String> = Vec::new();
    let mut carriers = 0usize;
    fn walk(
        items: &[syn::Item],
        old: &str,
        site: usize,
        at_site: &mut Vec<String>,
        carriers: &mut usize,
    ) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Struct(s) => {
                    if let syn::Fields::Named(fields) = &s.fields {
                        let mut has = false;
                        for f in &fields.named {
                            if f.ident.as_ref().is_some_and(|i| i == old) {
                                has = true;
                                if f.span().start().line == site {
                                    at_site.push(s.ident.to_string());
                                }
                            }
                        }
                        if has {
                            *carriers += 1;
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, old, site, at_site, carriers);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, old, site, &mut at_site, &mut carriers);
    if carriers < 1 || at_site.len() != 1 {
        return None;
    }
    at_site.into_iter().next()
}

/// The `enum E` whose variant `old` spans 1-based `site`, or `None`
/// under the same WI-14 rule as [`struct_of_field`].
fn enum_of_variant(pre: &str, old: &str, site: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut at_site: Vec<String> = Vec::new();
    let mut carriers = 0usize;
    fn walk(
        items: &[syn::Item],
        old: &str,
        site: usize,
        at_site: &mut Vec<String>,
        carriers: &mut usize,
    ) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Enum(e) => {
                    let mut has = false;
                    for v in &e.variants {
                        if v.ident == old {
                            has = true;
                            if v.span().start().line == site {
                                at_site.push(e.ident.to_string());
                            }
                        }
                    }
                    if has {
                        *carriers += 1;
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, old, site, at_site, carriers);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, old, site, &mut at_site, &mut carriers);
    if carriers < 1 || at_site.len() != 1 {
        return None;
    }
    at_site.into_iter().next()
}

/// Emit the single-hunk follow emission under `rule`: the covering
/// item range plus the container context header and the member pair.
fn emit(
    path: &str,
    rule: &str,
    file_old: Option<&str>,
    def_site: usize,
    container: &str,
    def: &MemberDef,
) -> String {
    let mut out = item_emission_header(path, rule, file_old, &[def_site]);
    out.push_str("# follow=uses\n");
    match def {
        MemberDef::Field(f) => {
            out.push_str(&format!("  struct {container} {{\n"));
            let prefix = field_prefix_hint(file_old, container, &f.old);
            out.push_str(&format!("- {prefix}{}: {}\n", f.old, f.ty));
            out.push_str(&format!("+ {prefix}{}: {}\n", f.new, f.ty));
        }
        MemberDef::Variant { old, new } => {
            out.push_str(&format!("  enum {container} {{\n"));
            out.push_str(&format!("- {old}\n+ {new}\n"));
        }
    }
    out
}

/// The `pub…` prefix on the definition field, if any (read back from
/// the pre-image so the emission spells the real visibility).
fn field_prefix_hint(file_old: Option<&str>, container: &str, old: &str) -> String {
    let Some(pre) = file_old else {
        return String::new();
    };
    let Ok(file) = syn::parse_str::<syn::File>(pre) else {
        return String::new();
    };
    for it in &file.items {
        if let syn::Item::Struct(s) = it {
            if s.ident == container {
                if let syn::Fields::Named(fields) = &s.fields {
                    for f in &fields.named {
                        if f.ident.as_ref().is_some_and(|i| i == old) {
                            let vis = &f.vis;
                            let rendered = quote::quote!(#vis).to_string();
                            // `quote!` renders `pub` with a trailing
                            // space already; normalize to one.
                            let rendered = rendered.trim().to_string();
                            if rendered.is_empty() {
                                return String::new();
                            }
                            return format!("{rendered} ");
                        }
                    }
                }
            }
        }
    }
    String::new()
}

/// WI-19b def-only single (fixtures 59/77): exactly one field/variant
/// definition pair, no use sites, no `# follow=uses`. The follow would
/// rewrite uses the diff did not touch; the def-only hunk renames the
/// definition and leaves every use ordinary (orphans). Same container
/// recovery + scratch line-equality probe as the follow path.
pub(crate) fn try_member_def_only(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let def = if let Some(f) = parse_field_pair(minuses[0], pluses[0]) {
        MemberDef::Field(f)
    } else if let Some((old, new)) = parse_variant_pair(minuses[0], pluses[0]) {
        MemberDef::Variant { old, new }
    } else {
        return None;
    };
    let (old, new) = def.idents();
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    let container = match &def {
        MemberDef::Field(f) => struct_of_field(pre, &f.old, changed[0])?,
        MemberDef::Variant { old, .. } => enum_of_variant(pre, old, changed[0])?,
    };
    let pre_lines: Vec<&str> = pre.lines().collect();
    let pre_line = pre_lines.get(changed[0].saturating_sub(1))?;
    let want = minuses[0].trim();
    if pre_line.trim() != want
        && pre_line.trim() != format!("{want},").as_str()
        && pre_line.trim_end_matches(',').trim() != want.trim_end_matches(',').trim()
    {
        return None;
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, &format!("rename_{old}_to_{new}"));
    // WI-19b: def-only singles emit the concrete definition hunk with no
    // directives (default `allow=one` matches the one proven site;
    // `# allow=many` is vacuous at N=1 and would miss the authored diet
    // canonically — 59/77 precedent).
    let mut out =
        item_emission_header(path, &rule, Some(pre), changed).replace("# allow=many\n", "");
    match &def {
        MemberDef::Field(f) => {
            out.push_str(&format!("  struct {container} {{\n"));
            let prefix = field_prefix_hint(Some(pre), &container, &f.old);
            out.push_str(&format!("- {prefix}{}: {}\n", f.old, f.ty));
            out.push_str(&format!("+ {prefix}{}: {}\n", f.new, f.ty));
        }
        MemberDef::Variant { old, new } => {
            out.push_str(&format!("  enum {container} {{\n"));
            out.push_str(&format!("- {old}\n+ {new}\n"));
        }
    }
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some((out, "member_rename"))
}

/// Apply `section` to `pre` in memory and require exact line equality:
/// every `changed[i]` post line trims to `pluses[i]`, every other line
/// is byte-identical to `pre`. `pre_lines` is `pre.lines()`.
fn scratch_reproduces(
    pre: &str,
    path: &str,
    section: &str,
    pre_lines: &[&str],
    changed: &[usize],
    pluses: &[&str],
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(_) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    let post = &files[path];
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() {
        return false;
    }
    let changed_set: std::collections::HashSet<usize> = changed.iter().copied().collect();
    for (i, &site) in changed.iter().enumerate() {
        let Some(post_line) = post_lines.get(site.saturating_sub(1)) else {
            return false;
        };
        if post_line.trim() != pluses[i].trim() {
            return false;
        }
        let _ = changed_set;
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        let line_no = idx + 1;
        if changed_set.contains(&line_no) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
    // The engine must report the definition match (a follow-only
    // rewrite with no definition is never this detector's emission).
    true
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

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

    const FIELD_PRE: &str = "struct Config {\n    timeout_ms: u32,\n    retries: u8,\n}\nfn f(c: Config) -> u32 {\n    c.timeout_ms + c.retries as u32\n}\nfn g() -> Config {\n    Config { timeout_ms: 5, retries: 1 }\n}\nfn h(c: Config) -> u32 {\n    let Config { timeout_ms: t, retries: _r } = c;\n    t\n}\n";

    fn field_section() -> String {
        section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    timeout_ms: u32,\n",
                "+    deadline_ms: u32,\n",
                "@@ -6,1 +6,1 @@\n",
                "-    c.timeout_ms + c.retries as u32\n",
                "+    c.deadline_ms + c.retries as u32\n",
                "@@ -9,1 +9,1 @@\n",
                "-    Config { timeout_ms: 5, retries: 1 }\n",
                "+    Config { deadline_ms: 5, retries: 1 }\n",
                "@@ -12,1 +12,1 @@\n",
                "-    let Config { timeout_ms: t, retries: _r } = c;\n",
                "+    let Config { deadline_ms: t, retries: _r } = c;\n",
            ),
        )
    }

    #[test]
    fn field_pair_extracts_delta() {
        let def =
            parse_field_pair("    timeout_ms: u32,", "    deadline_ms: u32,").expect("field pair");
        assert_eq!(def.old, "timeout_ms");
        assert_eq!(def.new, "deadline_ms");
        assert!(parse_field_pair("    timeout_ms: u32,", "    timeout_ms: u32,").is_none());
        assert!(parse_field_pair("    timeout_ms: u32,", "    timeout_ms: bool,").is_none());
        assert!(parse_field_pair("use crate::old;", "use crate::new;").is_none());
        assert!(parse_field_pair("Old", "New").is_none());
    }

    #[test]
    fn variant_pair_extracts_delta_with_suffix() {
        assert_eq!(
            parse_variant_pair("    Old,", "    New,"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_variant_pair("    Old = 3,", "    New = 3,"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            parse_variant_pair("    Old(u32),", "    New(u32),"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert!(parse_variant_pair("    Old(u32),", "    New(bool),").is_none());
    }

    #[test]
    fn field_fires_and_round_trips() {
        let raw = field_section();
        let pre = pre_map(&[("f.rs", FIELD_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("field member rename must compress");
        assert!(out.contains("struct Config {"), "got:\n{out}");
        assert!(out.contains("- timeout_ms: u32"), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        let post = &files["f.rs"];
        assert!(post.contains("deadline_ms: u32,"), "got: {post}");
        assert!(post.contains("c.deadline_ms"), "got: {post}");
        assert!(post.contains("Config { deadline_ms: 5"), "got: {post}");
        assert!(!post.contains("timeout_ms"), "got: {post}");
    }

    const VARIANT_PRE: &str = "enum Status {\n    Active(u32),\n    Idle,\n}\nuse crate::Status::Active;\nfn f(s: Status) -> u32 {\n    let a = Status::Active(1);\n    match s {\n        Status::Active(v) => v,\n        Status::Idle => 0,\n    }\n}\n";

    #[test]
    fn variant_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    Active(u32),\n",
                "+    Ready(u32),\n",
                "@@ -5,1 +5,1 @@\n",
                "-use crate::Status::Active;\n",
                "+use crate::Status::Ready;\n",
                "@@ -7,1 +7,1 @@\n",
                "-    let a = Status::Active(1);\n",
                "+    let a = Status::Ready(1);\n",
                "@@ -9,1 +9,1 @@\n",
                "-        Status::Active(v) => v,\n",
                "+        Status::Ready(v) => v,\n",
            ),
        );
        let pre = pre_map(&[("f.rs", VARIANT_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("variant member rename must compress");
        assert!(out.contains("enum Status {"), "got:\n{out}");
        assert!(out.contains("- Active\n"), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        let post = &files["f.rs"];
        assert!(post.contains("Ready(u32),"), "got: {post}");
        assert!(post.contains("use crate::Status::Ready;"), "got: {post}");
        assert!(post.contains("Status::Ready(1)"), "got: {post}");
        assert!(post.contains("Status::Ready(v) => v,"), "got: {post}");
        assert!(!post.contains("Active"), "got: {post}");
    }

    #[test]
    fn def_only_single_fires_and_round_trips() {
        // WI-19b: a lone field definition (uses untouched) compresses
        // to the definition hunk with no follow.
        let pre =
            "struct S {\n    old: u32,\n    keep: bool,\n}\nfn f(s: S) -> u32 {\n    s.old\n}\n";
        let raw = section("f.rs", "@@ -2,1 +2,1 @@\n-    old: u32,\n+    new: u32,\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("def-only single must fire");
        assert!(out.contains("struct S {"), "got:\n{out}");
        assert!(out.contains("- old: u32"), "got:\n{out}");
        assert!(out.contains("+ new: u32"), "got:\n{out}");
        assert!(
            !out.contains("follow=uses"),
            "def-only carries no follow, got:\n{out}"
        );
        validated_live(&out, &pre_map);
    }

    #[test]
    fn def_only_variant_single_fires() {
        let pre = "enum E {\n    Old,\n    Keep,\n}\n";
        let raw = section("f.rs", "@@ -2,1 +2,1 @@\n-    Old,\n+    New,\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("variant def-only must fire");
        assert!(out.contains("enum E {"), "got:\n{out}");
        assert!(!out.contains("follow=uses"), "got:\n{out}");
        validated_live(&out, &pre_map);
    }

    #[test]
    fn def_only_use_line_declines() {
        // A lone use-site line is not a definition — never a def-only.
        let pre = "struct S {\n    old: u32,\n}\nfn f(s: S) -> u32 {\n    s.old\n}\n";
        let mut used = Vec::new();
        assert!(super::try_member_def_only(
            &["    s.old"],
            &["    s.new"],
            "f.rs",
            Some(pre),
            &[4],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn raw_ident_pairs_decline() {
        // WI-9a residual: raw idents (`r#type`) are not plain idents
        // under `ident_len`, so they never classify as a definition
        // pair — they stay ordinary instead of misfiring.
        assert!(parse_field_pair("    r#type: u32,", "    r#new: u32,").is_none());
        assert!(parse_variant_pair("    r#Old,", "    r#New,").is_none());
    }

    #[test]
    fn shadowed_field_stays_ordinary() {
        let pre = "struct A {\n    old: u32,\n}\nstruct B {\n    old: u32,\n}\nfn f(a: A) -> u32 {\n    a.old\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    old: u32,\n",
                "+    new: u32,\n",
                "@@ -6,1 +6,1 @@\n",
                "-    a.old\n",
                "+    a.new\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none() || {
                let out =
                    infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).unwrap();
                !out.contains("follow=uses")
            },
            "shadowed fields must not raise with follow"
        );
    }

    #[test]
    fn bare_old_use_vetoes_variant_follow() {
        // A bare `Old` use is opaque: the engine vetoes the follow, so
        // the probe declines and the section stays ordinary (or at
        // least never emits a follow hunk).
        let pre = "enum E {\n    Old,\n    Keep,\n}\nuse E::Old;\nfn f() -> E {\n    Old\n}\n";
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    Old,\n",
                "+    New,\n",
                "@@ -6,1 +6,1 @@\n",
                "-    Old\n",
                "+    New\n",
            ),
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map);
        assert!(
            out.is_none() || !out.unwrap().contains("follow=uses"),
            "bare uses must not raise with follow"
        );
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    const XDEF_PRE: &str = "struct Config {\n    timeout_ms: u32,\n    retries: u8,\n}\n";
    const XUSE_PRE: &str = "fn f(c: Config) -> u32 {\n    c.timeout_ms + c.retries as u32\n}\nfn g() -> Config {\n    Config { timeout_ms: 5, retries: 1 }\n}\n";

    fn xdef_sections() -> Vec<String> {
        vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    timeout_ms: u32,\n+    deadline_ms: u32,\n",
            ),
            section(
                "b.rs",
                concat!(
                    "@@ -2,1 +2,1 @@\n",
                    "-    c.timeout_ms + c.retries as u32\n",
                    "+    c.deadline_ms + c.retries as u32\n",
                    "@@ -5,1 +5,1 @@\n",
                    "-    Config { timeout_ms: 5, retries: 1 }\n",
                    "+    Config { deadline_ms: 5, retries: 1 }\n",
                ),
            ),
        ]
    }

    #[test]
    fn cross_file_field_fires_and_round_trips() {
        let sections = xdef_sections();
        let pre = pre_map(&[("a.rs", XDEF_PRE), ("b.rs", XUSE_PRE)]);
        let mut used = Vec::new();
        let (out, path, rule) =
            try_member_intent_sweep(&sections, &pre, &mut used).expect("cross-file must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "claimed sections collapse, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("- timeout_ms: u32"), "got:\n{}", out[0]);
        assert!(!rule.is_empty());
        validated_live(&out[0], &pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["a.rs"].contains("deadline_ms: u32,"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["b.rs"].contains("c.deadline_ms"),
            "got: {}",
            files["b.rs"]
        );
        assert!(
            files["b.rs"].contains("Config { deadline_ms: 5"),
            "got: {}",
            files["b.rs"]
        );
        assert!(
            !files["a.rs"].contains("timeout_ms"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            !files["b.rs"].contains("timeout_ms"),
            "got: {}",
            files["b.rs"]
        );
    }

    #[test]
    fn cross_file_pipeline_compresses() {
        let sections = xdef_sections();
        let pre = pre_map(&[("a.rs", XDEF_PRE), ("b.rs", XUSE_PRE)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- timeout_ms: u32"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    #[test]
    fn cross_file_unproven_stays_ordinary_for_that_file() {
        // `b.rs` use is anchored (`c: Config`); `c.rs` use has an
        // unprovable receiver, so the follow skips it: the intent claims
        // the proven files, the unproven file stays ordinary, and the
        // joint patch round-trips byte-identical.
        let b_pre = "fn f(c: Config) -> u32 {\n    c.timeout_ms\n}\n";
        let c_pre = "fn h(x: Other) -> u32 {\n    x.timeout_ms\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    timeout_ms: u32,\n+    deadline_ms: u32,\n",
            ),
            section(
                "b.rs",
                "@@ -2,1 +2,1 @@\n-    c.timeout_ms\n+    c.deadline_ms\n",
            ),
            section(
                "c.rs",
                "@@ -2,1 +2,1 @@\n-    x.timeout_ms\n+    x.deadline_ms\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", XDEF_PRE), ("b.rs", b_pre), ("c.rs", c_pre)]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_member_intent_sweep(&sections, &pre, &mut used).expect("partial must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 2, "proven collapse + one ordinary, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[1].contains("x.deadline_ms"), "got:\n{}", out[1]);
        // Joint apply reproduces the full native post.
        let joint = out.join("");
        validated_live(&joint, &pre);
        let parsed = rsmpl_patch::parse_patch(&joint).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["b.rs"].contains("c.deadline_ms"),
            "got: {}",
            files["b.rs"]
        );
        assert!(
            files["c.rs"].contains("x.deadline_ms"),
            "got: {}",
            files["c.rs"]
        );
    }

    #[test]
    fn cross_file_single_use_section_stays_single_file() {
        // One def + one use in the same single section is the
        // single-file detector's shape — the cross-file pre-pass must not
        // steal it (canonical bytes stay stable).
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,1 +2,1 @@\n",
                "-    old: u32,\n",
                "+    new: u32,\n",
                "@@ -6,1 +6,1 @@\n",
                "-    a.old\n",
                "+    a.new\n",
            ),
        );
        let pre = pre_map(&[(
            "f.rs",
            "struct S {\n    old: u32,\n}\nfn f(a: S) -> u32 {\n    a.old\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_member_intent_sweep(std::slice::from_ref(&raw), &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn cross_file_variant_fires_and_round_trips() {
        // Variant path shares the pre-pass: enum def in one file, `E::Old`
        // tails in another.
        let a_pre = "enum Status {\n    Active(u32),\n    Idle,\n}\n";
        let b_pre = "fn f(s: Status) -> u32 {\n    let a = Status::Active(1);\n    0\n}\n";
        let sections = vec![
            section(
                "a.rs",
                "@@ -2,1 +2,1 @@\n-    Active(u32),\n+    Ready(u32),\n",
            ),
            section(
                "b.rs",
                "@@ -2,1 +2,1 @@\n-    let a = Status::Active(1);\n+    let a = Status::Ready(1);\n",
            ),
        ];
        let pre = pre_map(&[("a.rs", a_pre), ("b.rs", b_pre)]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_member_intent_sweep(&sections, &pre, &mut used).expect("variant must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        validated_live(&out[0], &pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["b.rs"].contains("Status::Ready(1)"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["b.rs"].contains("Active"), "got: {}", files["b.rs"]);
    }
}
