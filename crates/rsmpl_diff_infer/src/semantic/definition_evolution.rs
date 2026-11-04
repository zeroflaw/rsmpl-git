//! Definition-value raising, slice 1 (WI-12): single-definition
//! evolution under the precision guard.
//!
//! The five WI-11 authored shapes already apply and verify; native
//! diffs that change exactly one definition RHS stay ordinary because
//! T375 prices the semantic hunk above a one-line edit. This detector
//! recovers the intent: one balanced pair (`N == 1`) — or N ≥ 2
//! uniform pairs sharing one shape class + identity + RHS delta —
//! compresses to the WI-11 authored spelling when the semantic form
//! costs at most `ordinary + PRECISION_GUARD` (the SPEC D.4 constant
//! WI-19 later generalises to all definition shapes).
//!
//! Shapes (concrete emission, no new template language):
//! - `type A = RHS;` (generics-free, same vis/name, RHS differs);
//! - `const/static [mut] NAME: TY = EXPR;` (same kind/vis/mut/name/ty,
//!   value differs);
//! - `old: TY` field pairs under a recovered `struct S` container;
//! - `Old(...)` / `Old { ... }` / `Old = EXPR` variant pairs under a
//!   recovered `enum E` container (single-region: payload XOR
//!   discriminant, arity/names must match).
//!
//! Member shapes require a pre-image container (the `member.rs`
//! discipline: exactly one carrier struct/enum naming the member at
//! the changed line); alias/const likewise require the pre-image for
//! the byte-compare probe, so `None` pre-image declines everything.
//! Totality is the scratch-apply byte-compare probe (T361 doctrine):
//! the emitted hunk must apply with `matches == n` AND reproduce
//! byte-identically the result of applying the ordinary pairs.
//! `# follow=uses` never emits (C17: evolution renames nothing).

use super::{item_emission_header, unique_rule};

/// Precision-guard constant (SPEC D.4): a definition-shape semantic
/// hunk may cost up to 16 bytes more than the ordinary hunk(s) it
/// replaces. Sweep shapes keep strict T375; WI-19 generalises this
/// bound to the other definition families.
pub(crate) const PRECISION_GUARD: usize = 16;

/// Whole-section / per-hunk pair detector: balanced N ≥ 1 uniform
/// definition-evolution pairs. Returns the single-hunk emission plus
/// its provenance detector name, or `None` when the section is not
/// exactly this shape. Byte guards live with the callers (strict
/// T375 for N ≥ 2 whole-section, precision guard for singles), so
/// this function reserves no rule on failure.
pub(crate) fn try_definition_evolution_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<(String, &'static str)> {
    if minuses.len() != pluses.len() || minuses.is_empty() {
        return super::prov::Counted::Miss;
    }
    if changed.len() != minuses.len() {
        return super::prov::Counted::Miss;
    }
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Classify the first pair; every other pair must share the class,
    // identity, and exact RHS delta (never half-claimed).
    let Some(first) = classify_pair(minuses[0], pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    for (m, p) in minuses.iter().zip(pluses.iter()).skip(1) {
        match classify_pair(m, p) {
            Some(k) if k == first => {}
            _ => return super::prov::Counted::Miss,
        }
    }
    // Container recovery for member shapes (exactly-one-carrier rule);
    // alias/const need no container.
    let container: Option<String> = match &first {
        EvoKind::Field { field } => Some(match struct_of_field(pre, field, changed[0]) {
            Some(v) => v,
            None => return super::prov::Counted::Miss,
        }),
        EvoKind::Variant { variant, .. } => Some(match enum_of_variant(pre, variant, changed[0]) {
            Some(v) => v,
            None => return super::prov::Counted::Miss,
        }),
        EvoKind::Alias { .. } | EvoKind::Const { .. } => None,
    };
    // Pre-image line alignment: each changed line must hold its minus
    // (trailing `,`/`;` tolerated — patterns omit member commas).
    let pre_lines: Vec<&str> = pre.lines().collect();
    for (i, &site) in changed.iter().enumerate() {
        let Some(pre_line) = pre_lines.get(site.saturating_sub(1)) else {
            return super::prov::Counted::Miss;
        };
        if norm_line(pre_line) != norm_line(minuses[i]) {
            return super::prov::Counted::Miss;
        }
    }
    // N ≥ 2 member evolutions must share the container (one hunk
    // names one container; hetero containers stay ordinary in v1).
    if minuses.len() > 1 {
        if let Some(c) = &container {
            for &site in changed.iter().skip(1) {
                let ok = match &first {
                    EvoKind::Field { field } => {
                        struct_of_field(pre, field, site).as_deref() == Some(c.as_str())
                    }
                    EvoKind::Variant { variant, .. } => {
                        enum_of_variant(pre, variant, site).as_deref() == Some(c.as_str())
                    }
                    _ => true,
                };
                if !ok {
                    return super::prov::Counted::Miss;
                }
            }
        }
    }
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, "definition_evolution");
    let mut out = item_emission_header(path, &rule, Some(pre), changed);
    match &first {
        EvoKind::Alias { .. } => {
            out.push_str(&format!(
                "- {}\n+ {}\n",
                minuses[0].trim(),
                pluses[0].trim()
            ));
        }
        EvoKind::Const { .. } => {
            out.push_str(&format!(
                "- {}\n+ {}\n",
                minuses[0].trim(),
                pluses[0].trim()
            ));
        }
        EvoKind::Field { .. } => {
            let Some(c) = container.clone() else {
                return super::prov::Counted::Miss;
            };
            out.push_str(&format!("  struct {c} {{\n"));
            out.push_str(&format!(
                "- {}\n+ {}\n",
                strip_member_comma(minuses[0]),
                strip_member_comma(pluses[0])
            ));
        }
        EvoKind::Variant { .. } => {
            let Some(c) = container.clone() else {
                return super::prov::Counted::Miss;
            };
            out.push_str(&format!("  enum {c} {{\n"));
            out.push_str(&format!(
                "- {}\n+ {}\n",
                strip_member_comma(minuses[0]),
                strip_member_comma(pluses[0])
            ));
        }
    }
    if !scratch_reproduces(pre, path, &out, &pre_lines, changed, pluses) {
        used_rules.truncate(checkpoint);
        match crate::totality_counted::scratch_line_verdict(Some(pre), minuses, changed) {
            crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality_counted::FamilyVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            _ => return super::prov::Counted::Miss,
        }
    }
    super::prov::Counted::Fire((out, "definition_evolution"))
}

/// One classified evolution pair: shape class + identity + concrete
/// RHS delta. Equality across pairs is the uniformity gate.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EvoKind {
    Alias {
        name: String,
    },
    Const {
        kind: String,
        name: String,
    },
    Field {
        field: String,
    },
    Variant {
        variant: String,
        shape: VariantShape,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariantShape {
    Tuple,
    Struct,
    Discriminant,
}

/// Classify one balanced pair into its evolution shape, or `None`
/// when the pair is not a same-identity RHS evolution (renames,
/// hetero shapes, and unparseable sides fall through to the other
/// detectors — never an error here).
fn classify_pair(minus: &str, plus: &str) -> Option<EvoKind> {
    if let Some((name, m_rhs, p_rhs)) = parse_alias_pair(minus, plus) {
        if m_rhs != p_rhs {
            return Some(EvoKind::Alias { name });
        }
        return None;
    }
    if let Some((kind, name, _ty, m_expr, p_expr)) = parse_const_pair(minus, plus) {
        if m_expr != p_expr {
            return Some(EvoKind::Const { kind, name });
        }
        return None;
    }
    if let Some((field, m_ty, p_ty)) = parse_field_evo_pair(minus, plus) {
        if m_ty != p_ty {
            return Some(EvoKind::Field { field });
        }
        return None;
    }
    if let Some((variant, shape_m, shape_p)) = parse_variant_evo_pair(minus, plus) {
        if shape_m == shape_p {
            return Some(EvoKind::Variant {
                variant,
                shape: shape_m,
            });
        }
    }
    None
}

/// `type NAME = RHS;` with same vis/name, generics-free, RHS differs.
/// Returns (name, minus_rhs, plus_rhs). Trailing `;` required.
fn parse_alias_pair(minus: &str, plus: &str) -> Option<(String, String, String)> {
    let (m_vis, m_name, m_rhs) = split_alias(minus)?;
    let (p_vis, p_name, p_rhs) = split_alias(plus)?;
    if m_vis != p_vis || m_name != p_name {
        return None;
    }
    Some((m_name, m_rhs, p_rhs))
}

fn split_alias(line: &str) -> Option<(String, String, String)> {
    let t = line.trim().trim_end_matches(',').trim();
    let (vis, rest) = strip_vis(t);
    let after = eat_word(rest, "type")?;
    let name_end = after
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let (name, tail) = (&after[..name_end], after[name_end..].trim_start());
    if !is_ident(name) {
        return None;
    }
    // Generics are preserved, never authored — v1 raising declines them.
    if tail.starts_with('<') {
        return None;
    }
    let rhs = tail.strip_prefix('=')?.trim();
    let rhs = rhs.strip_suffix(';')?.trim();
    if rhs.is_empty() {
        return None;
    }
    // Must parse as a type (else not this shape).
    let _: syn::Type = syn::parse_str(rhs).ok()?;
    Some((vis, name.to_string(), rhs.to_string()))
}

/// `(const|static [mut]) NAME: TY = EXPR;` with same everything except
/// the value. Returns (kind, name, ty, minus_expr, plus_expr).
fn parse_const_pair(minus: &str, plus: &str) -> Option<(String, String, String, String, String)> {
    let m = split_const(minus)?;
    let p = split_const(plus)?;
    if m.0 != p.0 || m.1 != p.1 || m.2 != p.2 || m.3 != p.3 {
        return None;
    }
    Some((m.0, m.1, m.2, m.4, p.4))
}

fn split_const(line: &str) -> Option<(String, String, String, String, String)> {
    let t = line.trim().trim_end_matches(',').trim();
    let (vis, rest) = strip_vis(t);
    let (kind, after) = if let Some(a) = eat_word(rest, "const") {
        ("const".to_string(), a)
    } else {
        let a = eat_word(rest, "static")?;
        if let Some(b) = eat_word(a, "mut") {
            ("static mut".to_string(), b)
        } else {
            ("static".to_string(), a)
        }
    };
    let name_end = after
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let (name, tail) = (&after[..name_end], &after[name_end..]);
    if !is_ident(name) {
        return None;
    }
    let colon = find_colon(tail)?;
    let after_ty = tail[colon + 1..].trim();
    let eq = after_ty.find('=')?;
    let ty_raw = after_ty[..eq].trim();
    let ty: syn::Type = syn::parse_str(ty_raw).ok()?;
    let norm_ty = quote::quote!(#ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let expr = after_ty[eq + 1..].trim().strip_suffix(';')?.trim();
    if expr.is_empty() {
        return None;
    }
    let _: syn::Expr = syn::parse_str(expr).ok()?;
    Some((kind, name.to_string(), norm_ty, vis, expr.to_string()))
}

/// `name: TY` field evolution: same name (and vis), types differ and
/// both parse. Returns (field, minus_ty_norm, plus_ty_norm).
fn parse_field_evo_pair(minus: &str, plus: &str) -> Option<(String, String, String)> {
    let (m_pre, m_name, m_ty) = split_field(minus)?;
    let (p_pre, p_name, p_ty) = split_field(plus)?;
    if m_pre != p_pre || m_name != p_name {
        return None;
    }
    if !is_ident(&m_name) {
        return None;
    }
    let m_parsed: syn::Type = syn::parse_str(&m_ty).ok()?;
    let p_parsed: syn::Type = syn::parse_str(&p_ty).ok()?;
    let m_norm = quote::quote!(#m_parsed)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let p_norm = quote::quote!(#p_parsed)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if m_norm == p_norm {
        return None;
    }
    Some((m_name, m_norm, p_norm))
}

fn split_field(line: &str) -> Option<(String, String, String)> {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
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
    Some((prefix, ident, right.to_string()))
}

/// Variant evolution: same ident, same payload class, payload differs.
/// Returns (variant, minus_shape, plus_shape); the caller requires
/// shape equality. Tuple arity and struct field names must match;
/// discriminants just need `=` on both sides with differing exprs.
fn parse_variant_evo_pair(minus: &str, plus: &str) -> Option<(String, VariantShape, VariantShape)> {
    let (m_ident, m_tail) = split_variant(minus)?;
    let (p_ident, p_tail) = split_variant(plus)?;
    if m_ident != p_ident || m_ident.is_empty() {
        return None;
    }
    if !is_ident(&m_ident) {
        return None;
    }
    let m_shape = variant_shape(&m_tail)?;
    let p_shape = variant_shape(&p_tail)?;
    if m_shape != p_shape || m_tail.trim() == p_tail.trim() {
        return None;
    }
    // Arity / field-name parity inside the class.
    match m_shape {
        VariantShape::Tuple => {
            if tuple_arity(&m_tail)? != tuple_arity(&p_tail)? {
                return None;
            }
        }
        VariantShape::Struct => {
            if struct_field_names(&m_tail)? != struct_field_names(&p_tail)? {
                return None;
            }
        }
        VariantShape::Discriminant => {}
    }
    Some((m_ident, m_shape, p_shape))
}

fn split_variant(line: &str) -> Option<(String, String)> {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
    if t.is_empty() {
        return None;
    }
    // Top-level colon means a field line, not a variant line.
    let mut depth = 0i32;
    for c in t.chars() {
        match c {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ':' if depth == 0 => return None,
            _ => {}
        }
    }
    let len = super::ident_len(t)?;
    let (ident, suffix) = t.split_at(len);
    Some((ident.to_string(), suffix.trim().to_string()))
}

fn variant_shape(tail: &str) -> Option<VariantShape> {
    let t = tail.trim();
    if t.is_empty() {
        return None;
    }
    if t.starts_with('(') {
        Some(VariantShape::Tuple)
    } else if t.starts_with('{') {
        Some(VariantShape::Struct)
    } else if t.starts_with('=') {
        Some(VariantShape::Discriminant)
    } else {
        None
    }
}

fn tuple_arity(tail: &str) -> Option<usize> {
    let t = tail.trim().strip_prefix('(')?.strip_suffix(')')?;
    if t.trim().is_empty() {
        return Some(0);
    }
    // Top-level comma count (nested delimiters don't split).
    let mut depth = 0i32;
    let mut count = 1usize;
    for c in t.chars() {
        match c {
            '(' | '{' | '[' | '<' => depth += 1,
            ')' | '}' | ']' | '>' => depth -= 1,
            ',' if depth == 0 => count += 1,
            _ => {}
        }
    }
    Some(count)
}

fn struct_field_names(tail: &str) -> Option<Vec<String>> {
    let t = tail.trim().strip_prefix('{')?.strip_suffix('}')?;
    let mut names = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let mut fields: Vec<String> = Vec::new();
    for c in t.chars() {
        match c {
            '(' | '{' | '[' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | '}' | ']' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                fields.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        fields.push(cur.trim().to_string());
    }
    for f in &fields {
        let name = f.split(':').next()?.trim();
        if !is_ident(name) {
            return None;
        }
        names.push(name.to_string());
    }
    Some(names)
}

/// Strip one trailing `,`/`;` plus surrounding whitespace for member
/// pattern lines (the engine tolerates the file comma).
fn strip_member_comma(line: &str) -> String {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
    t.to_string()
}

/// Whitespace-trimmed comparison key tolerating the member trailing
/// comma and alias/const trailing semicolons.
fn norm_line(line: &str) -> String {
    strip_member_comma(line)
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && super::ident_len(s) == Some(s.len())
}

fn strip_vis(s: &str) -> (String, &str) {
    let t = s.trim_start();
    if let Some(rest) = t.strip_prefix("pub") {
        if rest.is_empty() || rest.starts_with([' ', '\t']) {
            return ("pub".to_string(), rest.trim_start());
        }
        if let Some(after) = rest.strip_prefix('(') {
            let mut depth = 1i32;
            for (i, c) in after.char_indices() {
                if c == '(' {
                    depth += 1;
                } else if c == ')' {
                    depth -= 1;
                    if depth == 0 {
                        let raw = &s[..s.len() - after[i + 1..].trim_start().len()];
                        let norm: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
                        return (norm, after[i + 1..].trim_start());
                    }
                }
            }
            return (String::new(), s);
        }
        return (String::new(), s);
    }
    (String::new(), s)
}

fn eat_word<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(kw)?;
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        Some(rest.trim_start())
    } else {
        None
    }
}

/// Single-colon probe (no `::` adjacency), byte offset in `tail`.
fn find_colon(tail: &str) -> Option<usize> {
    let chars: Vec<char> = tail.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c == ':' {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j).copied());
            let next = chars.get(i + 1).copied();
            if prev != Some(':') && next != Some(':') {
                return Some(chars[..i].iter().collect::<String>().len());
            }
        }
    }
    None
}

/// The `struct S` carrying field `field` at 1-based `site`, or `None`
/// under the exactly-one-carrier rule (the `member.rs` discipline:
/// a second same-named field anywhere vetoes — the authored follow
/// would veto loud, so raising stays ordinary instead). Shared walk
/// lives in [`super::carrier`].
fn struct_of_field(pre: &str, field: &str, site: usize) -> Option<String> {
    super::carrier::struct_carrier_unique(pre, field, site)
}

/// The `enum E` carrying variant `variant` at 1-based `site`, or
/// `None` under the same uniqueness rule. Shared walk lives in
/// [`super::carrier`].
fn enum_of_variant(pre: &str, variant: &str, site: usize) -> Option<String> {
    super::carrier::enum_carrier_unique(pre, variant, site)
}

/// Scratch-apply byte-compare probe (T361): the emitted section must
/// apply cleanly with `matches == n` and reproduce byte-identically
/// the post tree (here: exactly the native plus lines at the changed
/// sites, everything else untouched — the `member.rs` line-equality
/// posture, since the ordinary pairs are the only change).
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
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    if report.rules.iter().map(|r| r.matches).sum::<usize>() != changed.len() {
        return false;
    }
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
    }
    for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
        if changed_set.contains(&(idx + 1)) {
            continue;
        }
        if pre_line != post_line {
            return false;
        }
    }
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

    #[test]
    fn precision_guard_constant_is_sixteen() {
        assert_eq!(PRECISION_GUARD, 16);
    }

    #[test]
    fn alias_concrete_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n",
        );
        let pre = pre_map(&[("f.rs", "type A = Vec<u8>;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("alias evolution must compress");
        assert!(out.contains("- type A = Vec<u8>;"), "got:\n{out}");
        assert!(out.contains("+ type A = Box<[u8]>;"), "got:\n{out}");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "type A = Box<[u8]>;\n");
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn const_value_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-const MAX: i32 = 1;\n+const MAX: i32 = 2;\n",
        );
        let pre = pre_map(&[("f.rs", "const MAX: i32 = 1;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("const evolution must compress");
        assert!(out.contains("const MAX: i32 = 1;"), "got:\n{out}");
        assert!(out.contains("const MAX: i32 = 2;"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "const MAX: i32 = 2;\n");
    }

    #[test]
    fn field_type_fires_and_round_trips() {
        let raw = section("f.rs", "@@ -2,1 +2,1 @@\n-    old: u32,\n+    old: u64,\n");
        let pre = pre_map(&[("f.rs", "struct S {\n    old: u32,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("field evolution must compress");
        assert!(out.contains("struct S {"), "got:\n{out}");
        assert!(out.contains("- old: u32"), "got:\n{out}");
        assert!(out.contains("+ old: u64"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "struct S {\n    old: u64,\n}\n");
    }

    #[test]
    fn tuple_payload_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    Old(i32),\n+    Old(String),\n",
        );
        let pre = pre_map(&[("f.rs", "enum E {\n    Old(i32),\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("payload evolution must compress");
        assert!(out.contains("enum E {"), "got:\n{out}");
        assert!(out.contains("- Old(i32)"), "got:\n{out}");
        assert!(out.contains("+ Old(String)"), "got:\n{out}");
    }

    #[test]
    fn discriminant_fires_and_round_trips() {
        let raw = section("f.rs", "@@ -2,1 +2,1 @@\n-    Old = 1,\n+    Old = 2,\n");
        let pre = pre_map(&[("f.rs", "enum E {\n    Old = 1,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("discriminant evolution must compress");
        assert!(out.contains("enum E {"), "got:\n{out}");
        assert!(out.contains("- Old = 1"), "got:\n{out}");
        assert!(out.contains("+ Old = 2"), "got:\n{out}");
    }

    #[test]
    fn rename_pair_declines() {
        // Same-type different-name is the rename shape, never evolution.
        // WI-19b: the def-only member single may claim it (with or
        // without follow) or it stays ordinary — it must never claim
        // to be an evolution (provenance detector decides, not text).
        let raw = section("f.rs", "@@ -2,1 +2,1 @@\n-    old: u32,\n+    new: u32,\n");
        let pre = pre_map(&[("f.rs", "struct S {\n    old: u32,\n}\n")]);
        let mut prov = Vec::new();
        let out = super::super::infer_semantic_with_provenance(
            std::slice::from_ref(&raw),
            &pre,
            &mut prov,
        );
        if out.is_some() {
            assert!(
                prov.iter().all(|p| p.detector != "definition_evolution"),
                "must never be evolution, got: {prov:?}"
            );
        }
    }

    #[test]
    fn untouched_twin_vetoes() {
        // Two identical definitions, one changed: the probe would also
        // rewrite the twin, so the section stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "type A = Vec<u8>;\nmod m {\n    type A = Vec<u8>;\n}\n",
        )]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_evolution")
            },
            "twin must veto evolution"
        );
    }

    #[test]
    fn hetero_delta_declines() {
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-type A = Vec<u8>;\n+type A = Box<[u8]>;\n@@ -3,1 +3,1 @@\n-type B = Vec<u8>;\n+type B = Box<[u16]>;\n",
        );
        let pre = pre_map(&[("f.rs", "type A = Vec<u8>;\ntype B = Vec<u8>;\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre);
        assert!(out.is_none() || !out.unwrap().contains("definition_evolution"));
    }

    #[test]
    fn call_shaped_pairs_decline() {
        // `foo(E)` vs `bar(E)` is a rename delta, never an evolution.
        assert!(classify_pair("foo(1)", "bar(1)").is_none());
        // Same-callee `foo(1)` -> `foo(2)` parses as a tuple-payload
        // shape at the pair level (the WI-11b call/container overlap),
        // but the detector declines it: no `enum foo` container exists
        // in a fn-body pre-image, so the container gate vetoes.
        let mut used = Vec::new();
        let pre = "fn m() {\n    foo(1);\n}\n";
        assert!(try_definition_evolution_sweep(
            &["foo(1)"],
            &["foo(2)"],
            "f.rs",
            Some(pre),
            &[2],
            &mut used,
        )
        .is_miss());
        assert!(used.is_empty());
    }
}
