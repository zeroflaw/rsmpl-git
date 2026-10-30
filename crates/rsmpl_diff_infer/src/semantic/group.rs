//! WI-7a patch-level grouping (provenance) + WI-7b shared-rule re-emission.
//!
//! After per-section inference, hunks sharing one single-ident delta
//! (`Old` → `New`) across files are one cross-file intent (a rename
//! whose definition sits in one file and whose uses sit in others).
//! `assign_groups` labels them with [`HunkProvenance::group`]
//! (`rename_Old_to_New`). `apply_shared_rules` (WI-7b) re-emits those
//! hunks under the shared `rule=<group>` name, definition-first, with a
//! per-group T375 re-basis (a group that cannot pay its own rename cost
//! falls back to positional `rN`, keeping the `group` label).
//!
//! Groupable detectors are exactly those emitting one single-delta
//! pair per hunk: `item_rename`, `fn_slot_sweep`, `sig_evolution`,
//! `call_sweep`, `call_peel`, `qualified_call`, `turbofish`,
//! `macro_rename`, `method_to_method`, `method_to_fn`, `use_prefix`.
//! Same-callee shapes (`call_arg_add`, `call_literal`,
//! `method_same_literal`, `ref_strip`), template shapes
//! (`let_type_add`, `type_sweep`), and non-pair shapes (`verbatim`,
//! `delete`, `move`) never group. A group needs ≥2 hunks spanning ≥2
//! distinct files; single-file repeats (sweeps) and divergent deltas
//! stay `""`.

use std::collections::{HashMap, HashSet};

use super::prov::HunkProvenance;

/// Detectors whose hunks carry one rename-like ident delta.
fn groupable(detector: &str) -> bool {
    matches!(
        detector,
        "item_rename"
            | "fn_slot_sweep"
            | "sig_evolution"
            | "call_sweep"
            | "call_peel"
            | "qualified_call"
            | "turbofish"
            | "macro_rename"
            | "method_to_method"
            | "method_to_fn"
            | "use_prefix"
    )
}

/// Label cross-file same-delta hunks in `prov` from the emitted `patch`
/// text. Fail-open: an unparseable patch or unextractable hunk simply
/// leaves `group` empty (provenance-only, never a conversion failure).
pub(crate) fn assign_groups(prov: &mut [HunkProvenance], patch: &str) {
    let parsed = match rsmpl_patch::parse_patch(patch) {
        Ok(p) => p,
        Err(_) => return,
    };
    // Rule names are unique per emission (`r1…` in canonical order),
    // so each entry maps to exactly one hunk.
    let mut hunks: HashMap<&str, (Vec<&str>, Vec<&str>)> = HashMap::new();
    for section in &parsed.files {
        for hunk in &section.hunks {
            let rsmpl_patch::Hunk::Rsmpl(rh) = hunk else {
                continue;
            };
            let Some(rule) = rh.header.rule.as_deref() else {
                continue;
            };
            let mut minus = Vec::new();
            let mut plus = Vec::new();
            for line in &rh.lines {
                if line.is_removal() {
                    minus.push(line.text.as_str());
                } else if line.is_addition() {
                    plus.push(line.text.as_str());
                }
            }
            hunks.insert(rule, (minus, plus));
        }
    }
    // (Old, New) → prov indices carrying that delta.
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, entry) in prov.iter().enumerate() {
        if !groupable(entry.detector) || !entry.group.is_empty() {
            continue;
        }
        let Some((minus, plus)) = hunks.get(entry.rule.as_str()) else {
            continue;
        };
        if minus.len() != 1 || plus.len() != 1 {
            continue;
        }
        if let Some(delta) = hunk_delta(entry.detector, minus[0], plus[0]) {
            groups.entry(delta).or_default().push(i);
        }
    }
    for ((old, new), members) in &groups {
        if members.len() < 2 {
            continue;
        }
        let files: HashSet<&str> = members.iter().map(|&i| prov[i].file.as_str()).collect();
        if files.len() < 2 {
            continue;
        }
        let label = format!("rename_{old}_to_{new}");
        for &i in members {
            prov[i].group = label.clone();
        }
    }
}

/// WI-7b: re-emit grouped hunks under their shared `rule=<group>` name.
///
/// Call after [`assign_groups`]. For each non-empty `group`, every hunk
/// whose `prov` entry carries it is renamed from its positional `rN` to
/// the shared name (header `rule=` token only), and `prov[].rule` follows.
/// Per-group T375 re-basis: `extra = Σ(len(group) − len(old))` must be
/// strictly smaller than `Σ(saved_bytes)` for the group, else that group
/// reverts to its `rN` names (patch + prov) while keeping the `group`
/// label (WI-7a fallback state). File sections are then reordered
/// definition-first per group (minimal stable displacement; ungrouped
/// sections never move). Fail-open: any inconsistency leaves `out`
/// byte-identical.
pub(crate) fn apply_shared_rules(prov: &mut [HunkProvenance], out: &mut String) {
    if !prov.iter().any(|p| !p.group.is_empty()) {
        return;
    }
    // Group label → member indices (prov order).
    let mut order: Vec<String> = Vec::new();
    let mut members: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, p) in prov.iter().enumerate() {
        if p.group.is_empty() {
            continue;
        }
        members.entry(p.group.clone()).or_default().push(i);
        if !order.contains(&p.group) {
            order.push(p.group.clone());
        }
    }
    order.sort();
    // Per-group rename (with T375 fallback), tracking old names for the
    // revert path.
    let mut renamed: HashMap<String, Vec<(usize, String)>> = HashMap::new();
    for label in &order {
        let Some(ixs) = members.get(label) else {
            continue;
        };
        let mut olds: Vec<(usize, String)> = Vec::new();
        let mut extra: usize = 0;
        let mut saved: usize = 0;
        for &i in ixs {
            let old = prov[i].rule.clone();
            olds.push((i, old.clone()));
            extra += label.len().saturating_sub(old.len());
            saved += prov[i].saved_bytes;
        }
        if extra >= saved {
            continue;
        }
        for (i, old) in &olds {
            rename_rule_in_patch(out, old, label);
            prov[*i].rule = label.clone();
        }
        renamed.insert(label.clone(), olds);
    }
    if renamed.is_empty() {
        return;
    }
    reorder_sections_definition_first(prov, out, &renamed);
}

/// Replace the `rule=old` token with `rule=new` on RSmPL header lines
/// (`@@ … @@ rule=old`) only. Body lines never start with `@@`, so a
/// body occurrence of the token text cannot misfire.
fn rename_rule_in_patch(patch: &mut String, old: &str, new: &str) {
    let needle = format!("rule={old}");
    let replacement = format!("rule={new}");
    let mut lines: Vec<String> = Vec::new();
    let mut changed = false;
    for line in patch.lines() {
        if line.starts_with("@@") && line.contains(needle.as_str()) {
            lines.push(line.replacen(needle.as_str(), replacement.as_str(), 1));
            changed = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if changed {
        *patch = lines.join("\n");
        patch.push('\n');
    }
}

/// Definition-first section reorder for renamed groups.
///
/// For each renamed group (sorted label order), the subsequence of file
/// sections containing that group is stable-sorted definitions-first
/// (`item_rename` / `fn_slot_sweep` / `sig_evolution` detectors, else
/// path) and spliced back into the same slots, so ungrouped sections
/// never move. `prov` is re-sorted to match the new section order
/// (stable within a file). Conflicting multi-group constraints resolve
/// by label order (documented limitation).
fn reorder_sections_definition_first(
    prov: &mut [HunkProvenance],
    out: &mut String,
    renamed: &HashMap<String, Vec<(usize, String)>>,
) {
    let mut sections = split_sections(out);
    if sections.len() < 2 {
        return;
    }
    let mut labels: Vec<&String> = renamed.keys().collect();
    labels.sort();
    for label in labels {
        let Some(ixs) = renamed.get(label) else {
            continue;
        };
        let paths: Vec<String> = sections.iter().map(|s| section_path(s)).collect();
        let mut file_section: HashMap<&str, usize> = HashMap::new();
        for (idx, p) in paths.iter().enumerate() {
            file_section.entry(p.as_str()).or_insert(idx);
        }
        // Sections containing this group (dedup, keep first occurrence).
        let mut involved: Vec<usize> = Vec::new();
        for (i, _) in ixs {
            let file = prov[*i].file.as_str();
            if let Some(&s) = file_section.get(file) {
                if !involved.contains(&s) {
                    involved.push(s);
                }
            }
        }
        if involved.len() < 2 {
            continue;
        }
        let mut sorted = involved.clone();
        sorted.sort_by_key(|&s| {
            let def = ixs.iter().any(|(i, _)| {
                prov[*i].file == paths[s] && is_definition_detector(prov[*i].detector)
            });
            ((!def) as u8, paths[s].clone())
        });
        if sorted == involved {
            continue;
        }
        // Splice the sorted sections back into the same slots.
        let snapshot = sections.clone();
        for (slot, sec) in involved.iter().zip(sorted.iter()) {
            sections[*slot] = snapshot[*sec].clone();
        }
    }
    // Rebuild path list from the (possibly) reordered sections.
    let new_paths: Vec<String> = sections.iter().map(|s| section_path(s)).collect();
    // Stable prov reorder to match section order.
    let mut pos: HashMap<&str, usize> = HashMap::new();
    for (idx, p) in new_paths.iter().enumerate() {
        pos.entry(p.as_str()).or_insert(idx);
    }
    let mut idxs: Vec<usize> = (0..prov.len()).collect();
    idxs.sort_by_key(|&i| {
        (
            pos.get(prov[i].file.as_str())
                .copied()
                .unwrap_or(usize::MAX),
            i,
        )
    });
    let reordered_prov: Vec<HunkProvenance> = idxs.iter().map(|&i| prov[i].clone()).collect();
    prov.clone_from_slice(&reordered_prov);
    *out = sections.join("");
}

/// Split a patch into `diff --git …` sections (each retaining its
/// trailing newline content).
fn split_sections(patch: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in patch.lines() {
        if line.starts_with("diff --git ") && !current.is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.is_empty() {
        sections.push(current);
    }
    sections
}

/// Best-effort b-side path of one `diff --git …` section.
fn section_path(section: &str) -> String {
    let header = section.lines().next().unwrap_or("");
    let rest = header.strip_prefix("diff --git ").unwrap_or(header);
    if let Some(parts) = rsmpl_patch::split_git_paths(rest) {
        if parts.len() == 2 {
            let new = parts[1].strip_prefix("b/").unwrap_or(&parts[1]);
            return new.to_string();
        }
    }
    rest.to_string()
}

/// True for definition detectors (item/fn-header renames); all other
/// groupable detectors are use sites.
fn is_definition_detector(detector: &str) -> bool {
    matches!(detector, "item_rename" | "fn_slot_sweep" | "sig_evolution")
}

/// One hunk's ident delta, or `None` when the pair carries none (same
/// ident, unparseable, or multi-slot evolution without a rename).
fn hunk_delta(detector: &str, minus: &str, plus: &str) -> Option<(String, String)> {
    match detector {
        "item_rename" | "fn_slot_sweep" | "sig_evolution" => item_delta(minus, plus),
        "call_sweep" | "call_peel" | "qualified_call" | "turbofish" | "method_to_method"
        | "method_to_fn" => call_delta(minus, plus),
        "macro_rename" => macro_delta(minus, plus),
        "use_prefix" => use_delta(minus, plus),
        _ => None,
    }
}

/// Item/fn-header pair delta (`- [V Q] [pub…] kind Old…` /
/// `+ [V Q] [pub…] kind New…`, also `- fn Old` and slot headers).
/// Returns `None` when the idents agree (pure slot evolution).
fn item_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    let old = item_ident(minus)?;
    let new = item_ident(plus)?;
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    Some((old, new))
}

/// The renamed ident of one header pattern line.
fn item_ident(line: &str) -> Option<String> {
    let mut rest = line.trim_start();
    // Slot wildcards ride ahead of the kind (`V Q fn Old …`).
    loop {
        if let Some(tail) = rest.strip_prefix("V ") {
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix("Q ") {
            rest = tail;
        } else {
            break;
        }
    }
    // `fn Old…` after wildcards.
    if let Some(tail) = rest.strip_prefix("fn ") {
        return Some(plain_ident(tail));
    }
    // `pub… kind Old…` / `kind Old…`.
    if let Some(tail) = rest.strip_prefix("pub") {
        if tail.is_empty() || tail.starts_with([' ', '\t']) {
            rest = tail.trim_start();
        } else if tail.starts_with('(') {
            let end = tail.find(')')?;
            rest = tail[end + 1..].trim_start();
        } else {
            return None;
        }
    }
    let kind_end = rest.find([' ', '\t'])?;
    rest = rest[kind_end..].trim_start();
    // `impl Old` carries trait/self spellings; the last segment names
    // the item (`impl Old`, `impl Tr for Old` → `Old`).
    if rest.starts_with("impl ") {
        let tail = rest.strip_prefix("impl ")?;
        let last = tail.split_whitespace().last()?;
        return Some(plain_ident(last));
    }
    Some(plain_ident(rest))
}

/// Leading plain-ident slice of `s` (`Old`, `Old<T>`, `Old(` → `Old`).
fn plain_ident(s: &str) -> String {
    let mut len = 0usize;
    for (i, c) in s.char_indices() {
        if c == '_' || c.is_ascii_alphanumeric() {
            len = i + c.len_utf8();
        } else {
            break;
        }
    }
    s[..len].to_string()
}

/// Call/method pair delta: bare/qualified callee or method ident on
/// each side must differ (same-callee shapes return `None`).
fn call_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    let old = call_expr(minus)?;
    let new = call_expr(plus)?;
    match (old, new) {
        (CallShape::Call(o), CallShape::Call(n)) if o != n => Some((o, n)),
        (CallShape::Method(o), CallShape::Method(n)) if o != n => Some((o, n)),
        (CallShape::Method(o), CallShape::Call(n)) if o != n => Some((o, n)),
        _ => None,
    }
}

enum CallShape {
    Call(String),
    Method(String),
}

/// Callee (last path segment, turbofish ignored) or method ident of
/// one call-pattern side. Backticked concrete callees (WI-7b uppercase
/// constructors, SPEC 15.4) strip to their literal ident first — `syn`
/// cannot parse backticks.
fn call_expr(line: &str) -> Option<CallShape> {
    let stripped = line.trim().replace('`', "");
    let mut text = stripped.trim();
    text = text.strip_suffix(';').map(str::trim_end).unwrap_or(text);
    text = text.strip_suffix(',').map(str::trim_end).unwrap_or(text);
    let expr: syn::Expr = syn::parse_str(text).ok()?;
    match expr {
        syn::Expr::Call(call) => {
            if let syn::Expr::Path(path) = call.func.as_ref() {
                if path.qself.is_some() {
                    return None;
                }
                let seg = path.path.segments.last()?;
                Some(CallShape::Call(seg.ident.to_string()))
            } else {
                None
            }
        }
        syn::Expr::MethodCall(method) => Some(CallShape::Method(method.method.to_string())),
        _ => None,
    }
}

/// Macro-invocation pair delta (`old!(…)` → `new!(…)`, qualified
/// heads compare last segments only).
fn macro_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    let old = macro_head(minus)?;
    let new = macro_head(plus)?;
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    Some((old, new))
}

fn macro_head(line: &str) -> Option<String> {
    let text = line.trim().replace('`', "");
    let text = text.trim();
    let (head, _) = text.split_once('!')?;
    let last = head.split("::").last()?.trim();
    if last.is_empty() || !last.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(last.to_string())
}

/// `use`-prefix pair delta (last path segments must differ).
fn use_delta(minus: &str, plus: &str) -> Option<(String, String)> {
    let old = use_last(minus)?;
    let new = use_last(plus)?;
    if old == new || old.is_empty() || new.is_empty() {
        return None;
    }
    Some((old, new))
}

fn use_last(line: &str) -> Option<String> {
    let text = line.trim().replace('`', "");
    let text = text.trim().strip_prefix("use ")?.trim();
    let text = text.strip_suffix(';').map(str::trim).unwrap_or(text);
    Some(plain_ident(text.split("::").last()?.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn sections_pair(b_old: &str, b_new: &str) -> Vec<String> {
        let a = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n-struct Old {\n+struct New {\n     x: i32,\n }\n"
            .to_string();
        let b = format!(
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,3 +1,3 @@\n-struct {b_old} {{\n+struct {b_new} {{\n     y: i32,\n }}\n"
        );
        vec![a, b]
    }

    fn run(sections: &[String]) -> (Option<String>, Vec<HunkProvenance>) {
        let mut prov = Vec::new();
        let out =
            super::super::infer_semantic_with_provenance(sections, &HashMap::new(), &mut prov);
        (out, prov)
    }

    #[test]
    fn cross_file_shared_delta_groups() {
        let sections = sections_pair("Old", "New");
        let (out, prov) = run(&sections);
        assert!(out.is_some(), "both singles must compress");
        assert_eq!(prov.len(), 2, "prov: {prov:?}");
        assert_eq!(prov[0].group, "rename_Old_to_New", "prov: {prov:?}");
        assert_eq!(prov[1].group, "rename_Old_to_New", "prov: {prov:?}");
        let text = out.unwrap();
        rsmpl_patch::parse_patch(&text).expect("grouped patch must parse");
    }

    #[test]
    fn shared_rule_reemitted_for_group() {
        // WI-7b: grouped hunks share `rule=rename_Old_to_New` in bytes,
        // and prov rules follow (parseable, fixed-point input).
        let sections = sections_pair("Old", "New");
        let (out, prov) = run(&sections);
        let text = out.expect("both singles must compress");
        assert_eq!(
            text.matches("rule=rename_Old_to_New").count(),
            2,
            "text:\n{text}"
        );
        assert!(!text.contains("rule=r1"), "text:\n{text}");
        assert!(
            prov.iter().all(|p| p.rule == "rename_Old_to_New"),
            "prov: {prov:?}"
        );
        rsmpl_patch::parse_patch(&text).expect("shared-rule patch must parse");
    }

    #[test]
    fn t375_fallback_keeps_positional_names() {
        // WI-7b re-basis: a very long ident makes the shared name cost
        // more than the win, so hunks keep `rN` while `group` stays.
        let long_old = "O".repeat(60);
        let long_new = "N".repeat(60);
        let mk = |name: &str, old: &str, new: &str| {
            format!(
                "diff --git a/{name} b/{name}\n--- a/{name}\n+++ b/{name}\n@@ -1,3 +1,3 @@\n-struct {old} {{\n+struct {new} {{\n     x: i32,\n }}\n"
            )
        };
        let sections = vec![
            mk("a.rs", &long_old, &long_new),
            mk("b.rs", &long_old, &long_new),
        ];
        let (out, prov) = run(&sections);
        let text = out.expect("singles must compress");
        assert!(prov.iter().all(|p| !p.group.is_empty()), "prov: {prov:?}");
        assert!(
            !text.contains("rule=rename_"),
            "long shared name must fall back to rN:\n{text}"
        );
        assert!(text.contains("rule=r1"), "text:\n{text}");
        rsmpl_patch::parse_patch(&text).expect("fallback patch must parse");
    }

    #[test]
    fn definition_first_reorders_sections() {
        // WI-7b: use-file `a.rs` (call sweep) + def-file `z.rs` (item
        // rename), one shared delta — the definition section leads and
        // ungrouped sections never move.
        let uses = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-    Old(argument1);\n-    Old(argument2);\n+    New(argument1);\n+    New(argument2);\n"
            .to_string();
        let def = "diff --git a/z.rs b/z.rs\n--- a/z.rs\n+++ b/z.rs\n@@ -1,3 +1,3 @@\n-struct Old {\n+struct New {\n     x: i32,\n }\n"
            .to_string();
        let plain = "diff --git a/m.rs b/m.rs\n--- a/m.rs\n+++ b/m.rs\n@@ -1,1 +1,1 @@\n-old_line\n+new_line\n"
            .to_string();
        // Input order buries the definition last among grouped files.
        // (Emission collapses headers to single-path `diff --git a.rs`.)
        let (out, prov) = run(&[uses, plain, def]);
        let text = out.expect("grouped sections must compress");
        let a = text.find("diff --git a.rs").expect("a.rs present");
        let z = text.find("diff --git z.rs").expect("z.rs present");
        assert!(z < a, "definition z.rs must lead use a.rs:\n{text}");
        assert_eq!(
            text.matches("rule=rename_Old_to_New").count(),
            2,
            "both grouped hunks share the rule:\n{text}"
        );
        assert!(
            prov.iter().any(|p| p.rule == "rename_Old_to_New"),
            "prov: {prov:?}"
        );
        rsmpl_patch::parse_patch(&text).expect("reordered patch must parse");
    }

    #[test]
    fn divergent_deltas_never_group() {
        let sections = sections_pair("Old", "Other");
        let (out, prov) = run(&sections);
        assert!(out.is_some());
        assert_eq!(prov.len(), 2, "prov: {prov:?}");
        assert!(prov.iter().all(|p| p.group.is_empty()), "prov: {prov:?}");
    }

    #[test]
    fn single_file_repeat_never_groups() {
        // Two identical single-pair hunks in one section compress per
        // hunk (sweeps need N ≥ 2 hetero pairs); same file is a sweep,
        // not cross-file intent.
        let one = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n-struct Old {\n+struct New {\n     x: i32,\n }\n@@ -10,3 +10,3 @@\n-struct Old {\n+struct New {\n     y: i32,\n }\n"
            .to_string();
        let (out, prov) = run(std::slice::from_ref(&one));
        assert!(out.is_some(), "per-hunk singles must compress");
        assert!(prov.iter().all(|p| p.group.is_empty()), "prov: {prov:?}");
    }

    #[test]
    fn item_pair_extracts_delta() {
        assert_eq!(
            item_delta("struct Old", "struct New"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            item_delta("V Q fn Old G (...) W {", "V Q fn New G (...) W {"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            item_delta("pub struct Old", "pub struct New"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(item_delta("fn Old() {", "fn Old() {"), None);
    }

    #[test]
    fn same_callee_has_no_delta() {
        assert_eq!(call_delta("foo(1)", "foo(2)"), None);
        assert_eq!(
            call_delta("foo(E0)", "bar(E0)"),
            Some(("foo".to_string(), "bar".to_string()))
        );
        assert_eq!(
            call_delta("E.old_m(E0)", "new_fn(E, E0)"),
            Some(("old_m".to_string(), "new_fn".to_string()))
        );
    }

    #[test]
    fn backticked_callees_strip_to_literal_delta() {
        // WI-7b: uppercase constructor sweeps emit backticked callees
        // (SPEC 15.4); grouping strips to the literal ident.
        assert_eq!(
            call_delta("`Old`(E0)", "`New`(E0)"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(call_delta("`Old`(E0)", "`Old`(E1)"), None);
        // WI-7-upper: qualified, turbofish, and method heads strip the
        // same way (`call_expr` drops all backticks before parsing).
        assert_eq!(
            call_delta("crate::`Old`(E0)", "crate::`New`(E0)"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            call_delta("`Old`::<u8>(E0)", "`New`::<u8>(E0)"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            call_delta("crate::`Old`::<T>(E0)", "crate::`New`::<T>(E0)"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(
            call_delta("Receiver.`Old`(E0)", "Receiver.`New`(E0)"),
            Some(("Old".to_string(), "New".to_string()))
        );
    }

    #[test]
    fn macro_and_use_pairs_extract() {
        assert_eq!(
            macro_delta("old!(...)", "new!(...)"),
            Some(("old".to_string(), "new".to_string()))
        );
        assert_eq!(
            use_delta("use crate::Old", "use crate::New"),
            Some(("Old".to_string(), "New".to_string()))
        );
        assert_eq!(use_delta("use crate::Old", "use crate::Old"), None);
    }

    #[test]
    fn non_pair_shapes_never_group() {
        assert_eq!(hunk_delta("verbatim", "foo(1)", "bar(1)"), None);
        assert_eq!(hunk_delta("delete", "struct Old", ""), None);
        assert_eq!(hunk_delta("move", "fn f() {}", "fn f() {}"), None);
        assert_eq!(hunk_delta("call_arg_add", "foo(1)", "foo(1, 2)"), None);
        assert_eq!(hunk_delta("unknown_detector", "foo", "bar"), None);
    }
}
