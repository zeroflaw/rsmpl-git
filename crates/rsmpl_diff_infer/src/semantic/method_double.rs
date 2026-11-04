//! Method+prefix holed double-delta composition (WI-13 residue slice C).
//!
//! A method-call line carrying two deltas that share one rename —
//! `old_mod::obj.old_m(1);` → `new_mod::obj.new_m(1);` — compresses to
//! one holed hunk (`- old_mod::obj.old_m(E0)` /
//! `+ new_mod::obj.new_m(E0)`). The engine already applies the concrete
//! joint (probed); the raiser left N=1 as a concrete literal single
//! and N≥2 same-class as N concrete singles. Both totalities ride
//! `template_pair_counts` + `engine_verdict` (the concrete pair proves
//! its own site set), N=1 adds the T361 byte-compare
//! (`single_reproduces`). Detector name `call_sweep` (WI-13 residue
//! family, same as the N=1/N≥2 qualifier doubles + typed-let joint).
//!
//! Targeted: receiver-prefix rename + method rename only (receiver base
//! same, exactly one differing receiver prefix segment, token-equal
//! args per pair). Receiver-only or method-only single deltas decline
//! here (verbatim / method-to-method own them — no bypass);
//! `&`-receivers, leading `::`, `qself`, turbofish, generics,
//! base renames, multi-delta receivers, and bare receivers stay
//! ordinary (future slices).

use super::prov::Counted;
use super::{ident_len, strip_one_terminator, unique_rule};
use quote::ToTokens;

/// One parsed method double-delta pair: shared old/new receiver prefix
/// vectors are carried per site; the joint keeps them whole.
struct MethodDouble {
    old_recv: Vec<String>,
    new_recv: Vec<String>,
    old_method: String,
    new_method: String,
    arity: usize,
    old_raw: String,
    new_raw: String,
}

/// Whole-line or `let`-init method call (T449 parity): one trailing
/// `;`/`,` tolerated, `&`-led sides route to general-expr (never
/// claimed here). Returns the call with its span text intact.
fn as_method_call(line: &str) -> Option<syn::ExprMethodCall> {
    let text = strip_one_terminator(line.trim())?;
    if text.is_empty() || text.starts_with('&') {
        return None;
    }
    if let Ok(syn::Expr::MethodCall(call)) = syn::parse_str::<syn::Expr>(text) {
        return Some(call);
    }
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(line.trim()) {
        let init = local.init?;
        if init.diverge.is_some() {
            return None;
        }
        if let syn::Expr::MethodCall(call) = *init.expr {
            return Some(call);
        }
    }
    None
}

/// Receiver path segments for a generic-free path receiver (no
/// leading `::`, no `qself`, ≥2 segments). `None` for any other
/// receiver shape (bare idents, field accesses, calls — their
/// families' territory).
fn receiver_parts(call: &syn::ExprMethodCall) -> Option<Vec<String>> {
    let syn::Expr::Path(rp) = call.receiver.as_ref() else {
        return None;
    };
    if rp.qself.is_some() || rp.path.leading_colon.is_some() {
        return None;
    }
    if rp.path.segments.len() < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(rp.path.segments.len());
    for seg in rp.path.segments.iter() {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return None;
        }
        let name = seg.ident.to_string();
        if ident_len(&name)? != name.len() {
            return None;
        }
        out.push(name);
    }
    Some(out)
}

/// Parse a `-`/`+` line pair as a method double delta: both sides
/// parse (whole-line or `let`-init), turbofish-free, same receiver
/// segment count with the same base and exactly one differing prefix
/// segment, a real method delta, uniform arity with token-equal args
/// per pair. Single-delta pairs (receiver-only, method-only) decline
/// here — the verbatim / method-to-method floors own them.
fn parse_method_double_pair(minus: &str, plus: &str) -> Option<MethodDouble> {
    let o = as_method_call(minus)?;
    let n = as_method_call(plus)?;
    if o.turbofish.is_some() || n.turbofish.is_some() {
        return None;
    }
    let o_recv = receiver_parts(&o)?;
    let n_recv = receiver_parts(&n)?;
    if o_recv.len() != n_recv.len() {
        return None;
    }
    let (o_base, o_pre) = o_recv.split_last()?;
    let (n_base, n_pre) = n_recv.split_last()?;
    if o_base != n_base {
        return None;
    }
    let mut deltas = 0usize;
    for (a, b) in o_pre.iter().zip(n_pre.iter()) {
        if a != b {
            deltas += 1;
        }
    }
    if deltas != 1 {
        return None;
    }
    let old_method = o.method.to_string();
    let new_method = n.method.to_string();
    if ident_len(&old_method)? != old_method.len()
        || ident_len(&new_method)? != new_method.len()
        || old_method == new_method
    {
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
    Some(MethodDouble {
        old_recv: o_recv,
        new_recv: n_recv,
        old_method,
        new_method,
        arity: o.args.len(),
        old_raw: o.to_token_stream().to_string(),
        new_raw: n.to_token_stream().to_string(),
    })
}

/// Holed templates for the joint (`E0…` args, `literal_callee` on the
/// method per WI-7-upper, receiver raw — totality proves the concrete
/// receiver pair, so no bypass through over-match).
fn joint_templates(d: &MethodDouble, arity: usize) -> (String, String, String, String) {
    let mut vars = Vec::with_capacity(arity);
    for i in 0..arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let old_recv = d.old_recv.join("::");
    let new_recv = d.new_recv.join("::");
    let raw_minus = format!("{old_recv}.{}({args})", d.old_method);
    let raw_plus = format!("{new_recv}.{}({args})", d.new_method);
    let minus_tpl = format!(
        "{old_recv}.{}({args})",
        super::shape::literal_callee(&d.old_method)
    );
    let plus_tpl = format!(
        "{new_recv}.{}({args})",
        super::shape::literal_callee(&d.new_method)
    );
    (raw_minus, raw_plus, minus_tpl, plus_tpl)
}

/// WI-13 residue slice C: N=1 method+prefix double-delta single under
/// `PrecisionUnique` (allow-one, directive-free). Fires iff the pair
/// parses as a double delta, the frame rule holds over the single
/// call subterm, the re-parse guard passes (TOOL-5), and
/// `template_pair_counts` + `engine_verdict` prove want=1 with the
/// T361 byte-compare reproducing the line exactly.
pub(crate) fn try_method_double_single(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() != 1 || pluses.len() != 1 || changed.len() != 1 {
        return Counted::Miss;
    }
    let Some(pre) = file_old else {
        return Counted::Miss;
    };
    let Some(d) = parse_method_double_pair(minuses[0], pluses[0]) else {
        return Counted::Miss;
    };
    if !super::subline::frame_holds(
        minuses[0],
        pluses[0],
        &[(d.old_raw.clone(), d.new_raw.clone())],
    ) {
        return Counted::Miss;
    }
    let (raw_minus, raw_plus, minus_tpl, plus_tpl) = joint_templates(&d, d.arity);
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return Counted::Miss;
    }
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), &minus_tpl, &plus_tpl),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    if !super::general_expr::single_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &minus_tpl, &plus_tpl,
    ) {
        return Counted::Miss;
    }
    let rule = unique_rule(
        used_rules,
        &format!("calls_{}_to_{}", d.old_method, d.new_method),
    );
    let mut hunk = crate::emission_header(path, &rule, changed).replace("# allow=many\n", "");
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Counted::Fire(hunk)
}

/// WI-13 residue slice C: N≥2 shared-class method+prefix double-delta
/// sweep under `call_sweep` (Strict). Fires iff every pair parses via
/// `parse_method_double_pair` with one shared class (old/new receiver
/// vectors, old/new method, uniform arity), the per-pair frame rule
/// holds, the re-parse guard passes, and `template_pair_counts` +
/// `engine_verdict` prove want=N.
pub(crate) fn try_method_double_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Counted<String> {
    if minuses.len() < 2 || minuses.len() != pluses.len() || changed.len() != minuses.len() {
        return Counted::Miss;
    }
    let Some(pre) = file_old else {
        return Counted::Miss;
    };
    let mut first: Option<MethodDouble> = None;
    let mut arity: Option<usize> = None;
    let mut raws: Vec<(String, String)> = Vec::with_capacity(minuses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let Some(d) = parse_method_double_pair(m, p) else {
            return Counted::Miss;
        };
        if let Some(f) = first.as_ref() {
            if f.old_recv != d.old_recv
                || f.new_recv != d.new_recv
                || f.old_method != d.old_method
                || f.new_method != d.new_method
            {
                return Counted::Miss;
            }
        }
        if arity.is_some_and(|x| x != d.arity) {
            return Counted::Miss;
        }
        arity = Some(d.arity);
        raws.push((d.old_raw.clone(), d.new_raw.clone()));
        first = Some(d);
    }
    let Some(f) = first else {
        return Counted::Miss;
    };
    for ((m, p), (om, nm)) in minuses.iter().zip(pluses.iter()).zip(raws.iter()) {
        if !super::subline::frame_holds(m, p, &[(om.clone(), nm.clone())]) {
            return Counted::Miss;
        }
    }
    let (raw_minus, raw_plus, minus_tpl, plus_tpl) = joint_templates(&f, f.arity);
    if syn::parse_str::<syn::Expr>(&raw_minus).is_err()
        || syn::parse_str::<syn::Expr>(&raw_plus).is_err()
    {
        return Counted::Miss;
    }
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), &minus_tpl, &plus_tpl),
        minuses.len(),
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    let rule = unique_rule(
        used_rules,
        &format!("calls_{}_to_{}", f.old_method, f.new_method),
    );
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Counted::Fire(hunk)
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use std::collections::HashMap;

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
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("emitted patch must apply");
    }

    const PRE1: &str = "fn m() {\n    old_mod::obj.old_m(1);\n}\n";

    #[test]
    fn single_fires_holed() {
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    old_mod::obj.old_m(1);\n+    new_mod::obj.new_m(1);\n";
        let pre = pre_map(&[("f.rs", PRE1)]);
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre)
            .expect("method double single must fire");
        validated_live(&out, &pre);
        assert!(out.contains("- old_mod::obj.old_m(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_mod::obj.new_m(E0)\n"), "got: {out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("new_mod::obj.new_m(1);"));
        assert!(!files["f.rs"].contains("old_mod"));
    }

    #[test]
    fn sweep_fires_one_holed_hunk() {
        let pre = "fn m() {\n    old_mod::obj.old_m(1);\n    old_mod::obj.old_m(2);\n}\n";
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,2 +2,2 @@\n-    old_mod::obj.old_m(1);\n-    old_mod::obj.old_m(2);\n+    new_mod::obj.new_m(1);\n+    new_mod::obj.new_m(2);\n";
        let map = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &map)
            .expect("method double sweep must fire");
        validated_live(&out, &map);
        assert_eq!(out.matches("rule=").count(), 1, "one rule: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- old_mod::obj.old_m(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_mod::obj.new_m(E0)\n"), "got: {out}");
    }

    #[test]
    fn receiver_only_declines() {
        // Single-delta (method same) is the m2m/verbatim floor, never
        // claimed here.
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    old_mod::obj.old_m(1);\n+    new_mod::obj.old_m(1);\n";
        let pre = pre_map(&[("f.rs", "fn m() {\n    old_mod::obj.old_m(1);\n}\n")]);
        let mut used = Vec::new();
        let out = super::try_method_double_single(
            &["    old_mod::obj.old_m(1);"],
            &["    new_mod::obj.old_m(1);"],
            "f.rs",
            Some("fn m() {\n    old_mod::obj.old_m(1);\n}\n"),
            &[2],
            &mut used,
        );
        assert!(
            matches!(out, super::super::prov::Counted::Miss),
            "receiver-only must decline here, got: {out:?}"
        );
        let _ = (raw, pre);
    }

    #[test]
    fn method_only_declines() {
        let mut used = Vec::new();
        let out = super::try_method_double_single(
            &["    old_mod::obj.old_m(1);"],
            &["    old_mod::obj.new_m(1);"],
            "f.rs",
            Some(PRE1),
            &[2],
            &mut used,
        );
        assert!(
            matches!(out, super::super::prov::Counted::Miss),
            "method-only must decline here, got: {out:?}"
        );
    }

    #[test]
    fn base_changing_declines() {
        let mut used = Vec::new();
        let out = super::try_method_double_single(
            &["    old_mod::obj.old_m(1);"],
            &["    new_mod::other.new_m(1);"],
            "f.rs",
            Some(PRE1),
            &[2],
            &mut used,
        );
        assert!(
            matches!(out, super::super::prov::Counted::Miss),
            "base rename is a future slice, got: {out:?}"
        );
    }

    #[test]
    fn extra_twin_vetoes_counted() {
        let pre = "fn m() {\n    old_mod::obj.old_m(1);\n    old_mod::obj.old_m(9);\n}\n";
        let mut used = Vec::new();
        let out = super::try_method_double_single(
            &["    old_mod::obj.old_m(1);"],
            &["    new_mod::obj.new_m(1);"],
            "f.rs",
            Some(pre),
            &[2],
            &mut used,
        );
        assert!(
            matches!(
                out,
                super::super::prov::Counted::Totality { found: 2, want: 1 }
            ),
            "twin must count, got: {out:?}"
        );
    }
}
