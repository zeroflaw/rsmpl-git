//! General-expression raising (WI-6): bare-expression anti-unification.
//!
//! A balanced multi-site class (N ≥ 2) whose every changed line parses
//! whole-line as one `syn::Expr` of the same variant, anti-unified to a
//! single template pair with at most one `E{i}` hole. Hole positions are
//! subterms token-equal within each pair but hetero across sites
//! (ordered value correspondence, SPEC 15.7); every other position is a
//! literal anchor token-equal across all sites on its side; at least one
//! anchor carries the delta. Whole-line `Call`/`MethodCall` shapes stay
//! with the tighter call/method detectors (disjoint by construction);
//! macros veto (the engine never matches macro interiors, so a template
//! inlining macro text cannot re-match); all-identical line classes stay
//! with verbatim (T422). Totality is the real-pair engine probe
//! (`template_pair_holds`): the emitted pair must match exactly N
//! subterms in the pre-image — an untouched twin vetoes to ordinary.
//!
//! New module (D5: `antiunify.rs` and `generalized.rs` both sit near the
//! file-size gate; expression generalization is its own family).

use super::shape;
use super::{strip_one_terminator, unique_rule};
use quote::ToTokens;

/// Token-normalized text for agreement comparison.
fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

/// One anti-unified child position: either a literal anchor (uniform
/// across sites on each side) or a hole (hetero across sites, per-site
/// minus == plus).
enum Child {
    /// Literal anchor: first-site spelling per side (equal = stable,
    /// differ = the delta this template carries).
    Anchor(String, String),
    /// Preserved hole: `E{k}` binds per-site-equal text on both sides.
    Hole(usize),
}

/// Classify one expression-child column across N sites. `minus`/`plus`
/// hold the per-site token text. A uniform-both-sides column is an
/// anchor (stable or delta); a hetero column whose per-site values agree
/// across sides is a hole; a hetero column with within-pair differences
/// is two deltas, never one template.
fn classify_expr_child(minus: &[String], plus: &[String], hole: usize) -> Option<Child> {
    let minus_uniform = minus.iter().all(|v| *v == minus[0]);
    let plus_uniform = plus.iter().all(|v| *v == plus[0]);
    if minus_uniform && plus_uniform {
        return Some(Child::Anchor(minus[0].clone(), plus[0].clone()));
    }
    if minus == plus {
        return Some(Child::Hole(hole));
    }
    None
}

/// Classify one token column (operator, member, type, mutability):
/// uniform per side, or veto. Token columns never hole (a varying
/// operator is not a template position).
fn classify_tok_child(minus: &[String], plus: &[String]) -> Option<(String, String)> {
    if !minus.iter().all(|v| *v == minus[0]) || !plus.iter().all(|v| *v == plus[0]) {
        return None;
    }
    Some((minus[0].clone(), plus[0].clone()))
}

/// Variant key for the same-shape gate. Binary ops are folded into the
/// key per side only as "binary" (an op delta across sides is the
/// template's anchor, not a shape change); every other variant is its
/// own key.
fn variant(expr: &syn::Expr) -> Option<&'static str> {
    match expr {
        syn::Expr::Binary(_) => Some("binary"),
        syn::Expr::Unary(_) => Some("unary"),
        syn::Expr::Cast(_) => Some("cast"),
        syn::Expr::Field(_) => Some("field"),
        syn::Expr::Index(_) => Some("index"),
        syn::Expr::Reference(_) => Some("reference"),
        syn::Expr::Await(_) => Some("await"),
        _ => None,
    }
}

/// Anti-unify N ≥ 2 `(minus, plus)` whole-line expression pairs into one
/// template pair with at most one `E{i}` hole, or `None` (stays
/// ordinary). See the module docs for the guards.
pub(crate) fn antiunify_general_exprs(
    minus: &[syn::Expr],
    plus: &[syn::Expr],
) -> Option<(String, String)> {
    if minus.len() != plus.len() || minus.len() < 2 {
        return None;
    }
    let key = variant(&minus[0])?;
    if minus.iter().any(|e| variant(e) != Some(key)) {
        return None;
    }
    if plus.iter().any(|e| variant(e) != Some(key)) {
        return None;
    }
    // Token columns (operators/members/types/mutability) per side and
    // expression children in order, per variant.
    let mut tok_minus: Vec<Vec<String>> = Vec::new();
    let mut tok_plus: Vec<Vec<String>> = Vec::new();
    let mut expr_minus: Vec<Vec<String>> = Vec::new();
    let mut expr_plus: Vec<Vec<String>> = Vec::new();
    // Render closures rebuild each side's template from classified
    // children; one per variant (concrete positions keep first-site
    // spelling, holes ride `E{k}`).
    match (&minus[0], &plus[0]) {
        (syn::Expr::Binary(_), syn::Expr::Binary(_)) => {
            let collect = |es: &[syn::Expr],
                           left: &mut Vec<String>,
                           right: &mut Vec<String>,
                           op: &mut Vec<String>| {
                for e in es {
                    if let syn::Expr::Binary(b) = e {
                        left.push(tok(&b.left));
                        right.push(tok(&b.right));
                        op.push(tok(&b.op));
                    }
                }
            };
            let (mut ml, mut mr, mut mo) = (vec![], vec![], vec![]);
            let (mut pl, mut pr, mut po) = (vec![], vec![], vec![]);
            collect(minus, &mut ml, &mut mr, &mut mo);
            collect(plus, &mut pl, &mut pr, &mut po);
            tok_minus.push(mo);
            tok_plus.push(po);
            expr_minus.push(ml);
            expr_minus.push(mr);
            expr_plus.push(pl);
            expr_plus.push(pr);
        }
        (syn::Expr::Unary(_), syn::Expr::Unary(_)) => {
            let mut mo = vec![];
            let mut po = vec![];
            let mut me = vec![];
            let mut pe = vec![];
            for e in minus {
                if let syn::Expr::Unary(u) = e {
                    mo.push(tok(&u.op));
                    me.push(tok(&u.expr));
                }
            }
            for e in plus {
                if let syn::Expr::Unary(u) = e {
                    po.push(tok(&u.op));
                    pe.push(tok(&u.expr));
                }
            }
            tok_minus.push(mo);
            tok_plus.push(po);
            expr_minus.push(me);
            expr_plus.push(pe);
        }
        (syn::Expr::Cast(_), syn::Expr::Cast(_)) => {
            let mut me = vec![];
            let mut pe = vec![];
            let mut mt = vec![];
            let mut pt = vec![];
            for e in minus {
                if let syn::Expr::Cast(c) = e {
                    me.push(tok(&c.expr));
                    mt.push(tok(&c.ty));
                }
            }
            for e in plus {
                if let syn::Expr::Cast(c) = e {
                    pe.push(tok(&c.expr));
                    pt.push(tok(&c.ty));
                }
            }
            tok_minus.push(mt);
            tok_plus.push(pt);
            expr_minus.push(me);
            expr_plus.push(pe);
        }
        (syn::Expr::Field(_), syn::Expr::Field(_)) => {
            let mut me = vec![];
            let mut pe = vec![];
            let mut mm = vec![];
            let mut pm = vec![];
            for e in minus {
                if let syn::Expr::Field(f) = e {
                    me.push(tok(&f.base));
                    mm.push(tok(&f.member));
                }
            }
            for e in plus {
                if let syn::Expr::Field(f) = e {
                    pe.push(tok(&f.base));
                    pm.push(tok(&f.member));
                }
            }
            tok_minus.push(mm);
            tok_plus.push(pm);
            expr_minus.push(me);
            expr_plus.push(pe);
        }
        (syn::Expr::Index(_), syn::Expr::Index(_)) => {
            let mut mb = vec![];
            let mut pb = vec![];
            let mut mi = vec![];
            let mut pi = vec![];
            for e in minus {
                if let syn::Expr::Index(i) = e {
                    mb.push(tok(&i.expr));
                    mi.push(tok(&i.index));
                }
            }
            for e in plus {
                if let syn::Expr::Index(i) = e {
                    pb.push(tok(&i.expr));
                    pi.push(tok(&i.index));
                }
            }
            expr_minus.push(mb);
            expr_minus.push(mi);
            expr_plus.push(pb);
            expr_plus.push(pi);
        }
        (syn::Expr::Reference(_), syn::Expr::Reference(_)) => {
            let mut me = vec![];
            let mut pe = vec![];
            let mut mm = vec![];
            let mut pm = vec![];
            for e in minus {
                if let syn::Expr::Reference(r) = e {
                    me.push(tok(&r.expr));
                    mm.push(if r.mutability.is_some() {
                        "mut".to_string()
                    } else {
                        String::new()
                    });
                }
            }
            for e in plus {
                if let syn::Expr::Reference(r) = e {
                    pe.push(tok(&r.expr));
                    pm.push(if r.mutability.is_some() {
                        "mut".to_string()
                    } else {
                        String::new()
                    });
                }
            }
            tok_minus.push(mm);
            tok_plus.push(pm);
            expr_minus.push(me);
            expr_plus.push(pe);
        }
        (syn::Expr::Await(_), syn::Expr::Await(_)) => {
            let mut me = vec![];
            let mut pe = vec![];
            for e in minus {
                if let syn::Expr::Await(a) = e {
                    me.push(tok(&a.base));
                }
            }
            for e in plus {
                if let syn::Expr::Await(a) = e {
                    pe.push(tok(&a.base));
                }
            }
            expr_minus.push(me);
            expr_plus.push(pe);
        }
        _ => return None,
    }
    // Classify token columns (never holes) and expression children.
    let mut toks: Vec<(String, String)> = Vec::with_capacity(tok_minus.len());
    for (m, p) in tok_minus.iter().zip(tok_plus.iter()) {
        toks.push(classify_tok_child(m, p)?);
    }
    let mut holes = 0usize;
    let mut children: Vec<Child> = Vec::with_capacity(expr_minus.len());
    for (m, p) in expr_minus.iter().zip(expr_plus.iter()) {
        match classify_expr_child(m, p, holes) {
            Some(Child::Hole(_)) => {
                holes += 1;
                if holes > 1 {
                    // v1: at most one hole (multi-hole templates are
                    // future work — stays ordinary, never guessed).
                    return None;
                }
                children.push(Child::Hole(holes - 1));
            }
            Some(c) => children.push(c),
            None => return None,
        }
    }
    // At least one anchor carries the delta: a token column differing
    // across sides, or an anchor child differing across sides.
    let tok_delta = toks.iter().any(|(m, p)| m != p);
    let child_delta = children.iter().any(|c| match c {
        Child::Anchor(m, p) => m != p,
        Child::Hole(_) => false,
    });
    if !tok_delta && !child_delta {
        return None;
    }
    // At least one literal token anchor survives on each side: the
    // template is never holes alone. Every supported variant carries a
    // token column (operator/member/type/mutability slot — the reference
    // slot counts even when empty, since `&` itself is literal frame)
    // except index/await, whose bracket/dot-await frame is literal by
    // construction.
    let has_token_frame = !toks.is_empty() || key == "index" || key == "await";
    if !has_token_frame {
        return None;
    }
    let hole = |k: usize| format!("E{k}");
    let child_tpl = |c: &Child, side: bool| match c {
        Child::Anchor(m, p) => {
            if side {
                p.clone()
            } else {
                m.clone()
            }
        }
        Child::Hole(k) => hole(*k),
    };
    let tok_tpl = |t: &(String, String), side: bool| {
        if side {
            t.1.clone()
        } else {
            t.0.clone()
        }
    };
    let (minus_tpl, plus_tpl) = match key {
        "binary" => {
            let op_m = tok_tpl(&toks[0], false);
            let op_p = tok_tpl(&toks[0], true);
            (
                format!(
                    "{} {} {}",
                    child_tpl(&children[0], false),
                    op_m,
                    child_tpl(&children[1], false)
                ),
                format!(
                    "{} {} {}",
                    child_tpl(&children[0], true),
                    op_p,
                    child_tpl(&children[1], true)
                ),
            )
        }
        "unary" => (
            format!(
                "{}{}",
                tok_tpl(&toks[0], false),
                child_tpl(&children[0], false)
            ),
            format!(
                "{}{}",
                tok_tpl(&toks[0], true),
                child_tpl(&children[0], true)
            ),
        ),
        "cast" => (
            format!(
                "{} as {}",
                child_tpl(&children[0], false),
                tok_tpl(&toks[0], false)
            ),
            format!(
                "{} as {}",
                child_tpl(&children[0], true),
                tok_tpl(&toks[0], true)
            ),
        ),
        "field" => (
            format!(
                "{}.{}",
                child_tpl(&children[0], false),
                tok_tpl(&toks[0], false)
            ),
            format!(
                "{}.{}",
                child_tpl(&children[0], true),
                tok_tpl(&toks[0], true)
            ),
        ),
        "index" => (
            format!(
                "{}[{}]",
                child_tpl(&children[0], false),
                child_tpl(&children[1], false)
            ),
            format!(
                "{}[{}]",
                child_tpl(&children[0], true),
                child_tpl(&children[1], true)
            ),
        ),
        "reference" => {
            let prefix = |t: &(String, String), side: bool| {
                let m = tok_tpl(t, side);
                if m == "mut" {
                    "&mut ".to_string()
                } else {
                    "&".to_string()
                }
            };
            (
                format!(
                    "{}{}",
                    prefix(&toks[0], false),
                    child_tpl(&children[0], false)
                ),
                format!(
                    "{}{}",
                    prefix(&toks[0], true),
                    child_tpl(&children[0], true)
                ),
            )
        }
        "await" => (
            format!("{}.await", child_tpl(&children[0], false)),
            format!("{}.await", child_tpl(&children[0], true)),
        ),
        _ => return None,
    };
    if minus_tpl == plus_tpl {
        return None;
    }
    // Defensive: both sides must re-parse as expressions (the detector's
    // totality probe re-checks, but a non-parsing template must never
    // leave this core).
    if syn::parse_str::<syn::Expr>(&minus_tpl).is_err()
        || syn::parse_str::<syn::Expr>(&plus_tpl).is_err()
    {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// Parse one changed line as a whole-line expression (one trailing
/// `;`/`,` tolerated — the terminator never reaches the pattern).
/// Single-statement one-line `fn` frames unwrap to their inner
/// expression (WI-4 `frame`); the frame itself never reaches the
/// template — `frame_holds`-style totality plus the engine probe own
/// it, exactly as the sub-line extractors do.
fn parse_line_expr(line: &str) -> Option<syn::Expr> {
    let text = strip_one_terminator(line.trim())?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if !matches!(expr, syn::Expr::Let(_)) {
            return Some(expr);
        }
    }
    super::frame::fn_frame_inner(line)
}

/// One `let`-init sub-expression (WI-5): `let PAT[: TY] = INIT;` with
/// init present and no `let-else` diverge. The pattern/type ride each
/// site's own wrapper — the bare general template rewrites the init
/// subterm in place, preserving them (closure-let precedent).
/// `pub(crate)` so the `literal_frame` veto reuses the same parser
/// (REVIEW: no mirrored duplicate).
pub(crate) fn parse_let_init(line: &str) -> Option<syn::Expr> {
    let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) else {
        return None;
    };
    let init = local.init?;
    if init.diverge.is_some() {
        return None;
    }
    Some(*init.expr)
}

/// True when both template sides are anchored (WI-5 D10): neither side
/// is a bare literal — the template states its frame. Anchorless
/// shapes never jump the literal queue.
fn is_anchored(minus_tpl: &str, plus_tpl: &str) -> bool {
    syn::parse_str::<syn::Lit>(minus_tpl.trim()).is_err()
        && syn::parse_str::<syn::Lit>(plus_tpl.trim()).is_err()
}

/// N-site (N ≥ 2) general-expression sweep: every changed line parses
/// whole-line as one expression of the same variant, anti-unified to a
/// single template pair with at most one hole (see module docs). Emits
/// one `# allow=many` hunk (tight original range or `# sweep` per the
/// D.5 64-line rule, via `emission_header`). Detector name
/// `general_expr`.
pub(crate) fn try_general_expr_sweep(
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
    // All-identical classes stay for the tighter verbatim sweep (T422).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    // Whole-line first (tightest): every pair parses whole-line.
    let mut minus_exprs = Vec::with_capacity(minuses.len());
    let mut plus_exprs = Vec::with_capacity(pluses.len());
    let mut whole_ok = true;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        match (parse_line_expr(m), parse_line_expr(p)) {
            (Some(me), Some(pe)) => {
                minus_exprs.push(me);
                plus_exprs.push(pe);
            }
            _ => {
                whole_ok = false;
                break;
            }
        }
    }
    if whole_ok {
        for e in minus_exprs.iter().chain(plus_exprs.iter()) {
            let probe = shape::probe_expr(e);
            if probe.macros > 0 || probe.unsupported > 0 {
                return None;
            }
        }
        if let Some((minus_tpl, plus_tpl)) = antiunify_general_exprs(&minus_exprs, &plus_exprs) {
            if is_anchored(&minus_tpl, &plus_tpl)
                && crate::totality::template_pair_holds(
                    path,
                    file_old,
                    &minus_tpl,
                    &plus_tpl,
                    minuses.len(),
                )
            {
                let rule = unique_rule(used_rules, "general_expr");
                let mut hunk = crate::emission_header(path, &rule, changed);
                hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
                return Some(hunk);
            }
        }
        return None;
    }
    // WI-5 `let`-init subterm fallback: whole-line declined uniformly,
    // every pair carries its delta inside a `let` init. Anti-unify the
    // init column through the same WI-6 core; emission stays the bare
    // anchored template (the engine rewrites it inside each `let`).
    let mut minus_inits = Vec::with_capacity(minuses.len());
    let mut plus_inits = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        minus_inits.push(parse_let_init(m)?);
        plus_inits.push(parse_let_init(p)?);
    }
    for e in minus_inits.iter().chain(plus_inits.iter()) {
        let probe = shape::probe_expr(e);
        if probe.macros > 0 || probe.unsupported > 0 {
            return None;
        }
    }
    let (minus_tpl, plus_tpl) = antiunify_general_exprs(&minus_inits, &plus_inits)?;
    if !is_anchored(&minus_tpl, &plus_tpl) {
        return None;
    }
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "general_expr");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use super::antiunify_general_exprs;
    use std::collections::HashMap;

    fn exprs(texts: &[&str]) -> Vec<syn::Expr> {
        texts
            .iter()
            .map(|t| syn::parse_str::<syn::Expr>(t).expect("test expr must parse"))
            .collect()
    }

    fn anti(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
        antiunify_general_exprs(&exprs(minuses), &exprs(pluses))
    }

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

    #[test]
    fn wi23_field_sweep_fires_diet_and_round_trips() {
        // WI-23: a 2-site whole-line field delta raises to one diet
        // `# allow=many` hunk (no `# expr` line) and real-applies
        // byte-identical. Before the diet fix this class stayed
        // ordinary (the template never matched without a declaration,
        // and no sweep emitted one).
        let pre = "fn m() {\n    foo.timeout;\n    bar.timeout;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    foo.timeout;\n+    foo.timeout2;\n@@ -3 +3 @@\n-    bar.timeout;\n+    bar.timeout2;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("field sweep must fire");
        validated_live(&out, &pre_map);
        assert!(!out.contains("# expr"), "diet, no declaration: {out}");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("- E0.timeout\n"), "got: {out}");
        assert!(out.contains("+ E0.timeout2\n"), "got: {out}");
        let post = pre.replace(".timeout;", ".timeout2;");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], post);
    }

    #[test]
    fn binary_hole_left_fires() {
        // The WI-6 canonical shape: hetero left preserved, uniform
        // right delta.
        let out = anti(&["1 + 1", "2 + 1"], &["1 + 2", "2 + 2"]);
        assert_eq!(
            out,
            Some(("E0 + 1".to_string(), "E0 + 2".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn binary_hole_right_fires() {
        let out = anti(&["1 + a", "1 + b"], &["2 + a", "2 + b"]);
        assert_eq!(
            out,
            Some(("1 + E0".to_string(), "2 + E0".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn binary_uniform_delta_fires() {
        // No hole: a uniform concrete delta over uniform lines still
        // compresses (hetero lines are this detector's; identical
        // *pairs* stay with verbatim, T422 — enforced by the caller).
        let out = anti(&["x + 1", "x + 1"], &["x + 2", "x + 2"]);
        assert_eq!(
            out,
            Some(("x + 1".to_string(), "x + 2".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn binary_op_delta_with_hole_fires() {
        // The operator is the anchor carrying the delta.
        let out = anti(&["a + 1", "b + 1"], &["a - 1", "b - 1"]);
        assert_eq!(
            out,
            Some(("E0 + 1".to_string(), "E0 - 1".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn binary_two_holes_veto() {
        // v1 owns at most one hole — a second hetero-preserved child
        // stays ordinary, never guessed.
        let out = anti(&["a + b", "c + d"], &["a - b", "c - d"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn binary_hetero_delta_vetoes() {
        // Per-site deltas differ (1→2 vs 1→3): not one template.
        let out = anti(&["a + 1", "b + 1"], &["a + 2", "b + 3"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn binary_mixed_variant_vetoes() {
        let out = anti(&["a + b", "c + d"], &["a + b", "c - d"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn cross_variant_vetoes() {
        let out = anti(&["a + b", "c + d"], &["-a", "-c"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn unary_fires() {
        let out = anti(&["-a", "-b"], &["!a", "!b"]);
        assert_eq!(
            out,
            Some(("-E0".to_string(), "!E0".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn field_hole_base_fires() {
        let out = anti(&["a.timeout", "b.timeout"], &["a.timeout2", "b.timeout2"]);
        assert_eq!(
            out,
            Some(("E0.timeout".to_string(), "E0.timeout2".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn index_hole_base_fires() {
        let out = anti(&["a[0]", "b[0]"], &["a[1]", "b[1]"]);
        assert_eq!(
            out,
            Some(("E0[0]".to_string(), "E0[1]".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn cast_hole_expr_fires() {
        let out = anti(&["a as u32", "b as u32"], &["a as u64", "b as u64"]);
        assert_eq!(
            out,
            Some(("E0 as u32".to_string(), "E0 as u64".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn reference_delta_fires() {
        let out = anti(&["&a", "&b"], &["&mut a", "&mut b"]);
        assert_eq!(
            out,
            Some(("&E0".to_string(), "&mut E0".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn await_hole_base_fires() {
        let out = anti(&["a.await", "b.await"], &["c.await", "d.await"]);
        // Hetero bases differing within the pair: two deltas, veto.
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn await_concrete_delta_fires() {
        let out = anti(&["foo.await", "foo.await"], &["bar.await", "bar.await"]);
        assert_eq!(
            out,
            Some(("foo.await".to_string(), "bar.await".to_string())),
            "got: {out:?}"
        );
    }

    #[test]
    fn no_delta_vetoes() {
        let out = anti(&["a + 1", "b + 1"], &["a + 1", "b + 1"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn single_vetoes() {
        let out = anti(&["a + 1"], &["a + 2"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn call_whole_line_declines() {
        // Bare calls belong to the tighter call detectors.
        let out = anti(&["f(a)", "f(b)"], &["f(c)", "f(d)"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn method_whole_line_declines() {
        let out = anti(&["a.foo()", "b.foo()"], &["a.foo()", "b.foo()"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn macro_whole_line_declines() {
        let out = anti(&["m!(a)", "m!(b)"], &["m!(a)", "m!(b)"]);
        assert_eq!(out, None, "got: {out:?}");
    }

    #[test]
    fn template_sides_reparse() {
        // Every firing template must re-parse (defensive gate pin).
        let out = anti(&["a + 1", "b + 1"], &["a + 2", "b + 2"]).expect("must fire");
        assert!(syn::parse_str::<syn::Expr>(&out.0).is_ok());
        assert!(syn::parse_str::<syn::Expr>(&out.1).is_ok());
    }

    #[test]
    fn wi5_let_init_binary_fires_and_round_trips() {
        // WI-5 F5 probe: three `let`-init `x + 1` → `x + 2` sites raise
        // as the anchored `- E0 + 1` / `+ E0 + 2` template, never bare
        // `- 1` / `+ 2`, and real-apply byte-identical.
        let pre = "fn m() {\n    let a = x + 1;\n    let b = y + 1;\n    let c = z + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n-    let a = x + 1;\n-    let b = y + 1;\n-    let c = z + 1;\n+    let a = x + 2;\n+    let b = y + 2;\n+    let c = z + 2;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("let-init anchored sweep must fire");
        validated_live(&out, &pre_map);
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
        assert!(
            !out.contains("- 1\n"),
            "bare literal must not fire, got: {out}"
        );
        let post = pre.replace("+ 1", "+ 2");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], post);
    }

    #[test]
    fn wi5_whole_line_still_preferred() {
        // Whole-line expressions keep their winner: no `let` wrapper,
        // same anchored shape via the whole-line path.
        let pre = "fn m() {\n    x + 1;\n    y + 1;\n    z + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n-    x + 1;\n-    y + 1;\n-    z + 1;\n+    x + 2;\n+    y + 2;\n+    z + 2;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("whole-line anchored sweep must fire");
        validated_live(&out, &pre_map);
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
    }

    #[test]
    fn wi5_anchorless_bare_ident_stays_ordinary() {
        // Anchorless single-hole shapes never jump the literal queue:
        // bare-ident inits have no variant, so neither path claims.
        let pre = "fn m() {\n    let a = x;\n    let b = y;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let a = x;\n-    let b = y;\n+    let a = p;\n+    let b = q;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none(),
            "anchorless inits must stay ordinary"
        );
    }

    #[test]
    fn wi5_macro_init_vetoes() {
        // Macro inits never raise: the engine cannot match inside them.
        let pre = "fn m() {\n    let a = m!(x + 1);\n    let b = m!(y + 1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let a = m!(x + 1);\n-    let b = m!(y + 1);\n+    let a = m!(x + 2);\n+    let b = m!(y + 2);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let mut used = Vec::new();
        let minuses = ["    let a = m!(x + 1);", "    let b = m!(y + 1);"];
        let pluses = ["    let a = m!(x + 2);", "    let b = m!(y + 2);"];
        assert!(
            super::try_general_expr_sweep(&minuses, &pluses, "f.rs", Some(pre), &[2, 3], &mut used)
                .is_none(),
            "macro init must veto"
        );
        let _ = (raw, pre_map);
    }

    #[test]
    fn wi5_mixed_let_and_non_let_vetoes_general() {
        // Non-`let` statements stay bare in this slice: a mixed class
        // declines the `let`-init fallback (the `?` on `parse_let_init`).
        let pre = "fn m() {\n    let a = x + 1;\n    y + 1;\n}\n";
        let mut used = Vec::new();
        let minuses = ["    let a = x + 1;", "    y + 1;"];
        let pluses = ["    let a = x + 2;", "    y + 2;"];
        assert!(
            super::try_general_expr_sweep(&minuses, &pluses, "f.rs", Some(pre), &[2, 3], &mut used)
                .is_none(),
            "mixed let/non-let must veto general"
        );
    }

    #[test]
    fn wi5_single_pair_stays_ordinary() {
        // N ≥ 2 gate: one `let`-init pair never sweeps.
        let pre = "fn m() {\n    let a = x + 1;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let a = x + 1;\n+    let a = x + 2;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none(),
            "single pair must stay ordinary (WI-7 owns N=1)"
        );
    }
}
