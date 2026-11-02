//! Framed `let`-type-add arm (WI-14 slice 4), split from
//! [`super::generalized`] under the D5 1600-line gate.
//!
//! The same type-add class inside one-line `fn` frames
//! (`fn f0() { let x = E; }` → `fn f0() { let x: T = E; }`) raises to
//! the identical bare hunk as the own-line arm (D14: no frame text
//! emitted) via the `fn` Stmt rung — the only frame rung whose outer
//! is never itself a `Stmt::Local`, so the bare emission cannot match
//! an outer `let` first. `let`-outer frames (`let g = || { … };`)
//! stay ordinary by construction (unwrapping them would hand the
//! engine a template matching the outer `let`, over-matching); mixed
//! whole-line + framed sections stay ordinary (all-or-nothing per
//! side).

use super::antiunify::antiunify_let_type_add;
use super::unique_rule;
use quote::ToTokens;

/// Framed fallback for [`super::generalized::try_let_sweep`]: every
/// pair must unwrap through the `fn` Stmt rung with a frame-equal
/// outer (a frame delta alongside the inner rewrite vetoes — never a
/// partial claim). Same totality as the own-line arm: the AST count
/// descends into `fn` bodies (framed inners count), and the needles
/// are the whole minus lines actually present in the file.
pub(crate) fn try_framed_let_sweep(
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
    let mut inner_minuses: Vec<String> = Vec::with_capacity(minuses.len());
    let mut inner_pluses: Vec<String> = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let m_stmt = super::frame::fn_frame_stmt_inner(m)?;
        let p_stmt = super::frame::fn_frame_stmt_inner(p)?;
        let m_text = m_stmt.to_token_stream().to_string();
        let p_text = p_stmt.to_token_stream().to_string();
        if !super::subline::frame_holds(m, p, &[(m_text.clone(), p_text.clone())]) {
            return None;
        }
        inner_minuses.push(m_text);
        inner_pluses.push(p_text);
    }
    let inner_m: Vec<&str> = inner_minuses.iter().map(String::as_str).collect();
    let inner_p: Vec<&str> = inner_pluses.iter().map(String::as_str).collect();
    let ty = antiunify_let_type_add(&inner_m, &inner_p)?;
    if !crate::totality::let_untyped_holds(file_old, minuses, changed) {
        return None;
    }
    Some(emit_let_type_add(path, &ty, changed, used_rules))
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
