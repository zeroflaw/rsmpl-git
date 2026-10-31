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

/// N-site bare-call trailing-literal totality (T448): bare single-seg
/// no-turbofish call `cal` with `arity` args whose trailing arg flat-ws
/// equals `old_trailing` occurs exactly `minuses.len()` times.
/// `call_holds` (callee+arity) would over-count same-arity sites with a
/// different trailing literal (e.g. 7025a14's `true` site next to the 2x
/// `false` prize group); the trailing literal is the precise pre-image,
/// mirroring `method_literal_holds` (T420).
pub(crate) fn call_literal_holds(
    file_old: Option<&str>,
    cal: &str,
    arity: usize,
    old_trailing: &str,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    let want = super::semantic::flat_ws(old_trailing);
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
                            use quote::ToTokens;
                            if super::semantic::flat_ws(&last.to_token_stream().to_string())
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
    if l.count != n {
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
/// `indented` is false, else `impl`-block methods named `old`. The
/// whole-file count is a pre-gate only: per-block emissions each clear
/// their own engine probe (`probe_block_hunk`), which also enforces the
/// WI-4a header filter. Trait-block methods are excluded.
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

/// N-site let type-addition totality (WI-4a): untyped with-init
/// no-diverge `Stmt::Local` statements occur exactly `minuses.len()`
/// times. The emitted minus template `let P0 = E0;` matches every such
/// statement (ascription agreement is strict — already-typed lets
/// never match, `match_local` — and `E0` requires an init), so the
/// count is exactly the template's match domain. Same opaque handling
/// as the other counters.
pub(crate) fn let_untyped_holds(
    file_old: Option<&str>,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    struct L {
        count: usize,
    }
    impl<'ast> Visit<'ast> for L {
        fn visit_stmt(&mut self, node: &'ast syn::Stmt) {
            if let syn::Stmt::Local(local) = node {
                if !matches!(local.pat, syn::Pat::Type(_))
                    && local.init.as_ref().is_some_and(|i| i.diverge.is_none())
                {
                    self.count += 1;
                }
            }
            syn::visit::visit_stmt(self, node);
        }
    }
    let mut l = L { count: 0 };
    l.visit_file(&file);
    if l.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// Generic template totality (WI-4b): the emitted minus type template
/// `minus_template` (e.g. `Vec<T0>`) matches exactly `n` type subterms
/// in the pre-image file. Counted by the engine itself, not by a
/// hand-rolled syntactic walk: a `# sweep` self-rewrite probe (`- M` /
/// `+ M`, which parses and compiles exactly like the real emission) is
/// applied to a scratch tree and its match count must equal `n`
/// (IMPROVE.md WI-4). `None` pre-image keeps the legacy allow;
/// probe parse/apply failure vetoes. Opaque positions need no extra
/// veto: the engine never matches inside macros or string literals, so
/// hidden twins never count; nested matches count outer-only
/// (contained skips), exactly like the real sweep rewrites.
pub(crate) fn template_type_holds(
    path: &str,
    file_old: Option<&str>,
    minus_template: &str,
    n: usize,
) -> bool {
    if n == 0 || minus_template.contains('\n') {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    // WI-2c: the self-rewrite is an identity counting device, not a user
    // rewrite — `# optional` exempts it from the sweep change gate
    // (zero-match still yields `Ok(0) != n`, so verdicts are unchanged).
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n- {minus_template}\n+ {minus_template}\n",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

/// Paired-template totality (WI-9b1): the hetero macro pair
/// (`hetero_minus` / `hetero_plus`, e.g. `- assert_eq!(E0, TEXT_SECTION)`
/// / `+ assert_eq!(E0, TEXT_STRIPPED)`) must match exactly `n` sites in
/// the pre-image file. Unlike the self-rewrite probes, this one applies
/// the REAL pair: a self-rewrite (`- M` / `+ M`) would see every shared
/// name (including uppercase literal deltas like `TEXT_SECTION`) on both
/// sides and infer them as holes, counting a more general domain than
/// the emitted hunk matches. The real-pair probe runs the emitted
/// hunk's own diet inference (shared-names rule: `E0` → `expr`,
/// single-side idents stay literal) and counts minus matches on a
/// scratch tree. `None` pre-image keeps the legacy allow; probe
/// parse/apply failure vetoes. Conservative by construction: whole-file
/// count, so an untouched twin anywhere vetoes (same posture as
/// `template_expr_holds`).
pub(crate) fn template_pair_holds(
    path: &str,
    file_old: Option<&str>,
    hetero_minus: &str,
    hetero_plus: &str,
    n: usize,
) -> bool {
    if n == 0 || hetero_minus.contains('\n') || hetero_plus.contains('\n') {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n- {hetero_minus}\n+ {hetero_plus}\n",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

/// Fold totality (WI-1c): the emitted minus fold template `minus_template`
/// (e.g. `Box::new(E0)`, exactly one `E0` hole) must match exactly `n`
/// outermost sites in the pre-image file. Counted by the engine itself
/// with the real fold spelling: a `# sweep` + `# repeat` probe
/// (`- M` / `+ E0`) is applied to a scratch tree and its match count
/// must equal `n`. The `# repeat` matters: without it a nested site
/// counts once per nesting level, while the real sweep folds bottom-up
/// and counts outermost only (T20) — a self-rewrite probe would
/// over-count every nested site and veto every fold. `None` pre-image
/// keeps the legacy allow; probe parse/apply failure vetoes. Opaque
/// positions need no extra veto: the engine never matches inside
/// macros or string literals, so hidden twins never count.
pub(crate) fn template_repeat_holds(
    path: &str,
    file_old: Option<&str>,
    minus_template: &str,
    n: usize,
) -> bool {
    if n == 0 || minus_template.contains('\n') {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# repeat\n- {minus_template}\n+ E0\n",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

/// Sequence totality (WI-5c, WI-5d): the emitted K-line minus template
/// must match exactly `n` statement windows in the pre-image file.
/// Counted by the engine itself with the real pair spelling: a
/// `# sweep` probe carrying the K minus/plus lines is applied to a
/// scratch tree and its window-match count must equal `n` (blocks). An
/// untouched twin window anywhere in the file counts `n + 1` and
/// vetoes, so the group stays ordinary instead of silently rewriting
/// the twin. WI-5d single-block fusion is the n = 1 case (one window);
/// the probe is identical, only the count differs. `None` pre-image
/// keeps the legacy allow; probe parse/apply failure vetoes. Opaque
/// positions need no extra veto: sequence windows only match plain
/// `let` statements, which never sit inside macros or string literals.
pub(crate) fn template_sequence_holds(
    path: &str,
    file_old: Option<&str>,
    minus_tpls: &[String],
    plus_tpls: &[String],
    n: usize,
) -> bool {
    if n < 1 || !(2..=4).contains(&minus_tpls.len()) || minus_tpls.len() != plus_tpls.len() {
        return false;
    }
    if minus_tpls.iter().any(|l| l.contains('\n')) || plus_tpls.iter().any(|l| l.contains('\n')) {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
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
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}
/// whose uppercase callee infers as an `expr` hole) must match exactly
/// `n` sub-expressions in the pre-image file. Same self-rewrite probe
/// prescription as `template_type_holds` (`- M` / `+ M` under `# sweep`,
/// engine-applied to a scratch tree, match count must equal `n`):
/// a hole that also matches an untouched twin (probed: `Other(alpha)`
/// next to two `Old(alpha)` sites) counts `n + 1` and vetoes, so the
/// group stays ordinary instead of silently rewriting the twin.
/// `None` pre-image keeps the legacy allow; probe parse/apply failure
/// vetoes. Conservative by construction: the sweep probe counts
/// whole-file matches, so a twin outside the emitted range still vetoes
/// (same posture as `template_fn_holds`). Only valid when every shared
/// name in the template is a hole in the emitted hunk too — hetero
/// macro pairs with literal uppercase deltas must use
/// `template_pair_holds` (self-rewrite would infer the deltas).
pub(crate) fn template_expr_holds(
    path: &str,
    file_old: Option<&str>,
    minus_template: &str,
    n: usize,
) -> bool {
    if n == 0 || minus_template.contains('\n') {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    // WI-2c: identity counting device — `# optional` exempts the sweep
    // change gate (verdicts unchanged: zero-match yields `Ok(0) != n`).
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n- {minus_template}\n+ {minus_template}\n",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
}

/// Generic template totality for fn-header sweeps (WI-4c): the emitted
/// minus header `minus_header` (e.g. `fn old (...)`, concrete slots,
/// preserved slots omitted or `(...)`) must match exactly `n` fn items
/// in the pre-image file. Counted by the engine itself, not by a
/// hand-rolled syntactic walk: a `# sweep` self-rewrite probe
/// (`- H` / `+ H` with a `...` body, wrapped in `header` when `Some`)
/// is compiled and engine-applied to a scratch tree and its match
/// count must equal `n` (the `template_type_holds` prescription lifted
/// to items). Free-fn probes pass `None`; method probes pass the block's
/// verbatim header line (`Some("impl Svc {")`) — under the WI-4a header
/// filter an unwrapped method probe counts nothing, and a wrong-header
/// probe vetoes. `None` pre-image keeps the legacy allow; probe
/// parse/apply failure vetoes. Opaque positions need no extra veto:
/// the engine never matches fn items inside macro bodies or string
/// literals. Conservative by construction: the sweep probe counts
/// whole-file matches, so a same-name fn outside the emitted range
/// vetoes even a ranged emission that would not touch it (same
/// posture as `slot_holds`).
pub(crate) fn template_fn_holds(
    path: &str,
    file_old: Option<&str>,
    minus_header: &str,
    header: Option<&str>,
    n: usize,
) -> bool {
    if n == 0 || minus_header.contains('\n') {
        return false;
    }
    let Some(pre) = file_old else {
        return true;
    };
    let body = match header {
        Some(h) => format!(
            "  {h}\n      ...\n-     {minus_header}\n+     {minus_header}\n          ...\n      }}\n  }}\n"
        ),
        None => format!("- {minus_header}\n+ {minus_header}\n      ...\n  }}\n"),
    };
    // WI-2c: self-rewrite counting device — `# optional` exempts the
    // sweep change gate (verdicts unchanged).
    let probe = format!(
        "{}\n@@ -1,1 @@ rule=probe\n# sweep\n# optional\n{body}",
        crate::git_header(path)
    );
    let Ok(parsed) = rsmpl_patch::parse_patch(&probe) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    report.rules.iter().map(|r| r.matches).sum::<usize>() == n
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

/// N-site qualified-call totality (WI-3): full path `old_path` with
/// `arity` value args occurs exactly `minuses.len()` times. Only
/// generic-free paths count (the emitted pattern carries no turbofish,
/// and a bare-without-turbofish pattern never matches a turbofish site
/// — R3 parity, probed). Exact-path matching is probed: a bare twin
/// never matches a qualified pattern and vice versa.
pub(crate) fn qualified_call_holds(
    file_old: Option<&str>,
    old_path: &str,
    arity: usize,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    struct C<'a> {
        old_path: &'a str,
        arity: usize,
        count: usize,
    }
    impl<'ast> Visit<'ast> for C<'_> {
        fn visit_expr(&mut self, node: &'ast syn::Expr) {
            if let syn::Expr::Call(call) = node {
                if let syn::Expr::Path(p) = call.func.as_ref() {
                    // WI-7d: the full render (qself + leading + segs)
                    // decides — a bare twin, an opposite-leading twin,
                    // or a differing-qself twin never equals the
                    // emitted path and never counts (probed parity).
                    if p.path.segments.len() >= 2
                        && p.path
                            .segments
                            .iter()
                            .all(|s| matches!(s.arguments, syn::PathArguments::None))
                        && call.args.len() == self.arity
                        && render_full_path(p) == self.old_path
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
    if c.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// Canonical full-path render for totality counting: optional `qself`
/// qualifier plus the post-qualifier segments (`<T as Tr>::a::b` — the
/// trait path lives inside the qualifier, never doubled), with leading
/// `::` when qualifier-free — the same spelling the
/// qualified/turbofish detectors emit, so detector and counter agree
/// by construction (WI-7d).
fn render_full_path(p: &syn::ExprPath) -> String {
    let mut out = String::new();
    let mut skip = 0usize;
    if let (Some(q), pos) = super::semantic::qualifier_of(p) {
        out.push_str(&q);
        out.push_str("::");
        skip = pos;
    } else if p.path.leading_colon.is_some() {
        out.push_str("::");
    }
    let segs: Vec<String> = p
        .path
        .segments
        .iter()
        .skip(skip)
        .map(|s| s.ident.to_string())
        .collect();
    out.push_str(&segs.join("::"));
    out
}

/// N-site turbofish-call totality (WI-3): full path `old_path` with
/// `val_arity` value args and a last-segment turbofish of `turbo_arity`
/// args whose `concrete` positions (index, flat-ws text) match occurs
/// exactly `minuses.len()` times. Non-turbofish twins never match the
/// emitted `::<...>` pattern (R3, probed) so they never count;
/// colon2-spelling is ignored (a bare-`<>` twin counts — erring toward
/// veto). Prefix segments must be generic-free (the emission spells
/// them concrete).
pub(crate) fn turbofish_call_holds(
    file_old: Option<&str>,
    old_path: &str,
    val_arity: usize,
    turbo_arity: usize,
    concrete: &[(usize, String)],
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
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
                        && render_full_path(p) == self.old_path
                        && turbo_concrete_holds(p, self.turbo_arity, self.concrete)
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
    if c.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// The last-segment turbofish of a call path carries `turbo_arity` args
/// with every `concrete` position flat-equal (prefix segments
/// generic-free — the emission spells them concrete). Colon2-insensitive
/// (counts bare-`<>` spellings too — veto-safe).
fn turbo_concrete_holds(
    p: &syn::ExprPath,
    turbo_arity: usize,
    concrete: &[(usize, String)],
) -> bool {
    if p.path.segments.is_empty() {
        return false;
    }
    for seg in p.path.segments.iter().take(p.path.segments.len() - 1) {
        if !matches!(seg.arguments, syn::PathArguments::None) {
            return false;
        }
    }
    let last = p.path.segments.last();
    let Some(syn::PathArguments::AngleBracketed(args)) = last.map(|s| &s.arguments) else {
        return false;
    };
    if args.args.len() != turbo_arity {
        return false;
    }
    concrete.iter().all(|(i, want)| {
        args.args.iter().nth(*i).is_some_and(|a| {
            super::semantic::flat_ws(&a.to_token_stream().to_string())
                == super::semantic::flat_ws(want)
        })
    })
}

/// N-site turbofish-method totality (WI-3): method `method` with
/// `val_arity` value args and a `turbo_arity`-arg turbofish whose
/// `concrete` positions match occurs exactly `minuses.len()` times.
/// All receivers counted (incl `&`-wrapped — conservative, mirroring
/// `method_holds`: the pattern cannot match them, so an extra `&` site
/// vetoes).
pub(crate) fn turbofish_method_holds(
    file_old: Option<&str>,
    method: &str,
    val_arity: usize,
    turbo_arity: usize,
    concrete: &[(usize, String)],
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
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
                                    super::semantic::flat_ws(&a.to_token_stream().to_string())
                                        == super::semantic::flat_ws(want)
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
    if m.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site macro-rename totality (WI-3): single-segment macro invocations
/// named `old_name` (any delimiter, any body — the emitted `...`
/// matches all three delimiters, probed) occur exactly `minuses.len()`
/// times. Qualified twins (`crate::old!`) never match the bare pattern
/// (probed) so they never count.
pub(crate) fn macro_holds(
    file_old: Option<&str>,
    old_name: &str,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    struct M<'a> {
        old_name: &'a str,
        count: usize,
    }
    impl<'ast> Visit<'ast> for M<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node.path.leading_colon.is_none()
                && node.path.segments.len() == 1
                && node.path.segments[0].ident == self.old_name
                && matches!(node.path.segments[0].arguments, syn::PathArguments::None)
            {
                self.count += 1;
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let mut m = M { old_name, count: 0 };
    m.visit_file(&file);
    if m.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// N-site qualified macro-rename totality (WI-3 residue): multi-segment
/// macro invocations with the exact rendered path `old_path` (any
/// delimiter, any body — the emitted `...` matches all, probed) occur
/// exactly `minuses.len()` times. Bare twins never match the qualified
/// pattern and vice versa (probed), so only exact-path hits count.
pub(crate) fn qualified_macro_holds(
    file_old: Option<&str>,
    old_path: &str,
    minuses: &[&str],
    changed: &[usize],
) -> bool {
    let Some(file) = parse_pre(file_old) else {
        return file_old.is_none();
    };
    let n = minuses.len();
    struct M<'a> {
        old_path: &'a str,
        count: usize,
    }
    impl<'ast> Visit<'ast> for M<'_> {
        fn visit_macro(&mut self, node: &'ast syn::Macro) {
            if node.path.leading_colon.is_none() && render_macro_path(node) == self.old_path {
                self.count += 1;
            }
            syn::visit::visit_macro(self, node);
        }
    }
    let mut m = M { old_path, count: 0 };
    m.visit_file(&file);
    if m.count != n {
        return false;
    }
    !opaque_hit_lines(&file, &needles_of(minuses), changed)
}

/// Render a `syn::Macro` invocation path (`prefix::name` or bare).
fn render_macro_path(node: &syn::Macro) -> String {
    node.path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
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
            infer(&raw, pre).contains("rule=r1"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";
        assert!(!infer(&raw, pre2).contains("rule="), "extra site must veto");
    }
    #[test]
    fn call_literal_totality_keys_on_trailing() {
        // T448: the pre-image count keys on callee + trailing old literal,
        // not callee+arity alone -- an extra same-arity/different-trailing
        // site never matches the concrete pattern and must not veto.
        let raw = sec("@@ -1,4 +1,4 @@\n fn m() {\n-    make_widget(alpha, 1, false);\n-    make_widget(beta, 2, false);\n+    make_widget(alpha, 1, \"100644\");\n+    make_widget(beta, 2, \"100644\");\n }\n");
        let pre =
            "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n}\n";
        assert!(
            infer(&raw, pre).contains("rule=r1"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, false);\n}\n";
        assert!(
            !infer(&raw, pre2).contains("rule="),
            "extra same-literal site must veto"
        );
        let pre3 = "fn m() {\n    make_widget(alpha, 1, false);\n    make_widget(beta, 2, false);\n    make_widget(gamma, 3, true);\n}\n";
        assert!(
            infer(&raw, pre3).contains("rule=r1"),
            "different-trailing site must not veto, got: {}",
            infer(&raw, pre3)
        );
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
            infer(&raw, pre).contains("rule=r1"),
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
            infer(&raw, pre3).contains("rule=r1"),
            "different-arg site must not veto, got: {}",
            infer(&raw, pre3)
        );
    }
    #[test]
    fn strip_totality_fires_and_opaque_string_ignored_when_different() {
        let raw = sec("@@ -1,5 +1,5 @@\n fn m() {\n-    &dir.path().join(\"\"),\n-    &dir.path().join(\"\"),\n-    write(&dir.path().join(\"\"), \"a\");\n+    dir.path(),\n+    dir.path(),\n+    write(dir.path(), \"a\");\n }\n");
        let pre = "fn m() {\n    let a = &dir.path().join(\"\");\n    let b = &dir.path().join(\"\");\n    write(&dir.path().join(\"\"), \"a\");\n    let s = \"patch text\";\n}\n";
        assert!(
            infer(&raw, pre).contains("rule=r1"),
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
            infer(&raw, pre).contains("rule=r1"),
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
            infer(&raw, pre).contains("rule=r1"),
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
            infer(&raw, pre).contains("rule=r1"),
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
            infer(&raw, pre).contains("rule=r1"),
            "got: {}",
            infer(&raw, pre)
        );
        let pre2 = "use crate::old::A;\nuse crate::old::B;\nuse crate::old::C;\n";
        assert!(!infer(&raw, pre2).contains("rule="), "extra use must veto");
    }
    #[test]
    fn template_type_counts_engine_matches() {
        let pre = "fn m() {\n    let a: Vec<u32> = foo();\n    let b: Vec<String> = bar();\n    let c: u32 = baz();\n}\n";
        assert!(crate::totality::template_type_holds(
            "f.rs",
            Some(pre),
            "Vec<T0>",
            2
        ));
        assert!(!crate::totality::template_type_holds(
            "f.rs",
            Some(pre),
            "Vec<T0>",
            3
        ));
        // A narrower literal template counts only its own sites.
        assert!(crate::totality::template_type_holds(
            "f.rs",
            Some(pre),
            "Vec<u32>",
            1
        ));
    }
    #[test]
    fn template_expr_hole_twin_vetoes_and_exact_passes() {
        // WI-7-upper: an uppercase hole matches a same-shape twin, so
        // the count exceeds the group size and vetoes; without the twin
        // the count is exact and passes. `None` pre-image keeps the
        // legacy allow.
        let pre = "fn m() {\n    Old(alpha);\n    Old(alpha);\n    Other(alpha);\n}\n";
        assert!(!crate::totality::template_expr_holds(
            "f.rs",
            Some(pre),
            "Old(alpha)",
            2
        ));
        let clean = "fn m() {\n    Old(alpha);\n    Old(alpha);\n}\n";
        assert!(crate::totality::template_expr_holds(
            "f.rs",
            Some(clean),
            "Old(alpha)",
            2
        ));
        // Backticked literal counts exactly (the verbatim emission).
        assert!(crate::totality::template_expr_holds(
            "f.rs",
            Some(pre),
            "`Old`(alpha)",
            2
        ));
        // Let-shape hole risk is judged the same way.
        let let_pre =
            "fn m() {\n    let Old = foo();\n    let Old = foo();\n    let Other = foo();\n}\n";
        assert!(!crate::totality::template_expr_holds(
            "f.rs",
            Some(let_pre),
            "let Old = foo();",
            2
        ));
        assert!(crate::totality::template_expr_holds(
            "f.rs",
            None,
            "Old(alpha)",
            2
        ));
    }
    #[test]
    fn template_type_nested_counts_outer_only() {
        // Contained inner matches skip, exactly like the real sweep
        // rewrites (probed): two `Vec<Vec<u32>>` sites count 2, not 4.
        let pre =
            "fn m() {\n    let a: Vec<Vec<u32>> = foo();\n    let b: Vec<Vec<u32>> = bar();\n}\n";
        assert!(crate::totality::template_type_holds(
            "f.rs",
            Some(pre),
            "Vec<T0>",
            2
        ));
    }
    #[test]
    fn template_type_none_pre_allows_and_garbage_vetoes() {
        assert!(crate::totality::template_type_holds(
            "f.rs", None, "Vec<T0>", 2
        ));
        let pre = "fn m() {}\n";
        assert!(!crate::totality::template_type_holds(
            "f.rs",
            Some(pre),
            "Vec<",
            2
        ));
        assert!(!crate::totality::template_type_holds(
            "f.rs",
            Some(pre),
            "Vec<T0>",
            0
        ));
    }
    #[test]
    fn template_fn_counts_header_matches() {
        // The emitted minus header (wildcards + concrete slots) must
        // match exactly N fn items: `V Q` rides hetero vis, `(...)`
        // hetero params, omitted return matches any return.
        let pre =
            "pub fn alpha(x: i32) -> i32 {\n    x\n}\nfn alpha(y: String) -> String {\n    y\n}\n";
        assert!(crate::totality::template_fn_holds(
            "f.rs",
            Some(pre),
            "V Q fn alpha(...) {",
            None,
            2
        ));
        // Concrete slots narrow the domain: the params-concrete probe
        // matches only vis-less same-param sites (minus-omitted vis is
        // `Absent`, not match-any — the `V Q` pair is the wildcard).
        assert!(!crate::totality::template_fn_holds(
            "f.rs",
            Some(pre),
            "fn alpha(x : i32) {",
            None,
            2
        ));
        let pre_single =
            "pub fn other(x: i32) -> i32 {\n    x\n}\nfn alpha(x: i32) -> i32 {\n    x\n}\n";
        assert!(crate::totality::template_fn_holds(
            "f.rs",
            Some(pre_single),
            "fn alpha(x : i32) {",
            None,
            1
        ));
        // Method probes wrap in the block's verbatim header line and
        // count methods through the WI-4a filter.
        let pre_impl =
            "impl Svc {\n    fn alpha(x: i32) -> i32 {\n        x\n    }\n    fn alpha(y: String) -> String {\n        y\n    }\n}\n";
        assert!(crate::totality::template_fn_holds(
            "f.rs",
            Some(pre_impl),
            "fn alpha(...) {",
            Some("impl Svc {"),
            2
        ));
        // A wrong header matches nothing by construction.
        assert!(!crate::totality::template_fn_holds(
            "f.rs",
            Some(pre_impl),
            "fn alpha(...) {",
            Some("impl Other {"),
            2
        ));
        assert!(!crate::totality::template_fn_holds(
            "f.rs",
            Some(pre_impl),
            "fn alpha(...) {",
            None,
            2
        ));
    }
    #[test]
    fn template_fn_none_pre_allows_and_garbage_vetoes() {
        assert!(crate::totality::template_fn_holds(
            "f.rs",
            None,
            "fn alpha(...) {",
            None,
            2
        ));
        let pre = "fn m() {}\n";
        assert!(!crate::totality::template_fn_holds(
            "f.rs",
            Some(pre),
            "fn alpha(...) {",
            None,
            2
        ));
        assert!(!crate::totality::template_fn_holds(
            "f.rs",
            Some(pre),
            "fn alpha(",
            None,
            2
        ));
        assert!(!crate::totality::template_fn_holds(
            "f.rs",
            Some(pre),
            "fn alpha(...) {",
            None,
            0
        ));
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
