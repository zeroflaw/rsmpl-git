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
    file_old.and_then(|t| syn::parse_str::<syn::File>(t).ok())
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
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return None;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return None;
    };
    Some(report.rules.iter().map(|r| r.matches).sum::<usize>())
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
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return None;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return None;
    };
    Some(report.rules.iter().map(|r| r.matches).sum::<usize>())
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
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return None;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return None;
    };
    Some(report.rules.iter().map(|r| r.matches).sum::<usize>())
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
}
