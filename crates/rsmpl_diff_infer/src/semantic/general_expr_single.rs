//! N=1 general-expression single core: single-pair anti-unification plus its T361 probe.
//! split off verbatim under the WI-19 1900-line warning gate (D5): pure move,
//! no behavior change. Tests stay inline in the origin file per WI-22.
use super::general_expr::{is_anchored, parse_let_init, parse_line_expr, tok, variant};
use super::shape;
use super::unique_rule;

/// Single-pair anti-unification (WI-7 slice-4): one minus/plus `Expr`
/// pair of the same variant yields a template pair with exactly one
/// hole, or `None` (stays ordinary). Within-pair rule: an
/// expression-child position token-equal across minus/plus becomes
/// hole `E0`; a differing child becomes a concrete anchor delta.
/// Token columns ride as anchors (stable or delta, as in multis --
/// the operator/member/type token is the anchor beyond the hole).
/// Exactly one hole is required: binary/index need one equal and one
/// differing child; single-child variants need an equal child plus a
/// token delta. Zero-hole (pure literal) and two-delta shapes veto,
/// as do identical pairs. `Call`/`MethodCall` never arrive here
/// (`variant()` returns `None` for them -- the tighter call/method
/// detectors own those shapes); `Await` singles always veto (no token
/// delta exists, so a holed shape is identical and a delta shape is
/// hole-free).
/// `pub(crate)` so the WI-7 slice-19 sub-line single anti-unifies its
/// one extracted site through the same single-pair core (no mirrored
/// duplicate).
pub(crate) fn antiunify_general_expr_single(
    minus: &syn::Expr,
    plus: &syn::Expr,
) -> Option<(String, String)> {
    let key = variant(minus)?;
    if variant(plus) != Some(key) {
        return None;
    }
    // One hole renders `E0` on both sides; a delta renders per-side
    // concrete text.
    let hole = || ("E0".to_string(), "E0".to_string());
    let (minus_tpl, plus_tpl) = match (minus, plus) {
        (syn::Expr::Binary(m), syn::Expr::Binary(p)) => {
            let (lm, lp) = (tok(&m.left), tok(&p.left));
            let (rm, rp) = (tok(&m.right), tok(&p.right));
            let (om, op) = (tok(&m.op), tok(&p.op));
            // Exactly one hole: one child equal, the other differing
            // (a pure operator delta with stable operands is a
            // two-hole shape -- future work, never guessed).
            let left_hole = lm == lp;
            let right_hole = rm == rp;
            if left_hole == right_hole {
                return None;
            }
            let (l_m, l_p) = if left_hole { hole() } else { (lm, lp) };
            let (r_m, r_p) = if right_hole { hole() } else { (rm, rp) };
            (format!("{l_m} {om} {r_m}"), format!("{l_p} {op} {r_p}"))
        }
        (syn::Expr::Unary(m), syn::Expr::Unary(p)) => {
            let (om, op) = (tok(&m.op), tok(&p.op));
            let (em, ep) = (tok(&m.expr), tok(&p.expr));
            // Holed operand plus an operator delta; an operand delta
            // without a hole is a literal (veto).
            if em != ep || om == op {
                return None;
            }
            (format!("{om}E0"), format!("{op}E0"))
        }
        (syn::Expr::Cast(m), syn::Expr::Cast(p)) => {
            let (em, ep) = (tok(&m.expr), tok(&p.expr));
            let (tm, tp) = (tok(&m.ty), tok(&p.ty));
            if em != ep || tm == tp {
                return None;
            }
            (format!("E0 as {tm}"), format!("E0 as {tp}"))
        }
        (syn::Expr::Field(m), syn::Expr::Field(p)) => {
            let (bm, bp) = (tok(&m.base), tok(&p.base));
            let (mm, mp) = (tok(&m.member), tok(&p.member));
            if bm != bp || mm == mp {
                return None;
            }
            (format!("E0.{mm}"), format!("E0.{mp}"))
        }
        (syn::Expr::Index(m), syn::Expr::Index(p)) => {
            let (bm, bp) = (tok(&m.expr), tok(&p.expr));
            let (im, ip) = (tok(&m.index), tok(&p.index));
            // Exactly one hole, same as binary (the bracket frame is
            // literal by construction).
            let base_hole = bm == bp;
            let idx_hole = im == ip;
            if base_hole == idx_hole {
                return None;
            }
            let (b_m, b_p) = if base_hole { hole() } else { (bm, bp) };
            let (i_m, i_p) = if idx_hole { hole() } else { (im, ip) };
            (format!("{b_m}[{i_m}]"), format!("{b_p}[{i_p}]"))
        }
        (syn::Expr::Reference(m), syn::Expr::Reference(p)) => {
            let (em, ep) = (tok(&m.expr), tok(&p.expr));
            let (mm, mp) = (m.mutability.is_some(), p.mutability.is_some());
            // Holed pointee plus a mutability delta (`&` vs `&mut`).
            if em != ep || mm == mp {
                return None;
            }
            let minus_tpl = if mm { "&mut E0" } else { "&E0" }.to_string();
            let plus_tpl = if mp { "&mut E0" } else { "&E0" }.to_string();
            (minus_tpl, plus_tpl)
        }
        (syn::Expr::Await(_), syn::Expr::Await(_)) => return None,
        _ => return None,
    };
    if minus_tpl == plus_tpl {
        return None;
    }
    // Defensive: both sides must re-parse as expressions (the
    // totality probe re-checks, but a non-parsing template must never
    // leave this core).
    if syn::parse_str::<syn::Expr>(&minus_tpl).is_err()
        || syn::parse_str::<syn::Expr>(&plus_tpl).is_err()
    {
        return None;
    }
    Some((minus_tpl, plus_tpl))
}

/// WI-7 slice-4: whole-line general-expression single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses whole-line as one
/// expression of the same variant (frame-unwrapped, same entry as the
/// multi sweep -- `let`-init lines decline here and stay ordinary for
/// the `let` slice), the single-pair anti-unification yields exactly
/// one hole plus an anchor delta, a pre-image is present (`None`
/// vetoes -- uniqueness needs the whole file, D3), and
/// `template_pair_holds` proves exactly one site file-wide with zero
/// opaque hits (conditions (a)+(b) of `PrecisionUnique`; (c) is
/// structural: the template carries the operator/member/type token
/// beyond the `E0` hole). Emission is the WI-19b allow-one spelling
/// (tight range, directive-free, default `allow=one`). Detector name
/// `general_expr` (same family, provenance unchanged).
/// Plus-side re-verification is the engine's apply itself
/// (validated_live in tests, T361).
/// T361 byte-compare probe (attr_sweep `probe_reproduces` precedent
/// for N=1): the ranged allow-one emission built from `minus_tpl` /
/// `plus_tpl` must real-apply to the pre-image byte-identical to the
/// native single-line swap (`minus_line` -> `plus_line` at `line_no`).
/// Count equality alone (`template_pair_holds`) cannot see TokenStream
/// spacing drift (`use_it(a)` renders `use_it (a)`): the rewritten
/// region would carry template spacing instead of source spelling
/// (116 miniature). Any failure -- line mismatch, unparseable probe,
/// match-count mismatch, or byte mismatch -- vetoes the single. The
/// probe mirrors the real emission shape (tight original range,
/// directive-free allow-one) under a scratch id, so reservation via
/// `unique_rule` happens only for emissions that reproduce.
/// `pub(crate)` so the WI-7 slice-6 arg-template single reuses the same
/// byte gate (one probe, two callers — no mirrored duplicate).
pub(crate) fn single_reproduces(
    path: &str,
    pre: &str,
    line_no: usize,
    minus_line: &str,
    plus_line: &str,
    minus_tpl: &str,
    plus_tpl: &str,
) -> bool {
    if line_no == 0 {
        return false;
    }
    let mut lines: Vec<&str> = pre.split('\n').collect();
    let Some(cur) = lines.get(line_no - 1).copied() else {
        return false;
    };
    if cur != minus_line {
        return false;
    }
    lines[line_no - 1] = plus_line;
    let expected = lines.join("\n");
    let probe_section = format!(
        "{}\n@@ -{line_no},1 @@ rule=probe\n- {minus_tpl}\n+ {plus_tpl}\n",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe_section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == 1
        && files.get(path).is_some_and(|got| *got == expected)
}

/// Shared tail of the single-entry paths (whole-line, `let`-init, and
/// WI-7 slice-19 sub-line): anchor gate, file-wide uniqueness at n=1,
/// byte-compare pre-reservation, and the allow-one emission. Returns
/// `None` on any veto; reserves the rule name only for emissions that
/// reproduce.
/// `pub(crate)` so the slice-19 sub-line single reuses the same tail
/// (no mirrored emission).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_general_expr_single(
    path: &str,
    pre: &str,
    minus_line: &str,
    plus_line: &str,
    changed: &[usize],
    minus_tpl: &str,
    plus_tpl: &str,
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if !is_anchored(minus_tpl, plus_tpl) {
        return super::prov::Counted::Miss;
    }
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), minus_tpl, plus_tpl),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return super::prov::Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => {
            return super::prov::Counted::Opaque;
        }
        crate::totality_counted::FamilyVerdict::Unproven => {
            return super::prov::Counted::Miss;
        }
    }
    // Byte-compare pre-reservation: reserve the rule name only for
    // emissions that reproduce the native swap exactly.
    if !single_reproduces(
        path, pre, changed[0], minus_line, plus_line, minus_tpl, plus_tpl,
    ) {
        return super::prov::Counted::Miss;
    }
    let rule = unique_rule(used_rules, "general_expr");
    // Allow-one: directive-free ranged hunk (slices-1-3 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    super::prov::Counted::Fire(hunk)
}

pub(crate) fn try_general_expr_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return super::prov::Counted::Miss;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let Some(pre) = file_old else {
        return super::prov::Counted::Miss;
    };
    // Whole-line entry first (tightest, incl. WI-4 frame unwrap).
    if let (Some(minus), Some(plus)) = (parse_line_expr(minuses[0]), parse_line_expr(pluses[0])) {
        for e in [&minus, &plus] {
            let probe = shape::probe_expr(e);
            if probe.macros > 0 || probe.unsupported > 0 {
                return super::prov::Counted::Miss;
            }
        }
        let Some((minus_tpl, plus_tpl)) = antiunify_general_expr_single(&minus, &plus) else {
            return super::prov::Counted::Miss;
        };
        return emit_general_expr_single(
            path, pre, minuses[0], pluses[0], changed, &minus_tpl, &plus_tpl, used_rules,
        );
    }
    // WI-7 slice-5: `let`-init fallback (same WI-5 entry as the multi
    // sweep). `let` lines never parse whole-line as expressions; the
    // init column anti-unifies through the same single-pair core and
    // the bare anchored template rewrites the init subterm in place
    // (pattern/type ride each site's own wrapper). The byte-compare
    // probe validates the init entry exactly as the whole-line entry
    // (spacing-drift veto carries over).
    let Some(minus_init) = parse_let_init(minuses[0]) else {
        return super::prov::Counted::Miss;
    };
    let Some(plus_init) = parse_let_init(pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    for e in [&minus_init, &plus_init] {
        let probe = shape::probe_expr(e);
        if probe.macros > 0 || probe.unsupported > 0 {
            return super::prov::Counted::Miss;
        }
    }
    let Some((minus_tpl, plus_tpl)) = antiunify_general_expr_single(&minus_init, &plus_init) else {
        return super::prov::Counted::Miss;
    };
    emit_general_expr_single(
        path, pre, minuses[0], pluses[0], changed, &minus_tpl, &plus_tpl, used_rules,
    )
}
