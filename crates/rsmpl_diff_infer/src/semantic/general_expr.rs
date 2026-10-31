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
fn parse_line_expr(line: &str) -> Option<syn::Expr> {
    let text = strip_one_terminator(line.trim())?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    syn::parse_str::<syn::Expr>(text).ok()
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
    let mut minus_exprs = Vec::with_capacity(minuses.len());
    let mut plus_exprs = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let me = parse_line_expr(m)?;
        let pe = parse_line_expr(p)?;
        // Macro/unsupported veto (T448 precedent): the engine never
        // binds inside macro bodies, so a template inlining macro text
        // cannot re-match.
        for e in [&me, &pe] {
            let probe = shape::probe_expr(e);
            if probe.macros > 0 || probe.unsupported > 0 {
                return None;
            }
        }
        minus_exprs.push(me);
        plus_exprs.push(pe);
    }
    let (minus_tpl, plus_tpl) = antiunify_general_exprs(&minus_exprs, &plus_exprs)?;
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
    use super::antiunify_general_exprs;

    fn exprs(texts: &[&str]) -> Vec<syn::Expr> {
        texts
            .iter()
            .map(|t| syn::parse_str::<syn::Expr>(t).expect("test expr must parse"))
            .collect()
    }

    fn anti(minuses: &[&str], pluses: &[&str]) -> Option<(String, String)> {
        antiunify_general_exprs(&exprs(minuses), &exprs(pluses))
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
}
