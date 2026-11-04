//! WI-4 slice 2 counted verdicts for the remaining chain families.
//!
//! Sibling of `totality.rs` (which sits at the 1900-line watch line —
//! D5: new code lands here, never there). Same discipline as slice 1
//! (`CallVerdict`/`MethodVerdict`/`LetUntypedVerdict`): a shape match
//! with a twin elsewhere names its counts (`totality:found:want`) or
//! its opaque twin (`veto:opaque-hit`) instead of a bare `no-match`.
//!
//! Two probe kinds:
//! - Engine-probe counts (`template_*_counts`): the scratch self/real-
//!   pair probe the matching `*_holds` runs, but returning the file-wide
//!   match count (`Some(found)`) instead of `bool`. `None` = unproven
//!   (no pre-image entry, bad template, probe parse/apply failure).
//!   No opaque arm by construction (gap-4 statement): the engine never
//!   matches inside macro bodies or string literals, so hidden twins
//!   never count — there is no opaque text to veto on. `None` pre-image
//!   maps to `Holds` at the call site (legacy section-visible allow).
//! - Syntax-walk verdicts (`*_verdict`): the same file-wide walk the
//!   matching `*_holds` runs, returning the count plus the line-aware
//!   opaque arm (macro/string twin covering a changed old-side line).

use syn::visit::Visit;

/// Shared counted outcome for every slice-2 family (same shape as the
/// slice-1 verdicts). `Unproven` is the legacy unparseable-pre-image
/// veto (callers keep the `no-match` mapping — no pre-image means no
/// counts to name). `Holds` covers the legacy `None`-pre-image allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FamilyVerdict {
    Holds,
    CountMismatch { found: usize, want: usize },
    OpaqueHit,
    Unproven,
}

fn parse_pre(file_old: Option<&str>) -> Option<syn::File> {
    crate::probe_cache::cached_parse_pre(file_old)
}

fn opaque_hit(minuses: &[&str], file: &syn::File, changed: &[usize]) -> bool {
    struct Scan<'a> {
        needles: Vec<String>,
        changed: &'a [usize],
        hit: bool,
    }
    impl Scan<'_> {
        fn check(&mut self, hay: &str, start: usize, end: usize) {
            if self.hit {
                return;
            }
            for n in &self.needles {
                if crate::semantic::count_occurrences(hay, n) > 0
                    && crate::totality::covers_changed(start, end, self.changed)
                {
                    self.hit = true;
                    return;
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            let hay = crate::semantic::flat_ws(&node.tokens.to_string());
            let span = syn::spanned::Spanned::span(node);
            self.check(&hay, span.start().line, span.end().line);
        }
        fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
            let hay = crate::semantic::flat_ws(&node.value());
            let span = syn::spanned::Spanned::span(node);
            self.check(&hay, span.start().line, span.end().line);
        }
    }
    let needles: Vec<String> = minuses
        .iter()
        .filter_map(|m| {
            let t = m.trim();
            if t.is_empty() {
                return None;
            }
            let s = crate::semantic::strip_one_terminator(t).unwrap_or(t);
            let f = crate::semantic::flat_ws(s);
            if f.is_empty() {
                None
            } else {
                Some(f)
            }
        })
        .collect();
    let mut s = Scan {
        needles,
        changed,
        hit: false,
    };
    s.visit_file(file);
    s.hit
}

/// Engine real-pair count: apply `- minus` / `+ plus` under `# sweep`
/// to a scratch tree, return the file-wide match count.
fn real_pair_count(path: &str, pre: &str, minus: &str, plus: &str) -> Option<usize> {
    if minus.contains('\n') || plus.contains('\n') {
        return None;
    }
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n- {minus}\n+ {plus}\n",
        crate::git_header(path)
    );
    crate::probe_cache::run_probe(path, pre, &probe)
}

/// Slice-2a engine counts: real-pair probe (`template_pair_holds`
/// prescription). `None` pre-image is not callable — callers map it to
/// `Holds` before invoking (legacy allow).
pub(crate) fn template_pair_counts(
    path: &str,
    file_old: Option<&str>,
    minus: &str,
    plus: &str,
) -> Option<usize> {
    let pre = file_old?;
    real_pair_count(path, pre, minus, plus)
}

/// Fn-header engine count (`template_fn_holds` prescription, free-fn
/// `None` header). Method-block probes pass their verbatim header via
/// `template_fn_block_counts` below.
pub(crate) fn template_fn_counts(
    path: &str,
    file_old: Option<&str>,
    minus_header: &str,
) -> Option<usize> {
    let pre = file_old?;
    if minus_header.contains('\n') {
        return None;
    }
    let body = format!("- {minus_header}\n+ {minus_header}\n      ...\n  }}\n");
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n{body}",
        crate::git_header(path)
    );
    crate::probe_cache::run_probe(path, pre, &probe)
}

/// Fn-header engine count inside a verbatim `impl` block header (the
/// `probe_block_hunk` prescription with a self-pair: `block_hunk_body`
/// shape, `header` verbatim including its `{` — no extra brace).
pub(crate) fn template_fn_block_counts(
    path: &str,
    file_old: Option<&str>,
    minus_header: &str,
    header: &str,
) -> Option<usize> {
    let pre = file_old?;
    if minus_header.contains('\n') {
        return None;
    }
    let body = format!(
        "  {header}\n      ...\n-     {minus_header}\n+     {minus_header}\n          ...\n      }}\n  }}\n"
    );
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n{body}",
        crate::git_header(path)
    );
    crate::probe_cache::run_probe(path, pre, &probe)
}

/// Map an engine `Option<usize>` count to a verdict: `None` pre-image
/// holds (legacy allow); `None` with a pre-image is unproven; mismatch
/// names both counts.
pub(crate) fn engine_verdict(
    file_old: Option<&str>,
    found: Option<usize>,
    want: usize,
) -> FamilyVerdict {
    if file_old.is_none() {
        return FamilyVerdict::Holds;
    }
    match found {
        Some(f) if f == want => FamilyVerdict::Holds,
        Some(f) => FamilyVerdict::CountMismatch { found: f, want },
        None => FamilyVerdict::Unproven,
    }
}

/// WI-4 slice 2b-rest-b: sequence engine count
/// (`template_sequence_holds` prescription). `None` pre-image is not
/// callable — callers map it to `Holds` before invoking (legacy allow).
pub(crate) fn template_sequence_counts(
    path: &str,
    file_old: Option<&str>,
    minus_tpls: &[String],
    plus_tpls: &[String],
) -> Option<usize> {
    let pre = file_old?;
    if minus_tpls.len() != plus_tpls.len() || !(2..=4).contains(&minus_tpls.len()) {
        return None;
    }
    if minus_tpls.iter().any(|l| l.contains('\n')) || plus_tpls.iter().any(|l| l.contains('\n')) {
        return None;
    }
    let mut probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n",
        crate::git_header(path)
    );
    for m in minus_tpls {
        probe.push_str(&format!("- {m}\n"));
    }
    for p in plus_tpls {
        probe.push_str(&format!("+ {p}\n"));
    }
    crate::probe_cache::run_probe(path, pre, &probe)
}

/// WI-4 slice 2b-rest-b: fold engine count (`template_repeat_holds`
/// prescription: `# sweep` + `# repeat`, `- M` / `+ E0`). `None`
/// pre-image is not callable — callers map it to `Holds` first.
pub(crate) fn template_repeat_counts(
    path: &str,
    file_old: Option<&str>,
    minus_template: &str,
) -> Option<usize> {
    let pre = file_old?;
    if minus_template.contains('\n') {
        return None;
    }
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# repeat\n- {minus_template}\n+ E0\n",
        crate::git_header(path)
    );
    crate::probe_cache::run_probe(path, pre, &probe)
}

/// WI-4 slice 2b-rest-b: generic expression engine count
/// (`template_expr_holds` prescription: `- M` / `+ M` self-rewrite
/// under `# sweep` + `# optional`, engine-applied to a scratch tree,
/// match count must equal `n`). `None` pre-image is not callable —
/// callers map it to `Holds` before invoking (legacy allow).
pub(crate) fn template_expr_counts(
    path: &str,
    file_old: Option<&str>,
    minus_template: &str,
) -> Option<usize> {
    let pre = file_old?;
    if minus_template.contains('\n') {
        return None;
    }
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n- {minus_template}\n+ {minus_template}\n",
        crate::git_header(path)
    );
    crate::probe_cache::run_probe(path, pre, &probe)
}

/// WI-4 slice 2b-rest-b: peel verdict (mirrors `peel_holds` — the
/// `cal`/`head`/`len` syntax walk plus the line-aware opaque arm).
pub(crate) fn peel_verdict(
    file_old: Option<&str>,
    cal: &str,
    head: &str,
    len: usize,
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct P<'a> {
        cal: &'a str,
        head: &'a str,
        len: usize,
        count: usize,
    }
    impl<'ast> Visit<'ast> for P<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    if p.qself.is_none()
                        && p.path.leading_colon.is_none()
                        && p.path.segments.len() == 1
                        && p.path.segments[0].ident == self.cal
                        && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
                        && call.args.len() == 2
                    {
                        let mut args = call.args.iter();
                        let _first = args.next();
                        if let Some(syn::Expr::Reference(r)) = args.next() {
                            if r.mutability.is_none() {
                                if let syn::Expr::Array(arr) = r.expr.as_ref() {
                                    if arr.elems.len() == self.len {
                                        let mut elems = arr.elems.iter();
                                        if let Some(syn::Expr::Lit(lit)) = elems.next() {
                                            if let syn::Lit::Str(s) = &lit.lit {
                                                if s.value() == self.head {
                                                    self.count += 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut p = P {
        cal,
        head,
        len,
        count: 0,
    };
    p.visit_file(&file);
    if p.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: p.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-rest-b: generic twin counter for scratch-probe
/// families (member/assoc/definition/macro/trait/inherent/attr).
/// Counts pre-image lines whose trimmed text matches a minus line
/// (single trailing `,`/`;` tolerated — patterns omit member commas).
/// `None` pre-image is not callable (detectors needing it map to `Miss`
/// before invoking); unparseable pre-image is `Unproven`. A mismatch
/// names the twin class; a match means "no count blocker" (`Holds` —
/// the caller's scratch probe stays authoritative for the fire).
pub(crate) fn scratch_line_verdict(
    file_old: Option<&str>,
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(pre) = file_old else {
        return FamilyVerdict::Holds;
    };
    let _ = changed;
    let pre_lines: Vec<&str> = pre.lines().collect();
    let norm = |s: &str| {
        let t = s.trim().trim_end_matches(',').trim_end_matches(';').trim();
        t.to_string()
    };
    let wants: Vec<String> = minuses.iter().map(|m| norm(m)).collect();
    if wants.iter().any(|w| w.is_empty()) {
        return FamilyVerdict::Unproven;
    }
    let mut found = 0usize;
    for line in &pre_lines {
        let got = norm(line);
        if wants.contains(&got) {
            found += 1;
        }
    }
    if found != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found,
            want: minuses.len(),
        };
    }
    FamilyVerdict::Holds
}

use quote::ToTokens;

/// Call trailing-literal verdict (mirrors `call_literal_holds`).
pub(crate) fn call_literal_verdict(
    file_old: Option<&str>,
    cal: &str,
    arity: usize,
    old_trailing: &str,
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    let want = crate::semantic::flat_ws(old_trailing);
    struct L<'a> {
        cal: &'a str,
        arity: usize,
        want: &'a str,
        count: usize,
    }
    impl<'ast> Visit<'ast> for L<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    if p.qself.is_none()
                        && p.path.leading_colon.is_none()
                        && p.path.segments.len() == 1
                        && p.path.segments[0].ident == self.cal
                        && matches!(p.path.segments[0].arguments, syn::PathArguments::None)
                        && call.args.len() == self.arity
                    {
                        if let Some(last) = call.args.last() {
                            if crate::semantic::flat_ws(&last.to_token_stream().to_string())
                                == self.want
                            {
                                self.count += 1;
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut l = L {
        cal,
        arity,
        want: &want,
        count: 0,
    };
    l.visit_file(&file);
    if l.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: l.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// Method same-literal verdict (mirrors `method_literal_holds`).
pub(crate) fn method_literal_verdict(
    file_old: Option<&str>,
    method: &str,
    old_args: &[String],
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct L<'a> {
        method: &'a str,
        old_args: &'a [String],
        count: usize,
    }
    impl<'ast> Visit<'ast> for L<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::MethodCall(m) = node {
                if m.turbofish.is_none()
                    && m.method == self.method
                    && m.args.len() == self.old_args.len()
                {
                    let ok = m.args.iter().zip(self.old_args.iter()).all(|(a, b)| {
                        crate::semantic::flat_ws(&a.to_token_stream().to_string())
                            == crate::semantic::flat_ws(b)
                    });
                    if ok {
                        self.count += 1;
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut l = L {
        method,
        old_args,
        count: 0,
    };
    l.visit_file(&file);
    if l.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: l.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// `&`-strip verdict (mirrors `strip_holds`).
pub(crate) fn strip_verdict(
    file_old: Option<&str>,
    method: &str,
    args: &[String],
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct S<'a> {
        method: &'a str,
        args: &'a [String],
        count: usize,
    }
    impl<'ast> Visit<'ast> for S<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Reference(r) = node {
                if r.mutability.is_none() {
                    if let syn::Expr::MethodCall(m) = r.expr.as_ref() {
                        if m.turbofish.is_none()
                            && m.method == self.method
                            && m.args.len() == self.args.len()
                        {
                            let ok = m.args.iter().zip(self.args.iter()).all(|(a, b)| {
                                crate::semantic::flat_ws(&a.to_token_stream().to_string())
                                    == crate::semantic::flat_ws(b)
                            });
                            if ok {
                                self.count += 1;
                            }
                        }
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut s = S {
        method,
        args,
        count: 0,
    };
    s.visit_file(&file);
    if s.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: s.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-1: slot verdict (mirrors `slot_holds`).
pub(crate) fn slot_verdict(
    file_old: Option<&str>,
    old: &str,
    indented: bool,
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    fn free_items(items: &[syn::Item], old: &str, out: &mut usize) {
        for it in items {
            match it {
                syn::Item::Fn(f) if f.sig.ident == old => *out += 1,
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        free_items(inner, old, out);
                    }
                }
                _ => {}
            }
        }
    }
    fn impl_items(items: &[syn::Item], old: &str, out: &mut usize) {
        for it in items {
            match it {
                syn::Item::Impl(imp) => {
                    for m in &imp.items {
                        if let syn::ImplItem::Fn(f) = m {
                            if f.sig.ident == old {
                                *out += 1;
                            }
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        impl_items(inner, old, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut count = 0;
    if indented {
        impl_items(&file.items, old, &mut count);
    } else {
        free_items(&file.items, old, &mut count);
    }
    if count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-1: qualified-call verdict (mirrors
/// `qualified_call_holds`).
pub(crate) fn qualified_call_verdict(
    file_old: Option<&str>,
    old_path: &str,
    arity: usize,
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct C<'a> {
        old_path: &'a str,
        arity: usize,
        count: usize,
    }
    impl<'ast> Visit<'ast> for C<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    if p.path.segments.len() >= 2
                        && p.path
                            .segments
                            .iter()
                            .all(|s| matches!(s.arguments, syn::PathArguments::None))
                        && call.args.len() == self.arity
                        && crate::totality::render_full_path(p) == self.old_path
                    {
                        self.count += 1;
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut c = C {
        old_path,
        arity,
        count: 0,
    };
    c.visit_file(&file);
    if c.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: c.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-1: turbofish-call verdict (mirrors
/// `turbofish_call_holds`).
pub(crate) fn turbofish_call_verdict(
    file_old: Option<&str>,
    old_path: &str,
    val_arity: usize,
    turbo_arity: usize,
    concrete: &[(usize, String)],
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct C<'a> {
        old_path: &'a str,
        val_arity: usize,
        turbo_arity: usize,
        concrete: &'a [(usize, String)],
        count: usize,
    }
    impl<'ast> Visit<'ast> for C<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    if p.qself.is_none()
                        && call.args.len() == self.val_arity
                        && crate::totality::render_full_path(p) == self.old_path
                        && crate::totality::turbo_concrete_holds(p, self.turbo_arity, self.concrete)
                    {
                        self.count += 1;
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut c = C {
        old_path,
        val_arity,
        turbo_arity,
        concrete,
        count: 0,
    };
    c.visit_file(&file);
    if c.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: c.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-1: turbofish-method verdict (mirrors
/// `turbofish_method_holds`).
pub(crate) fn turbofish_method_verdict(
    file_old: Option<&str>,
    method: &str,
    val_arity: usize,
    turbo_arity: usize,
    concrete: &[(usize, String)],
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct M<'a> {
        method: &'a str,
        val_arity: usize,
        turbo_arity: usize,
        concrete: &'a [(usize, String)],
        count: usize,
    }
    impl<'ast> Visit<'ast> for M<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::MethodCall(m) = node {
                if m.method == self.method
                    && m.args.len() == self.val_arity
                    && m.turbofish.as_ref().is_some_and(|t| {
                        t.args.len() == self.turbo_arity
                            && self.concrete.iter().all(|(i, want)| {
                                t.args.iter().nth(*i).is_some_and(|a| {
                                    crate::semantic::flat_ws(&a.to_token_stream().to_string())
                                        == crate::semantic::flat_ws(want)
                                })
                            })
                    })
                {
                    self.count += 1;
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut m = M {
        method,
        val_arity,
        turbo_arity,
        concrete,
        count: 0,
    };
    m.visit_file(&file);
    if m.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: m.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-1: use-prefix verdict (mirrors `use_holds`).
pub(crate) fn use_verdict(
    file_old: Option<&str>,
    old_prefix: &[String],
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    fn collect<'a>(items: &'a [syn::Item], out: &mut Vec<&'a syn::UseTree>) {
        for it in items {
            match it {
                syn::Item::Use(u) => out.push(&u.tree),
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        collect(inner, out);
                    }
                }
                _ => {}
            }
        }
    }
    fn walk(tree: &syn::UseTree, minus: &[String], consumed: usize, out: &mut usize) {
        match tree {
            syn::UseTree::Path(p) => {
                if consumed < minus.len() && p.ident == minus[consumed] {
                    if consumed + 1 == minus.len() {
                        *out += 1;
                    } else {
                        walk(&p.tree, minus, consumed + 1, out);
                    }
                }
            }
            syn::UseTree::Group(g) => {
                for m in &g.items {
                    walk(m, minus, consumed, out);
                }
            }
            syn::UseTree::Name(nm) => {
                if consumed + 1 == minus.len()
                    && consumed < minus.len()
                    && nm.ident == minus[consumed]
                {
                    *out += 1;
                }
            }
            syn::UseTree::Rename(r) => {
                if consumed + 1 == minus.len()
                    && consumed < minus.len()
                    && r.ident == minus[consumed]
                {
                    *out += 1;
                }
            }
            syn::UseTree::Glob(_) => {}
        }
    }
    let mut trees = Vec::new();
    collect(&file.items, &mut trees);
    let mut count = 0;
    for t in trees {
        walk(t, old_prefix, 0, &mut count);
    }
    if count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

/// WI-4 slice 2b-1: macro-rename verdict (mirrors `macro_holds` /
/// `qualified_macro_holds`; `qualified` selects the exact-path walk).
pub(crate) fn macro_verdict(
    file_old: Option<&str>,
    old_path: &str,
    qualified: bool,
    minuses: &[&str],
    changed: &[usize],
) -> FamilyVerdict {
    let Some(file) = parse_pre(file_old) else {
        if file_old.is_none() {
            return FamilyVerdict::Holds;
        }
        return FamilyVerdict::Unproven;
    };
    struct M<'a> {
        old_path: &'a str,
        qualified: bool,
        count: usize,
    }
    impl<'ast> Visit<'ast> for M<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if self.qualified {
                if node.path.leading_colon.is_none()
                    && crate::totality::render_macro_path(node) == self.old_path
                {
                    self.count += 1;
                }
            } else if node.path.leading_colon.is_none()
                && node.path.segments.len() == 1
                && node.path.segments[0].ident == self.old_path
                && matches!(node.path.segments[0].arguments, syn::PathArguments::None)
            {
                self.count += 1;
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let mut m = M {
        old_path,
        qualified,
        count: 0,
    };
    m.visit_file(&file);
    if m.count != minuses.len() {
        return FamilyVerdict::CountMismatch {
            found: m.count,
            want: minuses.len(),
        };
    }
    if opaque_hit(minuses, &file, changed) {
        return FamilyVerdict::OpaqueHit;
    }
    FamilyVerdict::Holds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wi4_slice2a_engine_counts_name_both_counts() {
        // Real-pair probe: 3 file-wide sites vs 2 claimed.
        let pre = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";
        let found = template_pair_counts("f.rs", Some(pre), "foo(E0)", "bar(E0)").unwrap();
        assert_eq!(found, 3);
        assert_eq!(
            engine_verdict(Some(pre), Some(found), 2),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Exact holds; no pre-image holds (legacy allow); unproven on
        // unparseable pre-image.
        let pre2 = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        let found2 = template_pair_counts("f.rs", Some(pre2), "foo(E0)", "bar(E0)").unwrap();
        assert_eq!(
            engine_verdict(Some(pre2), Some(found2), 2),
            FamilyVerdict::Holds
        );
        assert_eq!(engine_verdict(None, None, 2), FamilyVerdict::Holds);
        assert_eq!(
            engine_verdict(Some("not rust {{{"), None, 2),
            FamilyVerdict::Unproven
        );
    }

    #[test]
    fn wi4_slice2a_fn_counts_name_mismatch() {
        // Free-fn header: 3 same-name sites vs 2 claimed.
        let pre = "fn alpha(x: i32) {\n    x\n}\nfn alpha(y: i32) {\n    y\n}\nfn alpha(z: i32) {\n    z\n}\n";
        let found = template_fn_counts("f.rs", Some(pre), "fn alpha(...) {").unwrap();
        assert_eq!(
            engine_verdict(Some(pre), Some(found), 2),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Block probe agrees with the scratch-apply prescription.
        let bpre = "impl Svc {\n    fn alpha(x: i32) {\n        x\n    }\n    fn alpha(y: i32) {\n        y\n    }\n}\n";
        let bfound =
            template_fn_block_counts("f.rs", Some(bpre), "fn alpha(...) {", "impl Svc {").unwrap();
        assert_eq!(
            engine_verdict(Some(bpre), Some(bfound), 2),
            FamilyVerdict::Holds
        );
    }

    #[test]
    fn wi4_slice2a_walk_verdicts_name_counts_and_opaque() {
        let minuses = ["    foo(1);", "    foo(2);"];
        let changed = [2, 3];
        // Call-literal twin: 3 trailing-`1` sites vs 2 claimed.
        assert_eq!(
            call_literal_verdict(
                Some("fn m() {\n    foo(1);\n    foo(1);\n    foo(1);\n}\n"),
                "foo",
                1,
                "1",
                &minuses,
                &changed,
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Method-literal exact holds.
        assert_eq!(
            method_literal_verdict(
                Some("fn m() {\n    r.old(1, 2);\n    r.old(1, 2);\n}\n"),
                "old",
                &["1".to_string(), "2".to_string()],
                &["    r.old(1, 2);", "    r.old(1, 2);"],
                &[2, 3],
            ),
            FamilyVerdict::Holds
        );
        // Opaque macro twin on a changed line.
        assert_eq!(
            call_literal_verdict(
                Some("fn m() {\n    foo(1);\n    g(m!(foo(1)));\n    foo(1);\n}\n"),
                "foo",
                1,
                "1",
                &minuses,
                &changed,
            ),
            FamilyVerdict::OpaqueHit
        );
        // Unproven on unparseable pre-image; Holds on None.
        assert_eq!(
            strip_verdict(
                Some("not rust {{{five"),
                "m",
                &["1".to_string()],
                &minuses,
                &changed
            ),
            FamilyVerdict::Unproven
        );
        assert_eq!(
            strip_verdict(None, "m", &["1".to_string()], &minuses, &changed),
            FamilyVerdict::Holds
        );
        // Strip mismatch names both counts.
        assert_eq!(
            strip_verdict(
                Some("fn m() {\n    let x = &r.old(1);\n    let y = &r.old(1);\n    let z = &r.old(1);\n}\n"),
                "old",
                &["1".to_string()],
                &["    let x = &r.old(1);", "    let y = &r.old(1);"],
                &[2, 3],
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
    }

    #[test]
    fn wi4_slice2b1_path_header_verdicts_name_counts() {
        // Slot: 3 same-name free fns vs 2 claimed.
        let slot_pre =
            "fn old(x: i32) {\n    x\n}\nfn old(y: i32) {\n    y\n}\nfn old(z: i32) {\n    z\n}\n";
        assert_eq!(
            slot_verdict(Some(slot_pre), "old", false, &["m1", "m2"], &[1, 5]),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        assert_eq!(
            slot_verdict(
                Some("fn old(x: i32) {\n    x\n}\nfn old(y: i32) {\n    y\n}\n"),
                "old",
                false,
                &["m1", "m2"],
                &[1, 4]
            ),
            FamilyVerdict::Holds
        );
        assert_eq!(
            slot_verdict(None, "old", false, &["m1", "m2"], &[1, 2]),
            FamilyVerdict::Holds
        );
        // Qualified: 3 `crate::old` sites vs 2 claimed; opaque twin.
        let qpre = "fn m() {\n    crate::old(1);\n    crate::old(2);\n    crate::old(3);\n}\n";
        assert_eq!(
            qualified_call_verdict(
                Some(qpre),
                "crate::old",
                1,
                &["    crate::old(1);", "    crate::old(2);"],
                &[2, 3]
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        assert_eq!(
            qualified_call_verdict(
                Some("fn m() {\n    crate::old(1);\n    g(m!(crate::old(1)));\n    crate::old(1);\n}\n"),
                "crate::old",
                1,
                &["    crate::old(1);", "    crate::old(1);"],
                &[2, 3]
            ),
            FamilyVerdict::OpaqueHit
        );
        // Turbofish call: 3 `foo::<u8>` sites vs 2 claimed.
        let tpre = "fn m() {\n    foo::<u8>(x);\n    foo::<u8>(y);\n    foo::<u8>(z);\n}\n";
        assert_eq!(
            turbofish_call_verdict(
                Some(tpre),
                "foo",
                1,
                1,
                &[(0, "u8".to_string())],
                &["    foo::<u8>(x);", "    foo::<u8>(y);"],
                &[2, 3]
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Turbofish method: exact holds.
        assert_eq!(
            turbofish_method_verdict(
                Some("fn m() {\n    r.old::<u8>(1);\n    r.old::<u8>(1);\n}\n"),
                "old",
                1,
                1,
                &[(0, "u8".to_string())],
                &["    r.old::<u8>(1);", "    r.old::<u8>(1);"],
                &[2, 3]
            ),
            FamilyVerdict::Holds
        );
        // Use-prefix: 3 same-prefix uses vs 2 claimed.
        let upre = "use crate::old::A;\nuse crate::old::B;\nuse crate::old::C;\n";
        assert_eq!(
            use_verdict(
                Some(upre),
                &["crate".to_string(), "old".to_string()],
                &["use crate::old::A;", "use crate::old::B;"],
                &[1, 2]
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        assert_eq!(
            use_verdict(
                Some("use crate::old::A;\nuse crate::old::B;\n"),
                &["crate".to_string(), "old".to_string()],
                &["use crate::old::A;", "use crate::old::B;"],
                &[1, 2]
            ),
            FamilyVerdict::Holds
        );
        // Macro: 3 `old_m!` sites vs 2 claimed; qualified holds.
        assert_eq!(
            macro_verdict(
                Some("fn m() {\n    old_m!(a);\n    old_m!(b);\n    old_m!(c);\n}\n"),
                "old_m",
                false,
                &["    old_m!(a);", "    old_m!(b);"],
                &[2, 3]
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        assert_eq!(
            macro_verdict(
                Some("fn m() {\n    crate::old_m!(a);\n    crate::old_m!(b);\n}\n"),
                "crate::old_m",
                true,
                &["    crate::old_m!(a);", "    crate::old_m!(b);"],
                &[2, 3]
            ),
            FamilyVerdict::Holds
        );
        // Unproven on unparseable pre-image; Holds on None.
        assert_eq!(
            use_verdict(
                Some("not rust {{{"),
                &["crate".to_string(), "old".to_string()],
                &["use crate::old::A;", "use crate::old::B;"],
                &[1, 2]
            ),
            FamilyVerdict::Unproven
        );
        assert_eq!(
            macro_verdict(None, "old_m", false, &["    old_m!(a);"], &[2]),
            FamilyVerdict::Holds
        );
    }

    #[test]
    fn wi4_slice2brest_a_arg_template_verdicts_name_counts() {
        // WI-4 slice 2b-rest-a: the 3 arg-template families share the
        // real-pair engine count (`template_pair_counts` +
        // `engine_verdict`). No opaque arm by construction (the engine
        // never matches inside macros/strings).
        // Call-perm: 3 `swap(E0,E1)->swap(E1,E0)` sites vs 2 claimed.
        let ppre = "fn m() {\n    swap(a, b);\n    swap(c, d);\n    swap(e, f);\n}\n";
        let pfound =
            template_pair_counts("f.rs", Some(ppre), "swap(E0, E1)", "swap(E1, E0)").unwrap();
        assert_eq!(
            engine_verdict(Some(ppre), Some(pfound), 2),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Call-arg-template: 2 `foo(E0, 1)->bar(E0, 2)` sites, exact holds.
        let cpre = "fn m() {\n    foo(a, 1);\n    foo(b, 1);\n}\n";
        let cfound = template_pair_counts("f.rs", Some(cpre), "foo(E0, 1)", "bar(E0, 2)").unwrap();
        assert_eq!(
            engine_verdict(Some(cpre), Some(cfound), 2),
            FamilyVerdict::Holds
        );
        // Method-arg-template: 3 `Receiver.old(E0, 1)` sites vs 2 claimed.
        let mpre = "fn m() {\n    r.old(a, 1);\n    r.old(b, 1);\n    r.old(c, 1);\n}\n";
        let mfound = template_pair_counts(
            "f.rs",
            Some(mpre),
            "Receiver.old(E0, 1)",
            "Receiver.new(E0, 2)",
        )
        .unwrap();
        assert_eq!(
            engine_verdict(Some(mpre), Some(mfound), 2),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Unproven on unparseable pre-image; Holds on None pre-image
        // (legacy section-visible allow).
        assert_eq!(
            engine_verdict(Some("not rust {{{"), None, 2),
            FamilyVerdict::Unproven
        );
        assert_eq!(engine_verdict(None, None, 2), FamilyVerdict::Holds);
    }

    #[test]
    fn wi4_slice2brest_b_remaining_verdicts_name_counts() {
        // WI-4 slice 2b-rest-b: sequence/repeat engine counts, the peel
        // syntax-walk verdict, and the scratch-twin counter.
        // Sequence: 2 matching windows vs 1 claimed.
        let spre = "fn a() {\n    let x = foo();\n    let y: u32 = bar(x);\n}\nfn b() {\n    let p = foo();\n    let q: u32 = bar(p);\n}\n";
        let smin = vec!["let P0 = E0;".to_string(), "let P1: T1 = E1;".to_string()];
        let splu = vec![
            "let P0: u32 = E0;".to_string(),
            "let P1: T1 = E1.wrapping_add(1);".to_string(),
        ];
        let sfound = template_sequence_counts("f.rs", Some(spre), &smin, &splu).unwrap();
        assert_eq!(
            engine_verdict(Some(spre), Some(sfound), 1),
            FamilyVerdict::CountMismatch { found: 2, want: 1 }
        );
        // Repeat: 2 `Box::new(E0)` sites vs 1 claimed.
        let rpre = "fn m() {\n    let a = Box::new(x);\n    let b = Box::new(y);\n}\n";
        let rfound = template_repeat_counts("f.rs", Some(rpre), "Box::new(E0)").unwrap();
        assert_eq!(
            engine_verdict(Some(rpre), Some(rfound), 1),
            FamilyVerdict::CountMismatch { found: 2, want: 1 }
        );
        // Peel: 3 peel sites vs 2 claimed.
        let ppre =
            "fn m() {\n    f(a, &[\"h\", x]);\n    f(b, &[\"h\", y]);\n    f(c, &[\"h\", z]);\n}\n";
        assert_eq!(
            peel_verdict(
                Some(ppre),
                "f",
                "h",
                2,
                &["    f(a, &[\"h\", x]);", "    f(b, &[\"h\", y]);"],
                &[2, 3],
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Peel exact holds.
        assert_eq!(
            peel_verdict(
                Some("fn m() {\n    f(a, &[\"h\", x]);\n    f(b, &[\"h\", y]);\n}\n"),
                "f",
                "h",
                2,
                &["    f(a, &[\"h\", x]);", "    f(b, &[\"h\", y]);"],
                &[2, 3],
            ),
            FamilyVerdict::Holds
        );
        // Scratch twin: 3 `old: u32` def lines vs 2 claimed.
        assert_eq!(
            scratch_line_verdict(
                Some("struct S {\n    old: u32,\n    old: u32,\n    old: u32,\n}\n"),
                &["    old: u32,", "    old: u32,"],
                &[2, 3],
            ),
            FamilyVerdict::CountMismatch { found: 3, want: 2 }
        );
        // Scratch exact holds; garbage pre-image with no matching line
        // names the 0:1 count gap; None is Holds (legacy allow).
        assert_eq!(
            scratch_line_verdict(
                Some("struct S {\n    old: u32,\n    old: u32,\n}\n"),
                &["    old: u32,", "    old: u32,"],
                &[2, 3],
            ),
            FamilyVerdict::Holds
        );
        assert_eq!(
            scratch_line_verdict(Some("not rust"), &["    old: u32,"], &[1]),
            FamilyVerdict::CountMismatch { found: 0, want: 1 }
        );
        assert_eq!(
            scratch_line_verdict(None, &["    old: u32,"], &[1]),
            FamilyVerdict::Holds
        );
    }
}
