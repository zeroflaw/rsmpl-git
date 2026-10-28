//! Method-call sweep detectors (T413 split from `super`): N-site
//! method-to-function fan-outs, method-to-method renames, and
//! `&`-receiver strip sweeps. Pure detectors over diff lines; dispatch,
//! shared helpers (`unique_rule`, `flat_ws`, `count_occurrences`,
//! `strip_one_terminator`, `ident_len`, `call_parts`) and totality gates
//! live in the parent.

use super::{call_parts, count_occurrences, flat_ws, ident_len, strip_one_terminator, unique_rule};

/// N-site (N ≥ 2) method-to-function fan-out (T345, SPEC 18.5): every
/// pair parses as `recv.old_method(args)` → `new_fn(recv, args)` with
/// one shared method ident, one shared function callee, uniform
/// minus-arity, and a per-site receiver-prepend delta (receiver texts
/// differ per site — `Receiver` binds per site like the engine).
/// Turbofish sites, `&`-led receivers (general-expr routing), and
/// method→method pairs stay passthrough. Emits one `# allow=many` +
/// `# scope=file` hunk (`# expr Receiver` + `# expr E{i}` per arg;
/// arity 0 needs only `Receiver`).
pub(crate) fn try_method_to_fn_sweep(
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
    let mut old_method: Option<String> = None;
    let mut new_func: Option<String> = None;
    let mut arity: Option<usize> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n, a) = parse_method_to_fn_pair(m, p)?;
        if old_method.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_func.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        old_method = Some(o);
        new_func = Some(n);
        arity = Some(a);
    }
    let (old_method, new_func, arity) = (old_method?, new_func?, arity?);
    if !crate::totality::method_holds(file_old, &old_method, arity, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_func}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = crate::git_header(path)
    );
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        hunk.push_str(&format!("# expr {v}\n"));
        vars.push(v);
    }
    let args = vars.join(", ");
    let tail = if args.is_empty() {
        "Receiver".to_string()
    } else {
        format!("Receiver, {args}")
    };
    hunk.push_str(&format!(
        "- Receiver.{old_method}({args})\n+ {new_func}({tail})\n"
    ));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a method-to-function refactor; return
/// (method ident, function callee, minus-arity) when the delta is a
/// receiver prepend (plus `[0]` token-equals the minus receiver, rest
/// token-equal pairwise).
fn parse_method_to_fn_pair(old_line: &str, new_line: &str) -> Option<(String, String, usize)> {
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
/// `# scope=file` hunk (`# expr Receiver` + `# expr E{i}` per arg;
/// arity 0 needs only `Receiver`).
pub(crate) fn try_method_to_method_sweep(
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
    let mut old_method: Option<String> = None;
    let mut new_method: Option<String> = None;
    let mut arity: Option<usize> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n, a) = parse_method_to_method_pair(m, p)?;
        if old_method.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_method.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        old_method = Some(o);
        new_method = Some(n);
        arity = Some(a);
    }
    let (old_method, new_method, arity) = (old_method?, new_method?, arity?);
    if old_method == new_method {
        return None;
    }
    if !crate::totality::method_holds(file_old, &old_method, arity, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_method}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = crate::git_header(path)
    );
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        hunk.push_str(&format!("# expr {v}\n"));
        vars.push(v);
    }
    let args = vars.join(", ");
    hunk.push_str(&format!(
        "- Receiver.{old_method}({args})\n+ Receiver.{new_method}({args})\n"
    ));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a method-to-method rename; return
/// (old method, new method, arity) when receivers and args are
/// token-equal pairwise and only the method ident differs.
fn parse_method_to_method_pair(old_line: &str, new_line: &str) -> Option<(String, String, usize)> {
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
/// tuples stay passthrough. Emits one `# allow=many` + `# scope=file` hunk
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
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = crate::git_header(path)
    );
    hunk.push_str(&format!(
        "- Receiver.{method}({})\n+ Receiver.{method}({})\n",
        old_args.join(", "),
        new_args.join(", ")
    ));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a same-method literal-arg delta; return
/// (method ident, old-arg token-texts, new-arg token-texts) when the method
/// ident is shared, receivers token-equal per site, and both sides parse
/// (one trailing `;`/`,` stripped). `ToTokens` spellings are emitted
/// verbatim (ref-strip precedent). Turbofish, `&`-led sides, arity
/// mismatch, zero arity (no delta possible), and non-ident methods stay
/// passthrough.
fn parse_method_same_literal_pair(
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
    let (syn::Expr::MethodCall(old_call), syn::Expr::MethodCall(new_call)) =
        (&old_expr, &new_expr)
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

/// N-site (N ≥ 2) `&`-receiver strip (T359): one `&recv.method(args)` per
/// pair, plus is minus with it replaced by `recv`; shared method+args bind
/// verbatim. One trailing `;`/`,` stripped; lone/turbofish/`&mut`/
/// divergent/non-receiver stay ordinary.
pub(crate) fn try_ref_method_strip_sweep(
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
    let mut args: Option<Vec<String>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (meth, argv) = parse_ref_strip_pair(m, p)?;
        if method.as_ref().is_some_and(|c| *c != meth) {
            return None;
        }
        if args.as_ref().is_some_and(|a| *a != argv) {
            return None;
        }
        method = Some(meth);
        args = Some(argv);
    }
    let (method, args) = (method?, args?);
    if !crate::totality::strip_holds(file_old, &method, &args, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{method}_strip"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = crate::git_header(path)
    );
    hunk.push_str(&format!(
        "- &Receiver.{method}({})\n+ Receiver\n",
        args.join(", ")
    ));
    Some(hunk)
}

/// Parse one `-`/`+` line pair as a `&`-receiver strip: the minus side
/// parses as an expr containing exactly one shared-reference-to-method
/// call, and the plus side is the minus with that span replaced by the
/// receiver. Returns (method ident, arg token-texts) on success.
fn parse_ref_strip_pair(old_line: &str, new_line: &str) -> Option<(String, Vec<String>)> {
    let minus = strip_one_terminator(old_line.trim())?;
    let plus = strip_one_terminator(new_line.trim())?;
    if minus.is_empty() || plus.is_empty() {
        return None;
    }
    let minus_expr: syn::Expr = syn::parse_str(minus).ok()?;
    let mut hits: Vec<RefStripHit> = Vec::new();
    collect_ref_strips(&minus_expr, &mut hits);
    if hits.len() != 1 {
        return None;
    }
    let hit = hits.pop()?;
    // Whitespace-flat compare: `ToTokens` spacing never matches source
    // spacing, and source lines carry indentation the pattern drops.
    // Explicit lifetimes (`&'a r.m(..)`) never qualify: `syn` drops the
    // lifetime, so the token text cannot prove its absence — check raw.
    if flat_ws(&hit.cand).contains("&'") {
        return None;
    }
    if count_occurrences(&flat_ws(minus), &flat_ws(&hit.cand)) != 1 {
        return None;
    }
    if flat_ws(minus).replacen(&flat_ws(&hit.cand), &flat_ws(&hit.recv), 1) != flat_ws(plus) {
        return None;
    }
    Some((hit.method, hit.args))
}

/// One `&recv.method(args)` span: token-texts for the containment
/// check plus the shared method/args for cross-site agreement.
struct RefStripHit {
    method: String,
    cand: String,
    recv: String,
    args: Vec<String>,
}

/// Walk an expr collecting every turbofish-free shared reference to a
/// method call (`&recv.method(args)`); explicit lifetimes and `&mut`
/// never qualify.
fn collect_ref_strips(expr: &syn::Expr, out: &mut Vec<RefStripHit>) {
    use quote::ToTokens;
    use syn::visit::Visit;
    struct Collector<'a>(&'a mut Vec<RefStripHit>);
    impl<'ast> syn::visit::Visit<'ast> for Collector<'_> {
        fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
            if node.mutability.is_none() {
                if let syn::Expr::MethodCall(call) = node.expr.as_ref() {
                    if call.turbofish.is_none() {
                        self.0.push(RefStripHit {
                            method: call.method.to_string(),
                            cand: node.to_token_stream().to_string(),
                            recv: call.receiver.to_token_stream().to_string(),
                            args: call
                                .args
                                .iter()
                                .map(|a| a.to_token_stream().to_string())
                                .collect(),
                        });
                    }
                }
            }
            syn::visit::visit_expr_reference(self, node);
        }
    }
    Collector(out).visit_expr(expr);
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn validated(patch: &str) {
        rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
    }

    #[test]
    fn method_to_fn_sweep_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n+    new_function(a, 1);\n+    new_function(b, 2);\n+    new_function(c, 3);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_function(Receiver, E0)\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn method_to_fn_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    source.old_operation(first, second);\n-    target.old_operation(third, fourth);\n-    other.old_operation(fifth, sixth);\n+    new_operation(source, first, second);\n+    new_operation(target, third, fourth);\n+    new_operation(other, fifth, sixth);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_method(E0)\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(out.contains("# expr E0\n"), "got: {out}");
        assert!(out.contains("- Receiver.old_operation(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_operation(E0)\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn method_to_method_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(first, second);\n-    b.old_method(third, fourth);\n-    c.old_method(fifth, sixth);\n+    a.new_method(first, second);\n+    b.new_method(third, fourth);\n+    c.new_method(fifth, sixth);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("rule=calls_contains_literal"), "got: {out}");
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(
            out.contains("- Receiver.contains(\"skills/rsmpl-git/SKILL.md\")\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.contains(\"skills/gitrs/SKILL.md\")\n"),
            "got: {out}"
        );
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn method_same_literal_multi_arg_sweep_compresses() {
        // Arity 2 with hetero receivers: both arg positions share their
        // old/new tuples across sites (comma-joined emission).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.route(\"skills/rsmpl-git/SKILL.md\", true);\n-    b.route(\"skills/rsmpl-git/SKILL.md\", true);\n+    a.route(\"skills/gitrs/SKILL.md\", false);\n+    b.route(\"skills/gitrs/SKILL.md\", false);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("rule=calls_route_literal"), "got: {out}");
        assert!(
            out.contains("- Receiver.route(\"skills/rsmpl-git/SKILL.md\", true)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.route(\"skills/gitrs/SKILL.md\", false)\n"),
            "got: {out}"
        );
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
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
    fn method_same_literal_short_stays_passthrough() {
        // T375: two short sites cost more as a semantic hunk
        // (rule + scope + receiver overhead) than as ordinary lines.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.f(\"x\");\n-    b.f(\"x\");\n+    a.f(\"y\");\n+    b.f(\"y\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo::<u8>(\"skills/rsmpl-git/SKILL.md\");\n-    b.foo::<u8>(\"skills/rsmpl-git/SKILL.md\");\n+    a.foo::<u8>(\"skills/gitrs/SKILL.md\");\n+    b.foo::<u8>(\"skills/gitrs/SKILL.md\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_same_literal_ref_receiver_stays_passthrough() {
        // `&`-led sides route to the general-expr matcher, never the
        // method-call shape (mirrors the m2m gate).
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.foo(\"skills/rsmpl-git/SKILL.md\");\n-    &b.foo(\"skills/rsmpl-git/SKILL.md\");\n+    &a.foo(\"skills/gitrs/SKILL.md\");\n+    &b.foo(\"skills/gitrs/SKILL.md\");\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_bare_and_nested_compresses() {
        // T359 shape: bare arg-position lines (trailing `,`) and a
        // `write(...)`-nested site share `&R.join("")` -> `R`.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    &dir.path().join(\"\"),\n-    &dir.path().join(\"\"),\n-    write(&dir.path().join(\"\"), \"a\");\n+    dir.path(),\n+    dir.path(),\n+    write(dir.path(), \"a\");\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- &Receiver.join(\"\")\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_shared_nonempty_arg_compresses() {
        // Shared literal args are the design point, not a hazard:
        // `&d.join("repo")` -> `d` at two sites compresses.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    copy_tree(&a.join(\"repo\"), x);\n-    copy_tree(&b.join(\"repo\"), y);\n+    copy_tree(a, x);\n+    copy_tree(b, y);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- &Receiver.join(\"repo\")\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_zero_arg_compresses() {
        // Empty arg list renders as `()` with no metavars.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    &alpha.shutdown();\n-    &beta.shutdown();\n-    &gamma.shutdown();\n+    alpha;\n+    beta;\n+    gamma;\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
}
