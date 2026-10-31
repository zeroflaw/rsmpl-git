//! WI-3 structural pattern templates for raising.
//!
//! `gen_pat_template` anti-unifies N ≥ 2 per-site `syn::Pat` values into
//! one template string: same-variant composites (tuple-struct, tuple,
//! struct, paren, reference, slice) recurse with literal anchors
//! preserved, varying leaves share holes by identical per-site value
//! vectors (`P0`, `P1`, … pat-namespace, SPEC 15.7), and anything that
//! cannot stay anchored vetoes (`None` — the caller falls back to the
//! whole-pattern `P0`, whose totality probe still gates).
//!
//! Guards (IMPROVE.md WI-3): same `syn` variant at every generalized
//! node; tuple-struct/struct paths token-equal (the literal anchor);
//! tuple/slice arity-equal; struct member sets equal with `..`-rest
//! agreement; reference mutability strict; generalization depth ≤ 2
//! (deeper variance vetoes); ≥ 1 literal anchor must survive (the
//! variant path, or a concrete leaf for pathless shapes); every hole
//! repeated across sites binds identical per-site values (shared names
//! by value-vector); output re-parses as `syn::Pat`.

use quote::ToTokens;

/// Original site spelling of a node parsed from `line` (single-line),
/// or `fallback` when spans are unusable. `tok` normalization inserts
/// spaces around `<`/`>`/`:` (`Option<i32>` → `Option < i32 >`), which
/// real-applies whitespace-dirty; slicing the first site keeps uniform
/// concrete positions byte-identical (WI-3 D1).
pub(crate) fn site_spelling<T>(line: &str, node: &T, fallback: &str) -> String
where
    T: ToTokens + syn::spanned::Spanned,
{
    let trimmed = line.trim();
    let span = node.span();
    if span.start().line == 1 && span.end().line == 1 {
        let s = rsmpl_pattern::char_col_to_byte(trimmed, span.start().column);
        let e = rsmpl_pattern::char_col_to_byte(trimmed, span.end().column);
        if s < e && e <= trimmed.len() {
            if let Some(t) = trimmed.get(s..e).filter(|t| !t.is_empty()) {
                return t.to_string();
            }
        }
    }
    fallback.to_string()
}

/// Anti-unify N ≥ 2 patterns into one template, or `None` when the
/// class is not one uniform structural delta.
pub(crate) fn gen_pat_template(pats: &[syn::Pat]) -> Option<String> {
    if pats.len() < 2 {
        return None;
    }
    let mut holes: Vec<Vec<String>> = Vec::new();
    let (tpl, anchored) = gen_node(pats, 0, &mut holes)?;
    if !anchored {
        return None;
    }
    // Defensive: the template must re-parse as a pattern (the
    // detector's probe re-checks, but a non-parsing template must
    // never leave this core).
    if syn::parse_str::<syn::Stmt>(&format!("let {tpl} = _;")).is_err() {
        return None;
    }
    Some(tpl)
}

/// Hole index for a varying leaf, shared by identical per-site value
/// vectors (SPEC 15.7 repeat equality).
fn hole_for(values: Vec<String>, holes: &mut Vec<Vec<String>>) -> usize {
    if let Some(i) = holes.iter().position(|h| *h == values) {
        return i;
    }
    holes.push(values);
    holes.len() - 1
}

fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

/// Recursive anti-unification. Returns the template fragment plus
/// whether a literal anchor survives inside it.
fn gen_node(
    pats: &[syn::Pat],
    depth: usize,
    holes: &mut Vec<Vec<String>>,
) -> Option<(String, bool)> {
    let first = tok(&pats[0]);
    if pats.iter().all(|p| tok(p) == first) {
        return Some((first, true));
    }
    if depth > 2 {
        return None;
    }
    match &pats[0] {
        syn::Pat::TupleStruct(_) => {
            let paths: Vec<String> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::TupleStruct(ps) => Some(tok(&ps.path)),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if paths.iter().any(|p| *p != paths[0]) {
                return None;
            }
            let arities: Vec<usize> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::TupleStruct(ps) => Some(ps.elems.len()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if arities.iter().any(|a| *a != arities[0]) {
                return None;
            }
            let mut args = Vec::with_capacity(arities[0]);
            for i in 0..arities[0] {
                let nth: Vec<syn::Pat> = pats
                    .iter()
                    .map(|p| match p {
                        syn::Pat::TupleStruct(ps) => Some(ps.elems[i].clone()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let (t, _) = gen_node(&nth, depth + 1, holes)?;
                args.push(t);
            }
            // The equal variant path is the literal anchor.
            Some((format!("{}({})", paths[0], args.join(", ")), true))
        }
        syn::Pat::Tuple(_) => {
            let arities: Vec<usize> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Tuple(pt) => Some(pt.elems.len()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if arities.iter().any(|a| *a != arities[0]) {
                return None;
            }
            let mut args = Vec::with_capacity(arities[0]);
            let mut anchored = false;
            for i in 0..arities[0] {
                let nth: Vec<syn::Pat> = pats
                    .iter()
                    .map(|p| match p {
                        syn::Pat::Tuple(pt) => Some(pt.elems[i].clone()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let (t, a) = gen_node(&nth, depth + 1, holes)?;
                anchored |= a;
                args.push(t);
            }
            if !anchored {
                return None;
            }
            Some((format!("({})", args.join(", ")), true))
        }
        syn::Pat::Struct(ps0) => {
            let paths: Vec<String> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Struct(ps) => Some(tok(&ps.path)),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if paths.iter().any(|p| *p != paths[0]) {
                return None;
            }
            let rests: Vec<bool> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Struct(ps) => Some(ps.rest.is_some()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if rests.iter().any(|r| *r != rests[0]) {
                return None;
            }
            // Strict member-set equality (shorthand variance vetoes:
            // a hole must never land in member position — the engine
            // matches members literally).
            let members0: Vec<String> = ps0.fields.iter().map(|f| tok(&f.member)).collect();
            for p in &pats[1..] {
                let syn::Pat::Struct(ps) = p else {
                    return None;
                };
                let members: Vec<String> = ps.fields.iter().map(|f| tok(&f.member)).collect();
                if members.len() != members0.len() || members.iter().any(|m| !members0.contains(m))
                {
                    return None;
                }
                for f in &ps.fields {
                    let g = ps0
                        .fields
                        .iter()
                        .find(|g| tok(&g.member) == tok(&f.member))?;
                    if f.colon_token.is_some() != g.colon_token.is_some() {
                        return None;
                    }
                }
            }
            let mut fields = Vec::with_capacity(members0.len());
            for f0 in &ps0.fields {
                let nth: Vec<syn::Pat> = pats
                    .iter()
                    .map(|p| match p {
                        syn::Pat::Struct(ps) => ps
                            .fields
                            .iter()
                            .find(|g| tok(&g.member) == tok(&f0.member))
                            .map(|g| (*g.pat).clone()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let (t, _) = gen_node(&nth, depth + 1, holes)?;
                let member = tok(&f0.member);
                if f0.colon_token.is_some() {
                    fields.push(format!("{member}: {t}"));
                } else {
                    // Shorthand: members are equal across sites, so a
                    // varying inner means the shorthand split differs —
                    // already vetoed by colon agreement above unless the
                    // inner is token-equal (concrete replay).
                    if t != tok(&f0.pat) {
                        return None;
                    }
                    fields.push(member);
                }
            }
            let mut out = format!("{} {{ {} }}", paths[0], fields.join(", "));
            if rests[0] {
                out = format!("{} {{ {}, .. }}", paths[0], fields.join(", "));
            }
            // The equal struct path is the literal anchor.
            Some((out, true))
        }
        syn::Pat::Paren(_) => {
            let inners: Vec<syn::Pat> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Paren(pp) => Some((*pp.pat).clone()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let (t, a) = gen_node(&inners, depth + 1, holes)?;
            if !a {
                return None;
            }
            Some((format!("({t})"), true))
        }
        syn::Pat::Reference(_) => {
            let muts: Vec<bool> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Reference(pr) => Some(pr.mutability.is_some()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if muts.iter().any(|m| *m != muts[0]) {
                return None;
            }
            let inners: Vec<syn::Pat> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Reference(pr) => Some((*pr.pat).clone()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            let (t, a) = gen_node(&inners, depth + 1, holes)?;
            if !a {
                return None;
            }
            let prefix = if muts[0] { "&mut " } else { "&" };
            Some((format!("{prefix}{t}"), true))
        }
        syn::Pat::Slice(_) => {
            let arities: Vec<usize> = pats
                .iter()
                .map(|p| match p {
                    syn::Pat::Slice(ps) => Some(ps.elems.len()),
                    _ => None,
                })
                .collect::<Option<_>>()?;
            if arities.iter().any(|a| *a != arities[0]) {
                return None;
            }
            // Arity 0 with variance is impossible (all-equal handled
            // above); an empty-vs-empty pair never reaches here.
            if arities[0] == 0 {
                return None;
            }
            let mut elems = Vec::with_capacity(arities[0]);
            let mut anchored = false;
            for i in 0..arities[0] {
                let nth: Vec<syn::Pat> = pats
                    .iter()
                    .map(|p| match p {
                        syn::Pat::Slice(ps) => Some(ps.elems[i].clone()),
                        _ => None,
                    })
                    .collect::<Option<_>>()?;
                let (t, a) = gen_node(&nth, depth + 1, holes)?;
                anchored |= a;
                elems.push(t);
            }
            if !anchored {
                return None;
            }
            Some((format!("[{}]", elems.join(", ")), true))
        }
        // Varying leaves (idents, literals, paths, `Or`, …) share one
        // hole by identical per-site values (SPEC 15.7). The hole
        // carries no anchor — the caller (or an anchored sibling)
        // must supply one.
        _ => {
            let values: Vec<String> = pats.iter().map(tok).collect();
            let i = hole_for(values, holes);
            Some((format!("P{i}"), false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::gen_pat_template;

    fn pats(texts: &[&str]) -> Vec<syn::Pat> {
        texts
            .iter()
            .map(|t| {
                let syn::Stmt::Local(local) =
                    syn::parse_str::<syn::Stmt>(&format!("let {t} = _;")).unwrap()
                else {
                    panic!("not a let: {t}");
                };
                local.pat
            })
            .collect()
    }

    #[test]
    fn tuple_struct_varying_leaf_shares_hole() {
        let p = pats(&["Some(a)", "Some(b)"]);
        assert_eq!(gen_pat_template(&p).as_deref(), Some("Some(P0)"));
    }

    #[test]
    fn tuple_struct_uniform_stays_concrete() {
        let p = pats(&["Some(x)", "Some(x)"]);
        // All-equal never reaches gen in the caller, but the core
        // returns the concrete form directly (token-stream spacing).
        assert_eq!(gen_pat_template(&p).as_deref(), Some("Some (x)"));
    }

    #[test]
    fn tuple_needs_one_anchor() {
        let p = pats(&["(a, b)", "(c, d)"]);
        assert!(gen_pat_template(&p).is_none());
        let q = pats(&["(a, b)", "(c, b)"]);
        assert_eq!(gen_pat_template(&q).as_deref(), Some("(P0, b)"));
    }

    #[test]
    fn struct_matches_by_member_name() {
        let p = pats(&["S { f : a }", "S { f : b }"]);
        assert_eq!(gen_pat_template(&p).as_deref(), Some("S { f: P0 }"));
    }

    #[test]
    fn struct_member_set_mismatch_vetoes() {
        let p = pats(&["S { f : a }", "S { f : a, g : b }"]);
        assert!(gen_pat_template(&p).is_none());
    }

    #[test]
    fn struct_rest_agreement_vetoes() {
        let p = pats(&["S { f : a, .. }", "S { f : b }"]);
        assert!(gen_pat_template(&p).is_none());
    }

    #[test]
    fn reference_mutability_vetoes() {
        let p = pats(&["& a", "& mut b"]);
        assert!(gen_pat_template(&p).is_none());
    }

    #[test]
    fn reference_fires_with_concrete_sibling() {
        let p = pats(&["& (a, b)", "& (c, b)"]);
        assert_eq!(gen_pat_template(&p).as_deref(), Some("&(P0, b)"));
    }

    #[test]
    fn different_variants_fall_back_to_none_at_root() {
        // Root family mismatch holes the whole pattern (no anchor),
        // so gen declines and the caller keeps whole-`P0`.
        let p = pats(&["Some(a)", "None"]);
        assert!(gen_pat_template(&p).is_none());
    }

    #[test]
    fn nested_variant_mismatch_holes_the_leaf() {
        // Outer anchor survives while the inner position varies by
        // variant: `Some(a)` vs `Some((b, c))` keeps `Some(P0)`.
        let p = pats(&["Some(a)", "Some((b, c))"]);
        assert_eq!(gen_pat_template(&p).as_deref(), Some("Some(P0)"));
    }

    #[test]
    fn or_with_holes_vetoes() {
        // Plain `let` takes no top-level `|` (PatNoTopAlt), so the
        // ambiguous shape wears parens — exactly as authored patterns do.
        let p = pats(&["(Some(a) | None)", "(Some(b) | None)"]);
        // `Or` is a varying leaf with no anchor.
        assert!(gen_pat_template(&p).is_none());
    }

    #[test]
    fn deep_variance_vetoes() {
        // Variance at depth 3 (`a` vs `x` three levels down) vetoes.
        let p = pats(&["Some((a, (b, c)))", "Some((a, (b, x)))"]);
        assert!(gen_pat_template(&p).is_none());
    }

    #[test]
    fn shared_values_share_holes() {
        // Identical per-site value vectors reuse the hole name.
        let p = pats(&["(a, a, z)", "(b, b, z)"]);
        assert_eq!(gen_pat_template(&p).as_deref(), Some("(P0, P0, z)"));
    }

    #[test]
    fn single_site_declines() {
        let p = pats(&["Some(a)"]);
        assert!(gen_pat_template(&p).is_none());
    }
}
