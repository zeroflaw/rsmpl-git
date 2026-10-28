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
//! means a diff site could be the opaque one (under-match/loss), so veto.
//! Extra opaque shapes with different text never veto (e.g. `join("a")`
//! vs `join("")`, `&E.join("")` vs `&dir.path().join("")`). Counting is
//! conservative: any extra same-shape real site vetoes.

use quote::ToTokens;
use syn::visit::Visit;

/// Parse the pre-image once; `None` = no entry (caller allows).
fn parse_pre(file_old: Option<&str>) -> Option<syn::File> {
    file_old.and_then(|t| syn::parse_str::<syn::File>(t).ok())
}

/// Opaque veto over stripped diff minus lines: flat-ws `needles` inside
/// any macro body or string value. Without line info a diff site could
/// be the opaque one, so any hit vetoes.
fn opaque_hit_lines(file: &syn::File, needles: &[String]) -> bool {
    struct Scan<'a> {
        needles: &'a [String],
        hit: bool,
    }
    impl<'ast> Visit<'ast> for Scan<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if self.hit {
                return;
            }
            let hay = super::semantic::flat_ws(&node.tokens.to_string());
            for n in self.needles {
                if super::semantic::count_occurrences(&hay, n) > 0 {
                    self.hit = true;
                    return;
                }
            }
        }
        fn visit_lit_str(&mut self, node: &'ast syn::LitStr) {
            if self.hit {
                return;
            }
            let hay = super::semantic::flat_ws(&node.value());
            for n in self.needles {
                if super::semantic::count_occurrences(&hay, n) > 0 {
                    self.hit = true;
                    return;
                }
            }
        }
    }
    let mut s = Scan {
        needles,
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
    !opaque_hit_lines(&file, &needles_of(minuses))
}

/// N-site method totality (m2f + m2m share the minus shape): `method`
/// with `arity` args occurs exactly `minuses.len()` times. All receivers
/// counted (incl `&`-wrapped — conservative).
pub(crate) fn method_holds(
    file_old: Option<&str>,
    method: &str,
    arity: usize,
    minuses: &[&str],
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
    !opaque_hit_lines(&file, &needles_of(minuses))
}

/// N-site `&`-strip totality: `&recv.method(args)` with token-equal args
/// occurs exactly `minuses.len()` times (turbofish/`&mut` excluded).
pub(crate) fn strip_holds(
    file_old: Option<&str>,
    method: &str,
    args: &[String],
    minuses: &[&str],
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
    !opaque_hit_lines(&file, &needles_of(minuses))
}

/// N-site slot totality: free `fn old` (file + inline mods) when
/// `indented` is false, else `impl`-block methods named `old` (advisory
/// header scans every impl under `# scope=file`; trait blocks excluded).
pub(crate) fn slot_holds(
    file_old: Option<&str>,
    old: &str,
    minuses: &[&str],
    indented: bool,
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
    !opaque_hit_lines(&file, &needles_of(minuses))
}

/// N-site use-prefix totality: file + inline-mod `use` spans whose
/// leading segments equal `old_prefix` occur exactly `minuses.len()`
/// times (tails preserved; group members each count). Fn-body uses never
/// match (SPEC 14.9) and are not counted.
pub(crate) fn use_holds(file_old: Option<&str>, old_prefix: &[String], minuses: &[&str]) -> bool {
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
    !opaque_hit_lines(&file, &needles_of(minuses))
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
    fn strip_totality_vetoes_same_line_in_string() {
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(1);\n+    bar(1);\n+    bar(1);\n }\n");
        let pre = "fn m() {\n    foo(1);\n    foo(1);\n    let s = \"foo(1);\";\n}\n";
        assert!(!infer(&raw, pre).contains("rule="), "string twin must veto");
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
}
