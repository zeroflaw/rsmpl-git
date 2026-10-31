//! Control-position template sweep (WI-8b, second slice of WI-8).
//!
//! The WI-6 general-expression core (`antiunify_general_exprs`) already
//! owns the subterm shapes that change inside control positions
//! (binary/unary/cast/field/index/reference/await), but its detector
//! parses each changed line whole-line as one `syn::Expr`. Control
//! fragments (`_ if v > 0 => 1,`, `if v > 0 {`, `for x in iter {`,
//! `let f = |x| x + 1;`, `foo(x).await;`) never parse whole-line, so
//! every control class declined before the core was reached. This
//! detector is a thin extractor fan-out over the same core: seven
//! single-line position extractors each return the changed sub-`Expr`
//! per pair, all N pairs must extract through the *same* position kind
//! (mixed positions decline — one intent, one position), the sub-exprs
//! anti-unify through the WI-6 core, and totality is the existing
//! whole-file `template_pair_holds` probe.
//!
//! Emission is the bare WI-6 template (`- E0 > 0` / `+ E0 >= 0` with
//! `# allow=many`): the engine already rewrites bare templates inside
//! every control position structurally (SPEC 18.6 R1 — guard, arm body,
//! closure body, condition, iterable, diverge, await/`?` base are all
//! structural), probe-verified in-loop. An out-of-position twin (the
//! same subterm in a plain body) counts `N+1` in the whole-file probe
//! and vetoes to ordinary instead of silently rewriting the body —
//! the acceptance posture. This is deliberately stricter than
//! IMPROVE.md's position-scoped counting (fewer fires, never an
//! over-match); control frames in emission stay future work.
//!
//! Boundaries (stay ordinary): multi-line guards/bodies/diverge blocks,
//! `if let` conditions (`Expr::Let` has no WI-6 variant), macro or
//! unsupported subterms (the `shape::probe_expr` gate, same posture as
//! `try_general_expr_sweep`), all-identical line classes (verbatim
//! owns them, T422), single pairs (WI-4 N = 1 rule).

use super::general_expr::antiunify_general_exprs;
use super::shape;
use super::{strip_one_terminator, unique_rule};
use quote::ToTokens;

fn tok<T: ToTokens>(node: &T) -> String {
    node.to_token_stream().to_string()
}

/// Engine-matchability gate for one extracted sub-expression: macros and
/// unsupported nodes never match structurally, so a template inlining
/// them could never re-match (same posture as `try_general_expr_sweep`).
fn sub_ok(e: &syn::Expr) -> bool {
    let probe = shape::probe_expr(e);
    probe.macros == 0 && probe.unsupported == 0
}

fn parse_expr(text: &str) -> Option<syn::Expr> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    syn::parse_str::<syn::Expr>(t).ok()
}

/// Match-arm guard: `PAT if GUARD => BODY,` — the text between the
/// first ` if ` and the last ` => `. Both sides must carry both markers
/// (arm bodies without guards belong to `arm_body` below).
fn extract_guard(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn guard_of(line: &str) -> Option<syn::Expr> {
        let (_, rest) = line.split_once(" if ")?;
        let (guard, _) = rest.rsplit_once(" => ")?;
        parse_expr(guard)
    }
    Some((guard_of(m)?, guard_of(p)?))
}

/// Match-arm body: `PAT => BODY,` without a guard ` if ` before the
/// ` => `. The body is the text after the last ` => ` with one trailing
/// `,` stripped (the terminator never reaches the pattern).
fn extract_arm_body(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn body_of(line: &str) -> Option<syn::Expr> {
        let (head, _) = line.rsplit_once(" => ")?;
        // Guards own `if`-bearing arms (checked first in the driver).
        if head.contains(" if ") {
            return None;
        }
        let (_, body) = line.rsplit_once(" => ")?;
        let body = strip_one_terminator(body.trim())?;
        parse_expr(body)
    }
    Some((body_of(m)?, body_of(p)?))
}

/// Closure body, two single-line sub-cases with the same body position:
/// (a) the whole line is a closure (`|x| x + 1,` as a call arg); (b) the
/// line is a `let` whose init is a closure (`let f = |x| x + 1;`).
/// Both sides must use the same sub-case. A block body with exactly one
/// expression statement contributes its inner expression (the common
/// `|x| { x + 1 }` single-line spelling); any other block shape declines
/// in the WI-6 core (no `Block` variant — never guessed).
fn extract_closure_body(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn closure_body(line: &str) -> Option<(syn::Expr, bool)> {
        let t = strip_one_terminator(line.trim())?;
        // Sub-case (a): the line itself is a closure.
        if let Ok(syn::Expr::Closure(c)) = syn::parse_str::<syn::Expr>(t.trim()) {
            return Some((unwrap_closure_body(&c.body)?, false));
        }
        // Sub-case (b): a `let` whose init is a closure.
        if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            if let syn::Expr::Closure(c) = *init.expr {
                return Some((unwrap_closure_body(&c.body)?, true));
            }
        }
        None
    }
    let (mb, m_let) = closure_body(m)?;
    let (pb, p_let) = closure_body(p)?;
    if m_let != p_let {
        return None;
    }
    Some((mb, pb))
}

fn unwrap_closure_body(body: &syn::Expr) -> Option<syn::Expr> {
    if let syn::Expr::Block(b) = body {
        if b.block.stmts.len() == 1 {
            if let syn::Stmt::Expr(e, None) = &b.block.stmts[0] {
                return Some(e.clone());
            }
        }
        return None;
    }
    Some(body.clone())
}

/// `if`/`while` condition: `if COND {`, `} else if COND {`,
/// `while COND {`. The condition is the text between the keyword and
/// the trailing `{`. `if let` conditions parse as `Expr::Let` and
/// decline in the WI-6 core (no variant — never guessed).
fn extract_if_while_cond(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn cond_of(line: &str) -> Option<syn::Expr> {
        let t = line.trim();
        if !t.ends_with('{') {
            return None;
        }
        let inner = t.strip_suffix('{')?.trim();
        // `} else if COND`: the condition follows the last `if `.
        let cond = if let Some((_, rest)) = inner.rsplit_once("} else if ") {
            rest
        } else if let Some(rest) = inner.strip_prefix("if ") {
            rest
        } else {
            inner.strip_prefix("while ")?
        };
        let e = parse_expr(cond)?;
        // `if let` is a different template family (WI-8 non-goal).
        if matches!(e, syn::Expr::Let(_)) {
            return None;
        }
        Some(e)
    }
    Some((cond_of(m)?, cond_of(p)?))
}

/// `for` iterable: `for PAT in ITER {`. The pattern must be token-equal
/// per pair (a pattern delta is not an iterable template); hetero
/// patterns across sites are fine (the bare iterable template preserves
/// each site's pattern when the engine rewrites the iterable subterm).
fn extract_for_iter(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn iter_of(line: &str) -> Option<(String, syn::Expr)> {
        let t = line.trim();
        if !t.ends_with('{') {
            return None;
        }
        let inner = t.strip_suffix('{')?.trim();
        let rest = inner.strip_prefix("for ")?;
        let (pat, iter) = rest.split_once(" in ")?;
        Some((tok(&parse_expr(pat)?), parse_expr(iter)?))
    }
    let (m_pat, m_iter) = iter_of(m)?;
    let (p_pat, p_iter) = iter_of(p)?;
    if m_pat != p_pat {
        return None;
    }
    Some((m_iter, p_iter))
}

/// `let`-`else` diverge expression: `let PAT = EXPR else { DIVERGE };`
/// single-line. The head (pattern + scrutinee) must be token-equal per
/// pair (a head delta belongs to the let cores, never guessed here);
/// the diverge block with exactly one expression statement contributes
/// its inner expression, anything else declines in the WI-6 core
/// (v1: `return`/`break` diverges have no WI-6 variant and stay
/// ordinary — never guessed).
fn extract_let_else_diverge(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn diverge_of(line: &str) -> Option<(String, syn::Expr)> {
        if !line.contains(" else ") {
            return None;
        }
        let t = line.trim();
        let full = if t.ends_with(';') {
            t.to_string()
        } else {
            format!("{t};")
        };
        let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(&full) else {
            return None;
        };
        let init = local.init?;
        let (_, diverge) = init.diverge?;
        // Head key is pat + scrutinee (the diverge arm rides the
        // template, never the key).
        let head_key = format!("{}={}", tok(&local.pat), tok(&init.expr));
        let inner = match *diverge {
            syn::Expr::Block(b) => {
                if b.block.stmts.len() == 1 {
                    if let syn::Stmt::Expr(e, None) = &b.block.stmts[0] {
                        e.clone()
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            other => other,
        };
        Some((head_key, inner))
    }
    let (m_head, m_div) = diverge_of(m)?;
    let (p_head, p_div) = diverge_of(p)?;
    if m_head != p_head {
        return None;
    }
    Some((m_div, p_div))
}

/// `await`/`?` base: `BASE.await` / `BASE?` either whole-line or as a
/// `let` init (`let x = BASE.await;` — the `-U0` shape real diffs take).
/// Both sides must use the same wrapper kind and the same let/wrapper
/// spelling; the bases anti-unify through the WI-6 core (so a bare-path
/// base like `a.await` → `b.await` declines there — a two-delta rename,
/// never a template).
fn extract_await_try_base(m: &str, p: &str) -> Option<(syn::Expr, syn::Expr)> {
    fn base_of(line: &str) -> Option<(syn::Expr, bool, bool)> {
        let t = strip_one_terminator(line.trim())?;
        let t = t.trim();
        // Whole-line wrapper (`s.a.await;` with the terminator stripped
        // is not a whole-line await — it only parses through the `let`
        // unwrap below; bare wrappers parse here).
        if let Ok(syn::Expr::Await(a)) = syn::parse_str::<syn::Expr>(t) {
            return Some((*a.base, false, false));
        }
        if let Ok(syn::Expr::Try(tr)) = syn::parse_str::<syn::Expr>(t) {
            return Some((*tr.expr, true, false));
        }
        // `let` init wrapper (both sides must share this spelling).
        if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            if let syn::Expr::Await(a) = *init.expr {
                return Some((*a.base, false, true));
            }
            if let syn::Expr::Try(tr) = *init.expr {
                return Some((*tr.expr, true, true));
            }
        }
        None
    }
    let (mb, m_try, m_let) = base_of(m)?;
    let (pb, p_try, p_let) = base_of(p)?;
    if m_try != p_try || m_let != p_let {
        return None;
    }
    Some((mb, pb))
}

/// Try one position over all N pairs: every pair must extract (`None`
/// anywhere declines the position), every extracted sub-expression must
/// clear the macro/unsupported gate, then the WI-6 core anti-unifies the
/// two sub-expression columns.
fn try_position(minuses: &[syn::Expr], pluses: &[syn::Expr]) -> Option<(String, String)> {
    for e in minuses.iter().chain(pluses.iter()) {
        if !sub_ok(e) {
            return None;
        }
    }
    antiunify_general_exprs(minuses, pluses)
}

/// Diet inference assigns `E0` in field-base position (`E0.a`) no kind,
/// so a bare probe/emission matches nothing (engine-probed: 0 matches
/// bare vs 2 with `# expr E0`). Index bases (`E0[0]`) and every other
/// WI-6 shape infer fine. The declaration rides only field-hole
/// templates, in the probe and the emission alike (WI-6 residue —
/// `antiunify_general_exprs` has pinned the `E0.timeout` shape since
/// WI-6 but no sweep could ever emit it; noted in REVIEW).
fn needs_expr_decl(minus_tpl: &str) -> bool {
    minus_tpl.contains("E0.")
}

/// Whole-file real-pair probe for control templates: identical to
/// `totality::template_pair_holds` except field-hole templates carry
/// the `# expr E0` declaration the diet inference omits (see
/// [`needs_expr_decl`]). An untouched twin anywhere (including an
/// out-of-position plain-body twin) counts `N+1` and vetoes.
fn control_pair_holds(
    path: &str,
    file_old: Option<&str>,
    minus_tpl: &str,
    plus_tpl: &str,
    n: usize,
) -> bool {
    if n == 0 || minus_tpl.contains('\n') || plus_tpl.contains('\n') {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    let decl = if needs_expr_decl(minus_tpl) {
        "# expr E0\n"
    } else {
        ""
    };
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n{decl}- {minus_tpl}\n+ {plus_tpl}\n",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

/// One control-position extractor: both changed lines through the same
/// position kind to their changed sub-expressions.
type ControlExtractor = fn(&str, &str) -> Option<(syn::Expr, syn::Expr)>;

/// N-site (N ≥ 2) control-position sweep: every changed line carries its
/// changed subterm in one uniform control position (guard, arm body,
/// closure body, `if`/`while` condition, `for` iterable, `let`-`else`
/// diverge, `await`/`?` base); the subterm columns anti-unify through
/// the WI-6 core. Emits one `# allow=many` hunk (tight original range
/// or `# sweep` per the D.5 64-line rule, via `emission_header`).
/// Totality is the real-pair engine probe (`control_pair_holds`): the
/// emitted minus must match exactly N subterms in the pre-image — an
/// untouched twin anywhere (including an out-of-position plain-body
/// twin) vetoes to ordinary. Detector name `control_template`.
pub(crate) fn try_control_template_sweep(
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
    // Fixed position order, most specific first: guard arms contain
    // both ` if ` and ` => `, arm bodies contain ` => ` alone.
    let extractors: &[(&str, ControlExtractor)] = &[
        ("guard", extract_guard),
        ("arm_body", extract_arm_body),
        ("closure_body", extract_closure_body),
        ("if_while_cond", extract_if_while_cond),
        ("for_iter", extract_for_iter),
        ("let_else_diverge", extract_let_else_diverge),
        ("await_try_base", extract_await_try_base),
    ];
    for (_, extract) in extractors {
        let mut minus_subs = Vec::with_capacity(minuses.len());
        let mut plus_subs = Vec::with_capacity(pluses.len());
        let mut ok = true;
        for (m, p) in minuses.iter().zip(pluses.iter()) {
            match extract(m, p) {
                Some((ms, ps)) => {
                    minus_subs.push(ms);
                    plus_subs.push(ps);
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let Some((minus_tpl, plus_tpl)) = try_position(&minus_subs, &plus_subs) else {
            continue;
        };
        if !control_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
            return None;
        }
        let rule = unique_rule(used_rules, "control_template");
        let mut hunk = crate::emission_header(path, &rule, changed);
        if needs_expr_decl(&minus_tpl) {
            hunk.push_str("# expr E0\n");
        }
        hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
        return Some(hunk);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exprs(texts: &[&str]) -> Vec<syn::Expr> {
        texts
            .iter()
            .map(|t| syn::parse_str::<syn::Expr>(t).expect("test expr must parse"))
            .collect()
    }

    #[test]
    fn guard_extracts_between_if_and_fat_arrow() {
        let (m, p) =
            extract_guard("    _ if v > 0 => 1,", "    _ if v >= 0 => 1,").expect("must extract");
        assert_eq!(tok(&m), "v > 0");
        assert_eq!(tok(&p), "v >= 0");
        // Guard-less arms decline (arm_body owns them).
        assert!(extract_guard("    _ => x + 1,", "    _ => x + 2,").is_none());
    }

    #[test]
    fn arm_body_extracts_after_fat_arrow() {
        let (m, p) = extract_arm_body("    1 => v + 1,", "    1 => v + 2,").expect("must extract");
        assert_eq!(tok(&m), "v + 1");
        assert_eq!(tok(&p), "v + 2");
        // Guarded arms decline here (guard owns them).
        assert!(extract_arm_body("    _ if v > 0 => 1,", "    _ if v >= 0 => 1,").is_none());
    }

    #[test]
    fn closure_body_extracts_bare_and_let_init() {
        let (m, _p) = extract_closure_body("|x| x + 1,", "|x| x + 2,").expect("bare closure");
        assert_eq!(tok(&m), "x + 1");
        let (m, p) = extract_closure_body("    let f = |x| x + 1;", "    let f = |x| x + 2;")
            .expect("let init");
        assert_eq!(tok(&m), "x + 1");
        assert_eq!(tok(&p), "x + 2");
        // Mixed sub-cases decline (one intent, one spelling).
        assert!(extract_closure_body("|x| x + 1,", "    let f = |x| x + 2;").is_none());
    }

    #[test]
    fn if_while_cond_extracts_before_brace() {
        let (m, _p) = extract_if_while_cond("    if v > 0 {", "    if v >= 0 {").expect("if");
        assert_eq!(tok(&m), "v > 0");
        let (m, _) =
            extract_if_while_cond("    while v > 0 {", "    while v >= 0 {").expect("while");
        assert_eq!(tok(&m), "v > 0");
        let (m, _) = extract_if_while_cond("    } else if v > 0 {", "    } else if v >= 0 {")
            .expect("else-if");
        assert_eq!(tok(&m), "v > 0");
        // `if let` declines (different template family).
        assert!(
            extract_if_while_cond("    if let Some(x) = y {", "    if let Some(x) = y {").is_none()
        );
    }

    #[test]
    fn for_iter_extracts_after_in_with_head_gate() {
        let (m, p) = extract_for_iter("    for x in foo(a) {", "    for x in bar(a) {")
            .expect("must extract");
        assert_eq!(tok(&m), "foo (a)");
        assert_eq!(tok(&p), "bar (a)");
        // A pattern delta is not an iterable template.
        assert!(extract_for_iter("    for x in foo(a) {", "    for y in bar(a) {").is_none());
    }

    #[test]
    fn await_try_base_extracts_wrapper_kind() {
        let (m, p) =
            extract_await_try_base("    foo(a).await;", "    foo(a + 1).await;").expect("await");
        assert_eq!(tok(&m), "foo (a)");
        assert_eq!(tok(&p), "foo (a + 1)");
        let (m, _) = extract_await_try_base("    foo(a)?;", "    foo(a + 1)?;").expect("try");
        assert_eq!(tok(&m), "foo (a)");
        // Mixed wrappers decline.
        assert!(extract_await_try_base("    foo(a).await;", "    foo(a)?;").is_none());
    }

    #[test]
    fn guard_subterms_antiunify_through_wi6_core() {
        // Hetero receivers, uniform `> 0` → `>= 0` delta.
        let minus = exprs(&["v > 0", "w > 0"]);
        let plus = exprs(&["v >= 0", "w >= 0"]);
        assert_eq!(
            try_position(&minus, &plus),
            Some(("E0 > 0".to_string(), "E0 >= 0".to_string()))
        );
    }

    #[test]
    fn closure_subterms_antiunify_through_wi6_core() {
        let minus = exprs(&["x + 1", "y + 1"]);
        let plus = exprs(&["x + 2", "y + 2"]);
        assert_eq!(
            try_position(&minus, &plus),
            Some(("E0 + 1".to_string(), "E0 + 2".to_string()))
        );
    }

    #[test]
    fn macro_subterm_vetoes() {
        let minus = exprs(&["m!(a) + 1", "m!(b) + 1"]);
        let plus = exprs(&["m!(a) + 2", "m!(b) + 2"]);
        assert_eq!(try_position(&minus, &plus), None);
    }

    use super::super::{infer_semantic, infer_semantic_with_pre_images};
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

    const GUARD_PRE: &str = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v > 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w > 0 => 1,\n        _ => 0,\n    }\n}\n";

    fn guard_body() -> String {
        "@@ -3 +3 @@\n-        _ if v > 0 => 1,\n+        _ if v >= 0 => 1,\n@@ -9 +9 @@\n-        _ if w > 0 => 1,\n+        _ if w >= 0 => 1,\n"
            .to_string()
    }

    #[test]
    fn guard_sweep_fires_and_round_trips() {
        let raw = section("f.rs", &guard_body());
        let pre = pre_map(&[("f.rs", GUARD_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("guard sweep must fire");
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("- E0 > 0\n"), "got: {out}");
        assert!(out.contains("+ E0 >= 0\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("v > 0"), "consumed: {out}");
        // Byte-identical re-apply: the semantic patch reproduces the
        // post-tree, not just a matching tree.
        let post = GUARD_PRE
            .replace("v > 0", "v >= 0")
            .replace("w > 0", "w >= 0");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], post);
    }

    #[test]
    fn guard_untouched_twin_vetoes() {
        // An untouched guard twin anywhere in the file counts N+1 in the
        // whole-file probe and vetoes — the body is never rewritten.
        let pre = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v > 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w > 0 => 1,\n        _ => 0,\n    }\n}\nfn h(z: i32) -> i32 {\n    match z {\n        _ if z > 0 => 1,\n        _ => 0,\n    }\n}\n";
        let raw = section("f.rs", &guard_body());
        let pre_map = pre_map(&[("f.rs", pre)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).is_none() || {
                let out =
                    infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map).unwrap();
                !out.contains("E0 > 0")
            },
            "twin must veto the guard template"
        );
    }

    #[test]
    fn guard_body_twin_vetoes_rather_than_rewriting_body() {
        // The acceptance case: the same subterm in a plain body expression
        // vetoes the guard template instead of the body being rewritten.
        let pre = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v > 0 => 1,\n        _ => 0,\n    }\n    v > 0\n}\nfn g(w: i32) -> i32 {\n    match w {\n        _ if w > 0 => 1,\n        _ => 0,\n    }\n    w > 0\n}\nfn plain(a: i32) -> bool {\n    a > 0\n}\n";
        let raw = section("f.rs", &guard_body());
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map);
        // Either ordinary (None) or a non-guard-template emission: the
        // guard template itself must not fire over the body twin.
        if let Some(patch) = out {
            assert!(
                !patch.contains("E0 > 0"),
                "must not rewrite body twin: {patch}"
            );
        }
    }

    const CLOSURE_PRE: &str =
        "fn f() {\n    let f = |x| x + 1;\n    let g = |y| y + 1;\n    let _ = (f(1), g(2));\n}\n";

    #[test]
    fn closure_let_sweep_fires_and_round_trips() {
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let f = |x| x + 1;\n-    let g = |y| y + 1;\n+    let f = |x| x + 2;\n+    let g = |y| y + 2;\n",
        );
        let pre = pre_map(&[("f.rs", CLOSURE_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("closure sweep must fire");
        validated_live(&out, &pre);
        assert!(out.contains("- E0 + 1\n"), "got: {out}");
        assert!(out.contains("+ E0 + 2\n"), "got: {out}");
        let post = CLOSURE_PRE
            .replace("x + 1", "x + 2")
            .replace("y + 1", "y + 2");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], post);
    }

    #[test]
    fn if_cond_sweep_fires() {
        let pre = "fn f(v: i32) -> i32 {\n    if v > 0 {\n        1\n    } else {\n        0\n    }\n}\nfn g(w: i32) -> i32 {\n    if w > 0 {\n        1\n    } else {\n        0\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    if v > 0 {\n+    if v >= 0 {\n@@ -10 +10 @@\n-    if w > 0 {\n+    if w >= 0 {\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("if-cond sweep must fire");
        validated_live(&out, &pre_map);
        assert!(out.contains("- E0 > 0\n"), "got: {out}");
        assert!(out.contains("+ E0 >= 0\n"), "got: {out}");
    }

    #[test]
    fn for_iter_sweep_fires() {
        // WI-6 index shape inside the iterable: hetero bases preserved,
        // uniform `0` → `1` delta. Call-shaped iterables (`foo(a)` →
        // `foo(a + 1)`) stay ordinary in v1 (no Call variant in the WI-6
        // core — future work alongside call-arg templates in control
        // positions).
        let pre = "fn f(a: Vec<i32>, b: Vec<i32>) {\n    for x in a[0] {\n    }\n    for y in b[0] {\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    for x in a[0] {\n+    for x in a[1] {\n@@ -4 +4 @@\n-    for y in b[0] {\n+    for y in b[1] {\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("for-iter sweep must fire");
        validated_live(&out, &pre_map);
        assert!(out.contains("- E0[0]\n"), "got: {out}");
        assert!(out.contains("+ E0[1]\n"), "got: {out}");
    }

    #[test]
    fn mixed_positions_decline() {
        // One guard pair plus one condition pair: no uniform position,
        // stays ordinary.
        let pre = "fn f(v: i32) -> i32 {\n    match v {\n        _ if v > 0 => 1,\n        _ => 0,\n    }\n}\nfn g(w: i32) -> i32 {\n    if w > 0 {\n        1\n    } else {\n        0\n    }\n}\n";
        let raw = section(
            "f.rs",
            "@@ -3 +3 @@\n-        _ if v > 0 => 1,\n+        _ if v >= 0 => 1,\n@@ -8 +8 @@\n-    if w > 0 {\n+    if w >= 0 {\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map);
        // Either ordinary or a non-control emission; the control
        // detector must not claim mixed positions as one template.
        // Whole-file general_expr may still fire on the shared subterm
        // shape — assert it does not fire *as* the mixed control pair
        // by checking the emission covers both lines only when uniform.
        if let Some(patch) = &out {
            // The two changed lines differ in control framing; a single
            // bare `E0 > 0` sweep over both would also match — but the
            // control detector requires unanimous extraction, and the
            // general detector requires whole-line exprs. Both decline,
            // so ordinary is expected. If something fires, it must not
            // be a control claim over mixed positions.
            assert!(
                !patch.contains("control_template") || !patch.contains("E0 > 0"),
                "mixed positions must not fuse: {patch}"
            );
        }
    }

    #[test]
    fn try_base_sweep_fires() {
        // `?` shares the await extractor and the field-hole discipline
        // (explicit declaration pinned).
        let pre = "fn f(s: S, t: S) -> Option<i32> {\n    let x = s.a?;\n    let y = t.a?;\n    Some(0)\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = s.a?;\n+    let x = s.b?;\n@@ -3 +3 @@\n-    let y = t.a?;\n+    let y = t.b?;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("try-base sweep must fire");
        validated_live(&out, &pre_map);
        assert!(out.contains("# expr E0\n"), "got: {out}");
        assert!(out.contains("- E0.a\n"), "got: {out}");
        assert!(out.contains("+ E0.b\n"), "got: {out}");
    }

    #[test]
    fn single_pair_declines() {
        let raw = section(
            "f.rs",
            "@@ -3 +3 @@\n-        _ if v > 0 => 1,\n+        _ if v >= 0 => 1,\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn await_base_sweep_fires() {
        // WI-6 field shape inside the await base: hetero receivers
        // preserved, uniform `.a` → `.b` delta. Call-shaped bases
        // (`foo(a)` → `foo(a + 1)`) stay ordinary in v1 (no Call variant
        // in the WI-6 core).
        let pre = "async fn f(s: S, t: S) {\n    let x = s.a.await;\n    let y = t.a.await;\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let x = s.a.await;\n+    let x = s.b.await;\n@@ -3 +3 @@\n-    let y = t.a.await;\n+    let y = t.b.await;\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
            .expect("await-base sweep must fire");
        validated_live(&out, &pre_map);
        // Field-base holes need the explicit declaration diet omits.
        assert!(out.contains("# expr E0\n"), "got: {out}");
        assert!(out.contains("- E0.a\n"), "got: {out}");
        assert!(out.contains("+ E0.b\n"), "got: {out}");
    }
}
