//! Qualified-path and turbofish call/method sweeps (WI-3). Split from
//! `super` per the D5 hygiene gate (`mod.rs` sits near 2000 lines).
//!
//! Two shapes, both infer-only (the engine already matches them —
//! probed green): qualified callee renames (`crate::old(x)` ->
//! `crate::new(x)`), and turbofish-carrying renames (bare, qualified,
//! and method calls with `::<...>`). Each N >= 2 with per-site
//! token-equal value args. Chain order is after the method sweeps and
//! before the bare-call sweep (tightest first, SPEC D.4).

use super::{ident_len, parse_call_expr, unique_rule};
use quote::ToTokens;

/// Canonical render of a qualified callee (`::a::b`, `crate::old_api`),
/// shared by the detector (old-path capture/emission) and totality
/// (pre-image counting) so the two agree by construction.
pub(crate) fn render_qualified(leading: bool, segs: &[String]) -> String {
    let mut out = String::new();
    if leading {
        out.push_str("::");
    }
    out.push_str(&segs.join("::"));
    out
}

/// A parsed qualified-call delta: ((leading `::`, prefix segments),
/// old last-segment, new last-segment, value arity).
type QualifiedDelta = ((bool, Vec<String>), String, String, usize);

/// A qualified callee: leading `::` flag, all segments, value args.
struct Qualified {
    leading: bool,
    segs: Vec<syn::PathSegment>,
    args: Vec<syn::Expr>,
}

/// Split an `Expr::Call` func path into its qualified parts; `None` for
/// method calls, qself (WI-6 resolver territory), bare single-segment
/// callees (the bare sweep owns those), and segment counts below 2.
fn qualified_parts(expr: &syn::Expr) -> Option<Qualified> {
    if let syn::Expr::Call(call) = expr {
        if let syn::Expr::Path(p) = call.func.as_ref() {
            if p.qself.is_some() || p.path.segments.len() < 2 {
                return None;
            }
            return Some(Qualified {
                leading: p.path.leading_colon.is_some(),
                segs: p.path.segments.iter().cloned().collect(),
                args: call.args.iter().cloned().collect(),
            });
        }
    }
    None
}

/// N-site (N >= 2) qualified-call rename: every pair parses as
/// `P::Old(args)` -> `P::New(args)` with a shared multi-segment prefix
/// (leading `::` shared verbatim), one shared old/new last-segment
/// delta, uniform arity, and per-site token-equal args. No generic args
/// on any segment (turbofish pairs belong to `try_turbofish_sweep`).
/// Emits one `# allow=many` hunk with the concrete path form (SPEC 19.3
/// prefers concrete paths; no `# symbol` tax).
pub(crate) fn try_qualified_call_sweep(
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
    let mut prefix: Option<(bool, Vec<String>)> = None;
    let mut old_cal: Option<String> = None;
    let mut new_cal: Option<String> = None;
    let mut arity: Option<usize> = None;
    let mut whole_ok = true;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some((pre, o, n, a)) = parse_qualified_pair(m, p) else {
            whole_ok = false;
            break;
        };
        if prefix.as_ref().is_some_and(|x| *x != pre) {
            return None;
        }
        if old_cal.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_cal.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        prefix = Some(pre);
        old_cal = Some(o);
        new_cal = Some(n);
        arity = Some(a);
    }
    if whole_ok {
        let ((leading, pre_segs), old_cal, new_cal, arity) = (prefix?, old_cal?, new_cal?, arity?);
        if old_cal == new_cal {
            return None;
        }
        let old_path = format!("{}{old_cal}", render_prefix(leading, &pre_segs));
        if !crate::totality::qualified_call_holds(file_old, &old_path, arity, minuses, changed) {
            return None;
        }
        return Some(emit_qualified_sweep(
            path, used_rules, leading, &pre_segs, &old_cal, &new_cal, arity, changed,
        ));
    }
    // WI-4 sub-line fallback (mixed whole/sub lines veto;
    // all-identical pairs defer to verbatim, T422).
    if super::subline::all_identical_pairs(minuses, pluses) {
        return None;
    }
    let mut sub_pre: Option<(bool, Vec<String>)> = None;
    let mut sub_old: Option<String> = None;
    let mut sub_new: Option<String> = None;
    let mut sub_arity: Option<usize> = None;
    let mut total = 0usize;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let sites = super::subline::extract_qual_sites(m, p)?;
        for s in &sites {
            if sub_pre.as_ref().is_some_and(|x| *x != s.prefix) {
                return None;
            }
            if sub_old.as_ref().is_some_and(|c| *c != s.old_cal) {
                return None;
            }
            if sub_new.as_ref().is_some_and(|c| *c != s.new_cal) {
                return None;
            }
            if sub_arity.is_some_and(|x| x != s.arity) {
                return None;
            }
            sub_pre = Some(s.prefix.clone());
            sub_old = Some(s.old_cal.clone());
            sub_new = Some(s.new_cal.clone());
            sub_arity = Some(s.arity);
            total += 1;
        }
    }
    let ((leading, pre_segs), old_cal, new_cal, arity) = (sub_pre?, sub_old?, sub_new?, sub_arity?);
    if old_cal == new_cal || total < 2 {
        return None;
    }
    let old_path = format!("{}{old_cal}", render_prefix(leading, &pre_segs));
    if !crate::totality::template_expr_holds(
        path,
        file_old,
        &format!("{old_path}({})", qual_args(arity)),
        total,
    ) {
        return None;
    }
    Some(emit_qualified_sweep(
        path, used_rules, leading, &pre_segs, &old_cal, &new_cal, arity, changed,
    ))
}

/// Shared emission for the whole-line and sub-line qualified paths
/// (identical hunks).
#[allow(clippy::too_many_arguments)]
fn emit_qualified_sweep(
    path: &str,
    used_rules: &mut Vec<String>,
    leading: bool,
    pre_segs: &[String],
    old_cal: &str,
    new_cal: &str,
    arity: usize,
    changed: &[usize],
) -> String {
    let rule = unique_rule(used_rules, &format!("calls_{old_cal}_to_{new_cal}"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    let mut vars = Vec::new();
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let new_path = format!("{}{new_cal}", render_prefix(leading, pre_segs));
    // WI-7-upper: an uppercase last segment infers as an `expr` hole
    // (bare `crate::Old(E0)` leaves plus-side `New` unbound) — backtick
    // the last segment only (full-path quotes are engine-rejected).
    // Lowercase paths pass through untouched (zero churn).
    let old_path = format!("{}{old_cal}", render_prefix(leading, pre_segs));
    let old_path = super::shape::literal_path_last(&old_path);
    let new_path = super::shape::literal_path_last(&new_path);
    hunk.push_str(&format!("- {old_path}({args})\n+ {new_path}({args})\n"));
    hunk
}

/// Comma-joined `E0…` arg list for the sub-line occurrence probe.
fn qual_args(arity: usize) -> String {
    (0..arity)
        .map(|i| format!("E{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render a shared prefix (leading `::` + segments + trailing `::`).
fn render_prefix(leading: bool, segs: &[String]) -> String {
    format!("{}::", render_qualified(leading, segs))
}

/// Parse a `-`/`+` line pair as a qualified-call rename; return
/// ((leading, prefix segments), old last-segment, new last-segment,
/// arity) when the delta is last-segment-only.
fn parse_qualified_pair(old_line: &str, new_line: &str) -> Option<QualifiedDelta> {
    let o = qualified_parts(&parse_call_expr(old_line)?)?;
    let n = qualified_parts(&parse_call_expr(new_line)?)?;
    // Leading `::` has no engine spelling (a `::p::Old(E0)` pattern
    // matches nothing — probed): decline, never emit.
    if o.leading || n.leading || o.leading != n.leading || o.segs.len() != n.segs.len() {
        return None;
    }
    // Shared prefix (all but last), token-equal and generic-free; the
    // last segment carries the delta, also generic-free.
    let mut pre = Vec::new();
    for (os, ns) in o.segs.iter().zip(n.segs.iter()).take(o.segs.len() - 1) {
        if os.to_token_stream().to_string() != ns.to_token_stream().to_string() {
            return None;
        }
        if !matches!(os.arguments, syn::PathArguments::None) {
            return None;
        }
        pre.push(os.ident.to_string());
    }
    let o_last = o.segs.last()?;
    let n_last = n.segs.last()?;
    if !matches!(o_last.arguments, syn::PathArguments::None)
        || !matches!(n_last.arguments, syn::PathArguments::None)
    {
        return None;
    }
    let old = o_last.ident.to_string();
    let new = n_last.ident.to_string();
    if ident_len(&old)? != old.len() || ident_len(&new)? != new.len() {
        return None;
    }
    if o.args.len() != n.args.len() {
        return None;
    }
    for (a, b) in o.args.iter().zip(n.args.iter()) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    Some(((o.leading, pre), old, new, o.args.len()))
}

/// One turbofish argument: token text plus whether it is a plain type
/// (only type positions may become the inferred `T` hole — a
/// lifetime/const hole is unexpressible, so those stay verbatim).
struct TurboArg {
    text: String,
    is_type: bool,
}

fn turbo_args(args: &syn::AngleBracketedGenericArguments) -> Vec<TurboArg> {
    args.args
        .iter()
        .map(|a| TurboArg {
            text: a.to_token_stream().to_string(),
            is_type: matches!(a, syn::GenericArgument::Type(_)),
        })
        .collect()
}

/// Last-segment turbofish of a call path (`::<...>` spelled with `::`;
/// `None` for turbofish-free or bare-`<>` segments — the emitted pattern
/// always uses the `::<>` form, so only `::` sites qualify).
fn last_turbo(seg: &syn::PathSegment) -> Option<&syn::AngleBracketedGenericArguments> {
    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
        if args.colon2_token.is_some() {
            return Some(args);
        }
    }
    None
}

/// One turbofish rename variant: bare call, qualified call (shared
/// prefix), or method call (receivers vary per site like the engine).
/// Derived `PartialEq` keeps the whole shape (variant + names + prefix)
/// shared across sites.
#[derive(Clone, PartialEq, Eq)]
enum TurboShape {
    BareCall {
        old: String,
        new: String,
    },
    QualCall {
        leading: bool,
        prefix: Vec<String>,
        old: String,
        new: String,
    },
    Method {
        old: String,
        new: String,
    },
}

impl TurboShape {
    /// Render `(minus pattern, plus pattern, rule tag)` for the decided
    /// turbo spelling (`u8` verbatim or `T` hole) and value arity.
    /// WI-7-upper: bare/qualified callee idents ride
    /// `literal_callee` (an uppercase `Old::<u8>` leaves plus-side `New`
    /// unbound); the Method arm stays plain — a turbofish-suffixed
    /// method ident never infers (the `::<` suffix defeats
    /// `is_candidate`, probed), so backticks would be pure churn.
    /// Prefixes stay verbatim (only the last segment can infer).
    fn render(&self, turbo: &str, arity: usize) -> (String, String, String) {
        let mut vars = Vec::new();
        for i in 0..arity {
            vars.push(format!("E{i}"));
        }
        let args = vars.join(", ");
        match self {
            TurboShape::BareCall { old, new } => (
                format!(
                    "{old}::<{turbo}>({args})",
                    old = super::shape::literal_callee(old)
                ),
                format!(
                    "{new}::<{turbo}>({args})",
                    new = super::shape::literal_callee(new)
                ),
                format!("{old}_to_{new}_turbo"),
            ),
            TurboShape::QualCall {
                leading,
                prefix,
                old,
                new,
            } => {
                let pre = render_prefix(*leading, prefix);
                (
                    format!(
                        "{pre}{old}::<{turbo}>({args})",
                        old = super::shape::literal_callee(old)
                    ),
                    format!(
                        "{pre}{new}::<{turbo}>({args})",
                        new = super::shape::literal_callee(new)
                    ),
                    format!("{old}_to_{new}_turbo"),
                )
            }
            TurboShape::Method { old, new } => (
                format!("Receiver.{old}::<{turbo}>({args})"),
                format!("Receiver.{new}::<{turbo}>({args})"),
                format!("{old}_to_{new}_turbo"),
            ),
        }
    }

    /// Pre-image totality for the variant: exact callee + value arity +
    /// turbo arity + concrete non-hole turbo texts. Non-turbofish twins
    /// never match the emitted pattern (R3, probed) so they never count;
    /// `&`-receivers count conservatively (the method pattern cannot
    /// match them, so an extra `&` site vetoes — safe direction).
    fn totality_holds(
        &self,
        file_old: Option<&str>,
        val_arity: usize,
        turbo_arity: usize,
        concrete: &[(usize, String)],
        minuses: &[&str],
        changed: &[usize],
    ) -> bool {
        match self {
            TurboShape::BareCall { old, .. } => crate::totality::turbofish_call_holds(
                file_old,
                old,
                val_arity,
                turbo_arity,
                concrete,
                minuses,
                changed,
            ),
            TurboShape::QualCall {
                leading,
                prefix,
                old,
                ..
            } => {
                let full = format!("{}::{old}", render_qualified(*leading, prefix));
                crate::totality::turbofish_call_holds(
                    file_old,
                    &full,
                    val_arity,
                    turbo_arity,
                    concrete,
                    minuses,
                    changed,
                )
            }
            TurboShape::Method { old, .. } => crate::totality::turbofish_method_holds(
                file_old,
                old,
                val_arity,
                turbo_arity,
                concrete,
                minuses,
                changed,
            ),
        }
    }
}

/// N-site (N >= 2) turbofish-carrying rename, one uniform variant per
/// hunk: bare calls (`foo::<u8>(x)` -> `bar::<u8>(x)`), qualified calls
/// (`crate::old::<u8>(x)` -> `crate::new::<u8>(x)`, same prefix rule as
/// the qualified sweep), or method calls (`recv.old::<u8>(x)` ->
/// `recv.new::<u8>(x)` with per-site token-equal receivers).
/// Value args token-equal per site with uniform value arity; turbofish
/// uniform turbo-arity with per-site minus token-equals plus; across
/// all 2N turbo lists at most one position varies — that position emits
/// an inferred `T` on both sides, the rest verbatim. A bare-without-
/// turbofish pattern never matches a turbofish site (R3 parity,
/// probed), so totality keys on the turbo shape exactly.
pub(crate) fn try_turbofish_sweep(
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
    let mut shape: Option<TurboShape> = None;
    let mut turbo_arity: Option<usize> = None;
    let mut val_arity: Option<usize> = None;
    let mut turbos: Vec<(Vec<TurboArg>, Vec<TurboArg>)> = Vec::new();
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (s, o_t, n_t, a) = parse_turbofish_pair(m, p)?;
        if o_t.len() != n_t.len() {
            return None;
        }
        if turbo_arity.is_some_and(|t| t != o_t.len()) {
            return None;
        }
        if val_arity.is_some_and(|v| v != a) {
            return None;
        }
        if shape.as_ref().is_some_and(|x| *x != s) {
            return None;
        }
        // Per-site minus turbo token-equals plus turbo (no turbofish
        // delta in v1 — that is unexpressible template territory).
        if o_t.iter().zip(n_t.iter()).any(|(a, b)| a.text != b.text) {
            return None;
        }
        shape = Some(s);
        turbo_arity = Some(o_t.len());
        val_arity = Some(a);
        turbos.push((o_t, n_t));
    }
    let (shape, turbo_arity, val_arity) = (shape?, turbo_arity?, val_arity?);
    if turbo_arity == 0 {
        return None;
    }
    // Across all 2N turbo lists, at most one position varies; a varying
    // position must be a type at every site (the `T` hole binds types).
    let mut hole: Option<usize> = None;
    for (i, first_arg) in turbos[0].0.iter().enumerate() {
        let first = first_arg.text.clone();
        if turbos
            .iter()
            .any(|(o, n)| o[i].text != first || n[i].text != first)
        {
            if hole.is_some() {
                return None;
            }
            if turbos.iter().any(|(o, n)| !o[i].is_type || !n[i].is_type) {
                return None;
            }
            hole = Some(i);
        }
    }
    let mut turbo_emit = Vec::new();
    let mut concrete: Vec<(usize, String)> = Vec::new();
    for (i, first_arg) in turbos[0].0.iter().enumerate().take(turbo_arity) {
        if hole == Some(i) {
            turbo_emit.push("T".to_string());
        } else {
            turbo_emit.push(first_arg.text.clone());
            concrete.push((i, first_arg.text.clone()));
        }
    }
    if !shape.totality_holds(
        file_old,
        val_arity,
        turbo_arity,
        &concrete,
        minuses,
        changed,
    ) {
        return None;
    }
    let (old_pat, new_pat, tag) = shape.render(&turbo_emit.join(", "), val_arity);
    let rule = unique_rule(used_rules, &format!("calls_{tag}"));
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {old_pat}\n+ {new_pat}\n"));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a turbofish-carrying rename; return
/// (variant shape, minus turbo args, plus turbo args, value arity).
/// `let`-init calls unwrap to their call (T449 precedent — the engine
/// rewrites the call sub-expression in place, probed for both the call
/// and method turbo shapes); `let-else` never qualifies. `&`-led sides
/// route to the general-expr matcher, never the call shapes.
fn parse_turbofish_pair(
    old_line: &str,
    new_line: &str,
) -> Option<(TurboShape, Vec<TurboArg>, Vec<TurboArg>, usize)> {
    if old_line.trim_start().starts_with('&') || new_line.trim_start().starts_with('&') {
        return None;
    }
    let old_expr = parse_call_expr(old_line)?;
    let new_expr = parse_call_expr(new_line)?;
    match (&old_expr, &new_expr) {
        (syn::Expr::Call(o_call), syn::Expr::Call(n_call)) => {
            parse_turbofish_call_pair(o_call, n_call)
        }
        (syn::Expr::MethodCall(o_call), syn::Expr::MethodCall(n_call)) => {
            parse_turbofish_method_pair(o_call, n_call)
        }
        _ => None,
    }
}

/// Bare/qualified turbofish call pair.
fn parse_turbofish_call_pair(
    o_call: &syn::ExprCall,
    n_call: &syn::ExprCall,
) -> Option<(TurboShape, Vec<TurboArg>, Vec<TurboArg>, usize)> {
    let (syn::Expr::Path(o_p), syn::Expr::Path(n_p)) = (o_call.func.as_ref(), n_call.func.as_ref())
    else {
        return None;
    };
    if o_p.qself.is_some() || n_p.qself.is_some() {
        return None;
    }
    if o_call.args.len() != n_call.args.len() {
        return None;
    }
    for (a, b) in o_call.args.iter().zip(n_call.args.iter()) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    let (o_segs, n_segs) = (&o_p.path.segments, &n_p.path.segments);
    if o_segs.len() != n_segs.len() || o_segs.is_empty() {
        return None;
    }
    if o_p.path.leading_colon != n_p.path.leading_colon {
        return None;
    }
    // Leading `::` has no engine spelling (probed) — decline here too.
    if o_p.path.leading_colon.is_some() {
        return None;
    }
    let o_last = o_segs.last()?;
    let n_last = n_segs.last()?;
    let o_t = turbo_args(last_turbo(o_last)?);
    let n_t = turbo_args(last_turbo(n_last)?);
    if o_segs.len() == 1 {
        let old = o_last.ident.to_string();
        let new = n_last.ident.to_string();
        if ident_len(&old)? != old.len() || ident_len(&new)? != new.len() || old == new {
            return None;
        }
        return Some((
            TurboShape::BareCall { old, new },
            o_t,
            n_t,
            o_call.args.len(),
        ));
    }
    // Qualified: shared prefix, generic-free, last-segment delta.
    let mut prefix = Vec::new();
    for (os, ns) in o_segs.iter().zip(n_segs.iter()).take(o_segs.len() - 1) {
        if os.to_token_stream().to_string() != ns.to_token_stream().to_string() {
            return None;
        }
        if !matches!(os.arguments, syn::PathArguments::None) {
            return None;
        }
        prefix.push(os.ident.to_string());
    }
    let old = o_last.ident.to_string();
    let new = n_last.ident.to_string();
    if ident_len(&old)? != old.len() || ident_len(&new)? != new.len() || old == new {
        return None;
    }
    Some((
        TurboShape::QualCall {
            leading: o_p.path.leading_colon.is_some(),
            prefix,
            old,
            new,
        },
        o_t,
        n_t,
        o_call.args.len(),
    ))
}

/// Method-call turbofish pair: per-site token-equal receivers, shared
/// old/new method idents, uniform arity.
fn parse_turbofish_method_pair(
    o_call: &syn::ExprMethodCall,
    n_call: &syn::ExprMethodCall,
) -> Option<(TurboShape, Vec<TurboArg>, Vec<TurboArg>, usize)> {
    let o_t = turbo_args(o_call.turbofish.as_ref()?);
    let n_t = turbo_args(n_call.turbofish.as_ref()?);
    if o_call.args.len() != n_call.args.len() {
        return None;
    }
    if o_call.receiver.to_token_stream().to_string()
        != n_call.receiver.to_token_stream().to_string()
    {
        return None;
    }
    for (a, b) in o_call.args.iter().zip(n_call.args.iter()) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    let old = o_call.method.to_string();
    let new = n_call.method.to_string();
    if ident_len(&old)? != old.len() || ident_len(&new)? != new.len() || old == new {
        return None;
    }
    Some((TurboShape::Method { old, new }, o_t, n_t, o_call.args.len()))
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
    fn qualified_call_sweep_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_handler_name(first_argument);\n-    crate::old_handler_name(second_argument);\n+    crate::new_handler_name(first_argument);\n+    crate::new_handler_name(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_handler_name(first_argument);\n    crate::old_handler_name(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(
            out.contains("- crate::old_handler_name(E0)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ crate::new_handler_name(E0)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn qualified_leading_colon_stays_passthrough() {
        // No engine spelling: a `::p::Old(E0)` pattern matches nothing
        // (probed `MATCH_CONFLICT`), so leading-`::` renames stay
        // ordinary rather than emitting an unmatchable hunk.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    ::framework::old_handler_name(first_argument);\n-    ::framework::old_handler_name(second_argument);\n+    ::framework::new_handler_name(first_argument);\n+    ::framework::new_handler_name(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn qualified_let_init_fires() {
        // T449 precedent: `let` inits unwrap to their call; the emission
        // stays a bare pattern and the engine rewrites in place.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_value = crate::old_handler_name(first_argument);\n-    let second_value = crate::old_handler_name(second_argument);\n+    let first_value = crate::new_handler_name(first_argument);\n+    let second_value = crate::new_handler_name(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let first_value = crate::old_handler_name(first_argument);\n    let second_value = crate::old_handler_name(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- crate::old_handler_name(E0)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn qualified_prefix_divergent_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_handler_name(first_argument);\n-    other::old_handler_name(second_argument);\n+    crate::new_handler_name(first_argument);\n+    other::new_handler_name(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn qualified_leading_colon_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    ::crate::old_handler_name(first_argument);\n-    crate::old_handler_name(second_argument);\n+    ::crate::new_handler_name(first_argument);\n+    crate::new_handler_name(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn qualified_turbofish_pair_stays_passthrough() {
        // Turbofish on the callee routes to the turbofish detector; a
        // turbo-arity-divergent pair defeats that too — fully ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_handler_name::<u8>(first_argument);\n-    crate::old_handler_name::<u8, u16>(second_argument);\n+    crate::new_handler_name::<u8>(first_argument);\n+    crate::new_handler_name::<u8, u16>(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn qualified_extra_site_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_handler_name(first_argument);\n-    crate::old_handler_name(second_argument);\n+    crate::new_handler_name(first_argument);\n+    crate::new_handler_name(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_handler_name(first_argument);\n    crate::old_handler_name(second_argument);\n    crate::old_handler_name(third_argument);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn qualified_bare_distractor_does_not_veto() {
        // R3/exact-path parity (probed): a bare `old_handler_name`
        // twin never matches the qualified pattern — it must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_handler_name(first_argument);\n-    crate::old_handler_name(second_argument);\n+    crate::new_handler_name(first_argument);\n+    crate::new_handler_name(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_handler_name(first_argument);\n    crate::old_handler_name(second_argument);\n    old_handler_name(bare_twin_argument);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn qualified_single_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    crate::old_handler_name(first_argument);\n+    crate::new_handler_name(first_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn qualified_qself_stays_passthrough() {
        // The engine leniently matches qself sites against concrete
        // patterns (probed) — the detector must not depend on that.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    <TargetType as ConversionTrait>::old_handler_name(first_argument);\n-    <TargetType as ConversionTrait>::old_handler_name(second_argument);\n+    <TargetType as ConversionTrait>::new_handler_name(first_argument);\n+    <TargetType as ConversionTrait>::new_handler_name(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn turbofish_bare_concrete_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<u8>(first_argument);\n-    old_deserializer_name::<u8>(second_argument);\n+    new_deserializer_name::<u8>(first_argument);\n+    new_deserializer_name::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_deserializer_name::<u8>(first_argument);\n    old_deserializer_name::<u8>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -2,2 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(
            out.contains("- old_deserializer_name::<u8>(E0)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ new_deserializer_name::<u8>(E0)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn turbofish_type_hole_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<u8>(first_argument);\n-    old_deserializer_name::<i32>(second_argument);\n+    new_deserializer_name::<u8>(first_argument);\n+    new_deserializer_name::<i32>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_deserializer_name::<u8>(first_argument);\n    old_deserializer_name::<i32>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- old_deserializer_name::<T>(E0)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ new_deserializer_name::<T>(E0)\n"),
            "got: {out}"
        );
        // The sweep must apply the per-site types, not the hole name.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("new_deserializer_name::<u8>(first_argument)"),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("new_deserializer_name::<i32>(second_argument)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn turbofish_method_hole_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    alpha.old_operation_name::<u8>(first_argument);\n-    beta.old_operation_name::<i32>(second_argument);\n+    alpha.new_operation_name::<u8>(first_argument);\n+    beta.new_operation_name::<i32>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    alpha.old_operation_name::<u8>(first_argument);\n    beta.old_operation_name::<i32>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- Receiver.old_operation_name::<T>(E0)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.new_operation_name::<T>(E0)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn turbofish_qualified_concrete_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::old_deserializer_name::<u8>(first_argument);\n-    crate::old_deserializer_name::<u8>(second_argument);\n+    crate::new_deserializer_name::<u8>(first_argument);\n+    crate::new_deserializer_name::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::old_deserializer_name::<u8>(first_argument);\n    crate::old_deserializer_name::<u8>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- crate::old_deserializer_name::<u8>(E0)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn turbofish_two_holes_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<u8, u16>(first_argument);\n-    old_deserializer_name::<i32, i64>(second_argument);\n+    new_deserializer_name::<u8, u16>(first_argument);\n+    new_deserializer_name::<i32, i64>(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn turbofish_per_site_turbo_delta_stays_passthrough() {
        // A per-site turbofish rewrite is not a rename (unexpressible
        // template territory) — the minus/plus turbo must agree per site.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<u8>(first_argument);\n-    old_deserializer_name::<u8>(second_argument);\n+    new_deserializer_name::<String>(first_argument);\n+    new_deserializer_name::<String>(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn turbofish_lifetime_hole_stays_passthrough() {
        // Only type positions may become the `T` hole; a varying
        // lifetime is unexpressible.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<'alpha>(first_argument);\n-    old_deserializer_name::<'beta>(second_argument);\n+    new_deserializer_name::<'alpha>(first_argument);\n+    new_deserializer_name::<'beta>(second_argument);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn turbofish_non_turbo_twin_does_not_veto() {
        // R3 parity (probed): a non-turbofish same-callee twin never
        // matches the `::<...>` pattern — it must not veto.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<u8>(first_argument);\n-    old_deserializer_name::<u8>(second_argument);\n+    new_deserializer_name::<u8>(first_argument);\n+    new_deserializer_name::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_deserializer_name::<u8>(first_argument);\n    old_deserializer_name::<u8>(second_argument);\n    old_deserializer_name(plain_twin_argument);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn turbofish_extra_site_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    old_deserializer_name::<u8>(first_argument);\n-    old_deserializer_name::<u8>(second_argument);\n+    new_deserializer_name::<u8>(first_argument);\n+    new_deserializer_name::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_deserializer_name::<u8>(first_argument);\n    old_deserializer_name::<u8>(second_argument);\n    old_deserializer_name::<u8>(third_argument);\n}\n",
        )]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn turbofish_let_wrapped_fires() {
        // T449 precedent (probed): `let` inits unwrap to their call and
        // the engine rewrites the call sub-expression in place.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = old_deserializer_name::<u8>(first_argument);\n-    let beta_value = old_deserializer_name::<i32>(second_argument);\n+    let alpha_value = new_deserializer_name::<u8>(first_argument);\n+    let beta_value = new_deserializer_name::<i32>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = old_deserializer_name::<u8>(first_argument);\n    let beta_value = old_deserializer_name::<i32>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- old_deserializer_name::<T>(E0)\n"),
            "got: {out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("new_deserializer_name::<i32>(second_argument)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn qualified_uppercase_fires_backticked() {
        // WI-7-upper: a bare `crate::Old(E0)` leaves plus-side `New`
        // unbound (diet infers the last segment as an `expr` hole) —
        // the last segment rides backticks (engine-probed spelling).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::Old(first_argument);\n-    crate::Old(second_argument);\n+    crate::New(first_argument);\n+    crate::New(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::Old(first_argument);\n    crate::Old(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- crate::`Old`(E0)\n"), "got: {out}");
        assert!(out.contains("+ crate::`New`(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("crate::New(first_argument)"),
            "got: {}",
            files["f.rs"]
        );
        assert!(
            files["f.rs"].contains("crate::New(second_argument)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn qualified_uppercase_prefix_only_stays_plain() {
        // WI-7-upper: only the last segment infers — an uppercase
        // prefix with a lowercase delta stays plain (probed: `MyMod`
        // binds nothing). Zero churn on the common path.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    MyMod::old_handler_name(first_argument);\n-    MyMod::old_handler_name(second_argument);\n+    MyMod::new_handler_name(first_argument);\n+    MyMod::new_handler_name(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    MyMod::old_handler_name(first_argument);\n    MyMod::old_handler_name(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- MyMod::old_handler_name(E0)\n"),
            "got: {out}"
        );
        assert!(!out.contains('`'), "no backticks expected, got: {out}");
    }

    #[test]
    fn turbofish_bare_uppercase_fires_backticked() {
        // WI-7-upper: bare `Old::<u8>(E0)` leaves plus-side `New`
        // unbound — the callee rides backticks (engine-probed).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    Old::<u8>(first_argument);\n-    Old::<u8>(second_argument);\n+    New::<u8>(first_argument);\n+    New::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    Old::<u8>(first_argument);\n    Old::<u8>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- `Old`::<u8>(E0)\n"), "got: {out}");
        assert!(out.contains("+ `New`::<u8>(E0)\n"), "got: {out}");
    }

    #[test]
    fn turbofish_qualified_uppercase_fires_backticked() {
        // WI-7-upper: qualified turbofish backticks the last segment
        // only (engine-probed: `crate::`Old`::<u8>(E0)` checks clean).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    crate::Old::<u8>(first_argument);\n-    crate::Old::<u8>(second_argument);\n+    crate::New::<u8>(first_argument);\n+    crate::New::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    crate::Old::<u8>(first_argument);\n    crate::Old::<u8>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- crate::`Old`::<u8>(E0)\n"), "got: {out}");
        assert!(out.contains("+ crate::`New`::<u8>(E0)\n"), "got: {out}");
    }

    #[test]
    fn turbofish_type_hole_uppercase_fires_backticked() {
        // WI-7-upper: the `T` hole still binds per-site types under a
        // backticked uppercase callee.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    Old::<u8>(first_argument);\n-    Old::<i32>(second_argument);\n+    New::<u8>(first_argument);\n+    New::<i32>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    Old::<u8>(first_argument);\n    Old::<i32>(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- `Old`::<T>(E0)\n"), "got: {out}");
        assert!(out.contains("+ `New`::<T>(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("New::<i32>(second_argument)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn turbofish_method_uppercase_stays_plain_and_exact() {
        // WI-7-upper: a turbofish-suffixed method ident never infers
        // (the `::<` suffix defeats `is_candidate`, probed — bindings
        // carry no `Old`), so the emission stays plain, and an `Other`
        // twin is never touched (the pattern matches literally).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    alpha.Old::<u8>(first_argument);\n-    beta.Old::<u8>(second_argument);\n+    alpha.New::<u8>(first_argument);\n+    beta.New::<u8>(second_argument);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    alpha.Old::<u8>(first_argument);\n    beta.Old::<u8>(second_argument);\n    gamma.Other::<u8>(twin_argument);\n}\n",
        )]);
        let out = super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("uppercase turbofish method must compress");
        validated_live(&out, &pre);
        assert!(!out.contains('`'), "no backticks expected, got: {out}");
        assert!(out.contains("- Receiver.Old::<u8>(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(
            files["f.rs"].contains("gamma.Other::<u8>(twin_argument)"),
            "twin must survive, got: {}",
            files["f.rs"]
        );
    }
}
