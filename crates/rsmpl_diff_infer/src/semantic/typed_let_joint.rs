//! Typed-`let` type+call shared-prefix joint (WI-13 residue slice B).
//!
//! A single typed-`let` line carrying two deltas that share one prefix
//! rename — `let w: oldmod::Foo = oldmod::bar(2);` →
//! `let w: newmod::Foo = newmod::bar(2);` — compresses to a two-hunk
//! one-rule joint (factored per WI-8: first hunk carries `rule=`,
//! second rides bare). The engine already applies each half alone and
//! the joint together (probed); the raiser never minted it, so the
//! line stayed ordinary. Both totalities want=1 (type via
//! `template_type_counts`, call via `template_pair_counts` +
//! `engine_verdict`) plus the D13 composed scratch-apply (both hunks
//! reproduce the plus line byte-identical). Detector name `call_sweep`
//! (WI-13 residue family, same as the N=1/N≥2 double-delta singles).
//!
//! Targeted: prefix rename only (`Ty` same, callee same, token-equal
//! args, token-equal binder pattern). `Ty`/callee renames, leading
//! `::`, `qself`, generics/turbofish, and multi-delta prefixes stay
//! ordinary (future slices).

use super::prov::Counted;
use super::{ident_len, unique_rule};
use quote::ToTokens;

/// Parsed shared-prefix joint: old/new prefix vectors, shared `Ty`,
/// shared callee, arity, and the four template texts.
struct TypedJoint {
    old_prefix: Vec<String>,
    new_prefix: Vec<String>,
    ty: String,
    callee: String,
    arity: usize,
}

/// Path-type segments for a generic-free `P::Ty` (no leading, no qself).
fn type_path_parts(ty: &syn::Type) -> Option<Vec<String>> {
    let syn::Type::Path(p) = ty else {
        return None;
    };
    if p.qself.is_some() || p.path.leading_colon.is_some() {
        return None;
    }
    if p.path.segments.is_empty() || p.path.segments.len() < 2 {
        return None;
    }
    for seg in p.path.segments.iter() {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return None;
        }
        let name = seg.ident.to_string();
        if ident_len(&name)? != name.len() {
            return None;
        }
    }
    Some(
        p.path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect(),
    )
}

/// Qualified call parts for a generic-free `P::callee(args)` (no leading,
/// no qself, no turbofish). Returns prefix segments, callee, args.
fn qual_call_parts(expr: &syn::Expr) -> Option<(Vec<String>, String, Vec<syn::Expr>)> {
    let syn::Expr::Call(call) = expr else {
        return None;
    };
    let syn::Expr::Path(p) = call.func.as_ref() else {
        return None;
    };
    if p.qself.is_some() || p.path.leading_colon.is_some() {
        return None;
    }
    if p.path.segments.len() < 2 {
        return None;
    }
    for seg in p.path.segments.iter() {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return None;
        }
        let name = seg.ident.to_string();
        if ident_len(&name)? != name.len() {
            return None;
        }
    }
    let segs: Vec<String> = p
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    let callee = segs.last()?.clone();
    let prefix = segs[..segs.len() - 1].to_vec();
    Some((prefix, callee, call.args.iter().cloned().collect()))
}

/// Parse a minus/plus typed-`let` line pair as a shared-prefix joint.
/// `None` for any non-conforming shape (pattern delta, `Ty`/callee
/// delta, arg delta, leading/qself/generics, prefix arity mismatch,
///
/// multi-delta prefixes).
fn parse_typed_joint(minus: &str, plus: &str) -> Option<TypedJoint> {
    let (m_pat, m_ty, m_init) = super::antiunify::parse_typed_let_full(minus)?;
    let (p_pat, p_ty, p_init) = super::antiunify::parse_typed_let_full(plus)?;
    if m_pat.to_token_stream().to_string() != p_pat.to_token_stream().to_string() {
        return None;
    }
    let m_ty_segs = type_path_parts(&m_ty)?;
    let p_ty_segs = type_path_parts(&p_ty)?;
    if m_ty_segs.len() != p_ty_segs.len() {
        return None;
    }
    let (m_pre, m_ty_name) = (
        m_ty_segs[..m_ty_segs.len() - 1].to_vec(),
        m_ty_segs.last()?.clone(),
    );
    let (p_pre, p_ty_name) = (
        p_ty_segs[..p_ty_segs.len() - 1].to_vec(),
        p_ty_segs.last()?.clone(),
    );
    if m_ty_name != p_ty_name {
        return None;
    }
    let (m_cpre, m_cal, m_args) = qual_call_parts(&m_init)?;
    let (p_cpre, p_cal, p_args) = qual_call_parts(&p_init)?;
    if m_cal != p_cal {
        return None;
    }
    if m_args.len() != p_args.len() {
        return None;
    }
    for (a, b) in m_args.iter().zip(p_args.iter()) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    // Shared-prefix law: the type prefix and the call prefix carry the
    // same old→new rename (same vectors on both positions), with
    // exactly one differing prefix segment.
    if m_pre != m_cpre || p_pre != p_cpre {
        return None;
    }
    if m_pre.len() != p_pre.len() || m_pre.is_empty() {
        return None;
    }
    let mut deltas = 0usize;
    for (a, b) in m_pre.iter().zip(p_pre.iter()) {
        if a != b {
            deltas += 1;
        }
    }
    if deltas != 1 {
        return None;
    }
    if m_pre == p_pre {
        return None;
    }
    Some(TypedJoint {
        old_prefix: m_pre,
        new_prefix: p_pre,
        ty: m_ty_name,
        callee: m_cal,
        arity: m_args.len(),
    })
}

/// D13 composed probe for the joint: the two-hunk probe emission must
/// apply with one match per hunk and reproduce the plus line
/// byte-identical (two-pair form of `single_reproduces`).
#[allow(clippy::too_many_arguments)]
fn joint_reproduces(
    path: &str,
    pre: &str,
    line_no: usize,
    minus_line: &str,
    plus_line: &str,
    type_old: &str,
    type_new: &str,
    call_old: &str,
    call_new: &str,
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
        "{}\n@@ -{line_no},1 @@ rule=probe\n- {type_old}\n+ {type_new}\n@@ -{line_no},1 @@ rule=probe\n- {call_old}\n+ {call_new}\n",
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
    report.rules.iter().map(|r| r.matches).sum::<usize>() == 2
        && files.get(path).is_some_and(|got| *got == expected)
}

/// N=1 typed-`let` shared-prefix joint under `call_sweep` (precision
/// single posture: uniqueness at n=1 per template plus the D13
/// composed byte-compare). Fires iff the pair parses as a shared
/// prefix rename, the type template is file-unique, the call template
/// proves at want=1, and the composed probe reproduces the line.
/// Emission is the factored two-hunk one-rule joint (type first =
/// source order, second hunk bare per WI-8 diet).
pub(crate) fn try_typed_let_joint_single(
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
    let Some(j) = parse_typed_joint(minuses[0], pluses[0]) else {
        return Counted::Miss;
    };
    let old_prefix = j.old_prefix.join("::");
    let new_prefix = j.new_prefix.join("::");
    let type_old = format!("{old_prefix}::{}", j.ty);
    let type_new = format!("{new_prefix}::{}", j.ty);
    let mut vars = Vec::with_capacity(j.arity);
    for i in 0..j.arity {
        vars.push(format!("E{i}"));
    }
    let args = vars.join(", ");
    let call_old = format!("{old_prefix}::{}({args})", j.callee);
    let call_new = format!("{new_prefix}::{}({args})", j.callee);
    // Defensive: emitted templates must re-parse (TOOL-5 precedent).
    if syn::parse_str::<syn::Type>(&type_old).is_err()
        || syn::parse_str::<syn::Type>(&type_new).is_err()
        || syn::parse_str::<syn::Expr>(&call_old).is_err()
        || syn::parse_str::<syn::Expr>(&call_new).is_err()
    {
        return Counted::Miss;
    }
    // WI-4 counted decline: the shape matched, so a failing probe names
    // its counts (`totality:`) — type first (verbatim-floor owner),
    // then the call.
    match crate::totality::template_type_counts(path, Some(pre), &type_old) {
        Some(1) => {}
        Some(found) => return Counted::Totality { found, want: 1 },
        None => return Counted::Miss,
    }
    match crate::totality_counted::engine_verdict(
        Some(pre),
        crate::totality_counted::template_pair_counts(path, Some(pre), &call_old, &call_new),
        1,
    ) {
        crate::totality_counted::FamilyVerdict::Holds => {}
        crate::totality_counted::FamilyVerdict::CountMismatch { found, want } => {
            return Counted::Totality { found, want };
        }
        crate::totality_counted::FamilyVerdict::OpaqueHit => return Counted::Opaque,
        crate::totality_counted::FamilyVerdict::Unproven => return Counted::Miss,
    }
    // D13: the composed joint must reproduce the native swap exactly.
    if !joint_reproduces(
        path, pre, changed[0], minuses[0], pluses[0], &type_old, &type_new, &call_old, &call_new,
    ) {
        return Counted::Miss;
    }
    if rsmpl_patch::parse_patch(&format!(
        "{}\n@@ -{},1 @@ rule=probe\n- {type_old}\n+ {type_new}\n",
        crate::git_header(path),
        changed[0]
    ))
    .is_err()
    {
        return Counted::Miss;
    }
    let rule = unique_rule(used_rules, "typed_let_joint");
    // Factored: first hunk carries the rule, second rides bare (WI-8).
    let hunk = format!(
        "{}\n@@ -{},1 @@ rule={rule}\n- {type_old}\n+ {type_new}\n@@ -{},1 @@\n- {call_old}\n+ {call_new}\n",
        crate::git_header(path),
        changed[0],
        changed[0]
    );
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

    const PRE: &str = "fn m() {\n    let w: oldmod::Foo = oldmod::bar(2);\n}\n";

    #[test]
    fn shared_prefix_joint_fires_factored() {
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    let w: oldmod::Foo = oldmod::bar(2);\n+    let w: newmod::Foo = newmod::bar(2);\n";
        let pre = pre_map(&[("f.rs", PRE)]);
        let out = infer_semantic_with_pre_images(&[raw.to_string()], &pre)
            .expect("shared-prefix joint must fire");
        validated_live(&out, &pre);
        assert_eq!(
            out.matches("rule=").count(),
            1,
            "one rule, factored second: {out}"
        );
        assert!(out.contains("- oldmod::Foo\n"), "got: {out}");
        assert!(out.contains("+ newmod::Foo\n"), "got: {out}");
        assert!(out.contains("- oldmod::bar(E0)\n"), "got: {out}");
        assert!(out.contains("+ newmod::bar(E0)\n"), "got: {out}");
        // Byte-identical re-apply.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert!(files["f.rs"].contains("let w: newmod::Foo = newmod::bar(2);"));
        assert!(!files["f.rs"].contains("oldmod"));
    }

    #[test]
    fn differing_prefix_declines() {
        // Type and call prefixes differ — not one shared rename.
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    let w: oldmod::Foo = othermod::bar(2);\n+    let w: newmod::Foo = newmod::bar(2);\n";
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let w: oldmod::Foo = othermod::bar(2);\n}\n",
        )]);
        assert!(
            infer_semantic_with_pre_images(&[raw.to_string()], &pre).is_none(),
            "differing prefixes must stay ordinary"
        );
    }

    #[test]
    fn ty_changing_declines() {
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    let w: oldmod::Foo = oldmod::bar(2);\n+    let w: newmod::Qux = newmod::bar(2);\n";
        let pre = pre_map(&[("f.rs", PRE)]);
        assert!(
            infer_semantic_with_pre_images(&[raw.to_string()], &pre).is_none(),
            "Ty rename is a future slice"
        );
    }

    #[test]
    fn callee_changing_declines() {
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    let w: oldmod::Foo = oldmod::bar(2);\n+    let w: newmod::Foo = newmod::qux(2);\n";
        let pre = pre_map(&[("f.rs", PRE)]);
        assert!(
            infer_semantic_with_pre_images(&[raw.to_string()], &pre).is_none(),
            "callee rename is a future slice"
        );
    }

    #[test]
    fn extra_type_twin_vetoes_counted() {
        // A second `oldmod::Foo` type elsewhere vetoes with totality 2:1.
        let pre = "fn m() {\n    let w: oldmod::Foo = oldmod::bar(2);\n    let v: oldmod::Foo = done();\n}\n";
        let raw = "diff --git a/f.rs b/f.rs\n@@ -2,1 +2,1 @@\n-    let w: oldmod::Foo = oldmod::bar(2);\n+    let w: newmod::Foo = newmod::bar(2);\n";
        let mut used = Vec::new();
        let out = super::try_typed_let_joint_single(
            &["    let w: oldmod::Foo = oldmod::bar(2);"],
            &["    let w: newmod::Foo = newmod::bar(2);"],
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
            "type twin must count, got: {out:?}"
        );
        let _ = raw;
    }

    #[test]
    fn extra_call_twin_vetoes_counted() {
        // A second `oldmod::bar` call elsewhere vetoes with totality 2:1.
        let pre =
            "fn m() {\n    let w: oldmod::Foo = oldmod::bar(2);\n    let v = oldmod::bar(3);\n}\n";
        let mut used = Vec::new();
        let out = super::try_typed_let_joint_single(
            &["    let w: oldmod::Foo = oldmod::bar(2);"],
            &["    let w: newmod::Foo = newmod::bar(2);"],
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
            "call twin must count, got: {out:?}"
        );
    }
}
