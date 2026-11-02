//! WI-3: binder identity keyed by binding, not by spelling.
//!
//! A single entry point over the compiled pattern: any minus/plus pair
//! that renames exactly one bound binder — however spelled (`let`,
//! closure-param, match-arm, `for` / `if let` / `while let`, or a
//! closure parameter nested in a `let` init, the F3 spelling) — yields
//! a [`BinderRename`]. Spellings that cannot prove single-binder
//! identity (multi-leaf destructuring deltas, init/type/body changes, `move`
//! deltas, multi-binder diffs, bare-`P` annotation add/drop) yield
//! `None` and keep general-template behaviour with no binder
//! privileges (SPEC 18.1, D11). Single-leaf `let` destructuring deltas
//! and rename-once sequence-contained renames (plain or single-leaf
//! destructuring at any position, plus authored init-substitution of
//! later in-window uses) yield scope-`Let` identity with the `let`
//! completion/orphan duty.

use std::collections::HashMap;

use crate::{GeneralShape, PatternKind};

/// Which syntactic position the renamed binder occupies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinderScope {
    /// `let old = E;` → `let new = E;`.
    Let,
    /// `|old| Body` → `|new| Body`.
    Closure,
    /// `match S { old => B, … }` → `match S { new => B, … }`.
    MatchArm,
    /// `for old in E { … }` → `for new in E { … }`.
    For,
    /// `if let old = E { … }` → `if let new = E { … }`.
    IfLet,
    /// `while let old = E { … }` → `while let new = E { … }`.
    WhileLet,
    /// F3: `let g = |old| E;` → `let g = |sum| E;` — the renamed binder
    /// lives in the `let` init closure; the outer `let` name is unchanged.
    LetClosure,
    /// A same-name `fn` header with one renamed parameter. The pattern
    /// crate never produces this from a kind alone (params live in
    /// `FnPattern`, and the hunk is needed); the engine fills it in.
    FnParam,
}

/// A compiled single-binder rename: the old/new ident plus the scope
/// whose unshadowed value uses complete or become orphans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinderRename {
    pub old: String,
    pub new: String,
    pub scope: BinderScope,
}

/// The single binder entry: `Some` iff `kind` renames exactly one bound
/// binder and nothing else in the pattern changes.
pub fn binder_rename(kind: &PatternKind) -> Option<BinderRename> {
    let (old, new, scope) = let_binder(kind)
        .map(|(o, n)| (o, n, BinderScope::Let))
        .or_else(|| closure_param(kind).map(|(o, n)| (o, n, BinderScope::Closure)))
        .or_else(|| match_arm(kind).map(|(o, n)| (o, n, BinderScope::MatchArm)))
        .or_else(|| for_loop(kind).map(|(o, n)| (o, n, BinderScope::For)))
        .or_else(|| if_let(kind).map(|(o, n)| (o, n, BinderScope::IfLet)))
        .or_else(|| while_let(kind).map(|(o, n)| (o, n, BinderScope::WhileLet)))
        .or_else(|| let_closure_param(kind).map(|(o, n)| (o, n, BinderScope::LetClosure)))
        .or_else(|| sequence_binder(kind).map(|(o, n)| (o, n, BinderScope::Let)))?;
    Some(BinderRename { old, new, scope })
}

/// Placeholder map (`__RSMPL_*` → declared name) for a general pattern.
fn ph_map(kind_placeholders: &[(String, String)]) -> HashMap<&str, &str> {
    kind_placeholders
        .iter()
        .map(|(name, ph)| (ph.as_str(), name.as_str()))
        .collect()
}

/// Map `__RSMPL_*` placeholder idents back to declared names, one whole
/// ident token at a time (single-hole `__RSMPL_E` is a string prefix of
/// the `...` marker `__RSMPL_ELLIPSIS_0`, so plain `str::replace`
/// would mangle markers).
fn normalize_placeholders(s: &str, ph_map: &HashMap<&str, &str>) -> String {
    if ph_map.is_empty() || !s.contains("__RSMPL_") {
        return s.to_string();
    }
    let mut keys: Vec<(&str, &str)> = ph_map.iter().map(|(k, v)| (*k, *v)).collect();
    keys.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        let mut matched: Option<(usize, &str)> = None;
        for (ph, name) in &keys {
            let pb = ph.as_bytes();
            if i + pb.len() > bytes.len() {
                continue;
            }
            if &bytes[i..i + pb.len()] != pb {
                continue;
            }
            let prev_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let next_ok = i + pb.len() == bytes.len() || !is_ident_byte(bytes[i + pb.len()]);
            if prev_ok && next_ok {
                matched = Some((pb.len(), *name));
                break;
            }
        }
        if let Some((len, name)) = matched {
            out.push_str(name);
            i += len;
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// General-shape accessor: `(minus_sub, plus_body, placeholders)` when
/// `kind` is a `General` hunk of the wanted shape with equal attrs.
type GeneralParts<'a> = (&'a str, &'a str, &'a [(String, String)]);
fn general_parts(kind: &PatternKind, want: GeneralShape) -> Option<GeneralParts<'_>> {
    let PatternKind::General {
        shape,
        minus_sub,
        plus_body,
        placeholders,
        minus_attrs,
        plus_attrs,
        ..
    } = kind
    else {
        return None;
    };
    if *shape != want || *minus_attrs != *plus_attrs {
        return None;
    }
    Some((minus_sub, plus_body, placeholders))
}

/// Single `x` binder (no subpattern); returns `(ident, has_mut)`.
/// Placeholder idents (`__RSMPL_*`) never count as binders.
fn single_ident_binder(pat: &syn::Pat) -> Option<(String, bool)> {
    match pat {
        syn::Pat::Ident(pid) => {
            if pid.subpat.is_some() {
                return None;
            }
            let name = pid.ident.to_string();
            if name.starts_with("__RSMPL_") {
                return None;
            }
            Some((name, pid.mutability.is_some()))
        }
        syn::Pat::Type(pt) => single_ident_binder(&pt.pat),
        _ => None,
    }
}

/// Type-annotation tokens of a pattern (`""` when unannotated).
/// NOTE: field access must happen outside `quote!` — `#pt.ty` would
/// interpolate `pt` plus a literal `.ty` (and the binder name with it,
/// so same-type renames would falsely veto).
fn pat_ty(pat: &syn::Pat, ph: &HashMap<&str, &str>) -> String {
    match pat {
        syn::Pat::Type(pt) => {
            let ty = &*pt.ty;
            normalize_placeholders(&quote::quote!(#ty).to_string(), ph)
        }
        _ => String::new(),
    }
}

fn render<T: quote::ToTokens>(node: &T) -> String {
    quote::quote!(#node).to_string()
}

/// Parse a general-expression pattern forgiving the block-tail `...`
/// ellipsis (not valid `syn` on its own): retry with the engine's
/// ellipsis-marker ident, which compares equal on both sides when both
/// were `...`.
fn parse_expr_forgiving(s: &str) -> Option<syn::Expr> {
    if let Ok(e) = syn::parse_str::<syn::Expr>(s) {
        return Some(e);
    }
    let fixed = s.replace("...", "__RSMPL_ELLIPSIS_0");
    syn::parse_str::<syn::Expr>(&fixed).ok()
}

/// Declared hole names (`P`, `E`, …) for a general pattern: idents that
/// replay matched fragments and never count as binders.
fn declared_holes(placeholders: &[(String, String)]) -> std::collections::HashSet<String> {
    placeholders.iter().map(|(n, _)| n.clone()).collect()
}

/// True when `name` is a hole/marker ident (never a binder).
fn is_hole_name(name: &str, declared: &std::collections::HashSet<String>) -> bool {
    name.starts_with("__RSMPL_") || declared.contains(name)
}

/// Whole-ident occurrence of `needle` in `hay` (ident-boundary gated).
fn contains_ident_bounded(hay: &str, needle: &str) -> bool {
    if needle.is_empty() || !hay.contains(needle) {
        return false;
    }
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    let mut i = 0;
    while i + nb.len() <= hb.len() {
        if &hb[i..i + nb.len()] == nb {
            let prev_ok = i == 0 || !is_ident_byte(hb[i - 1]);
            let next_ok = i + nb.len() == hb.len() || !is_ident_byte(hb[i + nb.len()]);
            if prev_ok && next_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Single-leaf destructuring diff: `Ok(Some((old, new)))` when `m`/`p`
/// are structurally equal except exactly one leaf plain-ident binder
/// differs; `Ok(None)` when token-equal; `Err(())` on any other shape
/// (multi-leaf, constructor/path/arity change, `Or`, hole concretion).
fn single_leaf_diff(
    m: &syn::Pat,
    p: &syn::Pat,
    ph: &HashMap<&str, &str>,
    declared: &std::collections::HashSet<String>,
) -> Result<Option<(String, String)>, ()> {
    fn merge(
        a: Option<(String, String)>,
        b: Option<(String, String)>,
    ) -> Result<Option<(String, String)>, ()> {
        match (a, b) {
            (None, None) => Ok(None),
            (Some(d), None) | (None, Some(d)) => Ok(Some(d)),
            (Some(_), Some(_)) => Err(()),
        }
    }
    match (m, p) {
        (syn::Pat::Ident(mm), syn::Pat::Ident(pp)) => {
            if mm.subpat.is_some() || pp.subpat.is_some() {
                return Err(());
            }
            let mn = mm.ident.to_string();
            let pn = pp.ident.to_string();
            let m_hole = is_hole_name(&mn, declared);
            let p_hole = is_hole_name(&pn, declared);
            if m_hole || p_hole {
                // Hole positions replay fragments: only token-equal
                // holes compare equal, anything else vetoes (concretion
                // is C21's domain, never binder identity).
                if mn == pn && mm.mutability.is_some() == pp.mutability.is_some() {
                    return Ok(None);
                }
                return Err(());
            }
            // Concrete leaves: delegate mut discipline to the caller
            // shape (same as plain `let`).
            if mn == pn {
                if mm.mutability.is_some() != pp.mutability.is_some() {
                    return Err(());
                }
                return Ok(None);
            }
            if mm.mutability.is_some() != pp.mutability.is_some() {
                return Err(());
            }
            // Same-type leaves compare at the top-level `pat_ty`
            // gate; inner `Type` nodes compare inline below.
            Ok(Some((mn, pn)))
        }
        (syn::Pat::Type(mm), syn::Pat::Type(pp)) => {
            if normalize_placeholders(&render(&mm.ty), ph)
                != normalize_placeholders(&render(&pp.ty), ph)
            {
                return Err(());
            }
            single_leaf_diff(&mm.pat, &pp.pat, ph, declared)
        }
        (syn::Pat::Paren(mm), syn::Pat::Paren(pp)) => {
            single_leaf_diff(&mm.pat, &pp.pat, ph, declared)
        }
        (syn::Pat::Reference(mm), syn::Pat::Reference(pp)) => {
            if mm.mutability.is_some() != pp.mutability.is_some() {
                return Err(());
            }
            single_leaf_diff(&mm.pat, &pp.pat, ph, declared)
        }
        (syn::Pat::Tuple(mm), syn::Pat::Tuple(pp)) => {
            if mm.elems.len() != pp.elems.len() {
                return Err(());
            }
            let mut acc: Option<(String, String)> = None;
            for (a, b) in mm.elems.iter().zip(pp.elems.iter()) {
                acc = merge(acc, single_leaf_diff(a, b, ph, declared)?)?;
            }
            Ok(acc)
        }
        (syn::Pat::TupleStruct(mm), syn::Pat::TupleStruct(pp)) => {
            if render(&mm.path) != render(&pp.path) || mm.elems.len() != pp.elems.len() {
                return Err(());
            }
            let mut acc: Option<(String, String)> = None;
            for (a, b) in mm.elems.iter().zip(pp.elems.iter()) {
                acc = merge(acc, single_leaf_diff(a, b, ph, declared)?)?;
            }
            Ok(acc)
        }
        (syn::Pat::Struct(mm), syn::Pat::Struct(pp)) => {
            if render(&mm.path) != render(&pp.path) || mm.fields.len() != pp.fields.len() {
                return Err(());
            }
            let mut acc: Option<(String, String)> = None;
            for (a, b) in mm.fields.iter().zip(pp.fields.iter()) {
                let shorthand = a.colon_token.is_none() && b.colon_token.is_none();
                if shorthand {
                    // Shorthand `S { old }` binds `old`: the member and
                    // the pat rename in lockstep, so a member change is
                    // the same single-leaf diff (not a second delta).
                    let ma = render(&a.member);
                    let mb = render(&b.member);
                    if ma != mb {
                        match single_leaf_diff(&a.pat, &b.pat, ph, declared)? {
                            Some((o, n)) => {
                                // Member text for shorthand is the
                                // bare ident; accept iff the pat diff
                                // renames the same ident.
                                let ma_id = ma
                                    .split_whitespace()
                                    .last()
                                    .unwrap_or("")
                                    .trim_end_matches(',');
                                let mb_id = mb
                                    .split_whitespace()
                                    .last()
                                    .unwrap_or("")
                                    .trim_end_matches(',');
                                if o != ma_id || n != mb_id {
                                    return Err(());
                                }
                                acc = merge(acc, Some((o, n)))?;
                            }
                            None => return Err(()),
                        }
                        continue;
                    }
                    acc = merge(acc, single_leaf_diff(&a.pat, &b.pat, ph, declared)?)?;
                    continue;
                }
                if render(&a.member) != render(&b.member)
                    || a.colon_token.is_some() != b.colon_token.is_some()
                {
                    return Err(());
                }
                acc = merge(acc, single_leaf_diff(&a.pat, &b.pat, ph, declared)?)?;
            }
            Ok(acc)
        }
        (syn::Pat::Slice(mm), syn::Pat::Slice(pp)) => {
            if mm.elems.len() != pp.elems.len() {
                return Err(());
            }
            let mut acc: Option<(String, String)> = None;
            for (a, b) in mm.elems.iter().zip(pp.elems.iter()) {
                acc = merge(acc, single_leaf_diff(a, b, ph, declared)?)?;
            }
            Ok(acc)
        }
        _ => {
            // Binder-free or opaque positions: token-equal only.
            if normalize_placeholders(&render(m), ph) == normalize_placeholders(&render(p), ph) {
                Ok(None)
            } else {
                Err(())
            }
        }
    }
}

/// `(old, new)` for a plain `let old = E;` → `let new = E;` rename with
/// identical init and type annotation, plus the single-leaf
/// destructuring twin (`let (a, old) = E;` → `let (a, new) = E;`,
/// struct/shorthand/slice/wrapper spellings — exactly one leaf
/// differs, same init/type discipline).
fn let_binder(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Stmt)?;
    let ph = ph_map(placeholders);
    let declared = declared_holes(placeholders);
    let Ok(syn::Stmt::Local(minus)) = syn::parse_str::<syn::Stmt>(minus_sub) else {
        return None;
    };
    let Ok(syn::Stmt::Local(plus)) = syn::parse_str::<syn::Stmt>(plus_body) else {
        return None;
    };
    if let (Some((old_id, old_mut)), Some((new_id, new_mut))) = (
        single_ident_binder(&minus.pat),
        single_ident_binder(&plus.pat),
    ) {
        // Plain pair present on both sides: the plain discipline owns
        // it (a destructuring leaf never reaches here).
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        if pat_ty(&minus.pat, &ph) != pat_ty(&plus.pat, &ph) {
            return None;
        }
        let minus_init = minus.init.as_ref().map(|i| render(&i.expr))?;
        let plus_init = plus.init.as_ref().map(|i| render(&i.expr))?;
        if normalize_placeholders(&minus_init, &ph) != normalize_placeholders(&plus_init, &ph) {
            return None;
        }
        return Some((old_id, new_id));
    }
    // Single-leaf destructuring twin: exactly one leaf differs, same
    // init/type discipline as the plain arm.
    if pat_ty(&minus.pat, &ph) != pat_ty(&plus.pat, &ph) {
        return None;
    }
    let minus_init = minus.init.as_ref().map(|i| render(&i.expr))?;
    let plus_init = plus.init.as_ref().map(|i| render(&i.expr))?;
    if normalize_placeholders(&minus_init, &ph) != normalize_placeholders(&plus_init, &ph) {
        return None;
    }
    match single_leaf_diff(&minus.pat, &plus.pat, &ph, &declared) {
        Ok(Some((o, n))) => Some((o, n)),
        _ => None,
    }
}

/// Sequence-contained `let` rename: K (2-4) `let` statements where
/// EXACTLY ONE position renames a binder — plain single-ident
/// (`let old = E;` -> `let new = E;`) or single-leaf destructuring
/// (`let (x, old) = E;` -> `let (x, new) = E;`, struct/shorthand/slice
/// twins) — with same init and type annotation. Every other position
/// is either placeholder-normalized token-equal or an
/// *init-substitution* spelling the intra-window use explicitly:
/// same pat, init differing only by `old` → `new` swaps
/// (`let b = old;` -> `let b = new;`). The window applies verbatim,
/// so the plus side keeps re-matching the post-image (SPEC 24.6);
/// post-window uses complete via the `let` follow/orphan machinery
/// with the window end as the floor.
///
/// The rename-once gate is the soundness discipline: substitution
/// positions must name the same pair, sit strictly *after* the
/// renamed position (a pre-binder use names an outer binder), and
/// round-trip exactly (partial swaps decline); any other delta —
/// hetero-fusion runs, deletions, same-position init changes,
/// `new`/earlier/pat mentions outside substitution — declines
/// (`None`) and keeps today's exempt behaviour.
fn sequence_binder(kind: &PatternKind) -> Option<(String, String)> {
    sequence_binder_positions(kind).map(|(_, _, old, new)| (old, new))
}

/// Pattern-side positions of a sequence binder rename: the renamed
/// index, the authored init-substitution indices, and the pair. The
/// orphan cover (`rsmpl_engine`) maps these onto target windows.
pub fn sequence_binder_positions(
    kind: &PatternKind,
) -> Option<(usize, Vec<usize>, String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Sequence)?;
    let ph = ph_map(placeholders);
    let declared = declared_holes(placeholders);
    if plus_body.trim().is_empty() {
        return None;
    }
    let Ok(minus_block) = syn::parse_str::<syn::Block>(&format!("{{ {minus_sub} }}")) else {
        return None;
    };
    let Ok(plus_block) = syn::parse_str::<syn::Block>(&format!("{{ {plus_body} }}")) else {
        return None;
    };
    if !(2..=4).contains(&minus_block.stmts.len())
        || plus_block.stmts.len() != minus_block.stmts.len()
    {
        return None;
    }
    let mut minus_locals = Vec::with_capacity(minus_block.stmts.len());
    let mut plus_locals = Vec::with_capacity(plus_block.stmts.len());
    for (m, p) in minus_block.stmts.iter().zip(plus_block.stmts.iter()) {
        let (syn::Stmt::Local(m), syn::Stmt::Local(p)) = (m, p) else {
            return None;
        };
        minus_locals.push(m);
        plus_locals.push(p);
    }
    // Exactly one pat-rename; every other position token-equal or a
    // clean init-substitution for the same pair.
    let mut found: Option<(usize, String, String)> = None;
    let mut substitutions: Vec<usize> = Vec::new();
    for (i, (m, p)) in minus_locals.iter().zip(plus_locals.iter()).enumerate() {
        if normalize_placeholders(&render(m), &ph) == normalize_placeholders(&render(p), &ph) {
            continue;
        }
        if let Some((old_id, new_id)) = sequence_position_rename(m, p, &ph, &declared) {
            if found.is_some() {
                return None;
            }
            found = Some((i, old_id, new_id));
            continue;
        }
        substitutions.push(i);
    }
    let (idx, old_id, new_id) = found?;
    // Substitution positions: same pat, init a total old→new swap
    // round-tripping exactly, strictly after the renamed position.
    for i in &substitutions {
        if *i < idx {
            return None;
        }
        let (m, p) = (minus_locals[*i], plus_locals[*i]);
        if normalize_placeholders(&render(&m.pat), &ph)
            != normalize_placeholders(&render(&p.pat), &ph)
        {
            return None;
        }
        if pat_ty(&m.pat, &ph) != pat_ty(&p.pat, &ph) {
            return None;
        }
        let m_init = m.init.as_ref().map(|x| render(&x.expr))?;
        let p_init = p.init.as_ref().map(|x| render(&x.expr))?;
        let m_norm = normalize_placeholders(&m_init, &ph);
        let p_norm = normalize_placeholders(&p_init, &ph);
        if !contains_ident_bounded(&m_norm, &old_id) {
            return None;
        }
        if replace_ident_bounded(&m_norm, &old_id, &new_id) != p_norm
            || replace_ident_bounded(&p_norm, &new_id, &old_id) != m_norm
        {
            return None;
        }
    }
    // Soundness on the remaining token-equal positions: `old` may
    // appear (an in-window use replays verbatim and strands as an
    // orphan when it names the renamed binder — honest `partial`,
    // never silent); `new` may not (the rename would capture it), nor
    // may another pat mention the pair (a second bound occurrence
    // unmans the single-binder proof).
    for (i, (m, p)) in minus_locals.iter().zip(plus_locals.iter()).enumerate() {
        if i == idx || substitutions.contains(&i) {
            continue;
        }
        for local in [m, p] {
            let text = normalize_placeholders(&render(local), &ph);
            if contains_ident_bounded(&text, &new_id) {
                return None;
            }
        }
        for local in [m, p] {
            let pat_text = normalize_placeholders(&render(&local.pat), &ph);
            if contains_ident_bounded(&pat_text, &old_id)
                || contains_ident_bounded(&pat_text, &new_id)
            {
                return None;
            }
        }
    }
    Some((idx, substitutions, old_id, new_id))
}

/// Replace whole-ident occurrences of `needle` with `replacement`
/// (ident-boundary gated).
fn replace_ident_bounded(hay: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || !hay.contains(needle) {
        return hay.to_string();
    }
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let hb = hay.as_bytes();
    let nb = needle.as_bytes();
    let mut out = String::with_capacity(hay.len());
    let mut i = 0;
    while i < hb.len() {
        if i + nb.len() <= hb.len() && &hb[i..i + nb.len()] == nb {
            let prev_ok = i == 0 || !is_ident_byte(hb[i - 1]);
            let next_ok = i + nb.len() == hb.len() || !is_ident_byte(hb[i + nb.len()]);
            if prev_ok && next_ok {
                out.push_str(replacement);
                i += nb.len();
                continue;
            }
        }
        let ch = hay[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// One sequence position rename: plain single-ident or single-leaf
/// destructuring, same init/type discipline as the `let` arm.
fn sequence_position_rename(
    m: &syn::Local,
    p: &syn::Local,
    ph: &HashMap<&str, &str>,
    declared: &std::collections::HashSet<String>,
) -> Option<(String, String)> {
    if let (Some((old_id, old_mut)), Some((new_id, new_mut))) =
        (single_ident_binder(&m.pat), single_ident_binder(&p.pat))
    {
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        if pat_ty(&m.pat, ph) != pat_ty(&p.pat, ph) {
            return None;
        }
        let minus_init = m.init.as_ref().map(|i| render(&i.expr))?;
        let plus_init = p.init.as_ref().map(|i| render(&i.expr))?;
        if normalize_placeholders(&minus_init, ph) != normalize_placeholders(&plus_init, ph) {
            return None;
        }
        return Some((old_id, new_id));
    }
    if pat_ty(&m.pat, ph) != pat_ty(&p.pat, ph) {
        return None;
    }
    let minus_init = m.init.as_ref().map(|i| render(&i.expr))?;
    let plus_init = p.init.as_ref().map(|i| render(&i.expr))?;
    if normalize_placeholders(&minus_init, ph) != normalize_placeholders(&plus_init, ph) {
        return None;
    }
    match single_leaf_diff(&m.pat, &p.pat, ph, declared) {
        Ok(Some((o, n))) => Some((o, n)),
        _ => None,
    }
}

/// Shared closure-parameter diff: exactly one single-ident parameter
/// may differ (same annotation discipline as `let`); qualifiers must
/// match; bodies are compared by the caller.
fn closure_param_diff(
    minus: &syn::ExprClosure,
    plus: &syn::ExprClosure,
    ph: &HashMap<&str, &str>,
) -> Option<(String, String)> {
    if render(&minus.asyncness) != render(&plus.asyncness)
        || render(&minus.movability) != render(&plus.movability)
        || render(&minus.capture) != render(&plus.capture)
        || render(&minus.lifetimes) != render(&plus.lifetimes)
        || render(&minus.constness) != render(&plus.constness)
        || render(&minus.output) != render(&plus.output)
        || minus.inputs.len() != plus.inputs.len()
    {
        return None;
    }
    let mut diff: Option<(String, String)> = None;
    for (mp, pp) in minus.inputs.iter().zip(plus.inputs.iter()) {
        let mn = normalize_placeholders(&render(mp), ph);
        let pn = normalize_placeholders(&render(pp), ph);
        if mn == pn {
            continue;
        }
        let (old_id, old_mut) = single_ident_binder(mp)?;
        let (new_id, new_mut) = single_ident_binder(pp)?;
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        if pat_ty(mp, ph) != pat_ty(pp, ph) {
            return None;
        }
        if diff.replace((old_id, new_id)).is_some() {
            return None;
        }
    }
    diff
}

/// `(old, new)` for `|old| Body` → `|new| Body` with
/// placeholder-normalized-equal bodies.
fn closure_param(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Expr::Closure(minus)) = syn::parse_str::<syn::Expr>(minus_sub) else {
        return None;
    };
    let Ok(syn::Expr::Closure(plus)) = syn::parse_str::<syn::Expr>(plus_body) else {
        return None;
    };
    let (old, new) = closure_param_diff(&minus, &plus, &ph)?;
    if normalize_placeholders(&render(&*minus.body), &ph)
        != normalize_placeholders(&render(&*plus.body), &ph)
    {
        return None;
    }
    Some((old, new))
}

/// `(old, new)` for `match S { old => B, … }` → `match S { new => B, … }`.
fn match_arm(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Expr::Match(minus)) = syn::parse_str::<syn::Expr>(minus_sub) else {
        return None;
    };
    let Ok(syn::Expr::Match(plus)) = syn::parse_str::<syn::Expr>(plus_body) else {
        return None;
    };
    if normalize_placeholders(&render(&*minus.expr), &ph)
        != normalize_placeholders(&render(&*plus.expr), &ph)
        || minus.arms.len() != plus.arms.len()
    {
        return None;
    }
    let mut diff: Option<(String, String)> = None;
    for (ma, pa) in minus.arms.iter().zip(plus.arms.iter()) {
        let mn = normalize_placeholders(&render(ma), &ph);
        let pn = normalize_placeholders(&render(pa), &ph);
        if mn == pn {
            continue;
        }
        let (old_id, old_mut) = single_ident_binder(&ma.pat)?;
        let (new_id, new_mut) = single_ident_binder(&pa.pat)?;
        if old_id == new_id || old_mut != new_mut {
            return None;
        }
        match (&ma.guard, &pa.guard) {
            (None, None) => {}
            (Some((_, mg)), Some((_, pg))) => {
                if normalize_placeholders(&render(&**mg), &ph)
                    != normalize_placeholders(&render(&**pg), &ph)
                {
                    return None;
                }
            }
            _ => return None,
        }
        if normalize_placeholders(&render(&*ma.body), &ph)
            != normalize_placeholders(&render(&*pa.body), &ph)
        {
            return None;
        }
        if diff.replace((old_id, new_id)).is_some() {
            return None;
        }
    }
    diff
}

/// `(old, new)` for `for old in E { … }` → `for new in E { … }`.
fn for_loop(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Some(syn::Expr::ForLoop(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::ForLoop(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    if render(&minus.label) != render(&plus.label) {
        return None;
    }
    let (old_id, old_mut) = single_ident_binder(&minus.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if normalize_placeholders(&render(&*minus.expr), &ph)
        != normalize_placeholders(&render(&*plus.expr), &ph)
        || normalize_placeholders(&render(&minus.body), &ph)
            != normalize_placeholders(&render(&plus.body), &ph)
    {
        return None;
    }
    Some((old_id, new_id))
}

/// `(old, new)` for `if let old = E { … } …` → `if let new = E { … } …`.
fn if_let(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Some(syn::Expr::If(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::If(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    let syn::Expr::Let(minus_let) = &*minus.cond else {
        return None;
    };
    let syn::Expr::Let(plus_let) = &*plus.cond else {
        return None;
    };
    let (old_id, old_mut) = single_ident_binder(&minus_let.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus_let.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if normalize_placeholders(&render(&*minus_let.expr), &ph)
        != normalize_placeholders(&render(&*plus_let.expr), &ph)
        || normalize_placeholders(&render(&minus.then_branch), &ph)
            != normalize_placeholders(&render(&plus.then_branch), &ph)
    {
        return None;
    }
    match (&minus.else_branch, &plus.else_branch) {
        (None, None) => {}
        (Some((_, a)), Some((_, b))) => {
            if normalize_placeholders(&render(&**a), &ph)
                != normalize_placeholders(&render(&**b), &ph)
            {
                return None;
            }
        }
        _ => return None,
    }
    Some((old_id, new_id))
}

/// `(old, new)` for `while let old = E { … }` → `while let new = E { … }`.
fn while_let(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Expr)?;
    let ph = ph_map(placeholders);
    let Some(syn::Expr::While(minus)) = parse_expr_forgiving(minus_sub) else {
        return None;
    };
    let Some(syn::Expr::While(plus)) = parse_expr_forgiving(plus_body) else {
        return None;
    };
    if render(&minus.label) != render(&plus.label) {
        return None;
    }
    let syn::Expr::Let(minus_let) = &*minus.cond else {
        return None;
    };
    let syn::Expr::Let(plus_let) = &*plus.cond else {
        return None;
    };
    let (old_id, old_mut) = single_ident_binder(&minus_let.pat)?;
    let (new_id, new_mut) = single_ident_binder(&plus_let.pat)?;
    if old_id == new_id || old_mut != new_mut {
        return None;
    }
    if normalize_placeholders(&render(&*minus_let.expr), &ph)
        != normalize_placeholders(&render(&*plus_let.expr), &ph)
        || normalize_placeholders(&render(&minus.body), &ph)
            != normalize_placeholders(&render(&plus.body), &ph)
    {
        return None;
    }
    Some((old_id, new_id))
}

/// F3 headline: `(old, new)` for `let g = |old| E;` → `let g = |new| E;` —
/// the outer `let` name is unchanged, exactly one closure parameter in
/// the init is renamed, and init bodies are placeholder-equal.
fn let_closure_param(kind: &PatternKind) -> Option<(String, String)> {
    let (minus_sub, plus_body, placeholders) = general_parts(kind, GeneralShape::Stmt)?;
    let ph = ph_map(placeholders);
    let Ok(syn::Stmt::Local(minus)) = syn::parse_str::<syn::Stmt>(minus_sub) else {
        return None;
    };
    let Ok(syn::Stmt::Local(plus)) = syn::parse_str::<syn::Stmt>(plus_body) else {
        return None;
    };
    // Outer `let` binder must be identical (this is not a `let` rename).
    if render(&minus.pat) != render(&plus.pat) {
        return None;
    }
    let minus_init = minus.init.as_ref().map(|i| render(&i.expr))?;
    let plus_init = plus.init.as_ref().map(|i| render(&i.expr))?;
    // A plain `let` rename has token-equal inits; here the inits differ
    // only in the closure parameter, so equal inits veto (owned by `let`).
    if normalize_placeholders(&minus_init, &ph) == normalize_placeholders(&plus_init, &ph) {
        return None;
    }
    let minus_expr = minus.init.as_ref().map(|i| &*i.expr)?;
    let plus_expr = plus.init.as_ref().map(|i| &*i.expr)?;
    let (Some(minus_cl), Some(plus_cl)) = (as_closure(minus_expr), as_closure(plus_expr)) else {
        return None;
    };
    let (old, new) = closure_param_diff(minus_cl, plus_cl, &ph)?;
    if normalize_placeholders(&render(&*minus_cl.body), &ph)
        != normalize_placeholders(&render(&*plus_cl.body), &ph)
    {
        return None;
    }
    Some((old, new))
}

/// Unwrap one layer: closures directly in the init, or parenthesised.
fn as_closure(expr: &syn::Expr) -> Option<&syn::ExprClosure> {
    match expr {
        syn::Expr::Closure(c) => Some(c),
        syn::Expr::Paren(p) => as_closure(&p.expr),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stmt(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Stmt,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    fn expr(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![("E".to_string(), "__RSMPL_E0".to_string())],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    fn renamed(kind: &PatternKind) -> Option<(String, String, BinderScope)> {
        binder_rename(kind).map(|b| (b.old, b.new, b.scope))
    }

    #[test]
    fn let_fires() {
        assert_eq!(
            renamed(&stmt("let total = __RSMPL_E0;", "let sum = E;")),
            Some(("total".to_string(), "sum".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn annotated_same_fires() {
        assert_eq!(
            renamed(&stmt("let total : u32 = __RSMPL_E0;", "let sum : u32 = E;")),
            Some(("total".to_string(), "sum".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn closure_fires() {
        assert_eq!(
            renamed(&expr("|total| __RSMPL_E0", "|sum| E")),
            Some(("total".to_string(), "sum".to_string(), BinderScope::Closure))
        );
    }

    #[test]
    fn match_arm_fires() {
        let kind = PatternKind::General {
            shape: GeneralShape::Expr,
            minus_sub: "match __RSMPL_E0 { total => __RSMPL_E1 , _ => 0 }".to_string(),
            plus_raw: "match E { sum => B , _ => 0 }".to_string(),
            plus_body: "match E { sum => B , _ => 0 }".to_string(),
            placeholders: vec![
                ("E".to_string(), "__RSMPL_E0".to_string()),
                ("B".to_string(), "__RSMPL_E1".to_string()),
            ],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        };
        assert_eq!(
            renamed(&kind),
            Some((
                "total".to_string(),
                "sum".to_string(),
                BinderScope::MatchArm
            ))
        );
    }

    #[test]
    fn for_fires() {
        assert_eq!(
            renamed(&expr(
                "for total in __RSMPL_E0 { ... }",
                "for sum in E { ... }"
            )),
            Some(("total".to_string(), "sum".to_string(), BinderScope::For))
        );
    }

    #[test]
    fn if_let_fires() {
        assert_eq!(
            renamed(&expr(
                "if let total = __RSMPL_E0 { ... }",
                "if let sum = E { ... }"
            )),
            Some(("total".to_string(), "sum".to_string(), BinderScope::IfLet))
        );
    }

    #[test]
    fn while_let_fires() {
        assert_eq!(
            renamed(&expr(
                "while let total = __RSMPL_E0 { ... }",
                "while let sum = E { ... }"
            )),
            Some((
                "total".to_string(),
                "sum".to_string(),
                BinderScope::WhileLet
            ))
        );
    }

    #[test]
    fn let_closure_fires_headline() {
        // F3: the escaping spelling now has an identity.
        assert_eq!(
            renamed(&stmt(
                "let g = | total | __RSMPL_E0 ;",
                "let g = | sum | E ;"
            )),
            Some((
                "total".to_string(),
                "sum".to_string(),
                BinderScope::LetClosure
            ))
        );
    }

    #[test]
    fn plain_let_rename_is_not_let_closure() {
        // Token-equal inits belong to `Let`, never `LetClosure`.
        assert_eq!(
            renamed(&stmt("let total = __RSMPL_E0;", "let sum = E;")).map(|r| r.2),
            Some(BinderScope::Let)
        );
    }

    #[test]
    fn init_change_vetoes() {
        assert_eq!(
            binder_rename(&stmt("let total = __RSMPL_E0;", "let sum = 2;")),
            None
        );
    }

    #[test]
    fn closure_body_change_vetoes() {
        assert_eq!(
            binder_rename(&expr("|total| __RSMPL_E0", "|sum| E + 1")),
            None
        );
    }

    #[test]
    fn move_delta_vetoes() {
        assert_eq!(
            binder_rename(&expr("move |total| __RSMPL_E0", "|sum| E")),
            None
        );
    }

    #[test]
    fn destructuring_single_leaf_fires() {
        // Residue slice 2: single-leaf destructuring diffs complete
        // like plain renames.
        assert_eq!(
            renamed(&stmt("let (a , b) = __RSMPL_E0;", "let (a , c) = E;")),
            Some(("b".to_string(), "c".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn destructuring_multi_leaf_vetoes() {
        assert_eq!(
            binder_rename(&stmt("let (a , b) = __RSMPL_E0;", "let (c , d) = E;")),
            None
        );
    }

    #[test]
    fn destructuring_struct_shorthand_slice_fires() {
        assert_eq!(
            renamed(&stmt(
                "let S { f : old , g } = __RSMPL_E0;",
                "let S { f : new , g } = E;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
        assert_eq!(
            renamed(&stmt("let S { old } = __RSMPL_E0;", "let S { new } = E;")),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
        assert_eq!(
            renamed(&stmt("let [a , old] = __RSMPL_E0;", "let [a , new] = E;")),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
        assert_eq!(
            renamed(&stmt("let &old = __RSMPL_E0;", "let &new = E;")),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn destructuring_path_change_vetoes() {
        assert_eq!(
            binder_rename(&stmt("let Some (old) = __RSMPL_E0;", "let Ok (new) = E;")),
            None
        );
        assert_eq!(
            binder_rename(&stmt(
                "let (a , old) = __RSMPL_E0;",
                "let (a , new , b) = E;"
            )),
            None
        );
    }

    #[test]
    fn outer_let_change_vetoes_let_closure() {
        // Renaming the outer `let` at the same time is not a binder
        // rename at all (two binders move).
        assert_eq!(
            binder_rename(&stmt(
                "let g = | total | __RSMPL_E0 ;",
                "let h = | sum | E ;"
            )),
            None
        );
    }

    fn norm(s: &str) -> String {
        let mut map = HashMap::new();
        map.insert("__RSMPL_E0", "E");
        map.insert("__RSMPL_E", "F");
        map.insert("__RSMPL_ELLIPSIS_0", "...");
        normalize_placeholders(s, &map)
    }

    #[test]
    fn placeholder_normalization_is_token_exact() {
        // Single-hole `__RSMPL_E` is a string prefix of the `...` marker
        // `__RSMPL_ELLIPSIS_0`: longest-first, whole-token only.
        assert_eq!(norm("__RSMPL_E"), "F");
        assert_eq!(norm("__RSMPL_ELLIPSIS_0"), "...");
        assert_eq!(norm("x__RSMPL_E"), "x__RSMPL_E");
        assert_eq!(
            norm("for total in __RSMPL_E { __RSMPL_ELLIPSIS_0 }"),
            "for total in F { ... }"
        );
    }

    #[test]
    fn non_binder_template_vetoes() {
        assert_eq!(binder_rename(&expr("total + __RSMPL_E0", "sum + E")), None);
    }

    #[test]
    fn bare_annotation_drop_vetoes() {
        assert_eq!(
            binder_rename(&expr("|total : u32| __RSMPL_E0", "|sum| E")),
            None
        );
    }

    #[test]
    fn two_binder_diff_vetoes() {
        assert_eq!(
            binder_rename(&expr("|total , other| __RSMPL_E0", "|sum , changed| E")),
            None
        );
    }

    fn seq(minus_sub: &str, plus_body: &str) -> PatternKind {
        PatternKind::General {
            shape: GeneralShape::Sequence,
            minus_sub: minus_sub.to_string(),
            plus_raw: plus_body.to_string(),
            plus_body: plus_body.to_string(),
            placeholders: vec![
                ("E0".to_string(), "__RSMPL_E0".to_string()),
                ("E1".to_string(), "__RSMPL_E1".to_string()),
                ("E2".to_string(), "__RSMPL_E2".to_string()),
            ],
            literals: Vec::new(),
            minus_attrs: Vec::new(),
            plus_attrs: Vec::new(),
            had_semi: false,
        }
    }

    #[test]
    fn sequence_final_rename_fires() {
        assert_eq!(
            renamed(&seq(
                "let a = __RSMPL_E0 ;\nlet old = __RSMPL_E1 ;",
                "let a = E0 ;\nlet new = E1 ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_non_final_rename_fires() {
        // Residue slice 2: rename-once at any position completes.
        assert_eq!(
            renamed(&seq(
                "let old = __RSMPL_E0 ;\nlet b = __RSMPL_E1 ;",
                "let new = E0 ;\nlet b = E1 ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_later_old_mention_fires() {
        // Residue slice 3: a later window init naming the binder replays
        // verbatim and strands as an orphan (honest `partial`); the arm
        // still fires so post-window uses complete.
        assert_eq!(
            renamed(&seq(
                "let old = __RSMPL_E0 ;\nlet b = old ;",
                "let new = E0 ;\nlet b = old ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_earlier_old_mention_fires() {
        // A pre-binder use names an outer binder and replays verbatim
        // untouched; the arm still fires for post-window completion.
        assert_eq!(
            renamed(&seq(
                "let b = old ;\nlet old = __RSMPL_E0 ;",
                "let b = old ;\nlet new = E0 ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_init_substitution_fires() {
        // Residue slice 3: the author spells the intra-window use
        // explicitly (`let b = old` -> `let b = new`); the window
        // applies verbatim and the plus side keeps re-matching.
        assert_eq!(
            renamed(&seq(
                "let old = __RSMPL_E0 ;\nlet b = old ;",
                "let new = E0 ;\nlet b = new ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_partial_substitution_declines() {
        // Only total swaps complete: a leftover `old` in the plus init
        // is a second delta, not a substitution.
        assert_eq!(
            binder_rename(&seq(
                "let old = __RSMPL_E0 ;\nlet b = f (old) ;",
                "let new = E0 ;\nlet b = f (new , old) ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_substitution_before_rename_declines() {
        // A substitution at a pre-binder position names an outer
        // binder: unowned, decline.
        assert_eq!(
            binder_rename(&seq(
                "let b = old ;\nlet old = __RSMPL_E0 ;",
                "let b = new ;\nlet new = E0 ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_same_position_init_change_declines() {
        // The renamed statement's own init must stay token-equal (it
        // names the outer scope, never completed).
        assert_eq!(
            binder_rename(&seq(
                "let old = old ;\nlet b = __RSMPL_E1 ;",
                "let new = new ;\nlet b = E1 ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_new_mention_declines() {
        // Another window statement naming the new ident risks capture.
        assert_eq!(
            binder_rename(&seq(
                "let old = __RSMPL_E0 ;\nlet b = new ;",
                "let new = E0 ;\nlet b = new ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_pat_mention_declines() {
        // `old` in another position's type annotation is not a value
        // use the engine may rewrite: decline.
        assert_eq!(
            binder_rename(&seq(
                "let old = __RSMPL_E0 ;\nlet b : old = __RSMPL_E1 ;",
                "let new = E0 ;\nlet b : old = E1 ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_multi_delta_declines() {
        // Hetero-fusion runs never enter binder identity.
        assert_eq!(
            binder_rename(&seq(
                "let a = __RSMPL_E0 ;\nlet old = __RSMPL_E1 ;",
                "let a = E0 ;\nlet new = E2 ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_destructuring_fires() {
        // Residue slice 3: single-leaf destructuring at a sequence
        // position completes like a plain rename.
        assert_eq!(
            renamed(&seq(
                "let a = __RSMPL_E0 ;\nlet (x , old) = __RSMPL_E1 ;",
                "let a = E0 ;\nlet (x , new) = E1 ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_destructuring_multi_leaf_declines() {
        assert_eq!(
            binder_rename(&seq(
                "let a = __RSMPL_E0 ;\nlet (x , y) = __RSMPL_E1 ;",
                "let a = E0 ;\nlet (u , v) = E1 ;"
            )),
            None
        );
    }

    #[test]
    fn sequence_destructuring_with_intra_fires() {
        // Destructuring rename plus a later in-window use replayed
        // verbatim (strands as orphan); plus the spelled-substitution
        // twin completing in-window.
        assert_eq!(
            renamed(&seq(
                "let (x , old) = __RSMPL_E0 ;\nlet b = old ;",
                "let (x , new) = E0 ;\nlet b = old ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
        assert_eq!(
            renamed(&seq(
                "let (x , old) = __RSMPL_E0 ;\nlet b = old ;",
                "let (x , new) = E0 ;\nlet b = new ;"
            )),
            Some(("old".to_string(), "new".to_string(), BinderScope::Let))
        );
    }

    #[test]
    fn sequence_positions_name_idx_and_substitutions() {
        // The orphan cover maps these onto target windows.
        let (idx, subs, old, new) = sequence_binder_positions(&seq(
            "let old = __RSMPL_E0 ;\nlet b = old ;\nlet c = old ;",
            "let new = E0 ;\nlet b = new ;\nlet c = old ;",
        ))
        .unwrap();
        assert_eq!((idx, old.as_str(), new.as_str()), (0, "old", "new"));
        assert_eq!(subs, vec![1]);
    }

    #[test]
    fn sequence_init_change_declines() {
        assert_eq!(
            binder_rename(&seq(
                "let a = __RSMPL_E0 ;\nlet old = __RSMPL_E1 ;",
                "let a = E0 ;\nlet new = 2 ;"
            )),
            None
        );
    }
}
