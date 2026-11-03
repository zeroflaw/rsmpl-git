//! Tree-aware canonical form (SPEC D.5, WI-1a + WI-1b).
//!
//! [`canonicalize_with_tree`] compares patches by what they rewrite, not
//! how they cite it: each contained/file-scope hunk with engine-proven
//! spans is re-emitted under the D.5 range rule (multi-site hunks: tight
//! range when their sites fit in 64 lines, else the `# sweep` dummy;
//! WI-8: single-site hunks of either cardinality collapse to the diet
//! directive-free tight actual range), so an authored sweep, an inferred
//! ranged hunk, an inferred precision single, and a stale-cited hunk over
//! the same sites share one canonical form. WI-1b adds spelling unification: an authored
//! `# symbol Old=crate::old_api` call hunk (`- Old(E)` / `+ New(E)`
//! with `# fallback=syntax`) is the same rewrite as the inferred
//! concrete diet form (`- crate::old_api(E0)` / `+ crate::new_api(E0)`),
//! so the tree-aware form rewrites the former to the latter (dropping
//! the `# symbol`/`# fallback=syntax` lines and diet-renaming declared
//! `# expr` / `# type` metavars positionally (`E` → `E0`, first type
//! hole → bare `T` per the turbofish emitter). Pure [`rsmpl_patch::canonicalize`]
//! stays the spelling-preserving comparator (fixed-point pins); this is
//! the meaning-preserving comparator for the cycle harness. WI-8b adds
//! template-diet unification (pre-apply, comparison-only): the
//! default-valued `# lang=rust` line drops, declared `# expr` holes in
//! single-pair call/method/general-expr shapes diet-rename positionally
//! while inferrer diet vocabulary stays verbatim, and framed single-site
//! fn pure-rename pairs contract to the bare `- fn Old` / `+ fn New` diet.
//!
//! Lives here (not in `rsmpl_patch`) because matching needs the engine
//! and the engine already depends on the patch crate — the reverse
//! dependency would be a cycle (PLAN.md documents the deviation). The
//! call-shape gate uses `syn`, likewise engine-side only.

use std::collections::HashMap;

use rsmpl_patch::{
    canonicalize_with_spans_parsed, parse_patch, Directive, Hunk, MetavarKind, ParseError,
};

use crate::single_site_eq::{post_tree_key, unify_single_site_verbatim_to_literal};

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
    // WI-1s1 raw snapshot: site evidence falls back to the raw spelling
    // when a normalization below drops a declaration the engine needs
    // (the rewrite itself is verified before it is kept).
    let raw_patch = patch.clone();
    // WI-1b: spell-normalize first so the apply below (hence the site
    // spans) reflects the concrete form both legs share.
    normalize_symbol_spelling(&mut patch);
    // WI-5 slice 1 diet-rename declared let-metavars (`P`/`T`/`E`) to
    // the positional diet the inferrer emits (`P0`/`T0`/`E0`), same
    // posture as the `# expr` rename inside symbol hunks.
    normalize_let_diet_spelling(&mut patch);
    // WI-8b: template-diet spell-normalize (same posture — the apply
    // below proves spans from the shared diet form, so same-site
    // spellings merge while the WI-8 range rule keeps site sets
    // pinned; comparison-only, application semantics untouched).
    normalize_template_diet(&mut patch);
    let mut files: HashMap<String, String> = pre.clone();
    for sec in &patch.files {
        if files.contains_key(&sec.target_path) {
            continue;
        }
        if sec.is_rename() {
            let old_rel = rsmpl_patch::stripped_path(&sec.old_path);
            if let Some(old_text) = pre.get(&old_rel).cloned() {
                files.insert(sec.target_path.clone(), old_text.clone());
                continue;
            }
        }
        files.insert(sec.target_path.clone(), String::new());
    }
    // WI-1s0 verification base: the seeded pre-tree, so the unified
    // patch below can re-prove the identical post-tree from scratch.
    let seeded = files.clone();
    // WI-1s1 raw site evidence: the spelling normalizations above may
    // drop declarations the engine needs in some positions (an
    // undeclared diet hole applies in `let`-init but not in
    // array-element position), so a leg that applies raw but not
    // normalized still contributes its site evidence to the gate. The
    // rewrite itself is verified before it is kept.
    let mut raw_files = seeded.clone();
    let mut raw_spans: HashMap<(String, String), Vec<(u32, u32)>> = HashMap::new();
    let raw_post: Option<HashMap<String, String>> =
        match crate::apply_patch_to_tree(&mut raw_files, &raw_patch) {
            Ok(raw_report) => {
                for entry in &raw_report.rules {
                    raw_spans
                        .entry((entry.file.clone(), entry.rule.clone()))
                        .or_default()
                        .extend(entry.spans.iter().copied());
                }
                Some(raw_files)
            }
            Err(_) => None,
        };
    let report = crate::apply_patch_to_tree(&mut files, &patch).ok();
    let applied = report.is_some();
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
    // WI-1s1 single-site instantiation equality (verbatim family, D17):
    // rewrite pure single-pair hunks that prove exactly one line site
    // to their instantiated literal pair, then keep the rewrite only
    // when re-applying the unified patch rebuilds the identical
    // post-tree (same key). Over-wide templates (two line sites) and
    // unclassifiable shapes fail the gate and stay byte-identical, so
    // comparison never heals a real divergence. Runs when either apply
    // proves a post-tree; the literal lines come from that post-tree.
    if applied || raw_post.is_some() {
        let post: &HashMap<String, String> = if applied {
            &files
        } else {
            raw_post.as_ref().expect("WI-1s1: raw post proves the gate")
        };
        let snapshot = patch.clone();
        let mut unified_spans = spans.clone();
        unify_single_site_verbatim_to_literal(
            &mut patch,
            pre,
            post,
            &mut unified_spans,
            &raw_spans,
        );
        let mut check = seeded;
        let same = crate::apply_patch_to_tree(&mut check, &patch).is_ok()
            && post_tree_key(&check) == post_tree_key(post);
        if same {
            spans = unified_spans;
        } else {
            patch = snapshot;
        }
    }
    Ok(canonicalize_with_spans_parsed(patch, &spans))
}

/// WI-1s0 instantiation-proof key (IMPROVE.md D17 premise).
///
/// Applies `patch_text` to a clone of `pre` and returns the
/// [`post_tree_key`] of the full post-tree, so key equality means
/// byte-identical post-trees. Returns `None` when the patch fails to
/// parse or apply — never panics, mirroring the fallback posture of
/// [`canonicalize_with_tree`]. Premise pin for the WI-1s1 equality
/// (which verifies through the same key); exercised by tests.
#[allow(dead_code)] // WI-1s1 verifies through `post_tree_key` directly; tests own this helper.
pub(crate) fn single_site_post_key(
    pre: &HashMap<String, String>,
    patch_text: &str,
) -> Option<String> {
    let patch = parse_patch(patch_text).ok()?;
    let mut files: HashMap<String, String> = pre.clone();
    for sec in &patch.files {
        if !files.contains_key(&sec.target_path) {
            files.insert(sec.target_path.clone(), String::new());
        }
    }
    crate::apply_patch_to_tree(&mut files, &patch).ok()?;
    Some(post_tree_key(&files))
}

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
fn normalize_template_diet(patch: &mut rsmpl_patch::Patch) {
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
    fn let_diet_undeclared_short_holes_unify() {
        // WI-7 slice 2: the undeclared diet-short spelling (`Some(P)`,
        // `E`, no declarations — fixture 84) shares the canonical form
        // with the declared long spelling (fixture 73) and the inferred
        // diet (`Some(P0)`, `E0`). The engine applies all three as the
        // same holes (SPEC 15.8 inference), so the rename is
        // meaning-preserving.
        let p = pre(&[(
            "src/lib.rs",
            "fn a(o: Option<i32>) {\n    let Some(v) = o;\n    let _ = v;\n}\n",
        )]);
        let undeclared = "diff --git src/lib.rs\n@@ -1,1 @@ rule=diet_some\n# sweep\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n";
        let declared = "diff --git src/lib.rs\n@@ -1,1 @@ rule=some_unwrap\n# sweep\n# pat P\n# expr E\n- let Some(P) = E;\n+ let Some(P) = E.unwrap_or(0);\n";
        let inferred = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- let Some(P0) = E0;\n+ let Some(P0) = E0.unwrap_or(0);\n";
        let a = canonicalize_with_tree(undeclared, &p).unwrap();
        let b = canonicalize_with_tree(declared, &p).unwrap();
        let c = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "undeclared:\n{a}\ndeclared:\n{b}");
        assert_eq!(a, c, "undeclared:\n{a}\ninferred:\n{c}");
        // WI-1s1: the single-site agreement instantiates to the
        // literal pair (D17) — the diet spelling agreement above still
        // holds, now via the shared instantiation.
        assert!(
            a.contains("- let Some(v) = o;\n+ let Some(v) = o.unwrap_or(0);\n"),
            "literal form:\n{a}"
        );
    }

    #[test]
    fn let_diet_undeclared_synthesis_vetoes() {
        // Three veto pins, one test: multi-letter undeclared names need
        // kind inference the comparison must not guess; an already-diet
        // inferrer leg stays a no-op (no bare single letter under the
        // ident-boundary matcher); a collision between the synthesized
        // target and an unrelated bare token keeps the hunk pinned.
        let p = pre(&[("src/lib.rs", LET_PRE)]);
        let multi = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- let Some(Val) = X;\n+ let Some(Val) = X.unwrap_or(0);\n";
        let got = canonicalize_with_tree(multi, &p).unwrap();
        assert!(
            got.contains("- let Some(Val) = X;"),
            "multi-letter untouched:\n{got}"
        );
        let inferred = inferred_let_patch();
        let diet = canonicalize_with_tree(&inferred, &p).unwrap();
        assert!(
            diet.contains("- let P0: T0 = E0;"),
            "inferrer leg already diet:\n{diet}"
        );
        let collision = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- let Some(P) = E;\n+ let Some(P) = P0;\n";
        let pinned = canonicalize_with_tree(collision, &p).unwrap();
        assert!(
            pinned.contains("- let Some(P) = E;"),
            "P→P0 target collides with bare P0 on the plus side, pinned:\n{pinned}"
        );
    }

    #[test]
    fn let_diet_multi_pair_untouched() {
        // WI-5c: K = 2–4 `let` sequences share the diet gate — the
        // declared names rename positionally across the whole hunk.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# pat P\n# expr E\n- let P = E;\n- let P = E;\n+ let P: u32 = E;\n+ let P: u32 = E;\n";
        let p = pre(&[("src/lib.rs", LET_PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            !got.contains("# pat P"),
            "diet declarations dropped:\n{got}"
        );
        assert!(
            got.contains("- let P0 = E0;\n- let P0 = E0;\n"),
            "positional diet:\n{got}"
        );
        // Five `let` lines exceed the K ≤ 4 sequence shape — out of
        // scope, the hunk survives byte-identical.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# pat P\n# expr E\n- let P = E;\n- let P = E;\n- let P = E;\n- let P = E;\n- let P = E;\n+ let P: u32 = E;\n+ let P: u32 = E;\n+ let P: u32 = E;\n+ let P: u32 = E;\n+ let P: u32 = E;\n";
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# pat P"), "five-pair untouched:\n{got}");
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

    #[test]
    fn symbol_turbofish_type_diet_unifies() {
        // WI-1b-turbofish: authored `Old::<U>(E)` + `# type U` diet-renames
        // to the inferred `::<T>(E0)` diet (first hole → bare `T`).
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n# type U\n- Old::<U>(E)\n+ New::<U>(E)\n";
        let inferred = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- crate::old_api::<T>(E0)\n+ crate::new_api::<T>(E0)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::old_api::<u8>(1);\n    crate::old_api::<i32>(2);\n}\n",
        )]);
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- crate::old_api::<T>(E0)\n+ crate::new_api::<T>(E0)\n"),
            "got:\n{a}"
        );
        assert!(!a.contains("# symbol"), "symbols dropped:\n{a}");
        assert!(!a.contains("# type"), "type decl dropped:\n{a}");
        assert!(!a.contains("# expr"), "expr decl dropped:\n{a}");
    }

    #[test]
    fn symbol_turbofish_type_identity_drops_decl() {
        // Already-`T` is a no-op rename that still drops the `# type`
        // declaration, unifying with the decl-free inferred diet.
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n# type T\n- Old::<T>(E)\n+ New::<T>(E)\n";
        let inferred = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- crate::old_api::<T>(E0)\n+ crate::new_api::<T>(E0)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::old_api::<u8>(1);\n    crate::old_api::<i32>(2);\n}\n",
        )]);
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(!a.contains("# type"), "decl dropped:\n{a}");
    }

    #[test]
    fn symbol_turbofish_type_collision_vetoes() {
        // A concrete `T` already in the pattern collides with the
        // `U` → `T` target — the hunk survives byte-identical.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n# type U\n- Old::<U>(T)\n+ New::<U>(T)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::old_api::<u8>(T);\n    crate::old_api::<i32>(T);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# type U"), "collision vetoes:\n{got}");
        assert!(got.contains("- Old::<U>(T)"), "body untouched:\n{got}");
    }

    #[test]
    fn symbol_expr_type_same_name_vetoes() {
        // SPEC 9.4: one name under two kinds never compiles — the error
        // shape survives comparison, never silently heals.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr T\n# type T\n- Old::<T>(T)\n+ New::<T>(T)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    crate::old_api::<u8>(x);\n    crate::old_api::<i32>(y);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# symbol Old=crate::old_api"), "got:\n{got}");
        assert!(got.contains("# type T"), "type decl stays:\n{got}");
    }

    const M2F_PRE: &str = "fn run(client: &Client) {\n    client.old_method(x);\n}\n";

    fn authored_m2f_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,3 @@ rule=m2f\n# lang=rust\n# allow=many\n# expr Receiver\n# expr Arg\n- Receiver.old_method(Arg)\n+ new_function(Receiver, Arg)\n"
            .to_string()
    }

    fn inferred_m2f_patch() -> String {
        "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- Receiver.old_method(E0)\n+ new_function(Receiver, E0)\n"
            .to_string()
    }

    #[test]
    fn wi8b_method_hole_diet_unifies() {
        // WI-8b N1+L1 (fixtures 11/78): declared `Receiver`/`Arg` vs
        // inferred `Receiver`/`E0` over the same single site — WI-1s1
        // instantiates the agreement to the literal pair (D17). The
        // diet agreement still holds on legs the gate declines (the
        // multi-site twins and the verification-reverted shapes).
        let p = pre(&[("src/lib.rs", M2F_PRE)]);
        let a = canonicalize_with_tree(&authored_m2f_patch(), &p).unwrap();
        let b = canonicalize_with_tree(&inferred_m2f_patch(), &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- client.old_method(x)\n+ new_function(client, x)\n"),
            "literal form:\n{a}"
        );
        assert!(!a.contains("# expr"), "decls dropped:\n{a}");
        assert!(!a.contains("# lang"), "default lang dropped:\n{a}");
    }

    #[test]
    fn wi8b_general_expr_hole_diet_unifies() {
        // WI-8b N1+L1 (fixture 79): declared `E` vs inferred `E0` —
        // WI-1s1 instantiates the agreement to the literal pair (D17).
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# scope=file\n# lang=rust\n# expr E\n- E + 1\n+ E + 2\n";
        let inferred = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- E0 + 1\n+ E0 + 2\n";
        let p = pre(&[("src/lib.rs", "fn m() {\n    tasty + 1;\n}\n")]);
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- tasty + 1\n+ tasty + 2\n"),
            "literal form:\n{a}"
        );
    }

    #[test]
    fn wi8b_expr_collision_vetoes() {
        // A declared `E` beside a *different* bare `E0` hole collides
        // (merging would join two holes): the hunk survives, decls and
        // all (lang still drops — L1 is independent of N1).
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# lang=rust\n# expr E\n- foo(E, E0)\n+ bar(E, E0)\n";
        let p = pre(&[(
            "src/lib.rs",
            "fn m() {\n    foo(a, b);\n    foo(c, d);\n}\n",
        )]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# expr E"), "decl stays:\n{got}");
        assert!(got.contains("- foo(E, E0)"), "body untouched:\n{got}");
        assert!(!got.contains("# lang"), "lang still drops:\n{got}");
    }

    #[test]
    fn wi8b_lang_drop_is_tree_only() {
        // Two-comparator contract: pure canonicalize preserves the
        // authored `# lang=rust` line; only the tree-aware form drops
        // the default.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# lang=rust\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        assert!(
            canonicalize(text).unwrap().contains("# lang=rust"),
            "pure keeps spelling"
        );
        let p = pre(&[("src/lib.rs", PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(!got.contains("# lang"), "tree drops default:\n{got}");
    }

    #[test]
    fn wi8b_lang_kept_beside_mode() {
        // Explicit `Mode` + `# lang=rust`: unknown interaction, the
        // lang line survives comparison.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# lang=rust\n# mode=tokens\n- foo(E0)\n+ bar(E0)\n";
        let p = pre(&[("src/lib.rs", PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# lang=rust"), "mode pins lang:\n{got}");
    }

    const FN_PRE: &str =
        "/// Adds one.\n#[inline]\nfn foo(x: i32) -> i32 {\n    x + 1\n}\n\nfn main() {}\n";

    fn terse_framed_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,5 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n     ...\n }\n"
            .to_string()
    }

    fn explicit_framed_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,5 @@ rule=r2\n# allow=many\n- V Q fn foo G (...) W {\n+ V Q fn bar G (...) W {\n     ...\n }\n"
            .to_string()
    }

    fn bare_patch() -> String {
        "diff --git src/lib.rs\n@@ -1,5 @@ rule=r1\n# allow=many\n- fn foo\n+ fn bar\n".to_string()
    }

    #[test]
    fn wi8b_framed_bare_three_way_unifies() {
        // WI-8b N3 (fixtures 01/51): terse-framed, explicit-framed,
        // and bare single-site pure renames share one tree-aware form.
        let p = pre(&[("src/lib.rs", FN_PRE)]);
        let a = canonicalize_with_tree(&terse_framed_patch(), &p).unwrap();
        let b = canonicalize_with_tree(&explicit_framed_patch(), &p).unwrap();
        let c = canonicalize_with_tree(&bare_patch(), &p).unwrap();
        assert_eq!(a, b, "terse:\n{a}\nexplicit:\n{b}");
        assert_eq!(b, c, "explicit:\n{b}\nbare:\n{c}");
        assert!(a.contains("- fn foo\n+ fn bar\n"), "bare diet:\n{a}");
        assert!(!a.contains("..."), "frame lines gone:\n{a}");
    }

    #[test]
    fn wi8b_framed_evolution_untouched() {
        // Minus/plus differ outside the name (signature evolution):
        // not a pure rename, the frame survives comparison.
        let text = "diff --git src/lib.rs\n@@ -1,5 @@ rule=s\n# lang=rust\n- fn foo(x: i32) {\n+ fn foo(x: u32) {\n     ...\n }\n";
        let p = pre(&[("src/lib.rs", "fn foo(x: i32) {\n    x\n}\n")]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(
            got.contains("- fn foo(x: i32) {\n"),
            "evolution keeps frame:\n{got}"
        );
    }

    #[test]
    fn wi8b_framed_multi_site_contracts_to_shared_bare() {
        // N3 is cardinality-blind by construction (pre-apply shape
        // gate): a two-site framed pure rename contracts on both legs
        // to the same bare sweep — sound because the mapping is
        // deterministic per leg and both legs prove identical spans
        // from the shared diet form.
        let terse = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- fn foo(...) {\n+ fn bar(...) {\n     ...\n }\n";
        let explicit = "diff --git src/lib.rs\n@@ -1,6 @@ rule=s\n# allow=many\n- V Q fn foo G (...) W {\n+ V Q fn bar G (...) W {\n     ...\n }\n";
        let p = pre(&[("src/lib.rs", "fn foo() {\n    1\n}\nfn foo() {\n    2\n}\n")]);
        let a = canonicalize_with_tree(terse, &p).unwrap();
        let b = canonicalize_with_tree(explicit, &p).unwrap();
        assert_eq!(a, b, "terse:\n{a}\nexplicit:\n{b}");
        assert!(a.contains("- fn foo\n+ fn bar\n"), "bare diet:\n{a}");
        assert!(a.contains("# allow=many"), "multi-site stays many:\n{a}");
    }

    #[test]
    fn wi8b_fn_header_with_expr_decl_untouched() {
        // N1 never touches `fn`-header lines (item name holes are not
        // expression holes): the decl survives even single-site.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# expr E\n- fn foo(E)\n+ fn bar(E)\n";
        let p = pre(&[("src/lib.rs", FN_PRE)]);
        let got = canonicalize_with_tree(text, &p).unwrap();
        assert!(got.contains("# expr E"), "fn-header decl stays:\n{got}");
    }

    #[test]
    fn wi1s0_instantiation_premise_holds() {
        // WI-1s0: authored templated spelling vs inferred literal/holed
        // diet over the same pre-image rebuild byte-identical post-trees.
        // This pins the D17 premise; WI-1s1 wires it into equality.
        // 95-style: verbatim concrete arg vs holed diet.
        let pre95 = "fn make_text(s: &str) -> String {\n    s.to_string()\n}\nfn make_text_new(s: &str) -> String {\n    s.to_string()\n}\nfn m(sub: &str) -> i32 {\n    let x = make_text(sub);\n    x.len() as i32\n}\n";
        let authored95 = "diff --git src/lib.rs\n@@ -1,1 @@ rule=verbatim\n# sweep\n- make_text(sub)\n+ make_text_new(sub)\n";
        let inferred95 =
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n# sweep\n- make_text(E0)\n+ make_text_new(E0)\n";
        let p95 = pre(&[("src/lib.rs", pre95)]);
        let a95 = single_site_post_key(&p95, authored95).expect("authored applies");
        let b95 = single_site_post_key(&p95, inferred95).expect("inferred applies");
        assert_eq!(a95, b95, "95-style pair shares one post-tree");
        // 83-style: infer-wildcard sweep vs literal single.
        let pre83 = "fn f() {\n    let v = 1 + 2;\n    let _ = v;\n}\n";
        let authored83 = "diff --git src/lib.rs\n@@ -1,1 @@ rule=infer_wild\n# sweep\n- let v = _;\n+ let v = 0;\n";
        let inferred83 =
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n# sweep\n- let v = 1 + 2;\n+ let v = 0;\n";
        let p83 = pre(&[("src/lib.rs", pre83)]);
        let a83 = single_site_post_key(&p83, authored83).expect("authored applies");
        let b83 = single_site_post_key(&p83, inferred83).expect("inferred applies");
        assert_eq!(a83, b83, "83-style pair shares one post-tree");
        // Negative control: same pre, different post discriminates.
        let other83 =
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n# sweep\n- let v = 1 + 2;\n+ let v = 9;\n";
        let c83 = single_site_post_key(&p83, other83).expect("control applies");
        assert_ne!(b83, c83, "different post-trees key differently");
        // Unapplicable patch yields None, never a panic.
        assert!(single_site_post_key(
            &p83,
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=nope\n# sweep\n- missing_fn(E0)\n+ missing_fn(E1)\n"
        )
        .is_none());
    }

    #[test]
    fn wi1s1_verbatim_holed_let_unifies() {
        // WI-1s1 (63-style): holed `let` template vs literal single over
        // one line share one tree-aware form (the instantiated literal).
        let p = pre(&[("src/lib.rs", "fn f() {\n    let buf = [7u8; 8];\n}\n")]);
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=repeat_hole\n# scope=file\n# expr E\n- let buf = [E; 8];\n+ let buf = [E; 16];\n";
        let inferred = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- let buf = [7u8; 8];\n+ let buf = [7u8; 16];\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- let buf = [7u8; 8];\n+ let buf = [7u8; 16];\n"),
            "literal diet:\n{a}"
        );
        assert!(!a.contains("# expr"), "decl dropped:\n{a}");
        assert!(!a.contains("# scope"), "locality dropped:\n{a}");
    }

    #[test]
    fn wi1s1_multi_subsite_line_unifies_at_line_granularity() {
        // WI-1s1 (47-style): one template matching two call subsites in
        // one line vs the literal whole-line single — one distinct line
        // span is one site (D17), so both spellings share one form.
        let p = pre(&[(
            "src/lib.rs",
            "fn use_it(x: i32) -> i32 {\n    x\n}\nfn f(a: i32, b: i32) -> i32 {\n    use_it(a) + use_it(b)\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -4,2 @@ rule=wrap_clone\n# allow=many\n- use_it(E0)\n+ use_it(E0.clone())\n";
        let inferred = "diff --git src/lib.rs\n@@ -5,1 @@ rule=r1\n- use_it(a) + use_it(b)\n+ use_it(a.clone()) + use_it(b.clone())\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("@@ -5,1 @@ rule=r1\n- use_it(a) + use_it(b)\n"),
            "diet single:\n{a}"
        );
        assert!(!a.contains("# allow"), "locality dropped:\n{a}");
    }

    #[test]
    fn wi1s1_undeclared_pat_and_control_unify() {
        // WI-1s1 (49-style): declaration-free holed struct-pat sweep vs
        // the literal single (bare holes infer per SPEC 15.8).
        let p = pre(&[(
            "src/lib.rs",
            "struct S {\n    f: i32,\n    g: i32,\n}\nfn f(e: S) -> i32 {\n    let S { f: a, g: b } = e;\n    a + b\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -6,1 @@ rule=swap_fields\n# sweep\n- let S { f: P, g: Q } = E;\n+ let S { f: Q, g: P } = E;\n";
        let inferred = "diff --git src/lib.rs\n@@ -6,1 @@ rule=r1\n- let S { f: a, g: b } = e;\n+ let S { f: b, g: a } = e;\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        // WI-1s1 (65-style): control-ish `let`-with-`match` template vs
        // the literal single.
        let p = pre(&[(
            "src/lib.rs",
            "fn f(o: Option<i32>) -> i32 {\n    let v = match o { Some(v) => 1, _ => 2 };\n    v\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -2,1 @@ rule=arm_bump\n# lang=rust\n# pat P\n# expr E\n- match E { Some(P) => 1, _ => 2 }\n+ match E { Some(P) => 9, _ => 2 }\n";
        let inferred = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- let v = match o { Some(v) => 1, _ => 2 };\n+ let v = match o { Some(v) => 9, _ => 2 };\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
    }

    #[test]
    fn wi1s1_wildcard_unifies() {
        // WI-1s1 (83-style): infer-wildcard sweep vs literal single.
        let p = pre(&[(
            "src/lib.rs",
            "fn f() {\n    let v = 1 + 2;\n    let _ = v;\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=infer_wild\n# scope=file\n# allow=many\n- let v = _;\n+ let v = 0;\n";
        let inferred =
            "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- let v = 1 + 2;\n+ let v = 0;\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- let v = 1 + 2;\n+ let v = 0;\n"),
            "literal:\n{a}"
        );
    }

    #[test]
    fn wi1s1_overwide_template_stays_divergent() {
        // WI-1s1 negative control: the template proves two line sites
        // while the literal proves one — different site sets never
        // equate, even though each leg applies cleanly on its own.
        let p = pre(&[("src/lib.rs", "fn m() {\n    foo(1);\n    foo(2);\n}\n")]);
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let inferred = "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- foo(1)\n+ bar(1)\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_ne!(
            a, b,
            "over-wide template must stay divergent:\n{a}\nvs\n{b}"
        );
        assert!(a.contains("# allow=many"), "template stays multi:\n{a}");
    }

    #[test]
    fn wi1s2_uppercase_head_stays_conservative_divergent() {
        // WI-1s2: the uppercase-head conservative-divergence pin. The
        // raiser backticks uppercase callees (probed: a single
        // `Old(1)` → `New(1)` site re-diffs as `` `Old`(E0) `` /
        // `` `New`(E0) ``). An authored backticked literal single over
        // the same site applies and rebuilds the identical post-tree
        // (tree legs close), yet the legs stay canonically distinct:
        // the shared plain-literal representative (`Old(1)` /
        // `New(1)`) does not apply — bare uppercase heads infer as
        // holes, so the plus side fails `PatternConflict` (unbound
        // `New`) — and each leg reverts to its snapshot. Divergence
        // by construction, never healed into equality.
        let p = pre(&[(
            "src/lib.rs",
            "fn Old(x: i32) -> i32 { x }\nfn New(x: i32) -> i32 { x + 1 }\nfn m() -> i32 {\n    Old(1)\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -4,1 @@ rule=r1\n- `Old`(1)\n+ `New`(1)\n";
        let inferred = "diff --git src/lib.rs\n@@ -4,1 @@ rule=r1\n- `Old`(E0)\n+ `New`(E0)\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_ne!(
            a, b,
            "uppercase-head spellings stay divergent:\n{a}\nvs\n{b}"
        );
        // Tree legs close: one site, one post-tree on both legs.
        let ka = single_site_post_key(&p, authored).expect("authored applies");
        let kb = single_site_post_key(&p, inferred).expect("inferred applies");
        assert_eq!(ka, kb, "same single site, same post-tree");
        // The plain representative is no pattern at all here.
        assert!(
            single_site_post_key(
                &p,
                "diff --git src/lib.rs\n@@ -4,1 @@ rule=r1\n- Old(1)\n+ New(1)\n"
            )
            .is_none(),
            "bare uppercase heads infer as holes — plain literal never applies"
        );
    }

    #[test]
    fn wi1s1_symbol_single_instantiates() {
        // WI-1s1 after WI-1b: the symbol spelling normalizes to the
        // concrete diet, then the single site instantiates to the
        // literal pair both legs share (D17). Symbol hunks with
        // `# follow=uses` (kept fallback) or error shapes never enter
        // — the gate only sees the locality + hole directives WI-1b
        // leaves behind.
        let p = pre(&[("src/lib.rs", "fn m() {\n    crate::old_api(1);\n}\n")]);
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# fallback=syntax\n# symbol Old=crate::old_api\n# symbol New=crate::new_api\n# expr E\n- Old(E)\n+ New(E)\n";
        let inferred =
            "diff --git src/lib.rs\n@@ -2,1 @@ rule=r1\n- crate::old_api(1)\n+ crate::new_api(1)\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- crate::old_api(1)\n+ crate::new_api(1)\n"),
            "literal form:\n{a}"
        );
        assert!(!a.contains("# symbol"), "symbols dropped:\n{a}");
    }

    #[test]
    fn wi2_bare_type_single_instantiates_to_concrete_delta() {
        // WI-2 slice 1: the bare-type equality arm. The authored holed
        // sweep (`Vec<T>` → `Box<[T]>`) and the inferred literal single
        // (`Vec<i32>` → `Box<[i32]>`) prove the same one line site; both
        // canonicalize to the concrete type delta — never to the whole
        // header lines (a lone `{`-terminated header is not an
        // applicable pattern: `unbalanced body block`, probed).
        let p = pre(&[(
            "src/lib.rs",
            "fn sum(values: Vec<i32>) -> i32 {\n    values.iter().sum()\n}\n",
        )]);
        let authored = "diff --git src/lib.rs\n@@ -1,1 @@ rule=box_param\n# lang=rust\n# type T\n- Vec<T>\n+ Box<[T]>\n";
        let inferred = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n- Vec<i32>\n+ Box<[i32]>\n";
        let a = canonicalize_with_tree(authored, &p).unwrap();
        let b = canonicalize_with_tree(inferred, &p).unwrap();
        assert_eq!(a, b, "authored:\n{a}\ninferred:\n{b}");
        assert!(
            a.contains("- Vec<i32>\n+ Box<[i32]>\n"),
            "concrete type delta:\n{a}"
        );
        assert!(!a.contains("# type"), "hole declarations dropped:\n{a}");
        // Tree legs close: one site, one post-tree on both legs.
        let ka = single_site_post_key(&p, authored).expect("authored applies");
        let kb = single_site_post_key(&p, inferred).expect("inferred applies");
        assert_eq!(ka, kb, "same single site, same post-tree");
    }

    #[test]
    fn wi2_bare_type_multi_delta_stays_divergent() {
        // Negative control: a single line carrying two type deltas has
        // no one-`Type` middle, so the arm declines and the legs stay
        // canonically distinct (never healed into equality).
        let p = pre(&[(
            "src/lib.rs",
            "fn m(a: Vec<u32>, b: Vec<String>) -> i32 {\n    0\n}\n",
        )]);
        let holed = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- Vec<T0>\n+ Box<[T0]>\n";
        let literal = "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n- Vec<u32>\n+ Box<[u32]>\n";
        let a = canonicalize_with_tree(holed, &p).unwrap();
        let b = canonicalize_with_tree(literal, &p).unwrap();
        assert_ne!(a, b, "multi-delta lines never unify:\n{a}\nvs\n{b}");
    }
}
