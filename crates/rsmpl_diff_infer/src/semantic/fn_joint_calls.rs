//! WI-8 slice 5: line-based call-shape helpers for the fn-joint family.
//!
//! Split from `fn_joint.rs` (D5: the joint file sat at 1984 lines).
//! Re-exported through `fn_joint` so existing importers are untouched.
//!

/// One call-callee site of `old`→`new`: both sides are the same call
/// shape (bare, qualified, turbofish, `let`-wrapped) whose callee head
/// swaps `old`→`new` with an equal qualifier. Argument deltas are
/// allowed here (they force suppress-only downstream); use lines,
/// method calls, fields, and macro heads return false.
pub(crate) fn is_call_site_of(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    let (Some((qm, cm, _, _)), Some((qp, cp, _, _))) = (call_parts(minus), call_parts(plus)) else {
        return false;
    };
    qm == qp && cm == old && cp == new
}

/// True when the pair is a callee-only rename of `old`→`new`:
/// qualifier, turbofish, and every argument token-identical.
pub(crate) fn is_callee_only(minus: &str, plus: &str, old: &str, new: &str) -> bool {
    if !is_call_site_of(minus, plus, old, new) {
        return false;
    }
    let (Some((_, _, tm, am)), Some((_, _, tp, ap))) = (call_parts(minus), call_parts(plus)) else {
        return false;
    };
    tm == tp && am == ap
}

/// `(qualifier, callee ident, turbofish, args)` of a call expression,
/// unwrapping one `let` init. `None` for method calls, macros,
/// non-calls, and qualified-self paths.
pub(crate) fn call_parts(line: &str) -> Option<(String, String, String, Vec<String>)> {
    let mut text = line.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return None;
    }
    // Two separate attempts (item_intent posture): a `let` init
    // stripped of its `;` parses as `Expr::Let`, which must not shadow
    // the `Stmt::Local` path — each attempt must yield a call itself.
    if let Ok(syn::Expr::Call(call)) = syn::parse_str::<syn::Expr>(text) {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            if path.qself.is_none() {
                return call_parts_of(path, &call.args);
            }
        }
        return None;
    }
    let syn::Stmt::Local(local) = syn::parse_str::<syn::Stmt>(line.trim()).ok()? else {
        return None;
    };
    let init = local.init?;
    let syn::Expr::Call(call) = init.expr.as_ref() else {
        return None;
    };
    let syn::Expr::Path(path) = call.func.as_ref() else {
        return None;
    };
    if path.qself.is_some() {
        return None;
    }
    call_parts_of(path, &call.args)
}

/// `(qualifier, callee ident, turbofish, args)` from a proven call path.
fn call_parts_of(
    path: &syn::ExprPath,
    args: &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>,
) -> Option<(String, String, String, Vec<String>)> {
    let mut segs = path.path.segments.iter();
    let mut qual: Vec<String> = Vec::new();
    let mut last = segs.next()?;
    for seg in segs {
        qual.push(quote::quote!(#last).to_string());
        last = seg;
    }
    // Method calls (`recv.method()`) parse as `ExprMethodCall`, never
    // here — but a path callee with a `self`-style receiver cannot
    // occur; no extra gate needed.
    // Canonical `::<…>` spelling (the turbofish-sweep precedent):
    // `quote!` renders `:: < u8 >` with spaces, which the probe's
    // trim comparison would read as a rewrite of the native
    // `::<u8>` line — the template must spell what the tree holds.
    let turbo = match &last.arguments {
        syn::PathArguments::None => String::new(),
        syn::PathArguments::AngleBracketed(g) => {
            let inner: Vec<String> = g
                .args
                .iter()
                .map(|a| quote::quote!(#a).to_string())
                .collect();
            format!("::<{}>", inner.join(", "))
        }
        args => quote::quote!(#args).to_string(),
    };
    let mut qualifier = String::new();
    if path.path.leading_colon.is_some() {
        qualifier.push_str("::");
    }
    qualifier.push_str(&qual.join("::"));
    let rendered: Vec<String> = args.iter().map(|a| quote::quote!(#a).to_string()).collect();
    Some((qualifier, last.ident.to_string(), turbo, rendered))
}

/// True when `line` is a call-callee site of `old` (any new spelling) —
/// the per-hunk suppression probe (plus side unconstrained).
pub(crate) fn is_call_callee_of(line: &str, old: &str) -> bool {
    let Some((_, callee, _, _)) = call_parts(line) else {
        return false;
    };
    callee == old
}
