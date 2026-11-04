//! Framed `let`-type-add arm (WI-14 slice 4 + arm/branch residue,
//! split from [`super::generalized`] under the D5 1600-line gate).
//!
//! The same type-add class inside one-line `fn` frames
//! (`fn f0() { let x = E; }` → `fn f0() { let x: T = E; }`) raises to
//! the identical bare hunk as the own-line arm (D14: no frame text
//! emitted) via the `fn` Stmt rung — the rung whose outer is never
//! itself a `Stmt::Local`, so the bare emission cannot match an
//! outer `let` first. Arm (`1 => { let a = E; a },`) and branch
//! (`if c { let a = E; a }`) rungs join under the same law (their
//! outers are arms/branches, never `Stmt::Local`); `let`-outer frames
//! (`let g = || { … };`, `let r = if … { let … };`) stay ordinary by
//! construction (unwrapping them would over-match the outer `let`);
//! mixed-rung and mixed whole-line + framed sections stay ordinary
//! (all-or-nothing per side).

use super::antiunify::antiunify_let_type_add;
use super::unique_rule;
use quote::ToTokens;

/// Framed fallback for [`super::generalized::try_let_sweep`]: every
/// pair must unwrap through one `Stmt` rung (`fn`, arm, or branch —
/// all-or-nothing per side, never mixed) with a frame-equal outer (a
/// frame delta alongside the inner rewrite vetoes — never a partial
/// claim). Same totality as the own-line arm: the AST count descends
/// into `fn` bodies, match arms, and block inits (framed inners
/// count), and the needles are the whole minus lines actually present
/// in the file.
pub(crate) fn try_framed_let_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> super::prov::Counted<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return super::prov::Counted::Miss;
    }
    for rung in [
        super::frame::fn_frame_stmt_inner,
        super::frame::arm_frame_stmt_inner,
        super::frame::branch_frame_stmt_inner,
    ] {
        let mut inner_minuses: Vec<String> = Vec::with_capacity(minuses.len());
        let mut inner_pluses: Vec<String> = Vec::with_capacity(pluses.len());
        let mut ok = true;
        for (m, p) in minuses.iter().zip(pluses.iter()) {
            let (Some(m_stmt), Some(p_stmt)) = (rung(m), rung(p)) else {
                ok = false;
                break;
            };
            let m_text = m_stmt.to_token_stream().to_string();
            let p_text = p_stmt.to_token_stream().to_string();
            if !super::subline::frame_holds(m, p, &[(m_text.clone(), p_text.clone())]) {
                ok = false;
                break;
            }
            inner_minuses.push(m_text);
            inner_pluses.push(p_text);
        }
        if !ok {
            continue;
        }
        let inner_m: Vec<&str> = inner_minuses.iter().map(String::as_str).collect();
        let inner_p: Vec<&str> = inner_pluses.iter().map(String::as_str).collect();
        let ty = match antiunify_let_type_add(&inner_m, &inner_p) {
            Some(t) => t,
            None => continue,
        };
        match crate::totality::let_untyped_verdict(file_old, minuses, changed) {
            crate::totality::LetUntypedVerdict::Holds => {}
            crate::totality::LetUntypedVerdict::CountMismatch { found, want } => {
                return super::prov::Counted::Totality { found, want };
            }
            crate::totality::LetUntypedVerdict::OpaqueHit => {
                return super::prov::Counted::Opaque;
            }
            crate::totality::LetUntypedVerdict::Unproven => {
                return super::prov::Counted::Miss;
            }
        }
        return super::prov::Counted::Fire(emit_let_type_add(path, &ty, changed, used_rules));
    }
    // No single rung owns every pair (mixed-rung sections included).
    super::prov::Counted::Miss
}

/// Shared emission for both `try_let_sweep` arms: the bare diet pair
/// over the tight original range.
pub(crate) fn emit_let_type_add(
    path: &str,
    ty: &str,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> String {
    let rule = unique_rule(used_rules, "let_type_add");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- let P0 = E0;\n+ let P0: {ty} = E0;\n"));
    hunk
}
