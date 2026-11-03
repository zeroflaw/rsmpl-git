//! Generalized anti-unification sweeps (WI-4a first slice, WI-4b type slice).
//!
//! Thin detectors over [`super::antiunify`]: `try_let_sweep` for let
//! type-addition (`let PAT = INIT;` → `let PAT: T = INIT;` with
//! per-site preserved `PAT`/`INIT` bound as inferred `P0`/`E0` and a
//! uniform concrete `T`), plus `try_type_sweep` for type rewrites
//! (`Vec<T>` → `Box<[T]>` with hetero `T` bound as inferred `T0…`).
//! Signature evolution (WI-4c) lives in [`super::sig_evolution`]
//! (split under the 2000-line gate, D5); the slot vocabulary stays
//! re-exported here so `generalized::parse_sig_slots` paths keep working.

pub(crate) use super::sig_evolution::{
    build_sig_header, classify_slot, parse_sig_slots, slot_col, vq_pair, Emit, SigSlots, SlotClass,
};

use super::antiunify::{
    antiunify_let_template, antiunify_let_type_add, antiunify_pat_swap, antiunify_sequence,
    antiunify_types, antiunify_untyped_let, TypedSite,
};
use super::unique_rule;
use quote::ToTokens;
use syn::spanned::Spanned;

/// N-site (N ≥ 2) let type-addition sweep: every pair adds the same
/// concrete type ascription to a preserved pattern and initializer,
/// both varying across sites (hence the `P0`/`E0` holes). Emits one
/// `# allow=many` hunk (tight original range, SPEC D.5) with diet
/// inferred holes — no `# pat`/`# expr` declarations (SPEC 15.8).
/// WI-14 slice 4 + arm/branch residue: the same class inside
/// one-line `fn` frames (`fn f0() { let x = E; }` →
/// `fn f0() { let x: T = E; }`), match-arm blocks
/// (`1 => { let a = E; a },`), and `if`/`else` branches
/// (`if c { let a = E; a } else { … }`) raises to the identical bare
/// hunk (D14: no frame text emitted) via one `Stmt` rung — each rung's
/// outer (fn/arm/branch) is never itself a `Stmt::Local`, so the bare
/// emission cannot match an outer `let` first. `let`-outer frames
/// (`let g = || { … };`, `let r = if … { let … };`) stay ordinary by
/// construction (unwrapping them would over-match the outer `let`).
pub(crate) fn try_let_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if let Some(ty) = antiunify_let_type_add(minuses, pluses) {
        if !crate::totality::let_untyped_holds(file_old, minuses, changed) {
            return None;
        }
        return Some(super::let_framed::emit_let_type_add(
            path, &ty, changed, used_rules,
        ));
    }
    // WI-14 slice 4: framed fallback lives in the `let_framed`
    // sibling (D5 split — production only, tests stay inline here).
    super::let_framed::try_framed_let_sweep(minuses, pluses, path, file_old, changed, used_rules)
}

/// N-site (N ≥ 2) type rewrite sweep: every pair carries the same type
/// delta with per-site varying subtrees bound as inferred `T0…` holes
/// (e.g. `Vec<T>` → `Box<[T]>`). Line shapes: bare `syn::Type` lines
/// (one `;`/`,` terminator tolerated), or `Stmt::Local` pairs with a
/// type ascription on both sides and per-site token-equal patterns and
/// initializers (a per-site init/pattern delta is a let-statement
/// template, not a type template — that is WI-4's `try_let_sweep`
/// generalization territory, not this detector). All-identical line
/// classes decline (verbatim stays the tighter winner, T422).
/// Totality is the generic engine-count check
/// (`template_type_holds`): the emitted minus template must match
/// exactly N type subterms in the pre-image — macro/string twins never
/// count (the engine never enters them), nested matches count
/// outer-only (contained skips), exactly like the real sweep rewrites.
pub(crate) fn try_type_sweep(
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
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut minus_sites = Vec::with_capacity(minuses.len());
    let mut plus_sites = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (mt, pt, m_text, p_text) = extract_type_pair(m, p)?;
        minus_sites.push(TypedSite {
            ty: mt,
            text: m_text,
        });
        plus_sites.push(TypedSite {
            ty: pt,
            text: p_text,
        });
    }
    let (minus_tpl, plus_tpl) = antiunify_types(&minus_sites, &plus_sites)?;
    if !crate::totality::template_type_holds(path, file_old, &minus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "type_sweep");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) joint let-template sweep (WI-5 slice 1): typed→typed
/// `let` pairs with per-site preserved pattern and type plus one
/// uniform initializer delta (e.g. `let P0: T0 = E0;` →
/// `let P0: T0 = E0.wrapping_add(1);` with hetero `P0`/`T0`/`E0`
/// across sites). Strictly generalizes the type-add-only slice (which
/// owns minus-untyped classes) and the type slice (which owns
/// per-site-preserved-init classes) — disjoint from both by
/// construction: this needs ascriptions on both sides plus a per-site
/// init delta. Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted minus must match exactly N
/// typed lets in the pre-image — an untouched twin vetoes, exactly
/// like the real sweep rewrites. Detector name `let_template`.
pub(crate) fn try_let_template_sweep(
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
    let (minus_tpl, plus_tpl) = antiunify_let_template(minuses, pluses)?;
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) untyped joint let-template sweep (WI-8 slice 1):
/// untyped→untyped `let` pairs with per-site preserved patterns plus
/// one uniform initializer delta (`let Some(P0) = E0;` →
/// `let Some(P0) = E0.unwrap_or(0);`, hetero `P0`/`E0` across
/// sites). The typed core's sibling without ascriptions (disjoint by
/// construction: this declines any ascribed side; the typed detector
/// above owns those classes). Uniform concrete deltas
/// (`let v = const { 1 + 2 };` → `… { 1 + 3 };`,
/// `let v = _;` → `let v = 0;`) ride the same core — WI-8's
/// const/infer init support is the `shape.rs` parity gate, not a
/// second template. Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted minus must match exactly N
/// untyped lets in the pre-image — an untouched twin vetoes.
/// Detector name `let_template` (same family as the typed core).
pub(crate) fn try_untyped_let_sweep(
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
    let (minus_tpl, plus_tpl) = antiunify_untyped_let(minuses, pluses)?;
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) tuple-position swap sweep (WI-8 slice 1):
/// `let (P0, P1) = E0;` → `let (P1, P0) = E0;` with per-site
/// preserved inits (a per-site init delta belongs to the untyped
/// core above, never guessed here). Totality is the same real-pair
/// engine-count check. Detector name `let_template` (same family).
pub(crate) fn try_pat_swap_sweep(
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
    let (minus_tpl, plus_tpl) = antiunify_pat_swap(minuses, pluses)?;
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-pair (N ≥ 4) repeated-block sequence sweep (WI-5c): N flat pairs
/// split block-major into ≥ 2 blocks × K = 2–4 `let` statements, each
/// position anti-unified across blocks (hetero positions ride holes,
/// identical ones stay concrete anchors). Emits one `# allow=many`
/// hunk (tight original range, SPEC D.5) with K minus/plus lines.
/// Contiguity gate: each block's old lines must be consecutive (block
/// bodies are consecutive statements — the detector only fuses runs the
/// engine matches as windows). Totality is the real-pair engine-count
/// check (`template_sequence_holds`): the emitted minus must match
/// exactly N blocks in the pre-image — an untouched twin window vetoes,
/// exactly like the real sweep rewrites. Detector name `sequence`.
pub(crate) fn try_sequence_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let n = minuses.len();
    if n != pluses.len() || n < 4 || changed.len() != n {
        return None;
    }
    // All-identical whole classes stay for the tighter verbatim sweep
    // (T422 precedent).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    // Largest K first: the most specific template wins the byte guard
    // naturally.
    for k in [4, 3, 2] {
        if !n.is_multiple_of(k) || n / k < 2 {
            continue;
        }
        let mut contiguous = true;
        for b in 0..n / k {
            let run = &changed[b * k..(b + 1) * k];
            for w in run.windows(2) {
                if w[1] != w[0] + 1 {
                    contiguous = false;
                    break;
                }
            }
            if !contiguous {
                break;
            }
        }
        if !contiguous {
            continue;
        }
        let (minus_tpls, plus_tpls) = antiunify_sequence(minuses, pluses, k)?;
        if !crate::totality::template_sequence_holds(path, file_old, &minus_tpls, &plus_tpls, n / k)
        {
            continue;
        }
        let rule = unique_rule(used_rules, "sequence");
        let mut hunk = crate::emission_header(path, &rule, changed);
        for l in &minus_tpls {
            hunk.push_str(&format!("- {l}\n"));
        }
        for l in &plus_tpls {
            hunk.push_str(&format!("+ {l}\n"));
        }
        return Some(hunk);
    }
    None
}

/// One pair's `(minus, plus)` types plus their original spellings: the
/// ascribed types of a `let` pair whose pattern and initializer are
/// token-equal (plus `let-else` diverge agreement — the engine never
/// moves into diverge targets). Bare-type full lines never occur in
/// `-U0` diffs of valid Rust (every type sits inside a larger line),
/// so there is deliberately no bare-`syn::Type` path: every pair site
/// is provably a type position, and the expression-domain hazard class
/// cannot arise. The texts are line-sliced via the types' own spans
/// and re-parsed, so every span inside them slices the text — template
/// rendering inherits the first site's exact spacing.
fn extract_type_pair(m: &str, p: &str) -> Option<(syn::Type, syn::Type, String, String)> {
    let (minus, plus) = match (
        syn::parse_str::<syn::Stmt>(m.trim()),
        syn::parse_str::<syn::Stmt>(p.trim()),
    ) {
        (Ok(syn::Stmt::Local(a)), Ok(syn::Stmt::Local(b))) => (a, b),
        _ => return None,
    };
    let (syn::Pat::Type(minus_pt), syn::Pat::Type(plus_pt)) = (&minus.pat, &plus.pat) else {
        return None;
    };
    if minus_pt.pat.to_token_stream().to_string() != plus_pt.pat.to_token_stream().to_string() {
        return None;
    }
    match (&minus.init, &plus.init) {
        (Some(a), Some(b)) => {
            if a.diverge.is_some() || b.diverge.is_some() {
                return None;
            }
            if a.expr.to_token_stream().to_string() != b.expr.to_token_stream().to_string() {
                return None;
            }
        }
        (None, None) => {}
        _ => return None,
    }
    let m_text = slice_type_text(m.trim(), &minus_pt.ty)?;
    let p_text = slice_type_text(p.trim(), &plus_pt.ty)?;
    let mt = syn::parse_str::<syn::Type>(&m_text).ok()?;
    let pt = syn::parse_str::<syn::Type>(&p_text).ok()?;
    Some((mt, pt, m_text, p_text))
}

/// Slice a type's original spelling from its single-line parent snippet
/// via the type's own span. Multi-line or out-of-bounds spans veto.
fn slice_type_text(line: &str, ty: &syn::Type) -> Option<String> {
    let span = ty.span();
    if span.start().line != 1 || span.end().line != 1 {
        return None;
    }
    // Span columns are Unicode scalars (SPEC 31.3), not bytes.
    let (s, e) = (
        rsmpl_pattern::char_col_to_byte(line, span.start().column),
        rsmpl_pattern::char_col_to_byte(line, span.end().column),
    );
    if s >= e || e > line.len() || !line.is_char_boundary(s) || !line.is_char_boundary(e) {
        return None;
    }
    Some(line[s..e].to_string())
}

#[cfg(test)]
mod tests {
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

    const PRE_THREE: &str = "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n}\n";

    fn three_pair_body() -> String {
        "@@ -1,5 +1,5 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n-    let gamma_value = compute_third(extra);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u32 = compute_second(other);\n+    let gamma_value: u32 = compute_third(extra);\n }\n"
            .to_string()
    }

    #[test]
    fn let_type_add_fires() {
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[("f.rs", PRE_THREE)]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        // Consumed: no native let lines survive.
        assert!(!out.contains("compute_first(input);"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# pat"), "got: {out}");
    }

    #[test]
    fn let_type_add_extra_untyped_let_vetoes() {
        // Totality: an untouched fourth untyped `let` in the pre-image
        // would be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n    let untouched_guard = compute_fourth(more);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn let_type_add_typed_twin_does_not_veto() {
        // An already-typed `let` never matches `let P0 = E0;` (ascription
        // agreement is strict), so it is not counted and never vetoes.
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n    let already: u32 = 1;\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn let_type_add_varying_type_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u64 = compute_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    const PRE_FRAMED_THREE: &str =
        "fn f0() { let x = foo(1); }\nfn f1() { let y = foo(2); }\nfn f2() { let z = foo(3); }\n";

    fn framed_three_pair_body() -> String {
        "@@ -1,3 +1,3 @@\n-fn f0() { let x = foo(1); }\n-fn f1() { let y = foo(2); }\n-fn f2() { let z = foo(3); }\n+fn f0() { let x: u32 = foo(1); }\n+fn f1() { let y: u32 = foo(2); }\n+fn f2() { let z: u32 = foo(3); }\n"
            .to_string()
    }

    #[test]
    fn framed_let_type_add_fires() {
        // WI-14 slice 4: the same type-add class inside one-line `fn`
        // frames raises to the identical bare hunk (no frame text).
        let raw = section("f.rs", &framed_three_pair_body());
        let pre = pre_map(&[("f.rs", PRE_FRAMED_THREE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        assert!(
            !out.contains("fn f0"),
            "no frame text in emission, got: {out}"
        );
    }

    #[test]
    fn framed_let_type_add_frame_delta_vetoes() {
        // A `fn` name delta alongside the inner rewrite is a frame
        // delta — the bare template would silently drop it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn f0() { let x = foo(1); }\n-fn f1() { let y = foo(2); }\n-fn f2() { let z = foo(3); }\n+fn g0() { let x: u32 = foo(1); }\n+fn g1() { let y: u32 = foo(2); }\n+fn g2() { let z: u32 = foo(3); }\n",
        );
        let pre = pre_map(&[("f.rs", PRE_FRAMED_THREE)]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_closure_outer_stays_ordinary() {
        // Over-match boundary: `let g = || { let x = … };` unwrapped to
        // the inner `let` would hand the engine a bare template that
        // matches the OUTER `let g` first (outermost-wins) — so the
        // `fn`-only Stmt rung declines and the section stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n fn m() {\n-    let g = || { let x = foo(1); };\n-    let h = || { let y = foo(2); };\n-    let k = || { let z = foo(3); };\n+    let g = || { let x: u32 = foo(1); };\n+    let h = || { let y: u32 = foo(2); };\n+    let k = || { let z: u32 = foo(3); };\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let g = || { let x = foo(1); };\n    let h = || { let y = foo(2); };\n    let k = || { let z = foo(3); };\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_arm_block_fires() {
        // WI-14 slice-4 residue: block-arm frames raise through the
        // arm Stmt rung to the identical bare hunk.
        let raw = section(
            "f.rs",
            "@@ -3,3 +3,3 @@\n fn m() {\n-        1 => { let a = foo(1); a },\n-        2 => { let b = foo(2); b },\n-        _ => { let c = foo(3); c },\n+        1 => { let a: u32 = foo(1); a },\n+        2 => { let b: u32 = foo(2); b },\n+        _ => { let c: u32 = foo(3); c },\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m(x: i32) -> i32 {\n    match x {\n        1 => { let a = foo(1); a },\n        2 => { let b = foo(2); b },\n        _ => { let c = foo(3); c },\n    }\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        assert!(!out.contains("=>"), "no frame text in emission, got: {out}");
    }

    #[test]
    fn framed_let_type_add_arm_two_lets_stay_ordinary() {
        // Ambiguous inner: two lets in one arm block decline.
        let raw = section(
            "f.rs",
            "@@ -3,2 +3,2 @@\n fn m() {\n-        1 => { let a = foo(1); let b = foo(2); a },\n-        2 => { let c = foo(3); let d = foo(4); c },\n+        1 => { let a: u32 = foo(1); let b = foo(2); a },\n+        2 => { let c: u32 = foo(3); let d = foo(4); c },\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m(x: i32) -> i32 {\n    match x {\n        1 => { let a = foo(1); let b = foo(2); a },\n        2 => { let c = foo(3); let d = foo(4); c },\n    }\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_branch_fires() {
        // WI-14 slice-4 residue: whole-line `if` branches raise through
        // the branch Stmt rung.
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n fn m() {\n-    if c { let a = foo(1); a } else { 0 }\n-    if c { let b = foo(2); b } else { 0 }\n-    if c { let c0 = foo(3); c0 } else { 0 }\n+    if c { let a: u32 = foo(1); a } else { 0 }\n+    if c { let b: u32 = foo(2); b } else { 0 }\n+    if c { let c0: u32 = foo(3); c0 } else { 0 }\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m(c: bool) -> i32 {\n    if c { let a = foo(1); a } else { 0 }\n    if c { let b = foo(2); b } else { 0 }\n    if c { let c0 = foo(3); c0 } else { 0 }\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        assert!(
            !out.contains("if c"),
            "no frame text in emission, got: {out}"
        );
    }

    #[test]
    fn framed_let_type_add_branch_let_outer_stays_ordinary() {
        // Over-match boundary: `let r = if … { let … };` unwrapped to
        // the inner `let` would match the outer `let r` first.
        let raw = section(
            "f.rs",
            "@@ -2,3 +2,3 @@\n fn m() {\n-    let r0 = if c { let a = foo(1); a } else { 0 };\n-    let r1 = if d { let b = foo(2); b } else { 0 };\n-    let r2 = if e { let c0 = foo(3); c0 } else { 0 };\n+    let r0 = if c { let a: u32 = foo(1); a } else { 0 };\n+    let r1 = if d { let b: u32 = foo(2); b } else { 0 };\n+    let r2 = if e { let c0: u32 = foo(3); c0 } else { 0 };\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m(c: bool, d: bool, e: bool) -> i32 {\n    let r0 = if c { let a = foo(1); a } else { 0 };\n    let r1 = if d { let b = foo(2); b } else { 0 };\n    let r2 = if e { let c0 = foo(3); c0 } else { 0 };\n    r0 + r1 + r2\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_mixed_rung_stays_ordinary() {
        // All-or-nothing per rung: one `fn`-frame pair plus two arm
        // pairs decline (neither rung owns the mixed class).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn f0() { let x = foo(1); }\n+fn f0() { let x: u32 = foo(1); }\n@@ -4,2 +4,2 @@\n-        1 => { let a = foo(2); a },\n-        2 => { let b = foo(3); b },\n+        1 => { let a: u32 = foo(2); a },\n+        2 => { let b: u32 = foo(3); b },\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f0() { let x = foo(1); }\nfn m(x: i32) -> i32 {\n    match x {\n        1 => { let a = foo(2); a },\n        2 => { let b = foo(3); b },\n    }\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_mixed_stays_ordinary() {
        // All-or-nothing per side: a section mixing own-line and
        // framed pairs declines (totality is whole-file either way,
        // and neither arm owns the mixed class).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a: u32 = foo(1);\n+    let b: u32 = foo(2);\n }\n@@ -5,2 +5,2 @@\n-fn f2() { let z = foo(3); }\n-fn f3() { let w = foo(4); }\n+fn f2() { let z: u32 = foo(3); }\n+fn f3() { let w: u32 = foo(4); }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = foo(1);\n    let b = foo(2);\n}\nfn f2() { let z = foo(3); }\nfn f3() { let w = foo(4); }\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_stray_untyped_vetoes() {
        // File-wide totality: one stray own-line untyped `let` (4 != 3)
        // would be silently rewritten — vetoes.
        let raw = section("f.rs", &framed_three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn f0() { let x = foo(1); }\nfn f1() { let y = foo(2); }\nfn f2() { let z = foo(3); }\nfn other() {\n    let stray = bar();\n    stray\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn framed_let_type_add_varying_type_vetoes() {
        // A varying inner type would need an unbound plus-side T0.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-fn f0() { let x = foo(1); }\n-fn f1() { let y = foo(2); }\n+fn f0() { let x: u32 = foo(1); }\n+fn f1() { let y: u64 = foo(2); }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f0() { let x = foo(1); }\nfn f1() { let y = foo(2); }\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn let_type_add_init_delta_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u32 = changed_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_single_vetoes() {
        // Singles never reach the pair chain (whole-section singles try
        // item rename only; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n+    let alpha_value: u32 = compute_first(input);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_unparseable_pre_vetoes() {
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    const PRE_TYPE: &str = "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_count: u32 = compute_third(extra);\n}\n";

    fn three_type_body() -> String {
        "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n-    let beta_value: Vec<String> = compute_second(other);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n+    let beta_value: Box<[String]> = compute_second(other);\n }\n"
            .to_string()
    }

    #[test]
    fn type_sweep_fires() {
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[("f.rs", PRE_TYPE)]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        // Templates inherit the sites' exact spacing (span-sliced).
        assert!(out.contains("- Vec<T0>\n"), "got: {out}");
        assert!(out.contains("+ Box<[T0]>\n"), "got: {out}");
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# type"), "got: {out}");
        // Consumed: no native type lines survive.
        assert!(!out.contains("Vec<u32>"), "got: {out}");
        assert!(
            out.len() < three_type_body().len() + 100,
            "type sweep must beat native bytes, got: {out}"
        );
    }

    #[test]
    fn type_sweep_fn_bound_fires() {
        // WI-6d: parenthesized `Fn` bound delta at depth 3 (Box angle →
        // trait bound → path args) raises through the full pipeline:
        // hetero inputs hole as `T0`, the uniform `u8` → `String`
        // output delta stays concrete, pre-image engine-count
        // totality holds (exactly 4 subterms). Same shape as
        // applyrs bench 27, which proves the authored direction.
        let pre = "fn a() {\n    let f: Box<dyn Fn(u8) -> u8> = build();\n    f\n}\nfn b() {\n    let f: Box<dyn Fn(u16) -> u8> = build();\n    f\n}\nfn c() {\n    let f: Box<dyn Fn(u32) -> u8> = build();\n    f\n}\nfn d() {\n    let f: Box<dyn Fn(bool) -> u8> = build();\n    f\n}\n";
        let body = "@@ -2 +2 @@\n-    let f: Box<dyn Fn(u8) -> u8> = build();\n+    let f: Box<dyn Fn(u8) -> String> = build();\n@@ -6 +6 @@\n-    let f: Box<dyn Fn(u16) -> u8> = build();\n+    let f: Box<dyn Fn(u16) -> String> = build();\n@@ -10 +10 @@\n-    let f: Box<dyn Fn(u32) -> u8> = build();\n+    let f: Box<dyn Fn(u32) -> String> = build();\n@@ -14 +14 @@\n-    let f: Box<dyn Fn(bool) -> u8> = build();\n+    let f: Box<dyn Fn(bool) -> String> = build();\n".to_string();
        let raw = section("f.rs", &body);
        let pres = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pres).unwrap();
        validated_live(&out, &pres);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- Box<dyn Fn(T0) -> u8>\n"), "got: {out}");
        assert!(out.contains("+ Box<dyn Fn(T0) -> String>\n"), "got: {out}");
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# type"), "got: {out}");
        // Consumed: no native type lines survive.
        assert!(!out.contains("Box<dyn Fn(u8) -> u8>"), "got: {out}");
        assert!(out.len() < body.len(), "type sweep must beat native bytes");
    }

    #[test]
    fn type_sweep_extra_type_site_vetoes() {
        // Totality: a third `Vec<..>` ascription in the pre-image would
        // be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_extra: Vec<u8> = compute_third(extra);\n    let gamma_count: u32 = done();\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn type_sweep_untyped_let_does_not_veto() {
        // An untyped `let` holds no type subterm, so the template never
        // matches it — unlike `let P0 = E0;`, it is invisible here.
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let plain = compute_third(extra);\n    let gamma_count: u32 = done();\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn type_sweep_init_delta_vetoes() {
        // A per-site initializer delta is a let-statement template, not
        // a type template.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n-    let beta_value: Vec<String> = compute_second(other);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n+    let beta_value: Box<[String]> = changed_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn type_sweep_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn type_sweep_unparseable_pre_vetoes() {
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    const PRE_EVO: &str =
        "fn alpha(x: i32) -> i32 {\n    x + 1\n}\nfn alpha(y: String) -> String {\n    y\n}\n";

    // Realistic `-U0` shape: each header-only change is its own hunk
    // (headers at old lines 1 and 4); whole-section pairing joins them.
    fn evo_return_body() -> String {
        "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32) -> u32 {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+fn beta(y: String) -> u32 {\n"
            .to_string()
    }

    #[test]
    fn sig_evo_rename_return_fires() {
        // Rename plus a uniform return-type addition; params hetero
        // across sites ride `(...)`, return rides omission→concrete.
        // The covering item range spans both items.
        let raw = section("f.rs", &evo_return_body());
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,6 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+ fn beta(...) -> u32 {"), "got: {out}");
        assert!(!out.contains("-fn alpha(x: i32)"), "got: {out}");
    }

    #[test]
    fn sig_evo_vis_params_mixed_fire() {
        // Hetero preserved vis rides the `V Q` pair (lone `V` is
        // inference-ambiguous — probed) while uniform params evolve
        // concrete; the uniform return stays concrete.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-pub fn alpha(x: i32) -> i32 {\n+pub fn beta(x: i32, y: u32) -> i32 {\n@@ -4,1 +4,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32, y: u32) -> i32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn alpha(x: i32) -> i32 {\n    x\n}\nfn alpha(x: i32) -> i32 {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- V Q fn alpha(x : i32) -> i32 {"),
            "got: {out}"
        );
        assert!(
            out.contains("+ V Q fn beta(x : i32, y : u32) -> i32 {"),
            "got: {out}"
        );
    }

    #[test]
    fn sig_evo_pure_evolution_no_rename_fires() {
        // Same name, uniform return addition — no rename involved.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn alpha(x: i32) -> u32 {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+fn alpha(y: String) -> u32 {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+ fn alpha(...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_generics_where_fire() {
        // Uniform `<T>` + `where` addition with rename; generics/where
        // hetero-preserved sites would ride omission instead.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta<T>(x: i32) -> i32 where T: Clone {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+fn beta<T>(y: String) -> String where T: Clone {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(
            out.contains("+ fn beta<T>(...) where T : Clone {"),
            "got: {out}"
        );
    }

    #[test]
    fn sig_evo_impl_context_fires() {
        // Uniformly indented methods evolve under the block's verbatim
        // header (WI-4a filter: the header is literally `impl Svc`,
        // so the verbatim emission reuses it).
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    fn alpha(x: i32) -> i32 {\n+    fn beta(x: i32) -> u32 {\n@@ -5,1 +5,1 @@\n-    fn alpha(y: String) -> String {\n+    fn beta(y: String) -> u32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(x: i32) -> i32 {\n        x\n    }\n    fn alpha(y: String) -> String {\n        y\n    }\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("impl Svc"), "got: {out}");
        assert!(out.contains("-     fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+     fn beta(...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_name_only_declines() {
        // Rename-only classes belong to the slot sweep (which runs
        // first) — a direct call proves this detector declines them.
        let minuses = [
            "fn alpha(x: i32) -> i32 {",
            "fn alpha(y: String) -> String {",
        ];
        let pluses = ["fn beta(x: i32) -> i32 {", "fn beta(y: String) -> String {"];
        assert!(super::super::try_sig_evolution_sweep(
            &minuses,
            &pluses,
            "f.rs",
            None,
            &[1, 2],
            &mut Vec::new()
        )
        .is_none());
    }

    #[test]
    fn sig_evo_divergent_evolution_vetoes() {
        // Different returns per site are not one uniform delta.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn alpha(x: i32) -> i32 {\n-fn alpha(y: String) -> String {\n+fn beta(x: i32) -> u32 {\n+fn beta(y: String) -> u64 {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sig_evo_extra_site_vetoes() {
        // Totality: a third untouched `fn alpha` in the pre-image would
        // be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &evo_return_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 {\n    x + 1\n}\nfn alpha(y: String) -> String {\n    y\n}\nfn alpha(z: u8) -> u8 {\n    z\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn sig_evo_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32) -> u32 {\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sig_evo_single_fires_with_pre_image() {
        // WI-19b: a vis+qual single with a pre-image compresses under
        // the precision guard (uniform-by-construction), byte-exact:
        // single-token slots re-render stably.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn helper(x: VeryLongTypeNameArgumentNumberOne) {\n+pub async fn helper(x: VeryLongTypeNameArgumentNumberOne) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn helper(x: VeryLongTypeNameArgumentNumberOne) {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("sig single must fire");
        assert!(out.contains("helper"), "got:\n{out}");
        validated_live(&out, &pre);
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "pub async fn helper(x: VeryLongTypeNameArgumentNumberOne) {\n    x\n}\n"
        );
    }

    #[test]
    fn sig_evo_single_params_evolution_rides_type_single() {
        // WI-19b/117, WI-2 slice 1: a params evolution is the
        // type-single's domain, never the sig detector's — the sig
        // rewrite would re-render the type with pretty spacing
        // (`Box<[i32]>` → `Box <[ i32 ]>`), so `try_sig_evolution_sweep`
        // keeps declining (pinned below) while the whole pipeline raises
        // the literal bare-type single (`- Vec<i32>`).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn sum(values: Vec<i32>) -> i32 {\n+fn sum(values: Box<[i32]>) -> i32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn sum(values: Vec<i32>) -> i32 {\n    values.iter().sum()\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("params evolution must ride the type single");
        assert!(out.contains("- Vec<i32>\n"), "got:\n{out}");
        assert!(out.contains("+ Box<[i32]>\n"), "got:\n{out}");
        let mut used = Vec::new();
        assert!(super::super::try_sig_evolution_sweep(
            &["fn sum(values: Vec<i32>) -> i32 {"],
            &["fn sum(values: Box<[i32]>) -> i32 {"],
            "f.rs",
            Some(&pre["f.rs"]),
            &[1],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn sig_evo_single_twin_vetoes() {
        // An untouched twin header vetoes via the engine-count totality.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn helper(x: i32) {\n+pub fn helper(x: i32) -> i64 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn helper(x: i32) {\n    x\n}\nfn helper(y: i32) {\n    y\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn sig_evo_single_zero_evolving_declines() {
        // Pure renames belong to the slot sweep, never here — even at N=1.
        let mut used = Vec::new();
        assert!(super::super::try_sig_evolution_sweep(
            &["fn alpha(x: i32) {"],
            &["fn beta(x: i32) {"],
            "f.rs",
            Some("fn alpha(x: i32) {\n    x\n}\n"),
            &[1],
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn sig_evo_unparseable_pre_vetoes() {
        let raw = section("f.rs", &evo_return_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn sig_evo_qual_evolution_fires() {
        // Uniform `async` addition with rename; qual rides concrete
        // while vis stays omitted on both sides (`V Q` pairs only when
        // a wildcard is needed — here neither slot needs one).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+async fn beta(x: i32) -> i32 {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+async fn beta(y: String) -> String {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+ async fn beta(...) {"), "got: {out}");
    }

    #[test]
    fn sig_evo_where_hetero_return_evolve_fires() {
        // Hetero preserved `where` rides `W` (lone-`W` is legal,
        // probed) while a uniform return is added — the plus side
        // drops `W` (`-> T W` is unparsable).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 where T: Clone {\n+fn beta(x: i32) -> u32 where T: Clone {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String where U: Debug {\n+fn beta(y: String) -> u32 where U: Debug {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 where T: Clone {\n    x\n}\nfn alpha(y: String) -> String where U: Debug {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) W {"), "got: {out}");
        assert!(out.contains("+ fn beta(...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_generics_hetero_return_evolve_fires() {
        // Hetero preserved generics ride `G` while a uniform return is
        // added — the `G`-without-`W` plus shape with a concrete
        // return.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha<T>(x: T) -> T {\n+fn beta<T>(x: T) -> u32 {\n@@ -4,1 +4,1 @@\n-fn alpha<U>(y: U) -> U {\n+fn beta<U>(y: U) -> u32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha<T>(x: T) -> T {\n    x\n}\nfn alpha<U>(y: U) -> U {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha G (...) {"), "got: {out}");
        assert!(out.contains("+ fn beta G (...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_return_removal_fires() {
        // Uniform return removal: minus concrete, plus omitted
        // (= delete, engine semantics); hetero-base removal declines.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32) {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> i32 {\n+fn beta(y: String) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 {\n    x\n}\nfn alpha(y: String) -> i32 {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) -> i32 {"), "got: {out}");
        assert!(out.contains("+ fn beta(...) {"), "got: {out}");
    }

    #[test]
    fn sig_evo_hetero_base_removal_vetoes() {
        // Return removal over a hetero base (one site already
        // return-less) would need minus-`Any`/plus-omitted, which
        // preserves instead of deleting — decline.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn alpha(x: i32) -> i32 {\n-fn alpha(y: String) {\n+fn beta(x: i32) {\n+fn beta(y: String) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    const PRE_JOINT: &str =
        "fn f() {\n    let a: i32 = 0;\n    let b: u32 = 1;\n    let c: i64 = 2;\n}\n";

    fn three_joint_body() -> String {
        "@@ -1,4 +1,4 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n-    let c: i64 = 2;\n+    let a: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(1);\n+    let c: i64 = 2.wrapping_add(1);\n }\n"
            .to_string()
    }

    #[test]
    fn joint_let_wrapping_fires() {
        // WI-5 slice 1: hetero pat/ty across sites ride P0/T0, the lit
        // inits ride E0, and the uniform `.wrapping_add(1)` delta is the
        // anchor. Consumed lines must vanish; diet holes need no
        // declarations (SPEC 15.8).
        let raw = section("f.rs", &three_joint_body());
        let pre = pre_map(&[("f.rs", PRE_JOINT)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- let P0: T0 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P0: T0 = E0.wrapping_add(1);\n"),
            "got: {out}"
        );
        assert!(
            !out.contains("-    let a: i32"),
            "changed lines must be consumed, got: {out}"
        );
        assert!(!out.contains("# pat"), "got: {out}");
        assert!(!out.contains("# type"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn joint_let_extra_twin_vetoes() {
        // Totality: an untouched fourth typed `let` in the pre-image
        // would be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_joint_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let a: i32 = 0;\n    let b: u32 = 1;\n    let c: i64 = 2;\n    let guard: u8 = 9;\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn joint_let_untyped_minus_vetoes() {
        // Minus-untyped classes belong to the type-add slice (which
        // runs first and keeps them); a direct call proves this
        // detector declines them.
        let minuses = ["let a = f(x);", "let b = f(y);"];
        let pluses = ["let a: u32 = f(x);", "let b: u32 = f(y);"];
        assert!(super::try_let_template_sweep(
            &minuses,
            &pluses,
            "f.rs",
            None,
            &[1, 2],
            &mut Vec::new()
        )
        .is_none());
    }

    #[test]
    fn joint_let_per_site_pat_delta_vetoes() {
        // A per-site pattern rename is not a joint template (rename
        // detectors own it) — never guessed here.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n+    let ax: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(1);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn joint_let_divergent_plus_vetoes() {
        // Different per-site deltas are not one uniform delta.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n+    let a: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn joint_let_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn f() {\n-    let a: i32 = 0;\n+    let a: i32 = 0.wrapping_add(1);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    // WI-5c detector proving shape: 2 blocks × 2 hetero positions no
    // single-class detector covers — position 0 is a type-addition,
    // position 1 a joint init-delta template (hetero pat/ty ride P1/T1).
    const PRE_SEQ: &str = "fn first() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\n\nfn second() {\n    let delta = compute_second(other);\n    let epsilon: u64 = wrap(delta);\n}\n";

    fn seq_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta: u32 = wrap(alpha);\n+    let alpha: u32 = compute_first(input);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n@@ -7,2 +7,2 @@\n-    let delta = compute_second(other);\n-    let epsilon: u64 = wrap(delta);\n+    let delta: u32 = compute_second(other);\n+    let epsilon: u64 = wrap(delta).wrapping_add(1);\n"
            .to_string()
    }

    #[test]
    fn sequence_two_blocks_fire() {
        // WI-5c canonical shape: 2 blocks × 2 hetero positions. One
        // shared header amortized over four lines.
        let raw = section("f.rs", &seq_body());
        let pre = pre_map(&[("f.rs", PRE_SEQ)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("- let P1: T1 = E1;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P1: T1 = E1.wrapping_add(1);\n"),
            "got: {out}"
        );
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# pat"), "got: {out}");
        assert!(!out.contains("# type"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        // Consumed: no native changed lines survive.
        assert!(!out.contains("compute_first(input);"), "got: {out}");
    }

    #[test]
    fn sequence_extra_twin_vetoes() {
        // Totality: an untouched twin window in the pre-image would be
        // silently rewritten — the WINDOW sweep vetoes. Each proved
        // line raises as its own WI-6-slice-1 literal verbatim single
        // (unique whole statements, byte-probe verified).
        let raw = section("f.rs", &seq_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn first() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\n\nfn second() {\n    let delta = compute_second(other);\n    let epsilon: u64 = wrap(delta);\n}\n\nfn third() {\n    let guard = compute_third(more);\n    let guard2: u64 = wrap(guard);\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("literal verbatim singles must fire");
        assert!(!out.contains("P0"), "no window template, got: {out}");
        for (m, p) in [
            (
                "- let alpha = compute_first(input);",
                "+ let alpha: u32 = compute_first(input);",
            ),
            (
                "- let beta: u32 = wrap(alpha);",
                "+ let beta: u32 = wrap(alpha).wrapping_add(1);",
            ),
            (
                "- let delta = compute_second(other);",
                "+ let delta: u32 = compute_second(other);",
            ),
            (
                "- let epsilon: u64 = wrap(delta);",
                "+ let epsilon: u64 = wrap(delta).wrapping_add(1);",
            ),
        ] {
            assert!(out.contains(m), "got: {out}");
            assert!(out.contains(p), "got: {out}");
        }
    }

    #[test]
    fn sequence_single_block_vetoes() {
        // One hetero block is not a repeated sequence (WI-5d owns
        // single-block fusion): the type-add position fails on the
        // typed pair, the joint position on the untyped pair — stays
        // ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta: u32 = wrap(alpha);\n+    let alpha: u32 = compute_first(input);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sequence_noncontiguous_vetoes() {
        // Hetero changed lines split across non-consecutive runs are not
        // block bodies: singles fail their class gates and the sequence
        // fails contiguity — stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    let alpha = compute_first(input);\n+    let alpha: u32 = compute_first(input);\n@@ -4,1 +4,1 @@\n-    let beta: u32 = wrap(alpha);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n@@ -7,1 +7,1 @@\n-    let delta = compute_second(other);\n+    let delta: u32 = compute_second(other);\n@@ -9,1 +9,1 @@\n-    let epsilon: u64 = wrap(delta);\n+    let epsilon: u64 = wrap(delta).wrapping_add(1);\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sig_evo_return_and_where_both_evolve_fire() {
        // Concrete return plus concrete `where` on the plus side is
        // plain Rust (`-> T where ...` only bans the `W` wildcard).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) {\n+fn beta(x: i32) -> u32 where T: Clone {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) {\n+fn beta(y: String) -> u32 where T: Clone {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) {\n    x\n}\nfn alpha(y: String) {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(
            out.contains("+ fn beta(...) -> u32 where T : Clone {"),
            "got: {out}"
        );
    }

    fn applied_post(patch: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        files
    }

    // WI-8 slice 1: untyped joint let template (hetero `Some(P0)`
    // patterns, uniform `E0`-hole method-wrap init delta).
    const PRE_SOME: &str = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let _ = (v, w);\n}\n";
    const POST_SOME: &str = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o.unwrap_or(0);\n    let Some(w) = p.unwrap_or(0);\n    let _ = (v, w);\n}\n";

    fn some_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let Some(v) = o;\n-    let Some(w) = p;\n+    let Some(v) = o.unwrap_or(0);\n+    let Some(w) = p.unwrap_or(0);\n"
            .to_string()
    }

    #[test]
    fn untyped_let_some_unwrap_fires_byte_identical() {
        let raw = section("f.rs", &some_body());
        let pre = pre_map(&[("f.rs", PRE_SOME)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        assert!(out.contains("- let Some(P0) = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let Some(P0) = E0.unwrap_or(0);\n"),
            "got: {out}"
        );
        assert!(!out.contains("# pat"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        let files = applied_post(&out, &pre);
        assert_eq!(
            files["f.rs"], POST_SOME,
            "real-apply must be byte-identical"
        );
    }

    #[test]
    fn untyped_let_extra_twin_vetoes() {
        // Totality: an untouched third `Some` let in the pre-image
        // would be silently rewritten — the HOLED sweep vetoes. Each
        // proved line raises as its own WI-6-slice-1 literal verbatim
        // single (unique whole statements, byte-probe verified).
        let raw = section("f.rs", &some_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn a(o: Option<i32>, p: Option<i32>, q: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let Some(z) = q;\n    let _ = (v, w, z);\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("literal verbatim singles must fire");
        assert!(!out.contains("Some(P0)"), "holed veto, got: {out}");
        assert!(out.contains("- let Some(v) = o;\n"), "got: {out}");
        assert!(
            out.contains("+ let Some(v) = o.unwrap_or(0);\n"),
            "got: {out}"
        );
        assert!(out.contains("- let Some(w) = p;\n"), "got: {out}");
        assert!(
            out.contains("+ let Some(w) = p.unwrap_or(0);\n"),
            "got: {out}"
        );
    }

    #[test]
    fn untyped_let_const_block_fires() {
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    let v = const { 1 + 2 };\n+    let v = const { 1 + 3 };\n@@ -5,1 +5,1 @@\n-    let w = const { 1 + 2 };\n+    let w = const { 1 + 3 };\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let v = const { 1 + 2 };\n}\nfn g() {\n    let w = const { 1 + 2 };\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = const { 1 + 2 };\n"), "got: {out}");
        assert!(out.contains("+ let P0 = const { 1 + 3 };\n"), "got: {out}");
    }

    #[test]
    fn untyped_let_infer_init_fires() {
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let v = _;\n-    let w = _;\n+    let v = 0;\n+    let w = 0;\n",
        );
        let pre = pre_map(&[("f.rs", "fn f() {\n    let v = _;\n    let w = _;\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = _;\n"), "got: {out}");
        assert!(out.contains("+ let P0 = 0;\n"), "got: {out}");
    }

    #[test]
    fn untyped_let_macro_init_stays_with_macro_detector() {
        // Nested-macro classes belong to the macro-interior sweep,
        // never to a whole-line let template.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n-    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n+    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n+    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(
            !out.contains("let P0"),
            "untyped-let must decline, got: {out}"
        );
        assert!(
            out.contains("- env!(\"CARGO_BIN_EXE_gitrs\")\n"),
            "got: {out}"
        );
    }

    #[test]
    fn untyped_let_typed_class_stays_typed() {
        // Chain precedence: ascribed classes still hit the typed
        // template first (this detector declines any ascribed side).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n+    let a: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(1);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(out.contains("- let P0: T0 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P0: T0 = E0.wrapping_add(1);\n"),
            "got: {out}"
        );
    }

    #[test]
    fn untyped_let_single_stays_ordinary() {
        // Singles never reach the pair chain (whole-section singles try
        // item rename only; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn f(o: Option<i32>) {\n-    let Some(v) = o;\n+    let Some(v) = o.unwrap_or(0);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    // WI-8 slice 1: tuple-position swap (`(P0, P1)` → `(P1, P0)`).
    const PRE_SWAP: &str = "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (x, y) = t;\n    let (m, n) = u;\n    let _ = (x, y, m, n);\n}\n";
    const POST_SWAP: &str = "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (y, x) = t;\n    let (n, m) = u;\n    let _ = (x, y, m, n);\n}\n";

    fn swap_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let (x, y) = t;\n-    let (m, n) = u;\n+    let (y, x) = t;\n+    let (n, m) = u;\n"
            .to_string()
    }

    #[test]
    fn pat_swap_fires_byte_identical() {
        let raw = section("f.rs", &swap_body());
        let pre = pre_map(&[("f.rs", PRE_SWAP)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        assert!(out.contains("- let (P0, P1) = E0;\n"), "got: {out}");
        assert!(out.contains("+ let (P1, P0) = E0;\n"), "got: {out}");
        let files = applied_post(&out, &pre);
        assert_eq!(
            files["f.rs"], POST_SWAP,
            "real-apply must be byte-identical"
        );
    }

    #[test]
    fn pat_swap_extra_twin_vetoes() {
        // An untouched same-shape tuple let would be silently
        // rewritten — the HOLED sweep vetoes. Each proved line raises
        // as its own WI-6-slice-1 literal verbatim single (unique
        // whole statements, byte-probe verified).
        let raw = section("f.rs", &swap_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn a(t: (i32, i32), u: (i32, i32), v: (i32, i32)) {\n    let (x, y) = t;\n    let (m, n) = u;\n    let (p, q) = v;\n    let _ = (x, y, m, n, p, q);\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("literal verbatim singles must fire");
        assert!(!out.contains("(P0, P1)"), "holed veto, got: {out}");
        assert!(out.contains("- let (x, y) = t;\n"), "got: {out}");
        assert!(out.contains("+ let (y, x) = t;\n"), "got: {out}");
        assert!(out.contains("- let (m, n) = u;\n"), "got: {out}");
        assert!(out.contains("+ let (n, m) = u;\n"), "got: {out}");
    }

    // WI-14 slice 3: N=1 vis/qual-evolution singles (same name).
    fn vis_single_body(minus: &str, plus: &str) -> String {
        format!("@@ -1,1 +1,1 @@\n-{minus}\n+{plus}\n")
    }

    #[test]
    fn vis_single_short_add_fires_concrete() {
        // Short headers lost the precision guard to the wild + allow
        // form; the N=1 concrete diet (no `G`/`W`, no `# allow=many`)
        // passes with `body - section ≈ +1`.
        let raw = section("f.rs", &vis_single_body("fn foo() {", "pub fn foo() {"));
        let pre = pre_map(&[("f.rs", "fn foo() {\n    1\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("short N=1 vis single must fire");
        assert!(out.contains("- fn foo() {\n"), "got:\n{out}");
        assert!(out.contains("+ pub fn foo() {\n"), "got:\n{out}");
        assert!(!out.contains('G'), "concrete, no wildcards, got:\n{out}");
        assert!(!out.contains("# allow=many"), "allow-one, got:\n{out}");
        let files = applied_post(&out, &pre);
        assert_eq!(files["f.rs"], "pub fn foo() {\n    1\n}\n");
    }

    #[test]
    fn vis_single_medium_add_fires_concrete() {
        let raw = section(
            "f.rs",
            &vis_single_body("fn foo(x: i32) {", "pub fn foo(x: i32) {"),
        );
        let pre = pre_map(&[("f.rs", "fn foo(x: i32) {\n    x\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("medium N=1 vis single must fire");
        assert!(out.contains("pub fn foo"), "got:\n{out}");
        assert!(!out.contains("# allow=many"), "allow-one, got:\n{out}");
        let files = applied_post(&out, &pre);
        assert_eq!(files["f.rs"], "pub fn foo(x: i32) {\n    x\n}\n");
    }

    #[test]
    fn vis_single_removal_and_qual_fire() {
        let raw = section("f.rs", &vis_single_body("pub fn foo() {", "fn foo() {"));
        let pre = pre_map(&[("f.rs", "pub fn foo() {\n    1\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis removal single must fire");
        assert!(out.contains("- pub fn foo() {\n"), "got:\n{out}");
        assert!(out.contains("+ fn foo() {\n"), "got:\n{out}");
        assert_eq!(applied_post(&out, &pre)["f.rs"], "fn foo() {\n    1\n}\n");

        let raw = section("f.rs", &vis_single_body("fn foo() {", "async fn foo() {"));
        let pre = pre_map(&[("f.rs", "fn foo() {\n    1\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("qual single must fire");
        assert!(out.contains("+ async fn foo() {\n"), "got:\n{out}");
        assert_eq!(
            applied_post(&out, &pre)["f.rs"],
            "async fn foo() {\n    1\n}\n"
        );
    }

    #[test]
    fn vis_single_line_body_fires_byte_identical() {
        // Single-line items never parsed as headers before this slice
        // (`parse_sig_slots` needed a trailing `{`).
        for (pre, post, minus, plus) in [
            (
                "fn foo() {}\n",
                "pub fn foo() {}\n",
                "fn foo() {}",
                "pub fn foo() {}",
            ),
            (
                "fn foo() { 1 }\n",
                "pub fn foo() { 1 }\n",
                "fn foo() { 1 }",
                "pub fn foo() { 1 }",
            ),
        ] {
            let raw = section("f.rs", &vis_single_body(minus, plus));
            let pre_map = pre_map(&[("f.rs", pre)]);
            let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre_map)
                .expect("single-line vis single must fire");
            assert!(out.contains("+ pub fn foo() {\n"), "got:\n{out}");
            assert_eq!(applied_post(&out, &pre_map)["f.rs"], post);
        }
        // Multi-line header parse is unchanged (regression pin).
        assert!(super::parse_sig_slots("fn foo() {").is_some());
        assert!(super::parse_sig_slots("fn foo() {}").is_some());
        assert!(super::parse_sig_slots("pub fn foo() { 1 }").is_some());
        assert!(super::parse_sig_slots("fn foo();").is_none());
        assert!(super::parse_sig_slots("struct S;").is_none());
    }

    #[test]
    fn vis_single_twin_vetoes() {
        // An untouched same-header twin matches the concrete minus too —
        // the engine-count probe (2 != 1) vetoes.
        let raw = section("f.rs", &vis_single_body("fn foo() {", "pub fn foo() {"));
        let pre = pre_map(&[("f.rs", "fn foo() {\n    1\n}\nfn foo() {\n    2\n}\n")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn vis_single_two_slot_vetoes() {
        // vis+return stays ordinary — the WI-8 two-slot domain, never
        // this single. (vis+qual together fires: both are single-token,
        // formatting-stable — pinned by
        // `sig_evo_single_fires_with_pre_image`.) The params-evolution
        // shape that used to sit in this veto loop now rides the WI-2
        // type single (pinned by
        // `sig_evo_single_params_evolution_rides_type_single` and the
        // `type_single` module tests).
        let (minus, plus, body) = (
            "fn foo() {",
            "pub fn foo() -> i32 {",
            "fn foo() {\n    1\n}\n",
        );
        let raw = section("f.rs", &vis_single_body(minus, plus));
        let pre = pre_map(&[("f.rs", body)]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none(),
            "{minus} → {plus} must stay ordinary"
        );
        // vis+qual together fires (both single-token slots).
        let raw = section(
            "f.rs",
            &vis_single_body("fn foo() {", "pub async fn foo() {"),
        );
        let pre = pre_map(&[("f.rs", "fn foo() {\n    1\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("vis+qual single must fire");
        assert!(out.contains("pub async fn foo"), "got:\n{out}");
    }

    #[test]
    fn vis_multi_hetero_unchanged() {
        // Hetero N≥2 vis sweep keeps the wild diet (regression pin).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha() {\n+pub fn alpha() {\n@@ -4,1 +4,1 @@\n-fn beta() {\n+pub fn beta() {\n",
        );
        let pre = pre_map(&[("f.rs", "fn alpha() {\n    1\n}\nfn beta() {\n    2\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("hetero N=2 must keep firing");
        assert!(out.contains("fn N"), "wild diet, got:\n{out}");
        assert!(out.contains("# allow=many"), "got:\n{out}");
    }

    // WI-14 slice 3 residue: method N=1 vis/qual singles. The detector
    // always fired; the stripped-section guard (what `diffrs` really
    // sees: collapsed header, no hunk titles) vetoed by 14 bytes. The
    // whole-section arm now allows 32 for N=1 indented `sig_evolution`
    // (mandatory WI-4a filter block priced, proof unchanged). These pins
    // go through `infer_rsmpl_with_pre_images` — the real `diffrs` path
    // — not just the unstripped per-section helper.
    fn method_vis_raw(minus: &str, plus: &str) -> String {
        format!(
            "diff --git a/f.rs b/f.rs\nindex 1111111..2222222 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -2 +2 @@ impl Svc {{\n-{minus}\n+{plus}\n"
        )
    }

    fn method_vis_pre() -> std::collections::HashMap<String, String> {
        pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(x: i32) {\n        x\n    }\n}\n",
        )])
    }

    fn method_vis_rsmpl(raw: &str, pre: &std::collections::HashMap<String, String>) -> String {
        crate::infer_rsmpl_with_pre_images(raw, pre)
            .expect("infer must not error")
            .rsmpl
    }

    #[test]
    fn method_vis_single_add_fires() {
        let raw = method_vis_raw("    fn alpha(x: i32) {", "    pub fn alpha(x: i32) {");
        let pre = method_vis_pre();
        let out = method_vis_rsmpl(&raw, &pre);
        assert!(out.contains("rule=r1"), "must compress, got:\n{out}");
        assert!(out.contains("impl Svc"), "block filter kept, got:\n{out}");
        assert!(out.contains("-     fn alpha(...) {"), "got:\n{out}");
        assert!(out.contains("+     pub fn alpha(...) {"), "got:\n{out}");
        assert!(!out.contains("# allow=many"), "allow-one, got:\n{out}");
        assert!(!out.contains('G'), "concrete, no wildcards, got:\n{out}");
        // Real-apply byte-identical.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "impl Svc {\n    pub fn alpha(x: i32) {\n        x\n    }\n}\n"
        );
    }

    #[test]
    fn method_vis_single_removal_and_qual_fire() {
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    pub fn alpha(x: i32) {\n        x\n    }\n}\n",
        )]);
        let raw = method_vis_raw("    pub fn alpha(x: i32) {", "    fn alpha(x: i32) {");
        let out = method_vis_rsmpl(&raw, &pre);
        assert!(out.contains("-     pub fn alpha(...) {"), "got:\n{out}");
        assert!(out.contains("+     fn alpha(...) {"), "got:\n{out}");

        let pre = method_vis_pre();
        let raw = method_vis_raw("    fn alpha(x: i32) {", "    async fn alpha(x: i32) {");
        let out = method_vis_rsmpl(&raw, &pre);
        assert!(out.contains("+     async fn alpha(...) {"), "got:\n{out}");
    }

    #[test]
    fn method_vis_single_twin_vetoes() {
        // An untouched same-header method elsewhere matches the concrete
        // minus too — the engine-count probe (2 != 1) vetoes.
        let raw = method_vis_raw("    fn alpha(x: i32) {", "    pub fn alpha(x: i32) {");
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(x: i32) {\n        x\n    }\n    fn alpha(x: i32) {\n        x\n    }\n}\n",
        )]);
        let out = method_vis_rsmpl(&raw, &pre);
        assert!(
            !out.contains("rule="),
            "twin must stay ordinary, got:\n{out}"
        );
    }

    #[test]
    fn method_vis_single_two_slot_vetoes() {
        // vis+return and type-level evolutions stay ordinary (WI-7).
        let pre_alpha = method_vis_pre();
        let raw = method_vis_raw(
            "    fn alpha(x: i32) {",
            "    pub fn alpha(x: i32) -> i32 {",
        );
        let out = method_vis_rsmpl(&raw, &pre_alpha);
        assert!(
            !out.contains("rule="),
            "vis+return must stay ordinary, got:\n{out}"
        );

        let pre_sum = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn sum(values: Vec<i32>) -> i32 {\n        values.iter().sum()\n    }\n}\n",
        )]);
        let raw = method_vis_raw(
            "    fn sum(values: Vec<i32>) -> i32 {",
            "    pub fn sum(values: Box<[i32]>) -> i32 {",
        );
        let out = method_vis_rsmpl(&raw, &pre_sum);
        assert!(
            !out.contains("rule="),
            "type-level evolution must stay ordinary, got:\n{out}"
        );
    }

    #[test]
    fn method_vis_single_fires_beside_ordinary_hunk() {
        // WI-14 slice-3 residue (a): two-hunk section — the method-vis
        // hunk raises as a `sig_evolution` single via the per-hunk
        // fallback (four-guard method-run price) while the unclaimable
        // literal hunk stays ordinary. Real-apply byte-identical.
        let raw = "diff --git a/f.rs b/f.rs\nindex 1111111..2222222 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -2 +2 @@ impl Svc {\n-    fn alpha(x: i32) {\n+    pub fn alpha(x: i32) {\n@@ -7 +7 @@\n-    1\n+    2\n";
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(x: i32) {\n        x\n    }\n}\nfn helper() {\n    1\n}\n",
        )]);
        let out = method_vis_rsmpl(raw, &pre);
        assert_eq!(out.matches("rule=").count(), 1, "got:\n{out}");
        assert!(out.contains("-     fn alpha(...) {"), "got:\n{out}");
        assert!(out.contains("+     pub fn alpha(...) {"), "got:\n{out}");
        assert!(
            out.contains("-    1\n+    2\n"),
            "literal hunk stays ordinary, got:\n{out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "impl Svc {\n    pub fn alpha(x: i32) {\n        x\n    }\n}\nfn helper() {\n    2\n}\n"
        );
    }

    #[test]
    fn method_vis_single_splits_mixed_hunk_with_literal() {
        // WI-14 slice-3 residue (b): one mixed hunk — the method-vis run
        // raises via the subhunk partition (scaled guard prices the
        // method run's filter block + extra header) while the literal
        // pair stays ordinary remainder. Real-apply byte-identical.
        let raw = "diff --git a/f.rs b/f.rs\nindex 1111111..2222222 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -2,2 +2,2 @@ impl Svc {\n-    fn alpha(x: i32) {\n-        1\n+    pub fn alpha(x: i32) {\n+        2\n";
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(x: i32) {\n        1\n    }\n}\n",
        )]);
        let out = method_vis_rsmpl(raw, &pre);
        assert_eq!(out.matches("rule=").count(), 1, "got:\n{out}");
        assert!(out.contains("-     fn alpha(...) {"), "got:\n{out}");
        assert!(out.contains("+     pub fn alpha(...) {"), "got:\n{out}");
        assert!(
            out.contains("-        1\n+        2\n"),
            "literal remainder stays ordinary, got:\n{out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "impl Svc {\n    pub fn alpha(x: i32) {\n        2\n    }\n}\n"
        );
    }
}
