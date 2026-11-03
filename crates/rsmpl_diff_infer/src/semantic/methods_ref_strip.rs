//! WI-22 method `&`-receiver strip axis (D5 split from `methods`):
//! N-site `&recv.method(args)` strip sweep plus its pair parser
//! and span collector. No behavior change.

use super::{count_occurrences, flat_ws, strip_one_terminator, unique_rule};

/// N-site (N ≥ 2) `&`-receiver strip (T359): one `&recv.method(args)` per
/// pair, plus is minus with it replaced by `recv`; shared method+args bind
/// verbatim. One trailing `;`/`,` stripped; lone/turbofish/`&mut`/
/// divergent/non-receiver stay ordinary.
pub(crate) fn try_ref_method_strip_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut method: Option<String> = None;
    let mut args: Option<Vec<String>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (meth, argv) = parse_ref_strip_pair(m, p)?;
        if method.as_ref().is_some_and(|c| *c != meth) {
            return None;
        }
        if args.as_ref().is_some_and(|a| *a != argv) {
            return None;
        }
        method = Some(meth);
        args = Some(argv);
    }
    let (method, args) = (method?, args?);
    if !crate::totality::strip_holds(file_old, &method, &args, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{method}_strip"));
    let argv = args.join(", ");
    let mut hunk = crate::emission_header(path, &rule, changed);
    // WI-7-upper: same hole class — a raw uppercase `&Receiver.Old(...)`
    // binds `Old` (probed bogus binding); backticks pin the literal.
    let lit = super::shape::literal_callee(&method);
    hunk.push_str(&format!("- &Receiver.{lit}({argv})\n+ Receiver\n"));
    Some(hunk)
}

/// Parse one `-`/`+` line pair as a `&`-receiver strip: the minus side
/// parses as an expr containing exactly one shared-reference-to-method
/// call, and the plus side is the minus with that span replaced by the
/// receiver. Returns (method ident, arg token-texts) on success.
fn parse_ref_strip_pair(old_line: &str, new_line: &str) -> Option<(String, Vec<String>)> {
    let minus = strip_one_terminator(old_line.trim())?;
    let plus = strip_one_terminator(new_line.trim())?;
    if minus.is_empty() || plus.is_empty() {
        return None;
    }
    let minus_expr: syn::Expr = syn::parse_str(minus).ok()?;
    let mut hits: Vec<RefStripHit> = Vec::new();
    collect_ref_strips(&minus_expr, &mut hits);
    if hits.len() != 1 {
        return None;
    }
    let hit = hits.pop()?;
    // Whitespace-flat compare: `ToTokens` spacing never matches source
    // spacing, and source lines carry indentation the pattern drops.
    // Explicit lifetimes (`&'a r.m(..)`) never qualify: `syn` drops the
    // lifetime, so the token text cannot prove its absence — check raw.
    if flat_ws(&hit.cand).contains("&'") {
        return None;
    }
    if count_occurrences(&flat_ws(minus), &flat_ws(&hit.cand)) != 1 {
        return None;
    }
    if flat_ws(minus).replacen(&flat_ws(&hit.cand), &flat_ws(&hit.recv), 1) != flat_ws(plus) {
        return None;
    }
    Some((hit.method, hit.args))
}

/// One `&recv.method(args)` span: token-texts for the containment
/// check plus the shared method/args for cross-site agreement.
struct RefStripHit {
    method: String,
    cand: String,
    recv: String,
    args: Vec<String>,
}

/// Walk an expr collecting every turbofish-free shared reference to a
/// method call (`&recv.method(args)`); explicit lifetimes and `&mut`
/// never qualify.
fn collect_ref_strips(expr: &syn::Expr, out: &mut Vec<RefStripHit>) {
    use quote::ToTokens;
    use syn::visit::Visit;
    struct Collector<'a>(&'a mut Vec<RefStripHit>);
    impl<'ast> syn::visit::Visit<'ast> for Collector<'_> {
        fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
            if node.mutability.is_none() {
                if let syn::Expr::MethodCall(call) = node.expr.as_ref() {
                    if call.turbofish.is_none() {
                        self.0.push(RefStripHit {
                            method: call.method.to_string(),
                            cand: node.to_token_stream().to_string(),
                            recv: call.receiver.to_token_stream().to_string(),
                            args: call
                                .args
                                .iter()
                                .map(|a| a.to_token_stream().to_string())
                                .collect(),
                        });
                    }
                }
            }
            syn::visit::visit_expr_reference(self, node);
        }
    }
    Collector(out).visit_expr(expr);
}
