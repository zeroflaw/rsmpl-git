//! Tree-aware canonical form (SPEC D.5, WI-1a + WI-1b).
//!
//! [`canonicalize_with_tree`] compares patches by what they rewrite, not
//! how they cite it: each `allow == many` hunk is matched against the
//! pre-image tree and re-emitted under the D.5 range rule (tight range
//! when its sites fit in 64 lines, else the `# sweep` dummy), so an
//! authored sweep and an inferred ranged hunk over the same sites share
//! one canonical form. WI-1b adds spelling unification: an authored
//! `# symbol Old=crate::old_api` call hunk (`- Old(E)` / `+ New(E)`
//! with `# fallback=syntax`) is the same rewrite as the inferred
//! concrete diet form (`- crate::old_api(E0)` / `+ crate::new_api(E0)`),
//! so the tree-aware form rewrites the former to the latter (dropping
//! the `# symbol`/`# fallback=syntax` lines and diet-renaming declared
//! `# expr` metavars positionally). Pure [`rsmpl_patch::canonicalize`]
//! stays the spelling-preserving comparator (fixed-point pins); this is
//! the meaning-preserving comparator for the cycle harness.
//!
//! Lives here (not in `rsmpl_patch`) because matching needs the engine
//! and the engine already depends on the patch crate — the reverse
//! dependency would be a cycle (PLAN.md documents the deviation). The
//! call-shape gate uses `syn`, likewise engine-side only.

use std::collections::HashMap;

use rsmpl_patch::{
    canonicalize_with_spans_parsed, parse_patch, Directive, Hunk, MetavarKind, ParseError,
};

/// Canonicalize `patch_text` against the pre-image tree `pre`
/// (repo-relative path → file text).
///
/// Site spans come from a real engine apply on a clone of `pre`, so the
/// normalization reflects what the patch actually matches. Missing
/// targets seed as empty (new-file leniency, mirroring the `applyrs`
/// CLI); rename sections seed the new side from the old side's content
/// (hunks cite old-side coordinates, SPEC T377). Any apply failure
/// (zero-match, overlap, unparseable target) falls back to pure
/// [`canonicalize`] — comparison never hard-fails on a divergent leg;
/// the harness tree legs still pin real breakage.
pub fn canonicalize_with_tree(
    patch_text: &str,
    pre: &HashMap<String, String>,
) -> Result<String, ParseError> {
    let mut patch = parse_patch(patch_text)?;
    // WI-1b: spell-normalize first so the apply below (hence the site
    // spans) reflects the concrete form both legs share.
    normalize_symbol_spelling(&mut patch);
    // WI-5 slice 1: diet-rename declared let-metavars (`P`/`T`/`E`) to
    // the positional diet the inferrer emits (`P0`/`T0`/`E0`), same
    // posture as the `# expr` rename inside symbol hunks.
    normalize_let_diet_spelling(&mut patch);
    let mut files: HashMap<String, String> = pre.clone();
    for sec in &patch.files {
        if files.contains_key(&sec.target_path) {
            continue;
        }
        if sec.is_rename() {
            let old_rel = rsmpl_patch::stripped_path(&sec.old_path);
            if let Some(old_text) = pre.get(&old_rel).cloned() {
                files.insert(sec.target_path.clone(), old_text);
                continue;
            }
        }
        files.insert(sec.target_path.clone(), String::new());
    }
    let report = crate::apply_patch_to_tree(&mut files, &patch).ok();
    // Fallback on apply failure (zero-match, overlap, unparseable
    // target): empty spans keep authored ranges while the spelling
    // normalization above still applies — comparison never hard-fails
    // on a divergent leg; the harness tree legs still pin real breakage.
    let mut spans: HashMap<(String, String), Vec<(u32, u32)>> = HashMap::new();
    if let Some(report) = report {
        for entry in &report.rules {
            spans
                .entry((entry.file.clone(), entry.rule.clone()))
                .or_default()
                .extend(entry.spans.iter().copied());
        }
    }
    Ok(canonicalize_with_spans_parsed(patch, &spans))
}

/// WI-1b spelling unification (SPEC D.5, SPEC 19.3): rewrite each
/// `# symbol Name=path` call hunk with `# fallback=syntax` to the
/// concrete-path diet form the inferrer emits, dropping the `# symbol`
/// lines (and `# fallback=syntax` unless `# follow=uses` is present,
/// where it is the WI-6a skip switch). Declared `# expr` metavars are
/// diet-renamed positionally (`E` → `E0` in first-appearance order) and
/// their declarations dropped, matching inferrer emission.
///
/// All-or-nothing per hunk: without `# fallback=syntax` the symbol form
/// errors while the concrete form applies (not equivalent); non-call
/// shapes, method shapes (C14 rejects a symbol method ident), `let`
/// wrappers, and multi-pair bodies are left byte-identical so error
/// shapes never silently heal; any leftover symbol token aborts the
/// hunk rewrite.
fn normalize_symbol_spelling(patch: &mut rsmpl_patch::Patch) {
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
            for line in &mut lines {
                if line.is_removal() || line.is_addition() {
                    line.text = replace_symbol_tokens(&line.text, &symbols);
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
            // Diet-rename declared `# expr` metavars positionally.
            let mut expr_names: Vec<String> = Vec::new();
            for d in &r.directives {
                if let Directive::MetaVar { kind, names } = d {
                    if *kind == MetavarKind::Expr {
                        for n in names {
                            if !expr_names.contains(n) {
                                expr_names.push(n.clone());
                            }
                        }
                    }
                }
            }
            if !diet_rename_expr_metavars(&mut lines, &expr_names) {
                continue;
            }
            r.lines = lines;
            let keep_fallback = r.follows_uses();
            r.directives.retain(|d| match d {
                Directive::Symbol { .. } => false,
                Directive::FallbackSyntax(true) => keep_fallback,
                Directive::MetaVar { kind, .. } => *kind != MetavarKind::Expr,
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
                true
            });
        }
    }
}

/// WI-5 slice 1 diet spelling: a single `-`/`+` `let` pair carrying
/// declared `# pat`/`# type`/`# expr` metavars is rewritten to the
/// positional diet the inferrer emits (`P` → `P0`, `T` → `T0`,
/// `E` → `E0` in first-appearance order per kind), and those
/// declarations are dropped. Lets the authored long spelling and the
/// inferred diet share one tree-aware canonical form (the `24-pat-sweep`
/// leg). All-or-nothing per hunk, mirroring the symbol-hunk posture:
/// symbol hunks, multi-pair bodies, non-`let` lines, and any token
/// collision leave the hunk byte-identical so error shapes never
/// silently heal.
fn normalize_let_diet_spelling(patch: &mut rsmpl_patch::Patch) {
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
            if minus.len() != 1 || plus.len() != 1 {
                continue;
            }
            if !minus[0].trim_start().starts_with("let ")
                || !plus[0].trim_start().starts_with("let ")
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

/// Gate: the hunk body is exactly one `-`/`+` call pair whose callees
/// are both bare declared symbols (optional `::<...>` turbofish rides,
/// T199). Method calls, `let` wrappers, and multi-pair bodies fail.
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

/// Replace whole-token occurrences of each symbol name with its
/// concrete path, skipping backticked literals (`` `Old` `` is an
/// explicit literal, never a symbol ref), `::`-qualified occurrences
/// (`crate::Old` is already qualified), and `$`-sigiled positions.
/// Turbofish suffixes ride (`Old::<u8>` → `path::<u8>`).
fn replace_symbol_tokens(text: &str, symbols: &[(String, String)]) -> String {
    let mut names: Vec<&(String, String)> = symbols.iter().collect();
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
            if let Some((_, path)) = hit {
                out.push_str(&concrete_path_for(path));
                i += hit.unwrap().0.len();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// True when `text` holds `name` as a bare token outside backticks.
fn has_bare_token(text: &str, name: &str) -> bool {
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

/// WI-1b diet rename: declared `# expr` metavars become positional
/// (`E` → `E0` in first-appearance order across the pattern lines) so
/// the authored form matches inferrer emission, which declares nothing.
/// Backticked literals never rename. Collisions (a target already
/// present as a different token) veto the whole hunk rewrite.
fn diet_rename_expr_metavars(lines: &mut [rsmpl_patch::BodyLine], expr_names: &[String]) -> bool {
    if expr_names.is_empty() {
        return true;
    }
    // Order by first appearance (minus lines, then plus lines) so the
    // positional names match inferrer emission order.
    let mut positioned: Vec<(usize, usize, String)> = Vec::new();
    let mut order = 0usize;
    for l in lines.iter() {
        if !(l.is_removal() || l.is_addition()) {
            continue;
        }
        order += 1;
        for n in expr_names {
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
    for n in expr_names {
        if !ordered.contains(n) {
            ordered.push(n.clone());
        }
    }
    let mut table: Vec<(String, String)> = Vec::new();
    for (i, n) in ordered.iter().enumerate() {
        table.push((n.clone(), format!("E{i}")));
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

/// Whole-token replacement from an explicit (source → target) table.
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

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_patch::canonicalize;

    fn pre(files: &[(&str, &str)]) -> HashMap<String, String> {
        files
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    const PRE: &str = "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n";

    fn ranged_patch() -> String {
        "diff --git src/lib.rs\n@@ -2,3 @@ rule=calls\n# allow=many\n- foo(E0)\n+ bar(E0)\n"
            .to_string()
    }

    fn sweep_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=calls\n# sweep\n- foo(E0)\n+ bar(E0)\n".to_string()
    }

    #[test]
    fn sweep_and_ranged_unify() {
        // Same rewrite, two locality spellings → one tree-aware form.
        let p = pre(&[("src/lib.rs", PRE)]);
        let a = canonicalize_with_tree(&sweep_patch(), &p).unwrap();
        let b = canonicalize_with_tree(&ranged_patch(), &p).unwrap();
        assert_eq!(a, b, "sweep:\n{a}\nranged:\n{b}");
        assert!(
            a.contains("@@ -2,3 @@ rule=r1\n# allow=many\n"),
            "tight range wins under 64 lines:\n{a}"
        );
    }

    #[test]
    fn pure_canonical_keeps_spellings_apart() {
        // Guard for the two-comparator contract: pure form preserves the
        // authored locality, tree-aware form unifies it.
        let pure_sweep = canonicalize(&sweep_patch()).unwrap();
        let pure_ranged = canonicalize(&ranged_patch()).unwrap();
        assert_ne!(pure_sweep, pure_ranged);
    }

    #[test]
    fn wide_spread_normalizes_to_sweep() {
        // 70 one-line sites span 70 lines > 64 → both spellings become
        // sweep. Both patches must cover all sites (a ranged hunk only
        // rewrites its cited range): the ranged twin cites the full
        // `@@ -2,70 @@` tight range here.
        let mut src = String::from("fn m() {\n");
        for i in 0..70 {
            src.push_str(&format!("    foo({i});\n"));
        }
        src.push_str("}\n");
        let wide_ranged =
            "diff --git src/lib.rs\n@@ -2,70 @@ rule=calls\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[("src/lib.rs", &src)]);
        let a = canonicalize_with_tree(&sweep_patch(), &p).unwrap();
        let b = canonicalize_with_tree(wide_ranged, &p).unwrap();
        assert_eq!(a, b);
        assert!(a.contains("# sweep"), "wide spread → sweep:\n{a}");
        assert!(a.contains("@@ -1,1 @@"), "wide spread → dummy:\n{a}");
    }

    #[test]
    fn zero_match_falls_back_to_pure_form() {
        // Nothing matches: comparison degrades to pure canonicalize,
        // never an error.
        let p = pre(&[("src/lib.rs", "fn m() {}\n")]);
        let got = canonicalize_with_tree(&sweep_patch(), &p).unwrap();
        assert_eq!(got, canonicalize(&sweep_patch()).unwrap());
    }

    #[test]
    fn allow_one_ranges_untouched() {
        // Single-site hunks keep their authored ranges — the T375
        // single-site equality is tree + fixed-point, not canonical.
        let text = "diff --git src/lib.rs\n@@ -2,1 @@ rule=one\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[("src/lib.rs", PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("@@ -2,1 @@ rule=r1\n"),
            "allow=one range must survive:\n{got}"
        );
    }

    #[test]
    fn multifile_keys_stay_per_file() {
        // Same rule name in two files normalizes each file's range
        // from that file's own sites only.
        let text = "diff --git src/a.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\ndiff --git src/b.rs\n@@ -2,2 @@ rule=s\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[
            ("src/a.rs", "fn a() {\n    foo(1);\n}\n"),
            ("src/b.rs", "fn b() {\n    foo(1);\n    foo(2);\n}\n"),
        ]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("@@ -2,1 @@"),
            "a.rs tightens to its own site:\n{got}"
        );
    }

    const SYM_PRE: &str = "fn f() {\n    crate::old_api(1);\n    crate::old_api(2);\n}\n";

    fn symbol_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_symbols\n# scope=file\n# allow=many\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n"
            .to_string()
    }

    fn concrete_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_symbols\n# scope=file\n# allow=many\n- crate::old_api(E0)\n+ crate::new_api(E0)\n"
            .to_string()
    }

    #[test]
    fn symbol_and_concrete_unify() {
        // WI-1b: authored `# symbol` + sweep vs inferred concrete +
        // ranged diet → one tree-aware form (concrete, tight, E0).
        let p = pre(&[("src/lib.rs", SYM_PRE)]);
        let a = canonicalize_with_tree(&symbol_patch(), &p).unwrap();
        let b = canonicalize_with_tree(&concrete_patch(), &p).unwrap();
        assert_eq!(a, b, "symbol:\n{a}\nconcrete:\n{b}");
        assert!(
            a.contains(
                "@@ -2,2 @@ rule=r1\n# allow=many\n- crate::old_api(E0)\n+ crate::new_api(E0)\n"
            ),
            "concrete diet tight form:\n{a}"
        );
        assert!(!a.contains("# symbol"), "symbols dropped:\n{a}");
        assert!(!a.contains("# fallback"), "fallback dropped:\n{a}");
        assert!(!a.contains("# expr"), "redundant decl dropped:\n{a}");
    }

    #[test]
    fn pure_canonical_keeps_symbol_spelling() {
        // Two-comparator contract: pure form preserves the authored
        // symbol spelling; only the tree-aware form unifies.
        let pure_sym = canonicalize(&symbol_patch()).unwrap();
        assert!(
            pure_sym.contains("# symbol Old=crate::old_api"),
            "got:\n{pure_sym}"
        );
        assert!(pure_sym.contains("- Old(E)"), "got:\n{pure_sym}");
    }

    #[test]
    fn symbol_turbofish_unifies() {
        // T199 expansion: `Old::<u8>` → `crate::old_api::<u8>`.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old::<u8>(E)\n+ New::<u8>(E)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::old_api::<u8>(1);\n    crate::old_api::<u8>(2);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("- crate::old_api::<u8>(E0)\n+ crate::new_api::<u8>(E0)\n"),
            "got:\n{got}"
        );
    }

    #[test]
    fn symbol_without_fallback_untouched() {
        // No fallback: the symbol form errors (SymbolConflict) while the
        // concrete form applies — not equivalent, so no rewrite.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";
        let p = pre(&[("src/lib.rs", SYM_PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# symbol Old=crate::old_api"), "got:\n{got}");
        assert!(got.contains("- Old(E)"), "got:\n{got}");
    }

    #[test]
    fn symbol_follow_keeps_fallback() {
        // `# follow=uses` + `# fallback=syntax`: the fallback is the
        // WI-6a skip switch, not the symbol fallback — it stays while
        // the symbols still rewrite.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# fallback=syntax\n# follow=uses\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";
        let p = pre(&[("src/lib.rs", SYM_PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(!got.contains("# symbol"), "symbols dropped:\n{got}");
        assert!(
            got.contains("# fallback=syntax"),
            "skip switch stays:\n{got}"
        );
        assert!(got.contains("- crate::old_api(E0)"), "got:\n{got}");
    }

    #[test]
    fn symbol_method_shape_untouched() {
        // C14: a symbol naming the method ident is rejected — the error
        // shape must survive comparison, not silently heal.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol old=old_fn\n# expr E\n- Receiver.old(E)\n+ Receiver.new(E)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    r.old_fn(1);\n    r.old_fn(2);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# symbol old=old_fn"), "got:\n{got}");
    }

    #[test]
    fn symbol_backticked_literal_untouched() {
        // `` `Old` `` is an explicit literal (SPEC 15.4), never a symbol
        // ref: the callee does not name a symbol, so no rewrite.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- `Old`(E)\n+ `New`(E)\n";
        let p = pre(&[("src/lib.rs", SYM_PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# symbol Old=crate::old_api"), "got:\n{got}");
    }

    #[test]
    fn symbol_uppercase_path_backticked() {
        // WI-7-upper parity: an uppercase path tail rides backticked on
        // the last segment only.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::Old\n# symbol New=crate::New\n# expr E\n- Old(E)\n+ New(E)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::Old(1);\n    crate::Old(2);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("- crate::`Old`(E0)\n+ crate::`New`(E0)\n"),
            "got:\n{got}"
        );
    }

    const LET_PRE: &str = "fn f() {\n    let a: i32 = 0;\n    let b: u32 = 1;\n}\n";

    fn authored_let_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=sweep_pats\n# scope=file\n# allow=many\n# pat P\n# type T\n# expr E\n- let P: T = E;\n+ let P: T = E.wrapping_add(1);\n"
            .to_string()
    }

    fn inferred_let_patch() -> String {
        "diff --git src/lib.rs\n@@ -2,2 @@ rule=r1\n# allow=many\n- let P0: T0 = E0;\n+ let P0: T0 = E0.wrapping_add(1);\n"
            .to_string()
    }

    #[test]
    fn let_diet_spelling_unifies() {
        // WI-5 slice 1: authored long spelling (`P`/`T`/`E` + scope=file
        // sweep) vs inferred diet (`P0`/`T0`/`E0` + ranged) share one
        // tree-aware form (the `24-pat-sweep` leg).
        let p = pre(&[("src/lib.rs", LET_PRE)]);
        let a = canonicalize_with_tree(&authored_let_patch(), &p).unwrap();
        let b = canonicalize_with_tree(&inferred_let_patch(), &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- let P0: T0 = E0;\n+ let P0: T0 = E0.wrapping_add(1);\n"),
            "diet form:\n{a}"
        );
        assert!(!a.contains("# pat"), "decl dropped:\n{a}");
        assert!(!a.contains("# type"), "decl dropped:\n{a}");
        assert!(!a.contains("# expr"), "decl dropped:\n{a}");
    }

    #[test]
    fn let_diet_multi_pair_untouched() {
        // Two-pair bodies are out of scope for the single-pair diet
        // gate — the hunk survives byte-identical.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# pat P\n# expr E\n- let P = E;\n- let P = E;\n+ let P = E;\n+ let P = E;\n";
        let p = pre(&[("src/lib.rs", LET_PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# pat P"), "multi-pair untouched:\n{got}");
    }

    #[test]
    fn symbol_multi_arg_diet_order() {
        // Positional diet rename follows first appearance: (E, F) → E0, E1
        // in minus order, repetition preserved.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E, F\n- Old(F, E)\n+ New(F, E)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::old_api(b, a);\n    crate::old_api(d, c);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("- crate::old_api(E0, E1)\n+ crate::new_api(E0, E1)\n"),
            "got:\n{got}"
        );
    }
}
