//! Pre-image subterm totality for the six sweep detectors (T409).
//!
//! T403 proved the hazard for verbatim sweeps (712ad9d: an unchanged
//! same-shape guard site would be silently rewritten); the other six
//! sweeps shared the same section-visible evidence gap. Each `*_holds`
//! returns true when the sweep may fire: `None` pre-image = legacy allow,
//! unparseable = veto, else the matchable-unit count in the pre-image
//! must equal the diff pair count with zero opaque-text hits. Opaque is
//! precise: each stripped diff minus line (flat-ws) searched inside macro
//! bodies and string interiors (engine-blind per SPEC 21/T360) — a hit
//! vetoes only when the container's span covers a changed old-side line
//! (T411: then a diff site could be the opaque one — under-match/loss).
//! Twins on untouched lines are unrelated and never veto. Empty changed
//! lines (unparseable header) keep the legacy veto-all.
//! Extra opaque shapes with different text never veto (e.g. `join("a")`
//! vs `join("")`, `&E.join("")` vs `&dir.path().join("")`). Counting is
//! conservative: any extra same-shape real site vetoes.

use quote::ToTokens;
use syn::spanned::Spanned;
use syn::visit::Visit;

/// Parse the pre-image once; `None` = no entry (caller allows).
fn parse_pre(file_old: Option<&str>) -> Option<syn::File> {
    file_old.and_then(|t| syn::parse_str::<syn::File>(t).ok())
}

/// Changed-line cover (T411): an opaque container spanning 1-based
/// `start..=end` lines can only hide a diff site when it covers a
/// changed old-side line. Empty `changed` (unparseable hunk header)
/// keeps the legacy veto-all.
pub(crate) fn covers_changed(start: usize, end: usize, changed: &[usize]) -> bool {
    changed.is_empty() || changed.iter().any(|&c| start <= c && c <= end)
}

/// Old-side changed line numbers for one section or hunk chunk (T411):
/// parse each `@@ -start[,len]` header, walk the body counting old-side
/// lines (context and `-` lines advance; `+` lines do not), and collect
/// the 1-based `-` line numbers the opaque veto checks container spans
/// against. A truly empty body line counts as context (blank context
/// lines carry no prefix). Unparseable headers contribute nothing —
/// callers treat an empty set as the legacy veto-all.
pub(crate) fn changed_old_lines(chunk: &str) -> Vec<usize> {
    fn old_start(header: &str) -> usize {
        // `@@ -start[,len] +... @@`: len defaults to 1; `@@ -0,0`
        // (pure addition) holds no old lines, so its start is unused.
        let first = header.split_whitespace().nth(1).unwrap_or("");
        let start = first
            .trim_start_matches('-')
            .split(',')
            .next()
            .unwrap_or("");
        start.parse::<usize>().unwrap_or(0)
    }
    let mut out = Vec::new();
    let mut old = 0usize;
    let mut in_hunk = false;
    for line in chunk.lines() {
        if line.starts_with("@@") {
            in_hunk = true;
            old = old_start(line);
            continue;
        }
        if !in_hunk {
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            if old > 0 {
                out.push(old);
            }
            old += 1;
        } else if line.starts_with(' ') || line.is_empty() {
            old += 1;
        }
        // `+` lines (and `\ No newline` markers) hold no old-side line.
    }
    out
}

/// Opaque veto over stripped diff minus lines (T411 line-aware):
/// flat-ws `needles` inside a macro body or string value veto only when
/// the container's span covers a changed old-side line (see
/// `covers_changed`).
fn opaque_hit_lines(file: &syn::File, needles: &[String], changed: &[usize]) -> bool {
    struct Scan<'a> {
        needles: &'a [String],
        changed: &'a [usize],
        hit: bool,
    }
    impl Scan<'_> {
        fn check(&mut self, hay: &str, start: usize, end: usize) {
            if self.hit {
                return;
            }
            for n in self.needles {
                if super::semantic::count_occurrences(hay, n) > 0
                    && covers_changed(start, end, self.changed)
                {
                    self.hit = true;
                    return;
                }
            }
        }
    }
    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            let hay = super::semantic::flat_ws(&node.tokens.to_string());
            let span = node.span();
            self.check(&hay, span.start().line, span.end().line);
        }
        fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
            let hay = super::semantic::flat_ws(&node.value());
            let span = node.span();
            self.check(&hay, span.start().line, span.end().line);
        }
    }
    let mut s = Scan {
        needles,
        changed,
        hit: false,
    };
    s.visit_file(file);
    s.hit
}

/// Stripped flat-ws needles from diff minus lines (one terminator off —
/// macro tokens carry none; `{`-terminated slot headers pass through).
fn needles_of(minuses: &[&str]) -> Vec<String> {
    minuses
        .iter()
        .filter_map(|m| {
            let t = m.trim();
            if t.is_empty() {
                return None;
            }
            let s = super::semantic::strip_one_terminator(t).unwrap_or(t);
            let f = super::semantic::flat_ws(s);
            if f.is_empty() {
                None
            } else {
                Some(f)
            }
        })
        .collect()
}

/// N-site bare-call totality: `old_cal` with `arity` args occurs exactly
/// `minuses.len()` times (single-seg, no turbofish — detector shape).
pub(crate) fn call_holds(
    file_old: Option<&str>,
    old_cal: &str,
    arity: usize,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    struct C<'a> {
        cal: &'a str,
        arity: usize,
        count: usize,
    }
    impl<'ast> Visit<'ast> for C<'_> {
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
                        self.count += 1;
                    }
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut c = C {
        cal: old_cal,
        arity,
        count: 0,
    };
    c.visit_file(&file);
    if c.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site array-head-peel totality (T441 slice 1): bare call `cal`
/// with exactly 2 args where the second is `&[head, …]` (immutable
/// ref-of-array, unescaped string head `head`, array length `len`)
/// occurs exactly `n` times. The sweep replays the head-stripped tail
/// positionally, so only the exact-length shape is matchable — other
/// arities/lengths never match and never count.
pub(crate) fn peel_holds(
    file_old: Option<&str>,
    cal: &str,
    head: &str,
    len: usize,
    n: usize,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
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
    if p.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site method totality (m2f + m2m share the minus shape): `method`
/// with `arity` args occurs exactly `minuses.len()` times. All receivers
/// counted (incl `&`-wrapped — conservative).
pub(crate) fn method_holds(
    file_old: Option<&str>,
    method: &str,
    arity: usize,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    struct M<'a> {
        method: &'a str,
        arity: usize,
        count: usize,
    }
    impl<'ast> Visit<'ast> for M<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::MethodCall(m) = node {
                if m.turbofish.is_none() && m.method == self.method && m.args.len() == self.arity {
                    self.count += 1;
                }
            }
            syn::visit::visit_expr(self, node);
        }
    }
    let mut m = M {
        method,
        arity,
        count: 0,
    };
    m.visit_file(&file);
    if m.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site same-method literal totality (T420): `method` with token-equal
/// old args occurs exactly `minuses.len()` times (turbofish excluded).
/// `method_holds` (callee+arity) would over-count same-method sites with
/// different args (e.g. other `contains("...")` calls never touched by a
/// concrete-literal pattern); the arg tuple is the precise pre-image.
/// All receivers counted (incl `&`-wrapped -- conservative, like
/// `method_holds`).
pub(crate) fn method_literal_holds(
    file_old: Option<&str>,
    method: &str,
    old_args: &[String],
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
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
                        super::semantic::flat_ws(&a.to_token_stream().to_string())
                            == super::semantic::flat_ws(b)
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
    if l.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site `&`-strip totality: `&recv.method(args)` with token-equal args
/// occurs exactly `minuses.len()` times (turbofish/`&mut` excluded).
pub(crate) fn strip_holds(
    file_old: Option<&str>,
    method: &str,
    args: &[String],
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
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
                                super::semantic::flat_ws(&a.to_token_stream().to_string())
                                    == super::semantic::flat_ws(b)
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
    if s.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site slot totality: free `fn old` (file + inline mods) when
/// `indented` is false, else `impl`-block methods named `old` (advisory
/// header scans every impl under `# scope=file`; trait blocks excluded).
pub(crate) fn slot_holds(
    file_old: Option<&str>,
    old: &str,
    minuses: &[&str],
    indented: bool,
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
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
    if count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site use-prefix totality: file + inline-mod `use` spans whose
/// leading segments equal `old_prefix` occur exactly `minuses.len()`
/// times (tails preserved; group members each count). Fn-body uses never
/// match (SPEC 14.9) and are not counted.
pub(crate) fn use_holds(
    file_old: Option<&str>,
    old_prefix: &[String],
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
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
    if count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    fn infer(raw: &str, pre: &str) -> String {
        let mut m = HashMap::new();
        m.insert("f.rs".to_string(), pre.to_string());
        crate::infer_rsmpl_with_pre_images(raw, &m).unwrap().rsmpl
    }
    fn sec(body: &str) -> String {
        format!("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n{body}")
    }
    #[test]
    fn call_totality_fires_and_vetoes_extra() {
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n }\n");
        let pre = "fn m() {\n    foo(1);\n    foo(2);\n}\n";
        assert!(
            infer(&raw, pre).contains("rule=calls_foo_to_bar"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";
        assert!(!infer(&raw, pre2).contains("rule="), "extra site must veto");
    }
    #[test]
    fn method_totality_vetoes_extra_receiver() {
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.old(1);\n+    a.new(1);\n+    b.new(1);\n }\n");
        let pre = "fn m() {\n    a.old(1);\n    b.old(1);\n    c.old(1);\n}\n";
        assert!(
            !infer(&raw, pre).contains("rule="),
            "extra method site must veto"
        );
    }
    #[test]
    fn method_literal_totality_fires_and_vetoes_extra() {
        // T420: the pre-image count keys on method + old-arg tuple, not
        // callee+arity alone -- an extra same-method/different-arg site
        // never matches the concrete-literal pattern and must not veto.
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    a.contains(\"skills/rsmpl-git/SKILL.md\");\n-    b.contains(\"skills/rsmpl-git/SKILL.md\");\n+    a.contains(\"skills/gitrs/SKILL.md\");\n+    b.contains(\"skills/gitrs/SKILL.md\");\n }\n");
        let pre = "fn m() {\n    a.contains(\"skills/rsmpl-git/SKILL.md\");\n    b.contains(\"skills/rsmpl-git/SKILL.md\");\n}\n";
        assert!(
            infer(&raw, pre).contains("rule=calls_contains_literal"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "fn m() {\n    a.contains(\"skills/rsmpl-git/SKILL.md\");\n    b.contains(\"skills/rsmpl-git/SKILL.md\");\n    c.contains(\"skills/rsmpl-git/SKILL.md\");\n}\n";
        assert!(
            !infer(&raw, pre2).contains("rule="),
            "extra same-literal site must veto"
        );
        let pre3 = "fn m() {\n    a.contains(\"skills/rsmpl-git/SKILL.md\");\n    b.contains(\"skills/rsmpl-git/SKILL.md\");\n    c.contains(\"other\");\n}\n";
        assert!(
            infer(&raw, pre3).contains("rule=calls_contains_literal"),
            "different-arg site must not veto, got: {}",
            infer(&raw, pre3)
        );
    }
    #[test]
    fn strip_totality_fires_and_opaque_string_ignored_when_different() {
        let raw = sec("@@ -1,5 +1,5 @@\n fn m() {\n-    &dir.path().join(\"\"),\n-    &dir.path().join(\"\"),\n-    write(&dir.path().join(\"\"), \"a\");\n+    dir.path(),\n+    dir.path(),\n+    write(dir.path(), \"a\");\n }\n");
        let pre = "fn m() {\n    let a = &dir.path().join(\"\");\n    let b = &dir.path().join(\"\");\n    write(&dir.path().join(\"\"), \"a\");\n    let s = \"patch text\";\n}\n";
        assert!(
            infer(&raw, pre).contains("calls_join_strip"),
            "got: {}",
            infer(&raw, pre)
        );
    }
    #[test]
    fn call_totality_fires_past_unrelated_string_twin() {
        // T411 line-aware: the string twin sits on line 4 while the hunk
        // changes old lines 2-3, so it cannot hide a diff site — the
        // sweep fires (the engine never matches inside literals).
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(1);\n+    bar(1);\n+    bar(1);\n }\n");
        let pre = "fn m() {\n    foo(1);\n    foo(1);\n    let s = \"foo(1);\";\n}\n";
        assert!(
            infer(&raw, pre).contains("rule=calls_foo_to_bar"),
            "got: {}",
            infer(&raw, pre)
        );
    }
    #[test]
    fn call_totality_vetoes_string_twin_on_changed_line() {
        // T411: the string twin sits on old line 3, which the hunk changes
        // (lines 2-3) — it could hide a diff site, so the sweep vetoes.
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(1);\n+    bar(1);\n+    bar(1);\n }\n");
        let pre = "fn m() {\n    foo(1);\n    let s = \"foo(1);\";\n    foo(1);\n}\n";
        assert!(
            !infer(&raw, pre).contains("rule="),
            "changed-line twin must veto"
        );
    }
    #[test]
    fn call_totality_fires_past_unrelated_macro_twin() {
        // T411: the macro twin sits on line 4, untouched by the hunk —
        // the engine leaves macro interiors alone, so the sweep fires.
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(1);\n+    bar(1);\n+    bar(1);\n }\n");
        let pre = "fn m() {\n    foo(1);\n    foo(1);\n    g(m!(foo(1)));\n}\n";
        assert!(
            infer(&raw, pre).contains("rule=calls_foo_to_bar"),
            "got: {}",
            infer(&raw, pre)
        );
    }
    #[test]
    fn call_totality_vetoes_macro_twin_on_changed_line() {
        // T411: `m!(foo(1))` on old line 3 (changed) could hide a diff
        // site — the engine never matches inside macros, so veto.
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(1);\n+    bar(1);\n+    bar(1);\n }\n");
        let pre = "fn m() {\n    foo(1);\n    g(m!(foo(1)));\n    foo(1);\n}\n";
        assert!(
            !infer(&raw, pre).contains("rule="),
            "changed-line twin must veto"
        );
    }
    #[test]
    fn slot_totality_fires_and_vetoes_extra() {
        let raw = sec("@@ -1,2 +1,2 @@\n-fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n-fn old_function_name(first_argument: OtherLongType, second_argument: SecondLongType) {\n+fn new_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n+fn new_function_name(first_argument: OtherLongType, second_argument: SecondLongType) {\n");
        let pre = "fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {}\nfn old_function_name(first_argument: OtherLongType, second_argument: SecondLongType) {}\n";
        assert!(
            infer(&raw, pre).contains("rename_old_function_name_to_new_function_name"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {}\nfn old_function_name(first_argument: OtherLongType, second_argument: SecondLongType) {}\nfn old_function_name(x: i32) {}\n";
        assert!(!infer(&raw, pre2).contains("rule="), "extra fn must veto");
    }
    #[test]
    fn use_totality_fires_and_vetoes_extra() {
        let raw = sec("@@ -1,2 +1,2 @@\n- use crate::old::A;\n- use crate::old::B;\n+ use crate::new::A;\n+ use crate::new::B;\n");
        let pre = "use crate::old::A;\nuse crate::old::B;\n";
        assert!(
            infer(&raw, pre).contains("rule=use_old_to_new"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "use crate::old::A;\nuse crate::old::B;\nuse crate::old::C;\n";
        assert!(!infer(&raw, pre2).contains("rule="), "extra use must veto");
    }
    #[test]
    fn unparseable_pre_vetoes() {
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n }\n");
        assert!(!infer(&raw, "not rust {{{").contains("rule="));
    }
    #[test]
    fn changed_old_lines_collects_minus_lines() {
        let chunk =
            "@@ -2,1 +2,1 @@\n ctx\n- old\n+ new\n ctx\n@@ -9,2 +10,3 @@\n-a\n-b\n+c\n+d\n+e\n";
        assert_eq!(crate::totality::changed_old_lines(chunk), vec![3, 9, 10]);
    }
    #[test]
    fn changed_old_lines_handles_elided_len_and_pure_addition() {
        // `@@ -1 +1 @@` elides len 1; `@@ -5,0` holds no old lines.
        let chunk = "@@ -1 +1 @@\n-a\n+b\n@@ -5,0 +6,3 @@\n+x\n+y\n+z\n";
        assert_eq!(crate::totality::changed_old_lines(chunk), vec![1]);
    }
    #[test]
    fn covers_changed_needs_a_covered_line() {
        assert!(crate::totality::covers_changed(4, 4, &[])); // legacy veto-all
        assert!(crate::totality::covers_changed(9, 9, &[2, 9]));
        assert!(crate::totality::covers_changed(8, 10, &[9]));
        assert!(!crate::totality::covers_changed(4, 4, &[2, 9]));
        assert!(!crate::totality::covers_changed(2, 3, &[4]));
    }
}
