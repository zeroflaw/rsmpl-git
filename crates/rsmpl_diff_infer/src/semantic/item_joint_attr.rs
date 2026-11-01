//! WI-5 slice 1: item rename + single attr-delta joint helpers.
//!
//! The detection branch lives in the item-intent pre-pass (it owns the
//! pair/hunk tables); this module holds only the pure helpers so the
//! intent file stays under the D5 line gate: the attr classifier and
//! the joint follow emission (attr pair above the terse def pair,
//! `# sweep` + `# follow=uses` — the authored Preserve-delta spelling
//! the engine already lowers and verifies).

/// True when the pair is a single-line attribute delta (forms
/// differ). Both sides must parse as a real outer attribute, so
/// `derive(Debug)` vs `derive(Debug, Clone)` joins while free text
/// never does. Name containment is deliberately *not* required: the
/// attr delta rides the definition rename (e.g. a `derive` add next
/// to a `struct` rename carries no `Old`/`New` ident).
pub(crate) fn is_attr_delta(minus: &str, plus: &str) -> bool {
    let m = minus.trim();
    let p = plus.trim();
    if m == p || m.is_empty() || p.is_empty() {
        return false;
    }
    if m.lines().count() != 1 || p.lines().count() != 1 {
        return false;
    }
    if !(m.starts_with("#[") && p.starts_with("#[")) {
        return false;
    }
    if !(m.ends_with(']') && p.ends_with(']')) {
        return false;
    }
    // Parseability probe: the line must parse as a real outer attribute
    // (rejects unclosed brackets, free text). `syn::Attribute` has no
    // `Parse` impl, so probe through a host item.
    let probe =
        |line: &str| syn::parse_str::<syn::File>(&format!("{line}\nstruct __Probe;\n")).is_ok();
    probe(m) && probe(p)
}

/// WI-5 joint search over the pre-pass pair/hunk tables: returns the
/// attr pair index when the def hunk holds exactly one attached
/// attr-delta pair (same file and section, attr site directly above
/// the def site) plus claimable uses otherwise, with no attr pair
/// anywhere else. `pair_claimable` is the caller's gate (it owns the
/// fn/item distinction); `is_fn` excludes `fn` defs outright (WI-4 B2
/// owns fn rename+attr). `None` means no joint — the caller falls back
/// to the pure-intent path (or declines on a real co-resident veto).
pub(crate) fn find_attr_joint(
    pairs: &[super::item_intent::Pair],
    hunk_pairs: &[Vec<usize>],
    def_hunk: usize,
    def_idx: usize,
    is_fn: bool,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
) -> Option<usize> {
    if is_fn {
        return None;
    }
    let mut bad: Vec<usize> = Vec::new();
    for pi in &hunk_pairs[def_hunk] {
        if *pi == def_idx {
            continue;
        }
        if !pair_claimable(&pairs[*pi]) {
            bad.push(*pi);
        }
    }
    if bad.len() != 1 {
        return None;
    }
    let pi = bad[0];
    let attached = pairs[pi].path == pairs[def_idx].path
        && pairs[pi].sec == pairs[def_idx].sec
        && pairs[def_idx].site == pairs[pi].site + 1;
    let elsewhere = pairs
        .iter()
        .enumerate()
        .any(|(j, p)| j != pi && is_attr_delta(&p.minus, &p.plus));
    if attached && !elsewhere && is_attr_delta(&pairs[pi].minus, &pairs[pi].plus) {
        Some(pi)
    } else {
        None
    }
}

/// Plain follow emission: sweep dummy + follow + terse def pair (moved
/// from the item-intent pre-pass verbatim so the D5 line gate holds;
/// the WI-1 concrete-vis law lives with the caller).
pub(crate) fn emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    if kind == "fn" {
        // WI-1: concrete vis rides the terse pair (never a `V` hole —
        // the intent proved one vis).
        out.push_str(&format!("- {prefix}fn {old}\n+ {prefix}fn {new}\n"));
    } else {
        out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    }
    out
}

/// Joint follow emission: the attr delta above the terse def pair.
/// `kind`/`prefix`/`old`/`new` are the `split_item` def parts; `attr_*`
/// are the raw native lines (trimmed on emit).
#[allow(clippy::too_many_arguments)]
pub(crate) fn joint_emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
    attr_minus: &str,
    attr_plus: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!(
        "- {}\n+ {}\n",
        attr_minus.trim(),
        attr_plus.trim()
    ));
    if kind == "fn" {
        out.push_str(&format!("- {prefix}fn {old}\n+ {prefix}fn {new}\n"));
    } else {
        out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifier_matrix() {
        assert!(is_attr_delta("#[derive(Debug)]", "#[derive(Debug, Clone)]"));
        assert!(is_attr_delta("  #[inline]  ", "  #[inline(never)]  "));
        assert!(!is_attr_delta("#[derive(Debug)]", "#[derive(Debug)]"));
        assert!(!is_attr_delta("struct Old {", "struct New {"));
        assert!(!is_attr_delta("#[derive(Debug)]", "struct New {"));
        assert!(!is_attr_delta("not an attr", "also not"));
        assert!(!is_attr_delta("#[unclosed", "#[derive(Debug)]"));
        assert!(!is_attr_delta("#[derive(Debug)", "#[derive(Debug)]"));
        assert!(!is_attr_delta("", "#[derive(Debug)]"));
    }

    #[test]
    fn emission_applies_jointly() {
        let pre = "#[derive(Debug)]\nstruct Old {\n    x: i32,\n}\nfn f(s: Old) {\n}\n";
        let emission = joint_emit(
            "a.rs",
            "r1",
            "struct",
            "",
            "Old",
            "New",
            "#[derive(Debug)]",
            "#[derive(Debug, Clone)]",
        );
        assert!(emission.contains("- #[derive(Debug)]\n"));
        assert!(emission.contains("+ #[derive(Debug, Clone)]\n"));
        assert!(emission.contains("- struct Old\n"));
        assert!(emission.contains("+ struct New\n"));
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("a.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint emission applies");
        assert!(files["a.rs"].contains("struct New"));
        assert!(files["a.rs"].contains("#[derive(Debug, Clone)]"));
        assert!(files["a.rs"].contains("fn f(s: New)"));
    }
}
