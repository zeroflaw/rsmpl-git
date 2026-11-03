//! Pre-apply spelling normalizations for the tree-aware canonical form
//! (SPEC D.5, WI-1b + WI-5 + WI-8b).
//!
//! Split from `canonical_tree.rs` under the 2000-line gate (D5): pure
//! move, no behavior change. `canonical_tree.rs` keeps
//! [`canonicalize_with_tree`](crate::canonical_tree::canonicalize_with_tree)
//! plus the inline tests; this module owns the three spelling
//! normalizations and their helpers.

use rsmpl_patch::{Directive, Hunk, MetavarKind};

/// WI-1b spelling unification (SPEC D.5, SPEC 19.3): rewrite each
/// `# symbol Name=path` call hunk with `# fallback=syntax` to the
/// concrete-path diet form the inferrer emits, dropping the `# symbol`
/// lines (and `# fallback=syntax` unless `# follow=uses` is present,
/// where it is the WI-6a skip switch). Declared `# expr` / `# type`
/// metavars are diet-renamed positionally (`E` → `E0` in
/// first-appearance order; first type hole → bare `T`, extras `T1`, …)
/// and their declarations dropped, matching inferrer emission
/// (WI-1b-turbofish: `Old::<U>(E)` + `# type U` → `::<T>(E0)`).
///
/// All-or-nothing per hunk: without `# fallback=syntax` the symbol form
/// errors while the concrete form applies (not equivalent); non-call
/// shapes, method shapes (C14 rejects a symbol method ident), `let`
/// wrappers, and multi-pair bodies are left byte-identical so error
/// shapes never silently heal; any leftover symbol token aborts the
/// hunk rewrite.
pub(crate) fn normalize_symbol_spelling(patch: &mut rsmpl_patch::Patch) {
    for sec in &mut patch.files {
        for hunk in &mut sec.hunks {
            let Hunk::Rsmpl(r) = hunk else { continue };
            let symbols: Vec<(String, String)> = r
                .directives
                .iter()
                .filter_map(|d| match d {
                    Directive::Symbol { name, path } => Some((name.clone(), path.clone())),
                    _ => None,
                })
                .collect();
            if symbols.is_empty() || !r.fallback_syntax() {
                continue;
            }
            if !is_single_symbol_call_pair(r, &symbols) {
                continue;
            }
            let mut lines = r.lines.clone();
            // WI-1b-follow: symbol targets pre-resolve to concrete paths
            // so the single backtick-aware walker below serves both the
            // symbol and diet renames (one scanner, not two).
            let resolved: Vec<(String, String)> = symbols
                .iter()
                .map(|(n, p)| (n.clone(), concrete_path_for(p)))
                .collect();
            for line in &mut lines {
                if line.is_removal() || line.is_addition() {
                    line.text = replace_token_table(&line.text, &resolved);
                }
            }
            // Leftover check: every symbol name must be gone from the
            // pattern lines (outside backticked literals).
            let leftover = lines.iter().any(|l| {
                (l.is_removal() || l.is_addition())
                    && symbols.iter().any(|(n, _)| has_bare_token(&l.text, n))
            });
            if leftover {
                continue;
            }
            // Diet-rename declared `# expr` / `# type` metavars
            // positionally (WI-1b-turbofish: `Old::<U>(E)` + `# type U`
            // → inferred `::<T>(E0)` diet; first type hole is bare `T`
            // per the turbofish emitter, extras `T1`, `T2`, …).
            let mut expr_names: Vec<String> = Vec::new();
            let mut type_names: Vec<String> = Vec::new();
            for d in &r.directives {
                if let Directive::MetaVar { kind, names } = d {
                    let bucket = match kind {
                        MetavarKind::Expr => &mut expr_names,
                        MetavarKind::Type => &mut type_names,
                        _ => continue,
                    };
                    for n in names {
                        if !bucket.contains(n) {
                            bucket.push(n.clone());
                        }
                    }
                }
            }
            // SPEC 9.4: one name under two kinds never compiles — leave
            // the error shape byte-identical, never silently heal it.
            if expr_names.iter().any(|n| type_names.contains(n)) {
                continue;
            }
            if !diet_rename_symbol_hole_metavars(&mut lines, &expr_names, &type_names) {
                continue;
            }
            r.lines = lines;
            let keep_fallback = r.follows_uses();
            r.directives.retain(|d| match d {
                Directive::Symbol { .. } => false,
                Directive::FallbackSyntax(true) => keep_fallback,
                Directive::MetaVar { kind, .. } => {
                    *kind != MetavarKind::Expr && *kind != MetavarKind::Type
                }
                _ => true,
            });
            r.raw_directives.retain(|l| {
                let t = l.trim_start_matches('#').trim_start();
                if t.starts_with("symbol") {
                    return false;
                }
                if t.starts_with("fallback") {
                    return keep_fallback;
                }
                if t.starts_with("expr") {
                    return false;
                }
                if t.starts_with("type") {
                    return false;
                }
                true
            });
        }
    }
}

/// WI-5 slice 1 diet spelling (+ WI-5c sequence widening): a `-`/`+`
/// `let` pair — or K = 2–4 `let` lines per side (a WI-5c sequence hunk)
/// — carrying declared `# pat`/`# type`/`# expr` metavars is rewritten
/// to the positional diet the inferrer emits (`P` → `P0`, `T` → `T0`,
/// `E` → `E0` in first-appearance order per kind across the whole hunk,
/// so sequence positions land on `P1`/`T1`/`E1` exactly like emission),
/// and those declarations are dropped. Lets the authored long spelling
/// and the inferred diet share one tree-aware canonical form (the
/// `24-pat-sweep` leg, sequences likewise). All-or-nothing per hunk,
/// mirroring the symbol-hunk posture: symbol hunks, non-`let` lines,
/// count-mismatched sides, and any token collision leave the hunk
/// byte-identical so error shapes never silently heal.
pub(crate) fn normalize_let_diet_spelling(patch: &mut rsmpl_patch::Patch) {
    use rsmpl_patch::{Directive, MetavarKind};
    for sec in &mut patch.files {
        for hunk in &mut sec.hunks {
            let rsmpl_patch::Hunk::Rsmpl(r) = hunk else {
                continue;
            };
            if r.directives
                .iter()
                .any(|d| matches!(d, Directive::Symbol { .. }))
            {
                continue;
            }
            let mut minus: Vec<&str> = Vec::new();
            let mut plus: Vec<&str> = Vec::new();
            for l in &r.lines {
                if l.is_removal() {
                    minus.push(l.text.as_str());
                } else if l.is_addition() {
                    plus.push(l.text.as_str());
                }
            }
            // WI-5c: single pairs and K = 2–4 `let` sequences alike
            // (count-matched sides only; anything else is not diet).
            if minus.len() != plus.len()
                || !(1..=4).contains(&minus.len())
                || !minus
                    .iter()
                    .chain(plus.iter())
                    .all(|l| l.trim_start().starts_with("let "))
            {
                continue;
            }
            let mut pats: Vec<String> = Vec::new();
            let mut tys: Vec<String> = Vec::new();
            let mut exprs: Vec<String> = Vec::new();
            for d in &r.directives {
                if let Directive::MetaVar { kind, names } = d {
                    let bucket = match kind {
                        MetavarKind::Pat => &mut pats,
                        MetavarKind::Type => &mut tys,
                        MetavarKind::Expr => &mut exprs,
                        _ => continue,
                    };
                    for n in names {
                        if !bucket.contains(n) {
                            bucket.push(n.clone());
                        }
                    }
                }
            }
            // WI-7 slice 2: undeclared diet-short holes. An authored let
            // template may spell holes bare (`Some(P)`, `E`) with no
            // declarations at all — the pattern compiler still infers
            // them (SPEC 15.8) and the engine applies them as holes, but
            // the buckets above stay empty and the hunk stays pinned.
            // Synthesize buckets from bare single-letter hole tokens by
            // conventional kind (`P`/`Q` → pat, `T` → type, `E` → expr)
            // so the undeclared spelling shares the declared/inferred
            // canonical form. Only single letters synthesize (a
            // multi-letter undeclared name needs kind inference the
            // comparison must not guess), and never a name already in
            // any bucket (SPEC 9.4: one name under two kinds never
            // compiles). The inferrer leg stays a no-op: its tokens
            // are already positional (`P0`, `E0`, …), which never
            // contain a single letter as a bare token under the
            // ident-boundary matcher below. Bare `T` synthesizes here
            // even though it is diet vocabulary for call shapes
            // (turbofish emitter): this pass only processes all-`let`
            // line sets, where the inferred type hole is `T0`, never
            // bare `T`. (A bare single-uppercase letter the author
            // meant literally is still a hole under engine semantics
            // — SPEC 15.8 inference — so the rename is
            // meaning-preserving.) The all-or-nothing
            // collision/leftover posture below owns correctness
            // exactly as for declared names.
            for name in ["P", "Q", "T", "E"] {
                if pats.contains(&name.to_string())
                    || tys.contains(&name.to_string())
                    || exprs.contains(&name.to_string())
                {
                    continue;
                }
                if r.lines
                    .iter()
                    .any(|l| (l.is_removal() || l.is_addition()) && has_bare_token(&l.text, name))
                {
                    match name {
                        "P" | "Q" => pats.push(name.to_string()),
                        "T" => tys.push(name.to_string()),
                        _ => exprs.push(name.to_string()),
                    }
                }
            }
            if pats.is_empty() && tys.is_empty() && exprs.is_empty() {
                continue;
            }
            // Positional order per kind: first appearance across the
            // pattern lines (minus, then plus — document order).
            let order_names = |names: &[String], lines: &[rsmpl_patch::BodyLine]| {
                let mut positioned: Vec<(usize, usize, String)> = Vec::new();
                for (order, l) in lines.iter().enumerate() {
                    if !(l.is_removal() || l.is_addition()) {
                        continue;
                    }
                    for n in names {
                        if positioned.iter().any(|(_, _, m)| m == n) {
                            continue;
                        }
                        if let Some(pos) = first_token_pos(&l.text, n) {
                            positioned.push((order, pos, n.clone()));
                        }
                    }
                }
                positioned.sort();
                let mut ordered: Vec<String> = positioned.into_iter().map(|(_, _, n)| n).collect();
                for n in names {
                    if !ordered.contains(n) {
                        ordered.push(n.clone());
                    }
                }
                ordered
            };
            let mut table: Vec<(String, String)> = Vec::new();
            for (names, prefix) in [(&pats, "P"), (&tys, "T"), (&exprs, "E")] {
                for (i, n) in order_names(names, &r.lines).iter().enumerate() {
                    table.push((n.clone(), format!("{prefix}{i}")));
                }
            }
            // Collision: a target present as a bare token that is not
            // itself renamed away vetoes the hunk rewrite.
            let sources: Vec<&str> = table.iter().map(|(s, _)| s.as_str()).collect();
            let mut collides = false;
            for l in r.lines.iter() {
                if !(l.is_removal() || l.is_addition()) {
                    continue;
                }
                for (s, t) in &table {
                    if s != t && has_bare_token(&l.text, t) && !sources.contains(&t.as_str()) {
                        collides = true;
                        break;
                    }
                }
                if collides {
                    break;
                }
            }
            if collides {
                continue;
            }
            for l in r.lines.iter_mut() {
                if !(l.is_removal() || l.is_addition()) {
                    continue;
                }
                l.text = replace_token_table(&l.text, &table);
            }
            // Every renamed-away name must be gone.
            let mut leftover = false;
            for (s, t) in &table {
                if s != t
                    && r.lines
                        .iter()
                        .any(|l| (l.is_removal() || l.is_addition()) && has_bare_token(&l.text, s))
                {
                    leftover = true;
                    break;
                }
            }
            if leftover {
                continue;
            }
            r.directives.retain(|d| match d {
                Directive::MetaVar { kind, .. } => !matches!(
                    kind,
                    MetavarKind::Pat | MetavarKind::Type | MetavarKind::Expr
                ),
                _ => true,
            });
            r.raw_directives.retain(|l| {
                let t = l.trim_start_matches('#').trim_start();
                !(t.starts_with("pat") || t.starts_with("type") || t.starts_with("expr"))
            });
        }
    }
}

/// WI-8b template-diet unification (SPEC D.5): pre-apply spelling
/// normalization in the WI-1b posture — the apply below proves spans
/// from the shared diet form, so same-site spellings merge while the
/// WI-8 range rule keeps site sets pinned (comparison-only;
/// application semantics untouched). Three independent per-hunk rules:
/// L1 drops default-valued `# lang=rust`; N1 diet-renames declared
/// `# expr` holes in single-pair call/method/general-expr shapes; N3
/// contracts framed fn pure-rename pairs to the bare diet (the bare
/// form matches a superset of the framed form's sites, but the mapping
/// is deterministic per leg — legs that land on identical bytes prove
/// identical spans, and the harness tree legs independently verify
/// both rebuilds).
pub(crate) fn normalize_template_diet(patch: &mut rsmpl_patch::Patch) {
    for sec in &mut patch.files {
        for hunk in &mut sec.hunks {
            let Hunk::Rsmpl(r) = hunk else { continue };
            // L1: explicit `# lang=rust` states the default parse mode
            // (Ast) the inferrer never writes. Skipped beside an
            // explicit `Mode` directive (unknown lang×mode interaction
            // stays pinned).
            if !r.directives.iter().any(|d| matches!(d, Directive::Mode(_))) {
                r.directives.retain(|d| !matches!(d, Directive::LangRust));
                r.raw_directives
                    .retain(|l| !(l.trim_start_matches('#').trim_start().starts_with("lang")));
            }
            // N3 before N1: contraction consumes framed pairs (which
            // carry no declared holes in the corpus); N1 then no-ops
            // on the bare diet.
            contract_framed_fn_single(r);
            diet_rename_declared_expr_holes(r);
        }
    }
}

/// WI-8b N3: contract a framed fn pure-rename pair to the bare
/// `- fn Old` / `+ fn New` diet the precision-single raiser emits
/// (fixtures 01 terse/explicit-framed, 51 framed-vs-bare). Runs
/// pre-apply so spans prove from the shared diet form (framed item
/// spans vs bare header-line spans would otherwise never agree).
/// Gates: contained/file scope, exactly one minus/plus pair, both
/// `fn` headers (optional `V Q ` prefix with equal presence)
/// token-identical outside the name, differing names, all other lines
/// pure frame context (`...`/`{`/`}`). Evolutions, concrete vis,
/// multi-pair bodies, the `N` name hole, and stray context stay
/// byte-identical.
fn contract_framed_fn_single(r: &mut rsmpl_patch::RsmplHunk) {
    if !matches!(
        r.scope(),
        rsmpl_patch::Scope::Contained | rsmpl_patch::Scope::File
    ) {
        return;
    }
    let mut minus_idx: Option<usize> = None;
    let mut plus_idx: Option<usize> = None;
    for (i, l) in r.lines.iter().enumerate() {
        if l.is_removal() {
            if minus_idx.is_some() {
                return;
            }
            minus_idx = Some(i);
        } else if l.is_addition() {
            if plus_idx.is_some() {
                return;
            }
            plus_idx = Some(i);
        } else {
            let t = l.text.trim();
            if !(t == "..." || t == "{" || t == "}") {
                return;
            }
        }
    }
    let (mi, pi) = match (minus_idx, plus_idx) {
        (Some(a), Some(b)) => (a, b),
        _ => return,
    };
    let (m_vq, m_name, m_rest) = match fn_name_and_rest(&r.lines[mi].text) {
        Some(x) => x,
        None => return,
    };
    let (p_vq, p_name, p_rest) = match fn_name_and_rest(&r.lines[pi].text) {
        Some(x) => x,
        None => return,
    };
    if m_vq != p_vq || m_rest != p_rest || m_name == p_name {
        return;
    }
    // The `N` fn-header name hole (SPEC 15.8) is a template, not a
    // rename — never contract it to a bare literal-looking pair.
    if m_name == "N" || p_name == "N" {
        return;
    }
    r.lines[mi].text = format!(" fn {m_name}");
    r.lines[pi].text = format!(" fn {p_name}");
    r.lines.retain(|l| l.is_removal() || l.is_addition());
}

/// Split a `fn` header line into (`V Q` prefix present, name, rest):
/// `V Q fn foo G (...) W {` → `(true, "foo", " G (...) W {")`,
/// `fn foo(...) {` → `(false, "foo", "(...) {")`,
/// `fn foo` → `(false, "foo", "")`. Anything else (concrete vis,
/// backticked names, non-headers) is `None`.
fn fn_name_and_rest(line: &str) -> Option<(bool, String, String)> {
    let t = line.trim_start();
    let (vq, t) = match t.strip_prefix("V Q ") {
        Some(r) => (true, r),
        None => (false, t),
    };
    let t = t.strip_prefix("fn ")?;
    let (name, rest) = match t.find(|c: char| !(c.is_alphanumeric() || c == '_')) {
        Some(i) => t.split_at(i),
        None => (t, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some((vq, name.to_string(), rest.to_string()))
}

/// WI-8b N1: diet-rename declared `# expr` holes in single-pair
/// call/method/general-expr shapes to the positional diet the
/// inferrer emits (`E` → `E0`, `Arg` → `E0`, …), keeping inferrer
/// diet vocabulary (`Receiver`, `E0…`, `P0…`, `T0…`, bare `T`,
/// `V Q G W N I`) verbatim and dropping all `# expr` declarations on
/// success (fixtures 11/78 method, 79 general). Inferrer legs carry
/// no declarations and are already diet, so this is a no-op for them
/// and both spellings land on one form. All-or-nothing per hunk:
/// symbol hunks, turbofish (`::<`) pairs, `let`-led or `fn`-header
/// lines, multi-pair bodies, any context line, unmentioned declared
/// names, and token collisions keep the hunk byte-identical.
fn diet_rename_declared_expr_holes(r: &mut rsmpl_patch::RsmplHunk) {
    use rsmpl_patch::MetavarKind;
    if r.directives
        .iter()
        .any(|d| matches!(d, Directive::Symbol { .. }))
    {
        return;
    }
    let mut minus = 0usize;
    let mut plus = 0usize;
    for l in &r.lines {
        if l.is_removal() || l.is_addition() {
            let t = l.text.trim_start();
            if t.starts_with("let ") || t.starts_with("fn ") || t.starts_with("V Q fn ") {
                return;
            }
            if l.text.contains("::<") {
                return;
            }
            if l.is_removal() {
                minus += 1;
            } else {
                plus += 1;
            }
        } else {
            return;
        }
    }
    if minus != 1 || plus != 1 {
        return;
    }
    let mut declared: Vec<String> = Vec::new();
    for d in &r.directives {
        if let Directive::MetaVar {
            kind: MetavarKind::Expr,
            names,
        } = d
        {
            for n in names {
                if !declared.contains(n) {
                    declared.push(n.clone());
                }
            }
        }
    }
    if declared.is_empty() {
        return;
    }
    // Every declared name must name a bare token in the pair (else an
    // unfamiliar shape — stay pinned).
    for n in &declared {
        if !r
            .lines
            .iter()
            .any(|l| (l.is_removal() || l.is_addition()) && has_bare_token(&l.text, n))
        {
            return;
        }
    }
    // Positional diet by first appearance across the pair (minus,
    // then plus — document order), mirroring inferrer emission: the
    // i-th renamed hole becomes `E{i}`; diet-vocabulary names stay
    // verbatim.
    let mut positioned: Vec<(usize, usize, String)> = Vec::new();
    let mut li = 0usize;
    for l in &r.lines {
        if !(l.is_removal() || l.is_addition()) {
            continue;
        }
        li += 1;
        for n in &declared {
            if positioned.iter().any(|(_, _, m)| m == n) {
                continue;
            }
            if let Some(pos) = first_token_pos(&l.text, n) {
                positioned.push((li, pos, n.clone()));
            }
        }
    }
    positioned.sort();
    let order: Vec<String> = positioned.into_iter().map(|(_, _, n)| n).collect();
    let mut table: Vec<(String, String)> = Vec::new();
    let mut idx = 0usize;
    for n in &order {
        if is_diet_hole(n) {
            continue;
        }
        table.push((n.clone(), format!("E{idx}")));
        idx += 1;
    }
    if table.is_empty() {
        // All-declared names already diet (e.g. redundant `# expr E0`):
        // still drop the redundant declarations to share the
        // declaration-free diet form — but only when every declared
        // name is diet vocabulary (no-op rename, pure decl strip).
        // (Falls through to the retain below.)
    } else {
        // Collision: a target present as a bare token that is not
        // itself renamed away vetoes the hunk rewrite.
        let sources: Vec<&str> = table.iter().map(|(s, _)| s.as_str()).collect();
        for l in r.lines.iter() {
            if !(l.is_removal() || l.is_addition()) {
                continue;
            }
            for (s, t) in &table {
                if s != t && has_bare_token(&l.text, t) && !sources.contains(&t.as_str()) {
                    return;
                }
            }
        }
        for l in r.lines.iter_mut() {
            if !(l.is_removal() || l.is_addition()) {
                continue;
            }
            l.text = replace_token_table(&l.text, &table);
        }
        for (s, t) in &table {
            if s != t
                && r.lines
                    .iter()
                    .any(|l| (l.is_removal() || l.is_addition()) && has_bare_token(&l.text, s))
            {
                return;
            }
        }
    }
    r.directives.retain(|d| {
        !matches!(
            d,
            Directive::MetaVar {
                kind: MetavarKind::Expr,
                ..
            }
        )
    });
    r.raw_directives
        .retain(|l| !(l.trim_start_matches('#').trim_start().starts_with("expr")));
}

/// Inferrer diet vocabulary (never renamed by N1): the method-receiver
/// hole, positional `E/P/T` holes, the turbofish single-hole `T`, and
/// the fn-header/item name holes.
fn is_diet_hole(name: &str) -> bool {
    if matches!(name, "Receiver" | "T" | "V" | "Q" | "G" | "W" | "N" | "I") {
        return true;
    }
    let b = name.as_bytes();
    b.len() >= 2
        && (b[0] == b'E' || b[0] == b'P' || b[0] == b'T')
        && b[1..].iter().all(|c| c.is_ascii_digit())
}
fn is_single_symbol_call_pair(r: &rsmpl_patch::RsmplHunk, symbols: &[(String, String)]) -> bool {
    let mut minus: Vec<&str> = Vec::new();
    let mut plus: Vec<&str> = Vec::new();
    for l in &r.lines {
        if l.is_removal() {
            minus.push(l.text.as_str());
        } else if l.is_addition() {
            plus.push(l.text.as_str());
        }
    }
    if minus.len() != 1 || plus.len() != 1 {
        return false;
    }
    if minus[0].trim_start().starts_with("let ") || plus[0].trim_start().starts_with("let ") {
        return false;
    }
    is_bare_symbol_call(minus[0], symbols) && is_bare_symbol_call(plus[0], symbols)
}

/// True when `text` parses as a bare `Name(args)` / `Name::<...>(args)`
/// call with `Name` in `symbols`.
fn is_bare_symbol_call(text: &str, symbols: &[(String, String)]) -> bool {
    let t = text.trim().trim_end_matches(';').trim();
    let Ok(syn::Expr::Call(call)) = syn::parse_str::<syn::Expr>(t) else {
        return false;
    };
    let syn::Expr::Path(p) = call.func.as_ref() else {
        return false;
    };
    if p.qself.is_some() || p.path.leading_colon.is_some() || p.path.segments.len() != 1 {
        return false;
    }
    let seg = &p.path.segments[0];
    if !matches!(
        seg.arguments,
        syn::PathArguments::None | syn::PathArguments::AngleBracketed(_)
    ) {
        return false;
    }
    symbols.iter().any(|(n, _)| n == &seg.ident.to_string())
}

/// Concrete-path render with WI-7-upper parity: an uppercase tail
/// segment rides backticked on the last segment only
/// (``crate::`Old` ``), lowercase passes through untouched.
fn concrete_path_for(path: &str) -> String {
    let (leading, rest) = match path.strip_prefix("::") {
        Some(r) => ("::", r),
        None => ("", path),
    };
    let segs: Vec<&str> = rest.split("::").collect();
    let last = segs.last().copied().unwrap_or("");
    if last.starts_with(|c: char| c.is_ascii_uppercase()) {
        let mut owned: Vec<String> = segs[..segs.len().saturating_sub(1)]
            .iter()
            .map(|s| s.to_string())
            .collect();
        owned.push(format!("`{last}`"));
        return format!("{leading}{}", owned.join("::"));
    }
    path.to_string()
}

/// True when `text` holds `name` as a bare token outside backticks.
/// Shared with `single_site_eq` (the WI-1s1 leftover-hole gate).
pub(crate) fn has_bare_token(text: &str, name: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_backtick = false;
    while i < chars.len() {
        if chars[i] == '`' {
            in_backtick = !in_backtick;
            i += 1;
            continue;
        }
        if !in_backtick
            && text_starts_at(&chars, i, name)
            && boundary_before(&chars, i)
            && boundary_after(&chars, i + name.len())
        {
            return true;
        }
        i += 1;
    }
    false
}

/// Byte (char) index of the first bare-token occurrence, if any.
fn first_token_pos(text: &str, name: &str) -> Option<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_backtick = false;
    while i < chars.len() {
        if chars[i] == '`' {
            in_backtick = !in_backtick;
            i += 1;
            continue;
        }
        if !in_backtick
            && text_starts_at(&chars, i, name)
            && boundary_before(&chars, i)
            && boundary_after(&chars, i + name.len())
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn text_starts_at(chars: &[char], at: usize, name: &str) -> bool {
    let want: Vec<char> = name.chars().collect();
    chars.len() >= at + want.len() && chars[at..at + want.len()] == want[..]
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// A token starts here unless preceded by an ident/`$`/`::` char.
/// `::Old` keeps its spelling (already qualified); `$Old` is sigiled
/// mode, out of scope for this pass.
fn boundary_before(chars: &[char], at: usize) -> bool {
    if at == 0 {
        return true;
    }
    let p = chars[at - 1];
    !(is_word_char(p) || p == ':')
}

/// A token ends here unless followed by an ident/`$` char. A following
/// `:` is allowed (turbofish `Old::<u8>` rewrites its head).
fn boundary_after(chars: &[char], at: usize) -> bool {
    if at >= chars.len() {
        return true;
    }
    !is_word_char(chars[at])
}

/// WI-1b diet rename (+ WI-1b-turbofish): declared `# expr` / `# type`
/// metavars become positional in one combined single-pass table so the
/// authored form matches inferrer emission, which declares nothing.
/// Expr names map `E` → `E0`, `E1`, … in first-appearance order; type
/// names map the first hole to bare `T` (the turbofish emitter's
/// single-hole spelling, `- foo::<T>(E0)`) and extras to `T1`, `T2`, …
/// (the inferrer never emits those, so a two-hole authored shape simply
/// fails to unify downstream instead of healing). Backticked literals
/// never rename. Collisions (a target already present as a different
/// token) veto the whole hunk rewrite. One pass (not two sequential
/// renames) so an expr target that is also a type source renames from
/// original spelling only.
fn diet_rename_symbol_hole_metavars(
    lines: &mut [rsmpl_patch::BodyLine],
    expr_names: &[String],
    type_names: &[String],
) -> bool {
    if expr_names.is_empty() && type_names.is_empty() {
        return true;
    }
    // Order each kind by first appearance (minus lines, then plus
    // lines) so the positional names match inferrer emission order.
    let order_names = |names: &[String]| {
        let mut positioned: Vec<(usize, usize, String)> = Vec::new();
        let mut order = 0usize;
        for l in lines.iter() {
            if !(l.is_removal() || l.is_addition()) {
                continue;
            }
            order += 1;
            for n in names {
                if positioned.iter().any(|(_, _, m)| m == n) {
                    continue;
                }
                if let Some(pos) = first_token_pos(&l.text, n) {
                    positioned.push((order, pos, n.clone()));
                }
            }
        }
        positioned.sort();
        let mut ordered: Vec<String> = positioned.into_iter().map(|(_, _, n)| n).collect();
        for n in names {
            if !ordered.contains(n) {
                ordered.push(n.clone());
            }
        }
        ordered
    };
    let mut table: Vec<(String, String)> = Vec::new();
    for (i, n) in order_names(expr_names).iter().enumerate() {
        table.push((n.clone(), format!("E{i}")));
    }
    for (i, n) in order_names(type_names).iter().enumerate() {
        let target = if i == 0 {
            "T".to_string()
        } else {
            format!("T{i}")
        };
        table.push((n.clone(), target));
    }
    // Collision: a target present as a bare token that is not itself
    // renamed away.
    let sources: Vec<&str> = table.iter().map(|(s, _)| s.as_str()).collect();
    for l in lines.iter() {
        if !(l.is_removal() || l.is_addition()) {
            continue;
        }
        for (s, t) in &table {
            if s != t && has_bare_token(&l.text, t) && !sources.contains(&t.as_str()) {
                return false;
            }
        }
    }
    for l in lines.iter_mut() {
        if !(l.is_removal() || l.is_addition()) {
            continue;
        }
        l.text = replace_token_table(&l.text, &table);
    }
    // Every declared name must be gone (or have been a no-op rename).
    for (s, t) in &table {
        if s != t
            && lines
                .iter()
                .any(|l| (l.is_removal() || l.is_addition()) && has_bare_token(&l.text, s))
        {
            return false;
        }
    }
    true
}

/// Whole-token replacement from an explicit (source → target) table,
/// skipping backticked literals (`` `Old` `` is an explicit literal,
/// never a rename ref), `::`-qualified occurrences (`crate::Old` is
/// already qualified), and `$`-sigiled positions. Turbofish suffixes
/// ride (`Old::<u8>` → `path::<u8>`). Serves both the WI-1b symbol
/// normalization (targets pre-resolved via [`concrete_path_for`]) and
/// the diet renames — one backtick-aware walker (WI-1b-follow).
fn replace_token_table(text: &str, table: &[(String, String)]) -> String {
    let mut names: Vec<&(String, String)> = table.iter().collect();
    names.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut in_backtick = false;
    while i < chars.len() {
        if chars[i] == '`' {
            in_backtick = !in_backtick;
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if !in_backtick {
            let mut hit: Option<&(String, String)> = None;
            for s in &names {
                if text_starts_at(&chars, i, &s.0)
                    && boundary_before(&chars, i)
                    && boundary_after(&chars, i + s.0.len())
                {
                    hit = Some(*s);
                    break;
                }
            }
            if let Some((s, t)) = hit {
                out.push_str(t);
                i += s.len();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
