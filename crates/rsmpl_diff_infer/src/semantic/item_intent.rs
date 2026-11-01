//! WI-15a item-intent raising (SPEC D.4 `item_intent`).
//!
//! One definition pair (`struct`/`enum`/`trait`/`type`/`mod`/`const`/
//! `static`/terse-`fn`, via [`super::split_item`]) joined with N-1
//! same-delta use lines (`impl Old`, `s: Old`, `-> Old`, `Old { … }`,
//! …) across one or more files compresses to the single definition
//! hunk *with* `# follow=uses` + `# sweep`. The engine follow rewrites
//! the rest at apply time (workspace-wide, WI-7a), so the emission is
//! one hunk.
//!
//! WI-9 `fn` arm: a plain top-level `fn old` pure-rename definition
//! (empty vis prefix, equal suffixes — any signature incl. `->`,
//! generics, `where`) joined with N-1 *call-callee* use lines of
//! `old` (bare, qualified, turbofish; `let`-wrapped included) emits
//! the same single hunk in the terse `- fn old` / `+ fn new` diet
//! form with `# sweep` + `# follow=uses`. Method calls, fields,
//! bare value paths, macro heads, and `use` lines veto the whole
//! intent (never a partial follow claim); the scratch-apply probe
//! below is the uniqueness guard (an off-diff same-name call fails
//! the unchanged-leg).
//!
//! Totality is a cross-file scratch-apply probe (member-rename
//! posture, WI-9b): the authored emission applied to an in-memory
//! clone of the pre-image must reproduce exactly the native plus lines
//! at every claimed changed site with every other line byte-identical
//! (a missed site fails the plus-leg, an overwritten twin fails the
//! unchanged-leg). `None` pre-image declines; twin definitions decline;
//! call-callee and macro-head uses decline to their dedicated detectors
//! (WI-7 grouped shapes keep their pins); `macro_rules!` and `mod` defs
//! decline to theirs.

use std::collections::{HashMap, HashSet};

use super::{split_item, try_item_rename, unique_rule};

/// One parsed eligible section: its index, b-side path, balanced
/// minus/plus lines, and old-side changed lines (aligned by position).
struct SectionPairs {
    idx: usize,
    path: String,
    minuses: Vec<String>,
    pluses: Vec<String>,
    changed: Vec<usize>,
}

/// One native pair at its old-side site.
struct Pair {
    sec: usize,
    path: String,
    minus: String,
    plus: String,
    site: usize,
}

/// Whole-patch item-intent pre-pass: returns the rebuilt section list
/// (claimed `.rs` sections replaced by the single semantic section at
/// the definition section's slot) plus the definition path and rule
/// for provenance, or `None` when the patch holds no provable intent.
/// Reserved rule names are released on decline.
pub(crate) fn try_item_intent_sweep(
    sections: &[String],
    pre: &HashMap<String, String>,
    used_rules: &mut Vec<String>,
) -> Option<(Vec<String>, String, String)> {
    let checkpoint = used_rules.len();
    // Eligible: `.rs` non-rename sections (non-`.rs` passes through;
    // rename sections own the file move and stay out of the intent).
    let mut eligible: Vec<usize> = Vec::new();
    // (section idx, path, minuses, pluses, changed)
    let mut parsed: Vec<SectionPairs> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        let header = sec.lines().next().unwrap_or("");
        if !header.starts_with("diff --git ") || sec.contains('\r') {
            continue;
        }
        if super::is_rename_header(header) {
            continue;
        }
        let rest = header.strip_prefix("diff --git ")?;
        let path = crate::b_side_path(rest)?;
        if !path.ends_with(".rs") {
            continue;
        }
        eligible.push(i);
        let (minuses, pluses, balanced) = section_pairs(sec);
        if !balanced || minuses.is_empty() {
            used_rules.truncate(checkpoint);
            return None;
        }
        let changed = crate::totality::changed_old_lines(sec);
        if changed.len() != minuses.len() {
            used_rules.truncate(checkpoint);
            return None;
        }
        parsed.push(SectionPairs {
            idx: i,
            path,
            minuses,
            pluses,
            changed,
        });
    }
    if parsed.is_empty() {
        return None;
    }
    // Flatten to one pair per changed line, in section order.
    let mut pairs: Vec<Pair> = Vec::new();
    for sp in &parsed {
        for ((m, p), s) in sp
            .minuses
            .iter()
            .zip(sp.pluses.iter())
            .zip(sp.changed.iter())
        {
            pairs.push(Pair {
                sec: sp.idx,
                path: sp.path.clone(),
                minus: m.clone(),
                plus: p.clone(),
                site: *s,
            });
        }
    }
    if pairs.len() < 2 {
        return None;
    }
    // Exactly one definition pair (a `split_item` delta): every other
    // pair is a use line sharing the delta.
    let mut def_idx: Option<usize> = None;
    let mut def_kind = String::new();
    let mut old_ident = String::new();
    let mut new_ident = String::new();
    let mut def_prefix = String::new();
    for (i, pair) in pairs.iter().enumerate() {
        if let Some((prefix, kind, old, new)) = split_delta(&pair.minus, &pair.plus) {
            if def_idx.is_some() {
                used_rules.truncate(checkpoint);
                return None;
            }
            def_idx = Some(i);
            def_kind = kind;
            old_ident = old;
            new_ident = new;
            def_prefix = prefix;
        }
    }
    let def_idx = def_idx?;
    if old_ident == new_ident || old_ident.is_empty() || new_ident.is_empty() {
        return None;
    }
    // WI-15a slice 1: `macro_rules!` intents belong to the dedicated
    // `macro_rules_rename` detector (per-arg token-tree holes); `mod`
    // renames join their file move in WI-15c. Both decline here.
    if def_kind == "macro_rules!" || def_kind == "mod" {
        return None;
    }
    // WI-9: `fn` intents claim call-callee uses (the follow domain);
    // every other shape keeps the item gate (any same-delta use line).
    let is_fn_intent = def_kind == "fn";
    // Use-line gate per non-def pair (cheap, ident-boundary): the probe
    // below is authoritative.
    for (i, pair) in pairs.iter().enumerate() {
        if i == def_idx {
            continue;
        }
        if pair.minus.trim() == pair.plus.trim() {
            return None;
        }
        if !contains_ident(&pair.minus, &old_ident) || !contains_ident(&pair.plus, &new_ident) {
            return None;
        }
        if is_fn_intent && !is_fn_call_use_site(&pair.minus, &old_ident) {
            return None;
        }
    }
    let def = &pairs[def_idx];
    // Pre-image for every involved file (precision-first).
    for pair in &pairs {
        if !pre.contains_key(&pair.path) {
            return None;
        }
    }
    // Twin-definition veto (unique-definition rule, SPEC 14.8): exactly
    // one `split_item` definition of `old` with the def kind across
    // the involved pre-images.
    if count_kind_defs(pre, &pairs, &def_kind, &old_ident) != 1 {
        return None;
    }
    // Dedicated-position carve (item shapes only — `fn` intents claim
    // exactly the call-callee position via the gate above): a use line
    // whose changed subterm sits in a call-callee or macro-head naming
    // `old` belongs to the call / macro detectors (`call_sweep`,
    // `macro_rules_rename`, …) and their pinned shapes — the intent
    // never claims it. Impl headers, fn headers, type positions,
    // struct literals/patterns, and `use` lines stay claimable; the
    // probe below is authoritative for them.
    if !is_fn_intent {
        for (i, pair) in pairs.iter().enumerate() {
            if i == def_idx {
                continue;
            }
            if is_dedicated_use_site(&pair.minus, &old_ident) {
                return None;
            }
        }
    }
    // The def pair must be emittable as a lone item rename (terse-`fn`
    // gates, column-0, suffix laws); otherwise the follow emission
    // would carry a shape the engine rejects. WI-9 `fn` arm: plain
    // top-level pure renames (empty prefix, equal suffixes — any
    // signature) skip the `try_item_rename` suffix laws (`->` fns
    // carry them; the scratch probe below subsumes the check since
    // the bare terse emission always parses).
    if is_fn_intent {
        if !def_prefix.is_empty() {
            return None;
        }
    } else {
        let mut probe_used = vec![format!("probe{}", used_rules.len())];
        let def_pre = pre.get(&def.path).map(String::as_str);
        try_item_rename(
            &def.minus,
            &def.plus,
            &def.path,
            def_pre,
            std::slice::from_ref(&def.site),
            &mut probe_used,
        )?;
    }
    // Build the emission: sweep dummy + follow + def pair.
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let emission = emit(
        &def.path,
        &rule,
        &def_kind,
        &def_prefix,
        &old_ident,
        &new_ident,
    );
    // Scratch-apply probe: every claimed site must show its native plus
    // line, every other line byte-identical (per file).
    if !probe_reproduces(pre, &emission, &pairs) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // T375: the one hunk must beat the replaced native bytes.
    let claimed_bytes: usize = parsed.iter().map(|sp| sections[sp.idx].len()).sum();
    if emission.len() >= claimed_bytes {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Rebuild: claimed sections collapse to the emission at the def
    // section's slot; anything unclaimed passes through byte-identical.
    let claimed: HashSet<usize> = eligible.into_iter().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, sec) in sections.iter().enumerate() {
        if !claimed.contains(&i) {
            out.push(sec.clone());
            continue;
        }
        if i == def.sec {
            out.push(emission.clone());
        }
    }
    Some((out, def.path.clone(), rule))
}

/// One section's minus/plus lines plus the balanced gate (same walk as
/// `chain.rs`: every hunk balanced, global counts equal, non-empty).
fn section_pairs(section: &str) -> (Vec<String>, Vec<String>, bool) {
    let mut minuses: Vec<String> = Vec::new();
    let mut pluses: Vec<String> = Vec::new();
    let mut hunk_minus = 0usize;
    let mut hunk_plus = 0usize;
    let mut balanced = true;
    let mut in_hunk = false;
    let mut saw_hunk = false;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if in_hunk && hunk_minus != hunk_plus {
                balanced = false;
            }
            in_hunk = true;
            saw_hunk = true;
            hunk_minus = 0;
            hunk_plus = 0;
            continue;
        }
        if !in_hunk {
            if line.starts_with("old mode ") || line.starts_with("new mode ") {
                balanced = false;
            }
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(line[1..].to_string());
            hunk_minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(line[1..].to_string());
            hunk_plus += 1;
        }
    }
    if in_hunk && hunk_minus != hunk_plus {
        balanced = false;
    }
    let ok = balanced && saw_hunk && minuses.len() == pluses.len() && !minuses.is_empty();
    (minuses, pluses, ok)
}

/// A `split_item` definition delta, or `None` for use lines.
fn split_delta(minus: &str, plus: &str) -> Option<(String, String, String, String)> {
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(plus.trim_end())?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    Some((old_prefix, old_kind, old_ident, new_ident))
}

/// Ident-boundary containment (never substring).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                return true;
            }
            token.clear();
        }
    }
    false
}

/// True when the use line's changed subterm sits in a dedicated
/// detector's position: a call callee or macro head naming `old`
/// (`Old(1)`, `m::Old::<T>(x)`, `old!(…)` incl. `let`-wrapped). Those
/// classes keep their pinned shapes; everything else (impl/fn headers,
/// type positions, struct literals/patterns, `use` lines, bare value
/// paths) stays claimable for the probe.
fn is_dedicated_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_dedicated_expr(&expr, old) {
            return true;
        }
    }
    // `let`-wrapped sites (`let b = old!(…);`) parse as statements.
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_dedicated_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

/// WI-9: true when the use line's changed subterm is a *call callee*
/// naming `old` — the plain-`fn` follow domain (bare, qualified,
/// turbofish; `let`-wrapped included). Method calls, fields, bare
/// value paths, macro heads, and `use` lines all return false and
/// veto the `fn` intent. Arguments never count — only the head.
fn is_fn_call_use_site(minus: &str, old: &str) -> bool {
    let mut text = minus.trim();
    if let Some(stripped) = super::strip_one_terminator(text) {
        text = stripped.trim_end();
    }
    if text.is_empty() {
        return false;
    }
    if let Ok(expr) = syn::parse_str::<syn::Expr>(text) {
        if is_fn_call_expr(&expr, old) {
            return true;
        }
    }
    if let Ok(syn::Stmt::Local(local)) = syn::parse_str::<syn::Stmt>(minus.trim()) {
        if let Some(init) = local.init {
            if is_fn_call_expr(&init.expr, old) {
                return true;
            }
        }
    }
    false
}

/// True for call callees naming `old` (qualified and turbofish
/// spellings included — the engine's scalar follow rewrites them;
/// method calls never count: different namespace).
fn is_fn_call_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// True for call callees and macro heads naming `old` (qualified and
/// turbofish spellings included; arguments never count — only the
/// head position).
fn is_dedicated_expr(expr: &syn::Expr, old: &str) -> bool {
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                if path.qself.is_none() && path.path.segments.last().is_some_and(|s| s.ident == old)
                {
                    return true;
                }
            }
            false
        }
        syn::Expr::Macro(m) => m.mac.path.segments.last().is_some_and(|s| s.ident == old),
        _ => false,
    }
}

/// Count `split_item` definitions of (`kind`, `ident`) across the
/// involved pre-images (one line per definition).
fn count_kind_defs(
    pre: &HashMap<String, String>,
    pairs: &[Pair],
    kind: &str,
    ident: &str,
) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut count = 0usize;
    for pair in pairs {
        if !seen.insert(pair.path.clone()) {
            continue;
        }
        if let Some(text) = pre.get(&pair.path) {
            for line in text.lines() {
                if let Some((_, k, id, _)) = split_item(line) {
                    if k == kind && id == ident {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Emit the single-hunk follow emission under `rule`.
fn emit(path: &str, rule: &str, kind: &str, prefix: &str, old: &str, new: &str) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    if kind == "fn" {
        out.push_str(&format!("- fn {old}\n+ fn {new}\n"));
    } else {
        out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    }
    out
}

/// Scratch-apply probe: the emission applied to a `pre` clone must show
/// each claimed plus line at its site with every other line identical.
fn probe_reproduces(pre: &HashMap<String, String>, emission: &str, pairs: &[Pair]) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut files = pre.clone();
    for section in &parsed.files {
        files.entry(section.target_path.clone()).or_default();
    }
    if rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).is_err() {
        return false;
    }
    // Per-file claim index: site -> plus trim.
    let mut claims: HashMap<&str, HashMap<usize, &str>> = HashMap::new();
    for pair in pairs {
        claims
            .entry(pair.path.as_str())
            .or_default()
            .insert(pair.site, pair.plus.trim());
    }
    for (path, pre_text) in pre {
        let Some(post_text) = files.get(path) else {
            return false;
        };
        let pre_lines: Vec<&str> = pre_text.lines().collect();
        let post_lines: Vec<&str> = post_text.lines().collect();
        if post_lines.len() != pre_lines.len() {
            return false;
        }
        let empty: HashMap<usize, &str> = HashMap::new();
        let claim = claims.get(path.as_str()).unwrap_or(&empty);
        for (idx, (pre_line, post_line)) in pre_lines.iter().zip(post_lines.iter()).enumerate() {
            let line_no = idx + 1;
            if let Some(want) = claim.get(&line_no) {
                if post_line.trim() != *want {
                    return false;
                }
            } else if pre_line != post_line {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::infer_semantic_with_pre_images;
    use super::*;
    use std::collections::HashMap;

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

    const A_PRE: &str = "struct Old {\n    x: i32,\n}\n";
    const B_PRE: &str = "fn f(s: Old) -> i32 {\n    0\n}\nimpl Old {\n    fn m(&self) {}\n}\n";

    fn sections_two_file() -> Vec<String> {
        vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-fn f(s: Old) -> i32 {\n+fn f(s: New) -> i32 {\n@@ -4,1 +4,1 @@\n-impl Old {\n+impl New {\n"
                .to_string(),
        ]
    }

    #[test]
    fn def_delta_extracts() {
        assert!(split_delta("struct Old {", "struct New {").is_some());
        assert!(split_delta("fn f(s: Old) -> i32 {", "fn f(s: New) -> i32 {").is_none());
        assert!(split_delta("impl Old {", "impl New {").is_none());
    }

    #[test]
    fn dedicated_positions_decline() {
        // Call callees and macro heads naming `old` belong to the
        // call/macro detectors — the intent never claims them.
        assert!(is_dedicated_use_site("    Old(1);", "Old"));
        assert!(is_dedicated_use_site("    m::Old(1);", "Old"));
        assert!(!is_dedicated_use_site("    Some(Old);", "Old"));
        assert!(is_dedicated_use_site("    old!(1);", "old"));
        assert!(is_dedicated_use_site("    let b = old!(2);", "old"));
        assert!(!is_dedicated_use_site("impl Old {", "Old"));
        assert!(!is_dedicated_use_site("fn f(s: Old) -> i32 {", "Old"));
        assert!(!is_dedicated_use_site("use m::Old;", "Old"));
        // A bare method ident is not a dedicated head (different
        // namespace); the probe declines it instead.
        assert!(!is_dedicated_use_site("    s.old();", "old"));
    }

    #[test]
    fn call_site_uses_decline_to_grouped_shape() {
        // Tuple-struct def + constructor call sites: the WI-7 grouped
        // shape (def + `call_sweep`, shared rule) keeps its pin — the
        // intent declines on the call-callee carve.
        let pre = "struct Old(i32);\n";
        let use_pre = "fn m() {\n    Old(1);\n    Old(2);\n}\n";
        let sections = vec![
            "diff --git a/z.rs b/z.rs\n--- a/z.rs\n+++ b/z.rs\n@@ -1,1 +1,1 @@\n-struct Old(i32);\n+struct New(i32);\n"
                .to_string(),
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2,2 +2,2 @@\n-    Old(1);\n-    Old(2);\n+    New(1);\n+    New(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[("z.rs", pre), ("a.rs", use_pre)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn method_use_fails_the_probe() {
        // `w.s.Old()` is a method ident (different namespace): the
        // follow leaves it, so the probe fails and the section stays
        // ordinary. (Capitals keep the ident gate green so the probe
        // is what declines.)
        let pre_a = "struct Old {\n    x: i32,\n}\n";
        let pre_b = "fn f(w: Wrap) -> i32 {\n    w.s.Old()\n}\n";
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,1 +2,1 @@\n-    w.s.Old()\n+    w.s.New()\n"
                .to_string(),
        ];
        let pre = pre_map(&[("a.rs", pre_a), ("b.rs", pre_b)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn twin_definitions_decline() {
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
        ];
        let pre = pre_map(&[("a.rs", A_PRE), ("b.rs", A_PRE)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn single_pair_declines() {
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n"
                .to_string(),
        ];
        let pre = pre_map(&[("a.rs", A_PRE)]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
    }

    #[test]
    fn two_file_intent_fires_and_round_trips() {
        let sections = sections_two_file();
        let pre = pre_map(&[("a.rs", A_PRE), ("b.rs", B_PRE)]);
        let mut used = Vec::new();
        let (out, path, rule) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("intent must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "one intent section, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("# sweep"), "got:\n{}", out[0]);
        assert!(out[0].contains("- struct Old"), "got:\n{}", out[0]);
        assert!(!rule.is_empty());
        validated_live(&out[0], &pre);
        // Full post trees equal the native post lines at claimed sites.
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["a.rs"].contains("struct New {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["b.rs"].contains("s: New"), "got: {}", files["b.rs"]);
        assert!(
            files["b.rs"].contains("impl New {"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["b.rs"].contains("Old"), "got: {}", files["b.rs"]);
    }

    #[test]
    fn full_pipeline_two_file_compresses() {
        let sections = sections_two_file();
        let pre = pre_map(&[("a.rs", A_PRE), ("b.rs", B_PRE)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        validated_live(&out, &pre);
    }

    // WI-9: plain-`fn` intents raise with follow.

    const FN_PRE: &str =
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n}\n";

    fn fn_sections_two_use() -> Vec<String> {
        vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1);\n+    let b = bar(2);\n"
                .to_string(),
        ]
    }

    fn assert_fn_follow_emission(out: &[String], pre: &HashMap<String, String>) {
        assert_eq!(out.len(), 1, "one intent section, got: {out:?}");
        assert!(out[0].contains("# follow=uses"), "got:\n{}", out[0]);
        assert!(out[0].contains("# sweep"), "got:\n{}", out[0]);
        assert!(out[0].contains("- fn foo"), "got:\n{}", out[0]);
        assert!(out[0].contains("+ fn bar"), "got:\n{}", out[0]);
        validated_live(&out[0], pre);
        let parsed = rsmpl_patch::parse_patch(&out[0]).unwrap();
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).unwrap();
        assert!(
            files["src/lib.rs"].contains("fn bar(x: i32) -> i32 {"),
            "got: {}",
            files["src/lib.rs"]
        );
        assert!(
            !files["src/lib.rs"].contains("foo"),
            "got: {}",
            files["src/lib.rs"]
        );
    }

    #[test]
    fn fn_call_head_gate() {
        // The `fn` follow domain is call callees only (bare,
        // qualified, `let`-wrapped); methods, fields, value paths,
        // macro heads, and `use` lines veto.
        assert!(is_fn_call_use_site("    foo(1);", "foo"));
        assert!(is_fn_call_use_site("    m::foo(2);", "foo"));
        assert!(is_fn_call_use_site("    let a = foo(1);", "foo"));
        assert!(!is_fn_call_use_site("    s.foo();", "foo"));
        assert!(!is_fn_call_use_site("    s.foo;", "foo"));
        assert!(!is_fn_call_use_site("    let x = foo;", "foo"));
        assert!(!is_fn_call_use_site("    foo!(2);", "foo"));
        assert!(!is_fn_call_use_site("    let b = foo!(2);", "foo"));
        assert!(!is_fn_call_use_site("use m::foo;", "foo"));
        assert!(!is_fn_call_use_site("    Some(foo);", "foo"));
    }

    #[test]
    fn fn_intent_fires_with_return_type() {
        // 46 shape: `->` return in the def suffix skips the
        // `try_item_rename` laws; the scratch probe proves the bare
        // terse emission instead.
        let sections = fn_sections_two_use();
        let pre = pre_map(&[("src/lib.rs", FN_PRE)]);
        let mut used = Vec::new();
        let (out, path, rule) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("fn intent must fire");
        assert_eq!(path, "src/lib.rs");
        assert!(!rule.is_empty());
        assert_fn_follow_emission(&out, &pre);
    }

    #[test]
    fn fn_intent_single_use_fires() {
        // 89 shape: def + one call collapses to the same hunk.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,1 +5,1 @@\n-    let a = foo(1);\n+    let a = bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let _ = a;\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("single-use must fire");
        assert_fn_follow_emission(&out, &pre);
    }

    #[test]
    fn fn_intent_qualified_call_fires() {
        // `m::foo` is in the scalar follow domain (round-4 WI-6a pin).
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = m::foo(2);\n+    let a = bar(1);\n+    let b = m::bar(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = m::foo(2);\n}\n",
        )]);
        let mut used = Vec::new();
        let (out, _, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("qualified must fire");
        assert_fn_follow_emission(&out, &pre);
    }

    #[test]
    fn fn_intent_cross_file_fires() {
        let sections = vec![
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,1 +2,1 @@\n-    foo(1);\n+    bar(1);\n"
                .to_string(),
        ];
        let pre = pre_map(&[
            ("a.rs", "fn foo(x: i32) -> i32 {\n    x + 1\n}\n"),
            ("b.rs", "fn m() {\n    foo(1);\n}\n"),
        ]);
        let mut used = Vec::new();
        let (out, path, _) =
            try_item_intent_sweep(&sections, &pre, &mut used).expect("cross-file must fire");
        assert_eq!(path, "a.rs");
        assert_eq!(out.len(), 1, "got: {out:?}");
        assert!(out[0].contains("- fn foo"), "got:\n{}", out[0]);
        validated_live(&out[0], &pre);
    }

    #[test]
    fn fn_intent_method_use_vetoes() {
        // A use pair whose head is a method ident is outside the
        // scalar follow domain — never a partial claim.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = s.foo();\n+    let a = bar(1);\n+    let b = s.bar();\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = s.foo();\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_macro_head_vetoes() {
        // `foo!(…)` is a macro head (different namespace) — the call
        // carve declines it.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    foo!(2);\n+    let a = bar(1);\n+    bar!(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    foo!(2);\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_off_diff_twin_fails_the_probe() {
        // An untouched `foo(3)` elsewhere fails the unchanged-leg:
        // the follow would rewrite it, so the intent declines instead
        // of under-claiming.
        let sections = fn_sections_two_use();
        let pre = pre_map(&[(
            "src/lib.rs",
            "fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n    let c = foo(3);\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_pub_prefix_declines() {
        // `pub fn` needs a vis the terse emission does not carry.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-pub fn foo(x: i32) -> i32 {\n+pub fn bar(x: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1);\n+    let b = bar(2);\n"
                .to_string(),
        ];
        let pre = pre_map(&[(
            "src/lib.rs",
            "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn main() {\n    let a = foo(1);\n    let b = foo(2);\n}\n",
        )]);
        let mut used = Vec::new();
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_evolution_declines() {
        // Signature deltas are evolutions (C17), never renames.
        let sections = vec![
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: i32) -> i32 {\n+fn bar(x: i32, y: i32) -> i32 {\n@@ -5,2 +5,2 @@\n-    let a = foo(1);\n-    let b = foo(2);\n+    let a = bar(1, 2);\n+    let b = bar(2, 3);\n"
                .to_string(),
        ];
        let mut used = Vec::new();
        let pre = pre_map(&[("src/lib.rs", FN_PRE)]);
        assert!(try_item_intent_sweep(&sections, &pre, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn fn_intent_full_pipeline_compresses() {
        let sections = fn_sections_two_use();
        let pre = pre_map(&[("src/lib.rs", FN_PRE)]);
        let out = infer_semantic_with_pre_images(&sections, &pre).expect("pipeline must compress");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- fn foo"), "got:\n{out}");
        validated_live(&out, &pre);
    }
}
