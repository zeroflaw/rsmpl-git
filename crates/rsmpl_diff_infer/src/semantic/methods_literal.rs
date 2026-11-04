//! Same-literal method-call family: sweep, N=1 single, pair parser, macro-arg guard.
//! split off verbatim under the WI-19 1900-line warning gate (D5): pure move,
//! no behavior change. Tests stay inline in the origin file per WI-22.
use super::arg_template::{parse_method_line, tok};
use super::prov::Counted;
use super::{ident_len, strip_one_terminator, unique_rule};

/// N-site (N >= 2) same-method literal-arg sweep (T420): every pair parses
/// as `recv.method(old...)` -> `recv.method(new...)` with one shared method
/// ident, uniform arity (at least 1), per-site token-equal receivers,
/// cross-site token-equal old-arg and new-arg tuples, and a real old/new
/// delta. Turbofish on either side, `&`-led receivers, and identical arg
/// tuples stay passthrough. Emits one tight-range `# allow=many` hunk
/// with concrete literal spellings (`# expr Receiver` only: backtick
/// literals are ident-only per SPEC 15.4, and a plus-only `E` would be
/// unbound). Rule `calls_{method}_literal`.
pub(crate) fn try_method_same_literal_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() < 2 {
        return Counted::Miss;
    }
    let mut method: Option<String> = None;
    let mut old_args: Option<Vec<String>> = None;
    let mut new_args: Option<Vec<String>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some((meth, olds, news)) = parse_method_same_literal_pair(m, p) else {
            return Counted::Miss;
        };
        if method.as_ref().is_some_and(|c| *c != meth) {
            return Counted::Miss;
        }
        if old_args.as_ref().is_some_and(|a| *a != olds) {
            return Counted::Miss;
        }
        if new_args.as_ref().is_some_and(|a| *a != news) {
            return Counted::Miss;
        }
        method = Some(meth);
        old_args = Some(olds);
        new_args = Some(news);
    }
    let (Some(method), Some(old_args), Some(new_args)) = (method, old_args, new_args) else {
        return Counted::Miss;
    };
    if old_args == new_args {
        return Counted::Miss;
    }
    // WI-4 slice 2a: the shape matched, so a failing probe names its
    // counts (`totality:`) or its opaque twin (`veto:opaque-hit`).
    match crate::totality_counted::method_literal_verdict(
        file_old, &method, &old_args, minuses, changed,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(used_rules, &format!("calls_{method}_literal"));
    let old = old_args.join(", ");
    let new = new_args.join(", ");
    let mut hunk = crate::emission_header(path, &rule, changed);
    // WI-7-upper: same hole class as the rename sweep — a raw uppercase
    // `Receiver.Old(...)` binds `Old` and over-matches twin methods.
    let lit = super::shape::literal_callee(&method);
    hunk.push_str(&format!(
        "- Receiver.{lit}({old})\n+ Receiver.{lit}({new})\n"
    ));
    Counted::Fire(hunk)
}

/// N-site (N == 1) same-method same-args literal-delta single (WI-7
/// slice-13, `PrecisionUnique`): the N=1 lifting of
/// `try_method_same_literal_sweep` above. Fires iff the pair parses
/// (whole-line or the shared T449 `let`-init entry via
/// `parse_method_line` — the emission stays the `Receiver.method(args)`
/// pattern and the engine rewrites the call sub-expression in place)
/// as one same-method call with the same per-site receiver, uniform
/// arity `k >= 1`, and old/new arg token-tuples with a real delta.
/// Method deltas stay with the slice-2 single, receiver deltas with
/// the m2f/general shapes, perms with the slice-11 entry, pure wraps
/// with the slice-7 entry (all tried earlier). Emission is the concrete
/// tuple (no `E` arg holes — `method_literal_holds` counts the exact
/// old-args tuple, so a holed template would match sites the proof
/// never counted; `Receiver` is the only hole, bound per site like the
/// multi). The T361 byte-compare pre-reservation
/// (`single_reproduces`, shared with the wrap/perm/literal singles)
/// vetoes any TokenStream spacing drift. Emission is the WI-19b
/// allow-one spelling (tight range, directive-free, default
/// `allow=one`). Detector name `method_same_literal` (same family,
/// provenance unchanged). The N >= 2 sweep above keeps its uniform
/// classes untouched.
pub(crate) fn try_method_same_literal_single(
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
    let Some((o_recv, o_method, o_args)) = parse_method_line(minuses[0]) else {
        return super::prov::Counted::Miss;
    };
    let Some((n_recv, n_method, n_args)) = parse_method_line(pluses[0]) else {
        return super::prov::Counted::Miss;
    };
    // Method deltas stay with the slice-2 single, receiver deltas with
    // the m2f/general shapes.
    if o_recv != n_recv || o_method != n_method || o_args.len() != n_args.len() || o_args.is_empty()
    {
        return super::prov::Counted::Miss;
    }
    for arg in o_args.iter().chain(n_args.iter()) {
        let probe = super::shape::probe_expr(arg);
        if probe.macros > 0 || probe.unsupported > 0 {
            return super::prov::Counted::Miss;
        }
    }
    let old_toks: Vec<String> = o_args.iter().map(tok).collect();
    let new_toks: Vec<String> = n_args.iter().map(tok).collect();
    if old_toks == new_toks {
        return super::prov::Counted::Miss;
    }
    // Shared tail (slice-21): totality + backtick + re-parse +
    // byte-compare + allow-one live in `method_subline` so the sub-line
    // arm reuses the same counter (no bypass, no mirrored emission).
    super::method_subline::emit_method_literal_single(
        path, pre, file_old, minuses[0], pluses[0], minuses, changed, &o_method, &old_toks,
        &new_toks, used_rules,
    )
}

/// Parse a `-`/`+` line pair as a same-method literal-arg delta; return
/// (method ident, old-arg token-texts, new-arg token-texts) when the method
/// ident is shared, receivers token-equal per site, and both sides parse
/// (one trailing `;`/`,` stripped). `ToTokens` spellings are emitted
/// verbatim (ref-strip precedent). Turbofish, `&`-led sides, arity
/// mismatch, zero arity (no delta possible), and non-ident methods stay
/// passthrough.
pub(crate) fn parse_method_same_literal_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(String, Vec<String>, Vec<String>)> {
    let old_text = strip_one_terminator(old_line.trim())?;
    let new_text = strip_one_terminator(new_line.trim())?;
    if old_text.is_empty() || new_text.is_empty() {
        return None;
    }
    if old_text.starts_with('&') || new_text.starts_with('&') {
        return None;
    }
    let old_expr: syn::Expr = syn::parse_str(old_text).ok()?;
    let new_expr: syn::Expr = syn::parse_str(new_text).ok()?;
    let (syn::Expr::MethodCall(old_call), syn::Expr::MethodCall(new_call)) = (&old_expr, &new_expr)
    else {
        return None;
    };
    if old_call.turbofish.is_some() || new_call.turbofish.is_some() {
        return None;
    }
    if old_call.args.len() != new_call.args.len() || old_call.args.is_empty() {
        return None;
    }
    use quote::ToTokens;
    if old_call.receiver.to_token_stream().to_string()
        != new_call.receiver.to_token_stream().to_string()
    {
        return None;
    }
    let old_method = old_call.method.to_string();
    let new_method = new_call.method.to_string();
    if old_method != new_method {
        return None;
    }
    if ident_len(&old_method)? != old_method.len() {
        return None;
    }
    // Trailing macros stay ordinary: the sweep inlines args as
    // TokenStream-normalized text, which the engine cannot match when a
    // macro hides inside (same shape as the call_literal veto — probed
    // raw and normalized spellings match nothing, so diffrs would emit
    // a patch that fails check).
    if old_call.args.iter().any(contains_macro_arg) || new_call.args.iter().any(contains_macro_arg)
    {
        return None;
    }
    let olds: Vec<String> = old_call
        .args
        .iter()
        .map(|a| a.to_token_stream().to_string())
        .collect();
    let news: Vec<String> = new_call
        .args
        .iter()
        .map(|a| a.to_token_stream().to_string())
        .collect();
    Some((old_method, olds, news))
}

/// True when an argument expression holds a macro invocation anywhere
/// inside (mirrors the literal.rs helper; local so the probe slice moves
/// no cross-module items).
fn contains_macro_arg(e: &syn::Expr) -> bool {
    struct Finder(bool);
    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_expr_macro(&mut self, _: &'ast syn::ExprMacro) {
            self.0 = true;
        }
    }
    let mut f = Finder(false);
    syn::visit::Visit::visit_expr(&mut f, e);
    f.0
}
