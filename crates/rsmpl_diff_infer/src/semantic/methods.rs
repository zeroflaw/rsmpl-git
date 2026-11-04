//! Method-call sweep detectors (T413 split from `super`): N-site
//! method-to-function fan-outs and method-to-method renames (the
//! `&`-receiver strip sweep lives in `methods_ref_strip`). Pure detectors over diff lines; dispatch,
//! shared helpers (`unique_rule`, `flat_ws`, `count_occurrences`,
//! `strip_one_terminator`, `ident_len`, `call_parts`) and totality gates
//! live in the parent.

use super::arg_template::{parse_method_line, tok};
use super::prov::Counted;
use super::{call_parts, ident_len, strip_one_terminator, unique_rule};

/// N-site (N ≥ 2) method-to-function fan-out (T345, SPEC 18.5): every
/// pair parses as `recv.old_method(args)` → `new_fn(recv, args)` with
/// one shared method ident, one shared function callee, uniform
/// minus-arity, and a per-site receiver-prepend delta (receiver texts
/// differ per site — `Receiver` binds per site like the engine).
/// Turbofish sites, `&`-led receivers (general-expr routing), and
/// method→method pairs stay passthrough. Emits one `# allow=many` +
/// tight-range hunk (`# expr Receiver` + `# expr E{i}` per arg;
/// arity 0 needs only `Receiver`).
pub(crate) fn try_method_to_fn_sweep(
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
    let mut old_method: Option<String> = None;
    let mut new_func: Option<String> = None;
    let mut arity: Option<usize> = None;
    let mut whole_ok = true;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some((o, n, a)) = parse_method_to_fn_pair(m, p) else {
            whole_ok = false;
            break;
        };
        if old_method.as_ref().is_some_and(|c| *c != o) {
            return Counted::Miss;
        }
        if new_func.as_ref().is_some_and(|c| *c != n) {
            return Counted::Miss;
        }
        if arity.is_some_and(|x| x != a) {
            return Counted::Miss;
        }
        old_method = Some(o);
        new_func = Some(n);
        arity = Some(a);
    }
    if whole_ok {
        let (Some(old_method), Some(new_func), Some(arity)) = (old_method, new_func, arity) else {
            return Counted::Miss;
        };
        // WI-4 slice 1: the shape matched, so a failing probe names its
        // counts (`totality:`) or its opaque twin (`veto:opaque-hit`).
        match crate::totality::method_verdict(file_old, &old_method, arity, minuses, changed) {
            crate::totality::MethodVerdict::Holds => {}
            crate::totality::MethodVerdict::CountMismatch { found, want } => {
                return Counted::Totality { found, want };
            }
            crate::totality::MethodVerdict::OpaqueHit => return Counted::Opaque,
            crate::totality::MethodVerdict::Unproven => return Counted::Miss,
        }
        return Counted::Fire(emit_method_to_fn(
            path,
            used_rules,
            &old_method,
            &new_func,
            arity,
            changed,
        ));
    }
    // WI-4 sub-line fallback (mixed whole/sub lines veto;
    // all-identical pairs defer to verbatim, T422).
    if super::subline::all_identical_pairs(minuses, pluses) {
        return Counted::Miss;
    }
    let mut sub_old: Option<String> = None;
    let mut sub_new: Option<String> = None;
    let mut sub_arity: Option<usize> = None;
    let mut total = 0usize;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some(sites) = super::subline::extract_method_fn_sites(m, p) else {
            return Counted::Miss;
        };
        for s in &sites {
            if sub_old.as_ref().is_some_and(|c| *c != s.old_method) {
                return Counted::Miss;
            }
            if sub_new.as_ref().is_some_and(|c| *c != s.new_func) {
                return Counted::Miss;
            }
            if sub_arity.is_some_and(|x| x != s.arity) {
                return Counted::Miss;
            }
            sub_old = Some(s.old_method.clone());
            sub_new = Some(s.new_func.clone());
            sub_arity = Some(s.arity);
            total += 1;
        }
    }
    let (Some(old_method), Some(new_func), Some(arity)) = (sub_old, sub_new, sub_arity) else {
        return Counted::Miss;
    };
    if total < 2 {
        return Counted::Miss;
    }
    if !crate::totality::template_expr_holds(
        path,
        file_old,
        &format!("Receiver.{old_method}({})", m2f_args(arity)),
        total,
    ) {
        return Counted::Miss;
    }
    Counted::Fire(emit_method_to_fn(
        path,
        used_rules,
        &old_method,
        &new_func,
        arity,
        changed,
    ))
}

/// Comma-joined `E0…` arg list for the sub-line occurrence probe.
pub(crate) fn m2f_args(arity: usize) -> String {
    (0..arity)
        .map(|i| format!("E{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shared emission for the whole-line and sub-line method→function
/// paths (identical hunks).
fn emit_method_to_fn(
    path: &str,
    used_rules: &mut Vec<String>,
    old_method: &str,
    new_func: &str,
    arity: usize,
    changed: &[usize],
) -> String {
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_func}"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        vars.push(v);
    }
    let args = vars.join(", ");
    let tail = if args.is_empty() {
        "Receiver".to_string()
    } else {
        format!("Receiver, {args}")
    };
    // WI-7-upper: an uppercase method ident infers as an `expr` hole —
    // a raw `Receiver.Old(E0)` silently matches (and rewrites) any twin
    // method (probed: `Other` rewritten with a bogus `Old` binding).
    // Backticks pin the literal; likewise an uppercase plus-side bare
    // callee would leave `New` unbound.
    hunk.push_str(&format!(
        "- Receiver.{old}({args})\n+ {new}({tail})\n",
        old = super::shape::literal_callee(old_method),
        new = super::shape::literal_callee(new_func),
    ));
    hunk
}

/// Parse a `-`/`+` line pair as a method-to-function refactor; return
/// (method ident, function callee, minus-arity) when the delta is a
/// receiver prepend (plus `[0]` token-equals the minus receiver, rest
/// token-equal pairwise).
pub(crate) fn parse_method_to_fn_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(String, String, usize)> {
    let mut old_text = old_line.trim();
    if old_text.is_empty() {
        return None;
    }
    if let Some(stripped) = old_text.strip_suffix(';') {
        old_text = stripped.trim_end();
    }
    let mut new_text = new_line.trim();
    if new_text.is_empty() {
        return None;
    }
    if let Some(stripped) = new_text.strip_suffix(';') {
        new_text = stripped.trim_end();
    }
    if old_text.starts_with('&') {
        return None;
    }
    let old_expr: syn::Expr = syn::parse_str(old_text).ok()?;
    let new_expr: syn::Expr = syn::parse_str(new_text).ok()?;
    let syn::Expr::MethodCall(old_call) = &old_expr else {
        return None;
    };
    if old_call.turbofish.is_some() {
        return None;
    }
    let (new_cal, new_args) = call_parts(&new_expr)?;
    if new_args.len() != old_call.args.len() + 1 {
        return None;
    }
    use quote::ToTokens;
    let recv_text = old_call.receiver.to_token_stream().to_string();
    if new_args.is_empty() {
        return None;
    }
    if new_args[0].to_token_stream().to_string() != recv_text {
        return None;
    }
    for (a, b) in old_call.args.iter().zip(new_args.iter().skip(1)) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    let method = old_call.method.to_string();
    if ident_len(&method)? != method.len() {
        return None;
    }
    Some((method, new_cal, old_call.args.len()))
}

/// N-site (N ≥ 2) method-to-method rename fan-out (T347): every pair
/// parses as `recv.old(args)` → `recv.new(args)` with one shared old
/// method ident, one shared new method ident, uniform arity, and a
/// per-site identical receiver and args (receiver texts differ across
/// sites — `Receiver` binds per site like the engine). Turbofish on
/// either side, `&`-led receivers (general-expr routing), and
/// same-method pairs stay passthrough. Emits one `# allow=many` +
/// tight-range hunk (`# expr Receiver` + `# expr E{i}` per arg;
/// arity 0 needs only `Receiver`).
pub(crate) fn try_method_to_method_sweep(
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
    let mut old_method: Option<String> = None;
    let mut new_method: Option<String> = None;
    let mut arity: Option<usize> = None;
    let mut whole_ok = true;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some((o, n, a)) = parse_method_to_method_pair(m, p) else {
            whole_ok = false;
            break;
        };
        if old_method.as_ref().is_some_and(|c| *c != o) {
            return Counted::Miss;
        }
        if new_method.as_ref().is_some_and(|c| *c != n) {
            return Counted::Miss;
        }
        if arity.is_some_and(|x| x != a) {
            return Counted::Miss;
        }
        old_method = Some(o);
        new_method = Some(n);
        arity = Some(a);
    }
    if whole_ok {
        let (Some(old_method), Some(new_method), Some(arity)) = (old_method, new_method, arity)
        else {
            return Counted::Miss;
        };
        if old_method == new_method {
            return Counted::Miss;
        }
        // WI-4 slice 1: counted decline like the m2f arm above.
        match crate::totality::method_verdict(file_old, &old_method, arity, minuses, changed) {
            crate::totality::MethodVerdict::Holds => {}
            crate::totality::MethodVerdict::CountMismatch { found, want } => {
                return Counted::Totality { found, want };
            }
            crate::totality::MethodVerdict::OpaqueHit => return Counted::Opaque,
            crate::totality::MethodVerdict::Unproven => return Counted::Miss,
        }
        return Counted::Fire(emit_method_to_method(
            path,
            used_rules,
            &old_method,
            &new_method,
            arity,
            changed,
        ));
    }
    // WI-4 sub-line fallback (mixed whole/sub lines veto;
    // all-identical pairs defer to verbatim, T422).
    if super::subline::all_identical_pairs(minuses, pluses) {
        return Counted::Miss;
    }
    let mut sub_old: Option<String> = None;
    let mut sub_new: Option<String> = None;
    let mut sub_arity: Option<usize> = None;
    let mut total = 0usize;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some(sites) = super::subline::extract_method_sites(m, p) else {
            return Counted::Miss;
        };
        for s in &sites {
            if sub_old.as_ref().is_some_and(|c| *c != s.old_method) {
                return Counted::Miss;
            }
            if sub_new.as_ref().is_some_and(|c| *c != s.new_method) {
                return Counted::Miss;
            }
            if sub_arity.is_some_and(|x| x != s.arity) {
                return Counted::Miss;
            }
            sub_old = Some(s.old_method.clone());
            sub_new = Some(s.new_method.clone());
            sub_arity = Some(s.arity);
            total += 1;
        }
    }
    let (Some(old_method), Some(new_method), Some(arity)) = (sub_old, sub_new, sub_arity) else {
        return Counted::Miss;
    };
    if old_method == new_method || total < 2 {
        return Counted::Miss;
    }
    if !crate::totality::template_expr_holds(
        path,
        file_old,
        &format!("Receiver.{old_method}({})", m2f_args(arity)),
        total,
    ) {
        return Counted::Miss;
    }
    Counted::Fire(emit_method_to_method(
        path,
        used_rules,
        &old_method,
        &new_method,
        arity,
        changed,
    ))
}

/// WI-7 slice-2: whole-line method-to-method rename single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses whole-line as
/// `recv.old(args)` → `recv.new(args)` with token-equal receivers/args,
/// `old != new`, a pre-image is present (`None` vetoes - uniqueness
/// needs the whole file, D3), and `method_holds` proves exactly one
/// `old_method/arity` site file-wide with zero opaque hits (conditions
/// (a)+(b); (c) is structural: the template always carries the method
/// ident token beyond the `Receiver`/`E` holes). Emission is the WI-19b
/// allow-one spelling (tight range, directive-free, default `allow=one`).
pub(crate) fn try_method_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    file_old?;
    let (old_method, new_method, arity) = parse_method_to_method_pair(minuses[0], pluses[0])?;
    if old_method == new_method {
        return None;
    }
    if !crate::totality::method_holds(file_old, &old_method, arity, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_method}"));
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let old_lit = super::shape::literal_callee(&old_method);
    let new_lit = super::shape::literal_callee(&new_method);
    // Allow-one: directive-free ranged hunk (59/77 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!(
        "- Receiver.{old_lit}({args})\n+ Receiver.{new_lit}({args})\n"
    ));
    Some(hunk)
}

/// WI-7 slice-3: whole-line method-to-function single (N=1) under
/// `PrecisionUnique`. Fires iff the pair parses whole-line as
/// `recv.old(args)` -> `new_fn(recv, args)` with a receiver-prepend
/// delta, a pre-image is present (`None` vetoes - uniqueness needs the
/// whole file, D3), and `method_holds` proves exactly one
/// `old_method/arity` site file-wide with zero opaque hits (conditions
/// (a)+(b); (c) is structural: the template carries both the method
/// ident and the function callee beyond the `Receiver`/`E` holes).
/// Emission is the WI-19b allow-one spelling (tight range,
/// directive-free, default `allow=one`).
pub(crate) fn try_method_to_fn_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    file_old?;
    let (old_method, new_func, arity) = parse_method_to_fn_pair(minuses[0], pluses[0])?;
    if !crate::totality::method_holds(file_old, &old_method, arity, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_func}"));
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let tail = if args.is_empty() {
        "Receiver".to_string()
    } else {
        format!("Receiver, {args}")
    };
    // Allow-one: directive-free ranged hunk (slices-1/2 precedent).
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!(
        "- Receiver.{old}({args})\n+ {new}({tail})\n",
        old = super::shape::literal_callee(&old_method),
        new = super::shape::literal_callee(&new_func),
    ));
    Some(hunk)
}

/// Shared emission for the whole-line and sub-line method→method paths
/// (identical hunks).
fn emit_method_to_method(
    path: &str,
    used_rules: &mut Vec<String>,
    old_method: &str,
    new_method: &str,
    arity: usize,
    changed: &[usize],
) -> String {
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_method}"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        vars.push(v);
    }
    let args = vars.join(", ");
    // WI-7-upper: an uppercase method ident infers as an `expr` hole
    // (raw `Receiver.Old(E0)` leaves plus-side `New` unbound) — backticks
    // pin the literal, lowercase passes through untouched.
    hunk.push_str(&format!(
        "- Receiver.{old}({args})\n+ Receiver.{new}({args})\n",
        old = super::shape::literal_callee(old_method),
        new = super::shape::literal_callee(new_method),
    ));
    hunk
}

/// Parse a `-`/`+` line pair as a method-to-method rename; return
/// (old method, new method, arity) when receivers and args are
/// token-equal pairwise and only the method ident differs.
pub(crate) fn parse_method_to_method_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(String, String, usize)> {
    let mut old_text = old_line.trim();
    if old_text.is_empty() {
        return None;
    }
    if let Some(stripped) = old_text.strip_suffix(';') {
        old_text = stripped.trim_end();
    }
    let mut new_text = new_line.trim();
    if new_text.is_empty() {
        return None;
    }
    if let Some(stripped) = new_text.strip_suffix(';') {
        new_text = stripped.trim_end();
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
    if old_call.args.len() != new_call.args.len() {
        return None;
    }
    use quote::ToTokens;
    if old_call.receiver.to_token_stream().to_string()
        != new_call.receiver.to_token_stream().to_string()
    {
        return None;
    }
    for (a, b) in old_call.args.iter().zip(new_call.args.iter()) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    let old_method = old_call.method.to_string();
    let new_method = new_call.method.to_string();
    if ident_len(&old_method)? != old_method.len() {
        return None;
    }
    if ident_len(&new_method)? != new_method.len() {
        return None;
    }
    Some((old_method, new_method, old_call.args.len()))
}

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
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut method: Option<String> = None;
    let mut old_args: Option<Vec<String>> = None;
    let mut new_args: Option<Vec<String>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (meth, olds, news) = parse_method_same_literal_pair(m, p)?;
        if method.as_ref().is_some_and(|c| *c != meth) {
            return None;
        }
        if old_args.as_ref().is_some_and(|a| *a != olds) {
            return None;
        }
        if new_args.as_ref().is_some_and(|a| *a != news) {
            return None;
        }
        method = Some(meth);
        old_args = Some(olds);
        new_args = Some(news);
    }
    let (method, old_args, new_args) = (method?, old_args?, new_args?);
    if old_args == new_args {
        return None;
    }
    if !crate::totality::method_literal_holds(file_old, &method, &old_args, minuses, changed) {
        return None;
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
    Some(hunk)
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
) -> Option<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return None;
    }
    // Uniqueness needs the whole file; section-visible fallback stays
    // ordinary (no guessing).
    let pre = file_old?;
    let (o_recv, o_method, o_args) = parse_method_line(minuses[0])?;
    let (n_recv, n_method, n_args) = parse_method_line(pluses[0])?;
    // Method deltas stay with the slice-2 single, receiver deltas with
    // the m2f/general shapes.
    if o_recv != n_recv || o_method != n_method || o_args.len() != n_args.len() || o_args.is_empty()
    {
        return None;
    }
    for arg in o_args.iter().chain(n_args.iter()) {
        let probe = super::shape::probe_expr(arg);
        if probe.macros > 0 || probe.unsupported > 0 {
            return None;
        }
    }
    let old_toks: Vec<String> = o_args.iter().map(tok).collect();
    let new_toks: Vec<String> = n_args.iter().map(tok).collect();
    if old_toks == new_toks {
        return None;
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

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;
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

    #[test]
    fn method_to_fn_sweep_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n+    new_function(a, 1);\n+    new_function(b, 2);\n+    new_function(c, 3);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.old_method(1);\n    b.old_method(2);\n    c.old_method(3);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_function(Receiver, E0)\n"), "got: {out}");
    }

    #[test]
    fn method_to_fn_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    source.old_operation(first, second);\n-    target.old_operation(third, fourth);\n-    other.old_operation(fifth, sixth);\n+    new_operation(source, first, second);\n+    new_operation(target, third, fourth);\n+    new_operation(other, fifth, sixth);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    source.old_operation(first, second);\n    target.old_operation(third, fourth);\n    other.old_operation(fifth, sixth);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(
            out.contains("- Receiver.old_operation(E0, E1)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ new_operation(Receiver, E0, E1)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn method_to_fn_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    source.old_operation();\n-    target.old_operation();\n-    other.old_operation();\n+    new_operation(source);\n+    new_operation(target);\n+    new_operation(other);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    source.old_operation();\n    target.old_operation();\n    other.old_operation();\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- Receiver.old_operation()\n"), "got: {out}");
        assert!(out.contains("+ new_operation(Receiver)\n"), "got: {out}");
    }

    #[test]
    fn method_to_fn_divergent_method_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.other(2);\n+    new_fn(a, 1);\n+    new_fn(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_divergent_callee_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.old(2);\n+    new_fn(a, 1);\n+    other_fn(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_receiver_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.old(2);\n+    new_fn(x, 1);\n+    new_fn(y, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old::<u8>(1);\n-    b.old::<u8>(2);\n+    new_fn(a, 1);\n+    new_fn(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_sweep_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n-    d.old_method(4);\n+    a.new_method(1);\n+    b.new_method(2);\n+    c.new_method(3);\n+    d.new_method(4);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.old_method(1);\n    b.old_method(2);\n    c.old_method(3);\n    d.old_method(4);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,4 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_method(E0)\n"), "got: {out}");
    }

    #[test]
    fn method_to_method_sweep_with_dotted_string_arg_compresses() {
        // T419: concrete string args containing `.` route to MethodCall
        // (router fix) — the existing m2m detector needs no changes
        // once classification is sound.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    alpha.old_operation(\"some.dotted.value\");\n-    beta.old_operation(\"some.dotted.value\");\n-    gamma.old_operation(\"some.dotted.value\");\n+    alpha.new_operation(\"some.dotted.value\");\n+    beta.new_operation(\"some.dotted.value\");\n+    gamma.new_operation(\"some.dotted.value\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    alpha.old_operation(\"some.dotted.value\");\n    beta.old_operation(\"some.dotted.value\");\n    gamma.old_operation(\"some.dotted.value\");\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(out.contains("- Receiver.old_operation(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_operation(E0)\n"), "got: {out}");
    }

    #[test]
    fn method_to_method_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(first, second);\n-    b.old_method(third, fourth);\n-    c.old_method(fifth, sixth);\n+    a.new_method(first, second);\n+    b.new_method(third, fourth);\n+    c.new_method(fifth, sixth);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.old_method(first, second);\n    b.old_method(third, fourth);\n    c.old_method(fifth, sixth);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(
            out.contains("- Receiver.old_method(E0, E1)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.new_method(E0, E1)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn method_to_method_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_operation();\n-    b.old_operation();\n-    c.old_operation();\n+    a.new_operation();\n+    b.new_operation();\n+    c.new_operation();\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.old_operation();\n    b.old_operation();\n    c.old_operation();\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- Receiver.old_operation()\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_operation()\n"), "got: {out}");
    }

    #[test]
    fn method_to_method_divergent_old_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.other(2);\n+    a.bar(1);\n+    b.bar(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_receiver_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    x.bar(1);\n+    y.bar(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_arg_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar(1);\n+    b.bar(3);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar::<u8>(1);\n+    b.bar::<u8>(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_ref_receiver_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.foo(1);\n-    &b.foo(2);\n+    &a.bar(1);\n+    &b.bar(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_sweep_compresses() {
        // T420: hetero receivers, same method, shared old/new literals.
        // Long literals so the T375 byte-guard wins; trailing commas
        // (arg-position sites) tolerated like the strip sweep.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    agents.contains(\"skills/rsmpl-git/SKILL.md\"),\n-    readme.contains(\"skills/rsmpl-git/SKILL.md\"),\n-    spec.contains(\"skills/rsmpl-git/SKILL.md\"),\n+    agents.contains(\"skills/gitrs/SKILL.md\"),\n+    readme.contains(\"skills/gitrs/SKILL.md\"),\n+    spec.contains(\"skills/gitrs/SKILL.md\"),\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    check(\n        agents.contains(\"skills/rsmpl-git/SKILL.md\"),\n        readme.contains(\"skills/rsmpl-git/SKILL.md\"),\n        spec.contains(\"skills/rsmpl-git/SKILL.md\"),\n    );\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(
            out.contains("- Receiver.contains(\"skills/rsmpl-git/SKILL.md\")\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.contains(\"skills/gitrs/SKILL.md\")\n"),
            "got: {out}"
        );
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn method_same_literal_multi_arg_sweep_compresses() {
        // Arity 2 with hetero receivers: both arg positions share their
        // old/new tuples across sites (comma-joined emission).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.route(\"skills/rsmpl-git/SKILL.md\", true);\n-    b.route(\"skills/rsmpl-git/SKILL.md\", true);\n+    a.route(\"skills/gitrs/SKILL.md\", false);\n+    b.route(\"skills/gitrs/SKILL.md\", false);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    check(\n        a.route(\"skills/rsmpl-git/SKILL.md\", true),\n        b.route(\"skills/rsmpl-git/SKILL.md\", true),\n    );\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
        assert!(
            out.contains("- Receiver.route(\"skills/rsmpl-git/SKILL.md\", true)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.route(\"skills/gitrs/SKILL.md\", false)\n"),
            "got: {out}"
        );
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn method_same_literal_divergent_old_stays_passthrough() {
        // Old literals differ across sites: no shared old tuple.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.contains(\"skills/rsmpl-git/SKILL.md\");\n-    b.contains(\"skills/old/SKILL.md\");\n+    a.contains(\"skills/gitrs/SKILL.md\");\n+    b.contains(\"skills/gitrs/SKILL.md\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_divergent_new_stays_passthrough() {
        // New literals differ across sites: no shared new tuple.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.contains(\"skills/rsmpl-git/SKILL.md\");\n-    b.contains(\"skills/rsmpl-git/SKILL.md\");\n+    a.contains(\"skills/gitrs/SKILL.md\");\n+    b.contains(\"skills/new/SKILL.md\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_receiver_rewrite_stays_passthrough() {
        // Per-site receivers must match: a receiver delta is not a
        // literal-arg sweep (and m2m declines on hetero receivers too).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.contains(\"skills/rsmpl-git/SKILL.md\");\n-    b.contains(\"skills/rsmpl-git/SKILL.md\");\n+    x.contains(\"skills/rsmpl-git/SKILL.md\");\n+    y.contains(\"skills/rsmpl-git/SKILL.md\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_single_site_stays_passthrough() {
        // N >= 2 only: a lone pair costs more as a semantic hunk.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    a.contains(\"skills/rsmpl-git/SKILL.md\");\n+    a.contains(\"skills/gitrs/SKILL.md\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_short_fires_as_diet() {
        // WI-1e diet (`# sweep`, no `# expr Receiver` decl): the short
        // literal sweep now wins T375 (byte-guard flip pinned).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.f(\"x\");\n-    b.f(\"x\");\n+    a.f(\"y\");\n+    b.f(\"y\");\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("diet wins T375");
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn method_same_literal_turbofish_raises_literal_frame() {
        // WI-7: the method sweep still declines turbofish (it cannot
        // spell `::<u8>`), but the lone literal delta is independently
        // raisable — the turbofish method is unchanged frame, and the
        // bare-literal hunk real-applies byte-identical (verified).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo::<u8>(\"skills/rsmpl-git/SKILL.md\");\n-    b.foo::<u8>(\"skills/rsmpl-git/SKILL.md\");\n+    a.foo::<u8>(\"skills/gitrs/SKILL.md\");\n+    b.foo::<u8>(\"skills/gitrs/SKILL.md\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.foo::<u8>(\"skills/rsmpl-git/SKILL.md\");\n    b.foo::<u8>(\"skills/rsmpl-git/SKILL.md\");\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("literal_frame must fire");
        validated_live(&out, &pre);
        assert!(
            out.contains("- \"skills/rsmpl-git/SKILL.md\"\n"),
            "got: {out}"
        );
        assert!(out.contains("+ \"skills/gitrs/SKILL.md\"\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    a.foo::<u8>(\"skills/gitrs/SKILL.md\");\n    b.foo::<u8>(\"skills/gitrs/SKILL.md\");\n}\n",
            "real-apply must be byte-identical"
        );
    }

    #[test]
    fn method_same_literal_ref_receiver_raises_literal_frame() {
        // WI-7: `&`-led sides route to the general-expr matcher, never
        // the method-call shape (mirrors the m2m gate) — but the lone
        // literal delta is independently raisable as a bare-literal
        // hunk (verified byte-identical below).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.foo(\"skills/rsmpl-git/SKILL.md\");\n-    &b.foo(\"skills/rsmpl-git/SKILL.md\");\n+    &a.foo(\"skills/gitrs/SKILL.md\");\n+    &b.foo(\"skills/gitrs/SKILL.md\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    &a.foo(\"skills/rsmpl-git/SKILL.md\");\n    &b.foo(\"skills/rsmpl-git/SKILL.md\");\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("literal_frame must fire");
        validated_live(&out, &pre);
        assert!(
            out.contains("- \"skills/rsmpl-git/SKILL.md\"\n"),
            "got: {out}"
        );
        assert!(out.contains("+ \"skills/gitrs/SKILL.md\"\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    &a.foo(\"skills/gitrs/SKILL.md\");\n    &b.foo(\"skills/gitrs/SKILL.md\");\n}\n",
            "real-apply must be byte-identical"
        );
    }

    #[test]
    fn method_same_literal_macro_arg_falls_through_to_nested() {
        // WI-9b2-method-probe: an arg hiding a macro invocation is not a
        // literal: TokenStream-normalized macro text is unmatchable, so
        // the method sweep declines and the nested macro sweep claims the
        // pair (validated_live proves the fall-through applies cleanly).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.push(wrap!(1));\n-    b.push(wrap!(1));\n+    a.push(wrap!(2));\n+    b.push(wrap!(2));\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.push(wrap!(1));\n    b.push(wrap!(1));\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            !out.contains("calls_push_literal"),
            "method sweep must not claim, got: {out}"
        );
        assert!(out.contains("- wrap!(1)\n"), "nested must fire, got: {out}");
        assert!(out.contains("+ wrap!(2)\n"), "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_bare_and_nested_compresses() {
        // T359 shape: bare arg-position lines (trailing `,`) and a
        // `write(...)`-nested site share `&R.join("")` -> `R`.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    &dir.path().join(\"\"),\n-    &dir.path().join(\"\"),\n-    write(&dir.path().join(\"\"), \"a\");\n+    dir.path(),\n+    dir.path(),\n+    write(dir.path(), \"a\");\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    check(\n        &dir.path().join(\"\"),\n        &dir.path().join(\"\"),\n        write(&dir.path().join(\"\"), \"a\"),\n    );\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,3 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- &Receiver.join(\"\")\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_shared_nonempty_arg_compresses() {
        // Shared literal args are the design point, not a hazard:
        // `&d.join("repo")` -> `d` at two sites compresses.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    copy_tree(&a.join(\"repo\"), x);\n-    copy_tree(&b.join(\"repo\"), y);\n+    copy_tree(a, x);\n+    copy_tree(b, y);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    copy_tree(&a.join(\"repo\"), x);\n    copy_tree(&b.join(\"repo\"), y);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- &Receiver.join(\"repo\")\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_zero_arg_compresses() {
        // Empty arg list renders as `()` with no metavars.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    &alpha.shutdown();\n-    &beta.shutdown();\n-    &gamma.shutdown();\n+    alpha;\n+    beta;\n+    gamma;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    &alpha.shutdown();\n    &beta.shutdown();\n    &gamma.shutdown();\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- &Receiver.shutdown()\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_divergent_args_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join(\"\");\n-    &b.join(\"x\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_divergent_method_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join(\"\");\n-    &b.push(\"\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_plus_not_receiver_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join(\"\");\n-    &b.join(\"\");\n+    a;\n+    c;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join::<T>(\"\");\n-    &b.join::<T>(\"\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_mut_ref_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &mut a.join(\"\");\n-    &mut b.join(\"\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_single_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    &a.join(\"\");\n+    a;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_uppercase_fires_backticked() {
        // WI-7-upper: a bare `Receiver.Old(E0)` leaves plus-side `New`
        // unbound — both method idents ride backticks (engine-probed).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.Old(first);\n-    b.Old(second);\n+    a.New(first);\n+    b.New(second);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.Old(first);\n    b.Old(second);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- Receiver.`Old`(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.`New`(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("a.New(first)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn method_to_fn_uppercase_backticks_and_ignores_twin() {
        // WI-7-upper: a raw `Receiver.Old(E0)` silently rewrote an
        // `Other` twin (probed post `new_fn(a, first)` x2 with a bogus
        // `Old` binding). Backticked, the twin survives.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.Old(first);\n-    b.Old(second);\n+    new_function(a, first);\n+    new_function(b, second);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.Old(first);\n    b.Old(second);\n    c.Other(third);\n}\n",
        )]);
        let out = super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("uppercase method-to-function must compress");
        validated_live(&out, &pre);
        assert!(out.contains("- Receiver.`Old`(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("c.Other(third)"),
            "twin must survive, got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("new_function(a, first)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn method_to_fn_uppercase_plus_callee_backticked() {
        // WI-7-upper: an uppercase plus-side bare callee (`New`) is
        // unbound without backticks — `` `New`(Receiver, E0) `` applies.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(first);\n-    b.old(second);\n+    New(a, first);\n+    New(b, second);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.old(first);\n    b.old(second);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("+ `New`(Receiver, E0)\n"), "got: {out}");
    }

    #[test]
    fn method_same_literal_uppercase_fires_backticked() {
        // WI-7-upper: same hole class — a raw uppercase
        // `Receiver.Old(...)` binds `Old` and over-matches twins.
        // (The sweep replays cross-site-identical tuples concretely.)
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.Old(shared, false);\n-    b.Old(shared, false);\n+    a.Old(shared, true);\n+    b.Old(shared, true);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    a.Old(shared, false);\n    b.Old(shared, false);\n    c.Other(shared, false);\n}\n",
        )]);
        let out = super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("uppercase same-literal must compress");
        validated_live(&out, &pre);
        assert!(
            out.contains("- Receiver.`Old`(shared, false)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.`Old`(shared, true)\n"),
            "got: {out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("c.Other(shared, false)"),
            "twin must survive, got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn ref_strip_uppercase_fires_backticked() {
        // WI-7-upper: same hole class (probed bogus `Old` binding on
        // the raw shape) — backticks pin the literal.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.Old(\"\");\n-    &b.Old(\"\");\n+    a;\n+    b;\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    &a.Old(\"\");\n    &b.Old(\"\");\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- &Receiver.`Old`(\"\")\n"), "got: {out}");
    }

    #[test]
    fn wi7_method_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-2: unique single `a.old_method(1)` ->
        // `a.new_method(1)` raises to an allow-one hunk and real-applies
        // byte-identical.
        let pre = "fn m() {\n    a.old_method(1);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    a.old_method(1);\n+    a.new_method(1);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_method(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    a.new_method(1);\n}\n");
    }

    #[test]
    fn wi7_method_single_twin_vetoes() {
        // An untouched same-method twin anywhere vetoes the HOLED
        // template (totality); the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    a.old_method(1);\n    b.old_method(9);\n}\n";
        assert!(
            super::try_method_single(
                &["    a.old_method(1);"],
                &["    a.new_method(1);"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "holed twin must veto at the core"
        );
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    a.old_method(1);\n+    a.new_method(1);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("literal verbatim single must fire");
        assert!(out.contains("- a.old_method(1)\n"), "got: {out}");
        assert!(out.contains("+ a.new_method(1)\n"), "got: {out}");
    }

    #[test]
    fn wi7_method_single_different_arity_twin_keeps_firing() {
        // Totality keys on method+arity: a different-arity twin never
        // matches the emitted pattern and never vetoes.
        let pre = "fn m() {\n    a.old_method(1);\n    b.old_method(1, 2);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    a.old_method(1);\n+    a.new_method(1);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
    }

    #[test]
    fn wi7_method_single_no_pre_image_vetoes() {
        // No whole-file evidence -> no guessing (D3).
        assert!(
            super::try_method_single(
                &["    a.old_method(1);"],
                &["    a.new_method(1);"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi7_method_single_uppercase_fires_literal() {
        // Uppercase method idents ride backticks (literal), else the
        // engine would infer them as `expr` holes.
        let pre = "fn m() {\n    a.Old(first);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    a.Old(first);\n+    a.New(first);\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("uppercase single must fire");
        assert!(out.contains("- Receiver.`Old`(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.`New`(E0)\n"), "got: {out}");
    }
    #[test]
    fn wi7_m2f_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-3: unique single `a.old(1)` -> `new_fn(a, 1)`
        // raises to an allow-one hunk and real-applies byte-identical.
        let pre = "fn m() {\n    a.old(1);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    a.old(1);\n+    new_fn(a, 1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique m2f single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(
            !out.contains("# allow=many"),
            "allow-one, no allow=many: {out}"
        );
        assert!(!out.contains("# sweep"), "ranged single, no sweep: {out}");
        assert!(out.contains("- Receiver.old(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_fn(Receiver, E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    new_fn(a, 1);\n}\n");
    }

    #[test]
    fn wi7_m2f_single_twin_vetoes() {
        // An untouched same-method twin anywhere vetoes the HOLED
        // template (totality); the unique whole line raises as a
        // WI-6-slice-1 literal verbatim single (byte-probe verified).
        let pre = "fn m() {\n    a.old(1);\n    b.old(9);\n}\n";
        assert!(
            super::try_method_to_fn_single(
                &["    a.old(1);"],
                &["    new_fn(a, 1);"],
                "f.rs",
                Some(pre),
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "holed twin must veto at the core"
        );
        let raw = section("f.rs", "@@ -2 +2 @@\n-    a.old(1);\n+    new_fn(a, 1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("literal verbatim single must fire");
        assert!(out.contains("- a.old(1)\n"), "got: {out}");
        assert!(out.contains("+ new_fn(a, 1)\n"), "got: {out}");
    }

    #[test]
    fn wi7_m2f_single_different_arity_twin_keeps_firing() {
        // Totality keys on method+arity: a different-arity twin never
        // matches the emitted pattern and never vetoes.
        let pre = "fn m() {\n    a.old(1);\n    b.old(1, 2);\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    a.old(1);\n+    new_fn(a, 1);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("different-arity twin must not veto");
        assert!(out.contains("- Receiver.old(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_fn(Receiver, E0)\n"), "got: {out}");
    }

    #[test]
    fn wi7_m2f_single_no_pre_image_vetoes() {
        // No whole-file evidence -> no guessing (D3).
        assert!(
            super::try_method_to_fn_single(
                &["    a.old(1);"],
                &["    new_fn(a, 1);"],
                "f.rs",
                None,
                &[2],
                &mut Vec::new()
            )
            .is_none(),
            "None pre-image must veto"
        );
    }

    #[test]
    fn wi7_m2f_single_zero_arity_fires() {
        // Receiver-prepend with no args: the tail is the bare receiver.
        let pre = "fn m() {\n    a.old();\n}\n";
        let raw = section("f.rs", "@@ -2 +2 @@\n-    a.old();\n+    new_fn(a);\n");
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("zero-arity m2f single must fire");
        assert!(out.contains("- Receiver.old()\n"), "got: {out}");
        assert!(out.contains("+ new_fn(Receiver)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    new_fn(a);\n}\n");
    }

    #[test]
    fn wi7_m2f_single_uppercase_fires_literal() {
        // Uppercase method/fn idents ride backticks (literal), else the
        // engine would infer them as `expr` holes. Detector-level: the
        // precision guard prices bytes, the template proves the sites.
        let out = super::try_method_to_fn_single(
            &["    a.Old(first);"],
            &["    New(a, first);"],
            "f.rs",
            Some("fn m() {\n    a.Old(first);\n}\n"),
            &[2],
            &mut Vec::new(),
        )
        .expect("uppercase m2f single must fire");
        assert!(out.contains("- Receiver.`Old`(E0)\n"), "got: {out}");
        assert!(out.contains("+ `New`(Receiver, E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emitted patch must parse");
        let mut files = pre_map(&[("f.rs", "fn m() {\n    a.Old(first);\n}\n")]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "fn m() {\n    New(a, first);\n}\n");
    }

    fn msingle(minus: &str, plus: &str, pre: Option<&str>, changed: &[usize]) -> Option<String> {
        super::try_method_same_literal_single(
            &[minus],
            &[plus],
            "f.rs",
            pre,
            changed,
            &mut Vec::new(),
        )
    }

    #[test]
    fn wi13_method_literal_single_unique_fires_allow_one_and_round_trips() {
        // WI-7 slice-13: unique single method same-literal delta raises
        // to an allow-one hunk and real-applies byte-identical. Long
        // receiver names clear the precision guard (`body <= hunk + 16`:
        // the concrete tuple saves the receiver spelling minus the
        // `Receiver` hole, pricing the rule header); short shapes stay
        // ordinary by design (pinned by fixture 49, which never reaches
        // this core through the guard).
        let pre = "fn m() {\n    agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, false);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, false);\n+    agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, \"100644\");\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("unique method literal single must fire");
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        assert!(!out.contains("# sweep"), "ranged single: {out}");
        assert!(
            out.contains("- Receiver.configure(alpha_argument_1, first_value_foo, false)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.configure(alpha_argument_1, first_value_foo, \"100644\")\n"),
            "got: {out}"
        );
        validated_live(&out, &pre_map);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains(
                "agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, \"100644\");"
            ),
            "got: {}",
            files["f.rs"]
        );
        // T361: the applied tree must be byte-identical to the true
        // post-tree, not merely containing the new line.
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, \"100644\");\n}\n",
            "applied tree must equal post-tree byte-for-byte"
        );
        // Fixed point: re-diff of the applied tree is itself.
        let out2 =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("must fire");
        assert_eq!(out, out2, "fixed point");
    }

    #[test]
    fn wi13_method_literal_single_let_init_fires() {
        let pre = "fn m() {\n    let v = agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, false);\n}\n";
        let raw = section(
            "f.rs",
            "@@ -2 +2 @@\n-    let v = agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, false);\n+    let v = agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, \"100644\");\n",
        );
        let pre_map = pre_map(&[("f.rs", pre)]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("let-init method literal single must fire");
        assert!(
            out.contains("- Receiver.configure(alpha_argument_1, first_value_foo, false)\n"),
            "got: {out}"
        );
        assert!(!out.contains("# allow=many"), "allow-one: {out}");
        validated_live(&out, &pre_map);
        // T361: byte-identical post-tree through the `let`-init entry
        // (the emission stays the bare `Receiver.method(args)` pattern;
        // the engine rewrites the call sub-expression in place).
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre_map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "fn m() {\n    let v = agents_with_long_name_here.configure(alpha_argument_1, first_value_foo, \"100644\");\n}\n",
            "applied tree must equal post-tree byte-for-byte"
        );
    }

    #[test]
    fn wi13_method_literal_single_arity_one_fires() {
        // Arity 1 needs no zero-hole veto: `Receiver` is already the
        // hole, so `recv.set(false)` → `recv.set("x")` is a holed
        // template (unlike the bare-call k=0 verbatim shape).
        let out = msingle(
            "    agents_with_long_name_here.set(false);",
            "    agents_with_long_name_here.set(\"100644\");",
            Some("fn m() {\n    agents_with_long_name_here.set(false);\n}\n"),
            &[2],
        )
        .expect("arity-one method literal single must fire");
        assert!(out.contains("- Receiver.set(false)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.set(\"100644\")\n"), "got: {out}");
    }

    #[test]
    fn wi13_method_literal_single_twin_vetoes() {
        assert!(
            msingle(
                "    recv.configure(a, false);",
                "    recv.configure(a, \"100644\");",
                Some(
                    "fn m() {\n    recv.configure(a, false);\n    other.configure(a, false);\n}\n"
                ),
                &[2]
            )
            .is_none(),
            "same-method same-args twin must veto"
        );
    }

    #[test]
    fn wi13_method_literal_single_different_args_twin_fires() {
        // Totality keys on the exact old-args tuple: a same-method site
        // with different args never matches and never vetoes.
        let out = msingle(
            "    recv.configure(a, false);",
            "    recv.configure(a, \"100644\");",
            Some("fn m() {\n    recv.configure(a, false);\n    recv.configure(b, true);\n}\n"),
            &[2],
        )
        .expect("different-args twin must not veto");
        assert!(out.contains("- Receiver.configure(a, false)"), "got: {out}");
    }

    #[test]
    fn wi13_method_literal_single_method_and_receiver_deltas_decline() {
        assert!(
            msingle(
                "    recv.old_method(a, false);",
                "    recv.new_method(a, \"100644\");",
                Some("fn m() {\n    recv.old_method(a, false);\n}\n"),
                &[2]
            )
            .is_none(),
            "method delta must decline here"
        );
        assert!(
            msingle(
                "    old_recv.configure(a, false);",
                "    new_recv.configure(a, \"100644\");",
                Some("fn m() {\n    old_recv.configure(a, false);\n}\n"),
                &[2]
            )
            .is_none(),
            "receiver delta must decline here"
        );
    }

    #[test]
    fn wi13_method_literal_single_none_identity_macro_veto() {
        assert!(
            msingle(
                "    recv.configure(a, false);",
                "    recv.configure(a, \"100644\");",
                None,
                &[2]
            )
            .is_none(),
            "None pre-image must veto"
        );
        assert!(
            msingle(
                "    recv.configure(a, false);",
                "    recv.configure(a, false);",
                Some("fn m() {\n    recv.configure(a, false);\n}\n"),
                &[2]
            )
            .is_none(),
            "identity must veto"
        );
        assert!(
            msingle(
                "    recv.configure(m!(x), false);",
                "    recv.configure(m!(x), \"100644\");",
                Some("fn m() {\n    recv.configure(m!(x), false);\n}\n"),
                &[2]
            )
            .is_none(),
            "macro arg must veto"
        );
    }

    #[test]
    fn wi13_method_literal_single_uppercase_fires_literal() {
        let out = msingle(
            "    recv.OldMethod(first);",
            "    recv.OldMethod(second);",
            Some("fn m() {\n    recv.OldMethod(first);\n}\n"),
            &[2],
        )
        .expect("uppercase method literal single must fire");
        assert!(
            out.contains("- Receiver.`OldMethod`(first)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.`OldMethod`(second)\n"),
            "got: {out}"
        );
    }
}
