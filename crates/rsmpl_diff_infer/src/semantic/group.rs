//! WI-7a patch-level grouping (provenance-only).
//!
//! After per-section inference, hunks sharing one single-ident delta
//! (`Old` → `New`) across files are one cross-file intent (a rename
//! whose definition sits in one file and whose uses sit in others).
//! This pass labels them with a shared [`HunkProvenance::group`]
//! (`rename_Old_to_New`) and changes no patch bytes: rules stay
//! canonical `r1…`, order untouched, so cycles stay fixed-point and
//! the T375 guard is unaffected. Shared-`rule=` re-emission is WI-7b.
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
/// one call-pattern side.
fn call_expr(line: &str) -> Option<CallShape> {
    let mut text = line.trim();
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
    let text = line.trim();
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
    let text = line.trim().strip_prefix("use ")?.trim();
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
