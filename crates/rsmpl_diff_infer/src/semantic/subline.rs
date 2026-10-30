//! Sub-line site extraction (WI-4). Split from the detector modules
//! per the D5 hygiene gate.
//!
//! Whole-line detectors decline any line that does not parse as exactly
//! one call of their shape (`foo(1) + foo(2)` parses as `Binary`, not
//! `Call`). When every pair of a section fails whole-line the same way,
//! the pair may still carry a uniform rewrite at sub-line positions.
//! Each extractor below parses both sides as one `Expr` (terminator
//! stripped, `let`-init unwrapped — the `parse_call_expr` discipline),
//! collects the family subterms in source order, pairs them in order,
//! and checks the WI-9b2b frame rule on `flat_ws` text: the plus line
//! must equal the minus line under sequential subterm replacement, so
//! any frame delta or same-line distractor (a second site left
//! unchanged, hence a pair with no delta) vetoes. Turbofish sub-line
//! stays ordinary (generic-arg rendering risk — pinned, not attempted).

use quote::ToTokens;
use syn::visit::Visit;

/// One extracted bare-call site: the detector delta (rendered subterm
/// texts live only in the frame check below).
pub(crate) struct BareSite {
    pub(crate) old_cal: String,
    pub(crate) new_cal: String,
    pub(crate) arity: usize,
}

/// One extracted method→method site.
pub(crate) struct MethodSite {
    pub(crate) old_method: String,
    pub(crate) new_method: String,
    pub(crate) arity: usize,
}

/// One extracted method→function site.
pub(crate) struct MethodFnSite {
    pub(crate) old_method: String,
    pub(crate) new_func: String,
    pub(crate) arity: usize,
}

/// One extracted qualified-call site.
pub(crate) struct QualSite {
    pub(crate) prefix: (bool, Vec<String>),
    pub(crate) qself: (Option<String>, usize),
    pub(crate) old_cal: String,
    pub(crate) new_cal: String,
    pub(crate) arity: usize,
}

/// Parse one diff line as an expression: strip one trailing `;`/`,`
/// (call sites are sub-expressions; the terminator never reaches the
/// pattern), try `Expr`, else unwrap a `let`-init like `parse_call_expr`
/// (`let-else` never qualifies). `Expr::Let` scrutinees decline.
fn side_expr(line: &str) -> Option<syn::Expr> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return None;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if !matches!(expr, syn::Expr::Let(_)) {
            return Some(expr);
        }
    }
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            Some(*init.expr)
        }
        _ => None,
    }
}

/// Collect bare single-segment turbofish-free calls in source order.
fn bare_calls(expr: &syn::Expr) -> Vec<(String, Vec<syn::Expr>, String)> {
    struct C(Vec<(String, Vec<syn::Expr>, String)>);
    impl<'ast> Visit<'ast> for C {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    if p.qself.is_none()
                        && p.path.leading_colon.is_none()
                        && p.path.segments.len() == 1
                        && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
                    {
                        let name = p.path.segments[0].ident.to_string();
                        if super::ident_len(&name).is_some_and(|l| l == name.len()) {
                            self.0.push((
                                name,
                                call.args.iter().cloned().collect(),
                                node.to_token_stream().to_string(),
                            ));
                        }
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut c = C(Vec::new());
    c.visit_expr(expr);
    c.0
}

/// Collect turbofish-free method calls in source order.
fn method_calls(expr: &syn::Expr) -> Vec<(String, String, Vec<syn::Expr>, String)> {
    struct M(Vec<(String, String, Vec<syn::Expr>, String)>);
    impl<'ast> Visit<'ast> for M {
        fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
            if node.turbofish.is_none() {
                let method = node.method.to_string();
                if super::ident_len(&method).is_some_and(|l| l == method.len()) {
                    self.0.push((
                        method,
                        node.receiver.to_token_stream().to_string(),
                        node.args.iter().cloned().collect(),
                        syn::Expr::MethodCall(node.clone())
                            .to_token_stream()
                            .to_string(),
                    ));
                }
            }
            syn::visit::visit_expr_method_call(self, node);
        }
    }
    let mut m = M(Vec::new());
    m.visit_expr(expr);
    m.0
}

/// A collected qualified call: shared-prefix key, `qself` qualifier
/// key, last segment, value args, and the rendered subterm text for
/// the frame check.
type QualCall = (
    (bool, Vec<String>),
    (Option<String>, usize),
    String,
    Vec<syn::Expr>,
    String,
);

/// Collect multi-segment generic-free calls in source order (WI-7d:
/// leading `::` and `qself` are recorded as match keys, never vetoed).
fn qualified_calls(expr: &syn::Expr) -> Vec<QualCall> {
    struct Q(Vec<QualCall>);
    impl<'ast> Visit<'ast> for Q {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    if p.path.segments.len() >= 2 {
                        let leading = p.path.leading_colon.is_some();
                        let (qself, qself_pos) = super::qualifier_of(p);
                        let mut generic_free = true;
                        let mut segs = Vec::new();
                        for s in p.path.segments.iter() {
                            if !matches!(s.arguments, syn::PathArguments::None) {
                                generic_free = false;
                                break;
                            }
                            segs.push(s.ident.to_string());
                        }
                        if generic_free {
                            let last = segs.last().cloned().unwrap_or_default();
                            // Post-qualifier prefix only (the qualifier
                            // carries the trait path — never doubled);
                            // the qualifier itself rides the `qself` key.
                            let skip = qself_pos.min(segs.len().saturating_sub(1));
                            let pre = segs[skip..segs.len() - 1].to_vec();
                            if super::ident_len(&last).is_some_and(|l| l == last.len()) {
                                self.0.push((
                                    (leading, pre),
                                    (qself, qself_pos),
                                    last,
                                    call.args.iter().cloned().collect(),
                                    node.to_token_stream().to_string(),
                                ));
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut q = Q(Vec::new());
    q.visit_expr(expr);
    q.0
}

/// True when every line of the section is textually identical (the
/// verbatim domain): all-identical pairs belong to the tighter
/// frame-preserving verbatim sweep (T422 precedent), so sub-line
/// extraction declines and lets verbatim claim them. Hetero pairs —
/// what verbatim cannot express — stay sub-line territory.
pub(crate) fn all_identical_pairs(minuses: &[&str], pluses: &[&str]) -> bool {
    let (Some(first_minus), Some(first_plus)) = (minuses.first(), pluses.first()) else {
        return false;
    };
    minuses.iter().all(|m| m == first_minus) && pluses.iter().all(|p| p == first_plus)
}

/// Frame check: flat plus equals flat minus under sequential subterm
/// replacement (WI-9b2b discipline lifted to N subterms per line).
fn frame_holds(minus_line: &str, plus_line: &str, pairs: &[(String, String)]) -> bool {
    let mut cur = super::flat_ws(minus_line.trim());
    for (m, p) in pairs {
        let mf = super::flat_ws(m);
        let pf = super::flat_ws(p);
        if !cur.contains(&mf) {
            return false;
        }
        cur = cur.replacen(&mf, &pf, 1);
    }
    cur == super::flat_ws(plus_line.trim())
}

/// Extract bare-call sites from one line pair: same subterm count
/// (≥1), per-site token-equal args, one shared old/new callee within
/// the line, a real delta per site, and the frame rule.
pub(crate) fn extract_bare_sites(minus: &str, plus: &str) -> Option<Vec<BareSite>> {
    if minus.trim().starts_with('&') {
        return None;
    }
    let m_expr = side_expr(minus)?;
    let p_expr = side_expr(plus)?;
    let m_calls = bare_calls(&m_expr);
    let p_calls = bare_calls(&p_expr);
    if m_calls.is_empty() || m_calls.len() != p_calls.len() {
        return None;
    }
    let mut sites = Vec::new();
    let mut pairs = Vec::new();
    for ((o_cal, o_args, o_raw), (n_cal, n_args, n_raw)) in m_calls.iter().zip(p_calls.iter()) {
        if o_args.len() != n_args.len() {
            return None;
        }
        for (a, b) in o_args.iter().zip(n_args.iter()) {
            if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
                return None;
            }
        }
        if o_cal == n_cal {
            // No delta at this position: a same-line distractor or an
            // unchanged twin — veto the line, never silently skip it.
            return None;
        }
        sites.push(BareSite {
            old_cal: o_cal.clone(),
            new_cal: n_cal.clone(),
            arity: o_args.len(),
        });
        pairs.push((o_raw.clone(), n_raw.clone()));
    }
    // Within-line uniformity (cross-line uniformity is the caller's).
    let (o0, n0, a0) = (
        sites[0].old_cal.clone(),
        sites[0].new_cal.clone(),
        sites[0].arity,
    );
    if sites
        .iter()
        .any(|s| s.old_cal != o0 || s.new_cal != n0 || s.arity != a0)
    {
        return None;
    }
    if !frame_holds(minus, plus, &pairs) {
        return None;
    }
    Some(sites)
}

/// Extract method→method sites from one line pair.
pub(crate) fn extract_method_sites(minus: &str, plus: &str) -> Option<Vec<MethodSite>> {
    if minus.trim().starts_with('&') || plus.trim().starts_with('&') {
        return None;
    }
    let m_expr = side_expr(minus)?;
    let p_expr = side_expr(plus)?;
    let m_calls = method_calls(&m_expr);
    let p_calls = method_calls(&p_expr);
    if m_calls.is_empty() || m_calls.len() != p_calls.len() {
        return None;
    }
    let mut sites = Vec::new();
    let mut pairs = Vec::new();
    for ((o_m, o_r, o_args, o_raw), (n_m, n_r, n_args, n_raw)) in m_calls.iter().zip(p_calls.iter())
    {
        if o_r != n_r || o_args.len() != n_args.len() {
            return None;
        }
        for (a, b) in o_args.iter().zip(n_args.iter()) {
            if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
                return None;
            }
        }
        if o_m == n_m {
            return None;
        }
        sites.push(MethodSite {
            old_method: o_m.clone(),
            new_method: n_m.clone(),
            arity: o_args.len(),
        });
        pairs.push((o_raw.clone(), n_raw.clone()));
    }
    let (o0, n0, a0) = (
        sites[0].old_method.clone(),
        sites[0].new_method.clone(),
        sites[0].arity,
    );
    if sites
        .iter()
        .any(|s| s.old_method != o0 || s.new_method != n0 || s.arity != a0)
    {
        return None;
    }
    if !frame_holds(minus, plus, &pairs) {
        return None;
    }
    Some(sites)
}

/// Extract method→function sites from one line pair: each minus
/// method call pairs with a plus bare call whose first arg is the
/// minus receiver and whose rest args are token-equal.
pub(crate) fn extract_method_fn_sites(minus: &str, plus: &str) -> Option<Vec<MethodFnSite>> {
    if minus.trim().starts_with('&') {
        return None;
    }
    let m_expr = side_expr(minus)?;
    let p_expr = side_expr(plus)?;
    let m_calls = method_calls(&m_expr);
    let p_calls = bare_calls(&p_expr);
    if m_calls.is_empty() || m_calls.len() != p_calls.len() {
        return None;
    }
    let mut sites = Vec::new();
    let mut pairs = Vec::new();
    for ((o_m, o_r, o_args, o_raw), (n_cal, n_args, n_raw)) in m_calls.iter().zip(p_calls.iter()) {
        if n_args.len() != o_args.len() + 1 {
            return None;
        }
        if n_args.is_empty() || n_args[0].to_token_stream().to_string() != *o_r {
            return None;
        }
        for (a, b) in o_args.iter().zip(n_args.iter().skip(1)) {
            if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
                return None;
            }
        }
        sites.push(MethodFnSite {
            old_method: o_m.clone(),
            new_func: n_cal.clone(),
            arity: o_args.len(),
        });
        pairs.push((o_raw.clone(), n_raw.clone()));
    }
    let (o0, n0, a0) = (
        sites[0].old_method.clone(),
        sites[0].new_func.clone(),
        sites[0].arity,
    );
    if sites
        .iter()
        .any(|s| s.old_method != o0 || s.new_func != n0 || s.arity != a0)
    {
        return None;
    }
    if !frame_holds(minus, plus, &pairs) {
        return None;
    }
    Some(sites)
}

/// Extract qualified-call sites from one line pair: shared prefix
/// within the line, last-segment delta per site, token-equal args.
pub(crate) fn extract_qual_sites(minus: &str, plus: &str) -> Option<Vec<QualSite>> {
    let m_expr = side_expr(minus)?;
    let p_expr = side_expr(plus)?;
    let m_calls = qualified_calls(&m_expr);
    let p_calls = qualified_calls(&p_expr);
    if m_calls.is_empty() || m_calls.len() != p_calls.len() {
        return None;
    }
    let mut sites = Vec::new();
    let mut pairs = Vec::new();
    for ((o_pre, o_q, o_last, o_args, o_raw), (n_pre, n_q, n_last, n_args, n_raw)) in
        m_calls.iter().zip(p_calls.iter())
    {
        if o_pre != n_pre || o_q != n_q || o_args.len() != n_args.len() {
            return None;
        }
        for (a, b) in o_args.iter().zip(n_args.iter()) {
            if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
                return None;
            }
        }
        if o_last == n_last {
            return None;
        }
        sites.push(QualSite {
            prefix: o_pre.clone(),
            qself: o_q.clone(),
            old_cal: o_last.clone(),
            new_cal: n_last.clone(),
            arity: o_args.len(),
        });
        pairs.push((o_raw.clone(), n_raw.clone()));
    }
    let (p0, q0, o0, n0, a0) = (
        sites[0].prefix.clone(),
        sites[0].qself.clone(),
        sites[0].old_cal.clone(),
        sites[0].new_cal.clone(),
        sites[0].arity,
    );
    if sites.iter().any(|s| {
        s.prefix != p0 || s.qself != q0 || s.old_cal != o0 || s.new_cal != n0 || s.arity != a0
    }) {
        return None;
    }
    if !frame_holds(minus, plus, &pairs) {
        return None;
    }
    Some(sites)
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
    fn bare_two_per_line_fires() {
        // WI-4: two `foo` calls per line anti-unify to one bare-call
        // hunk; totality counts occurrences (4), not lines (2).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    foo(1) + foo(2);\n-    foo(3) + foo(4);\n+    bar(1) + bar(2);\n+    bar(3) + bar(4);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(1) + foo(2);\n    foo(3) + foo(4);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
    }

    #[test]
    fn bare_two_per_line_distractor_vetoes() {
        // Same-line distractor: the second call never changes, so the
        // line carries no uniform rewrite — stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    foo(1) + foo(2);\n-    foo(3) + foo(4);\n+    bar(1) + foo(2);\n+    bar(3) + foo(4);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn bare_two_per_line_frame_change_vetoes() {
        // The frame around the calls changed too (`x` → `y`) — not a
        // pure subterm rewrite, stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    foo(1) + x;\n-    foo(2) + x;\n+    bar(1) + y;\n+    bar(2) + y;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn bare_two_per_line_hetero_callee_vetoes() {
        // Different callees per position in one line — hetero, stays
        // ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    foo(1) + baz(2);\n-    foo(3) + baz(4);\n+    bar(1) + qux(2);\n+    bar(3) + qux(4);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn bare_two_per_line_extra_twin_vetoes() {
        // Occurrence totality: an untouched `foo` twin outside the diff
        // counts (6 sites vs 4 diff sites) and vetoes.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn m() {\n-    foo(1) + foo(2);\n+    bar(1) + bar(2);\n }\n",
        );
        let pre = pre_map(&[("f.rs", "fn m() {\n    foo(1) + foo(2);\n    foo(9);\n}\n")]);
        assert!(
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
                .is_none()
        );
    }

    #[test]
    fn bare_two_per_line_single_pair_stays_ordinary() {
        // N = 1 pair stays ordinary even with two sites (T375: no token
        // win for a single pair as a sweep).
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn m() {\n-    foo(1) + foo(2);\n+    bar(1) + bar(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_two_per_line_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let x = a.old(1) + b.old(2);\n-    let y = c.old(3) + d.old(4);\n-    let z = e.old(5) + f.old(6);\n+    let x = a.new(1) + b.new(2);\n+    let y = c.new(3) + d.new(4);\n+    let z = e.new(5) + f.new(6);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let x = a.old(1) + b.old(2);\n    let y = c.old(3) + d.old(4);\n    let z = e.old(5) + f.old(6);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- Receiver.old(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new(E0)\n"), "got: {out}");
    }

    #[test]
    fn method_fn_two_per_line_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let x = a.old(1) + b.old(2);\n-    let y = c.old(3) + d.old(4);\n-    let z = e.old(5) + f.old(6);\n+    let x = new(a, 1) + new(b, 2);\n+    let y = new(c, 3) + new(d, 4);\n+    let z = new(e, 5) + new(f, 6);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let x = a.old(1) + b.old(2);\n    let y = c.old(3) + d.old(4);\n    let z = e.old(5) + f.old(6);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- Receiver.old(E0)\n"), "got: {out}");
        assert!(out.contains("+ new(Receiver, E0)\n"), "got: {out}");
    }

    #[test]
    fn qualified_two_per_line_fires() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let x = crate::api::old(1) + crate::api::old(2);\n-    let y = crate::api::old(3) + crate::api::old(4);\n-    let z = crate::api::old(5) + crate::api::old(6);\n+    let x = crate::api::new(1) + crate::api::new(2);\n+    let y = crate::api::new(3) + crate::api::new(4);\n+    let z = crate::api::new(5) + crate::api::new(6);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let x = crate::api::old(1) + crate::api::old(2);\n    let y = crate::api::old(3) + crate::api::old(4);\n    let z = crate::api::old(5) + crate::api::old(6);\n}\n",
        )]);
        let out =
            super::super::infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- crate::api::old(E0)\n"), "got: {out}");
        assert!(out.contains("+ crate::api::new(E0)\n"), "got: {out}");
    }
}

#[cfg(test)]
mod verbatim_deferral_tests {
    use super::super::infer_semantic;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    #[test]
    fn identical_unary_pairs_defer_to_verbatim() {
        // T422: all-identical pairs belong to the frame-preserving
        // verbatim sweep even though each line holds a uniform
        // sub-line call delta — the call sweep must not shadow it
        // (bench-15 / fixture-37 shape).
        let raw = section(
            "f.rs",
            "@@ -9,3 +9,3 @@\n fn m(flag: i32) {\n-    !ready(flag);\n-    !ready(flag);\n+    !ready_done(flag);\n+    !ready_done(flag);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).expect("must compress");
        assert!(out.contains("- !ready(flag)\n"), "got: {out}");
        assert!(out.contains("+ !ready_done(flag)\n"), "got: {out}");
        assert!(
            !out.contains("- ready(E0)\n"),
            "call sweep must not shadow, got: {out}"
        );
    }
}
