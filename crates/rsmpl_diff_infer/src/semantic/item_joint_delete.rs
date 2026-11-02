//! WI-21 slices 1–3: struct rename + field-deletion / enum rename +
//! variant-deletion joints.
//!
//! The detection branch lives in the item-intent pre-pass (it owns the
//! pair/hunk tables); this module holds the pure helpers so the intent
//! file stays under the D5 line gate: the shape-A/shape-B classifiers,
//! the same-container carrier checks (the deletion's pre-image carrier is
//! the renamed container), the shared-rule two-hunk emissions (deletion
//! hunk first, rename follow hunk second — both match against original
//! coordinates, so order is reader convention, not engine requirement),
//! and the multi-file D13 probe (reused from the addition joint — the
//! probe is shape-agnostic: composed emission vs claimed native
//! full-post equality).
//!
//! Two native shapes raise. Shape A (distant): a balanced def-rename
//! hunk plus a pure-deletion (`@@ -N +M,0 @@`) member hunk in the same
//! file+section — today this partitions into a rename intent plus a
//! leftover ordinary deletion. Shape B (adjacent): def change and
//! deletion adjacent merge into one unbalanced (2-minus, 1-plus) hunk
//! whose minus[0]/plus[0] is the def rename and minus[1] the member line —
//! today the pair model skips the hunk entirely so the section stays
//! wholly ordinary (the shape-B trap). Both raise to one shared-rule
//! emission; decline falls back to today's posture (partition for A,
//! ordinary for B).
//!
//! Slice-3 law: untouched live uses never block the joint. The D13
//! probe owns touched-use safety (the emission keeps uses the native
//! removed or rewrote, so the posts mismatch and the joint declines);
//! the engine owns untouched-use honesty (the deletion hunk reports
//! anchored uses of the deleted member as `orphans`, capping the run
//! at `partial` via `global_semantics`). Unrelated same-named idents
//! are not anchored uses, so they fire `verified` (WI-14 siblings
//! never veto).
//!
//! Single deletion, single-file only: struct and enum arms are disjoint
//! by kind (field vs variant parsing); two deletions, cross-kind
//! members, cross-file members, and attr+deletion combos decline (each
//! pinned, each a follow-up).
//!
//! D5: this module also owns the pre-pass call-site helpers
//! (`find_deletion_joints` + `claim_deletion_hunks`) so `item_intent.rs`
//! stays under the 2000-line gate (verbatim move of the two arm blocks
//! plus the two claim inserts).

/// Both deletion arms for one def section (at most one fires — struct
/// and enum halves are mutually exclusive by kind; stacking with any
/// other joint declines at the call site).
pub(crate) struct DeletionJoints {
    pub(crate) field: Option<DelJoint>,
    pub(crate) variant: Option<DelJoint>,
}

/// Run the field-deletion arm (WI-21 slice 1) then the variant-deletion
/// arm (WI-21 slice 2). Each arm runs only when every earlier joint
/// declines; the variant arm additionally requires the field arm to
/// decline. Pure helper — the caller owns rule reservation release.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_deletion_joints(
    pairs: &[super::item_intent::Pair],
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
    unbals: &[super::item_joint_field_add::Unbal],
    del_shape_bs: &[DelShapeB],
    attr_joint: Option<usize>,
    field_joint: Option<usize>,
    variant_joint: Option<usize>,
    add_joint: Option<&super::item_joint_field_add::AddJoint>,
    variant_add_joint: Option<&super::item_joint_field_add::AddJoint>,
) -> DeletionJoints {
    let field = if attr_joint.is_none()
        && field_joint.is_none()
        && variant_joint.is_none()
        && add_joint.is_none()
        && variant_add_joint.is_none()
    {
        find_field_del_joint(
            pairs,
            def_idx,
            is_fn,
            def_kind,
            old,
            pre,
            pair_claimable,
            unbals,
            del_shape_bs,
        )
    } else {
        None
    };
    // The enum half runs only when the attr/evo/addition/field-del
    // arms decline — struct and enum halves are mutually exclusive by
    // kind so cross-kind members never joint.
    let variant = if attr_joint.is_none()
        && field_joint.is_none()
        && variant_joint.is_none()
        && add_joint.is_none()
        && variant_add_joint.is_none()
        && field.is_none()
    {
        find_variant_del_joint(
            pairs,
            def_idx,
            is_fn,
            def_kind,
            old,
            pre,
            pair_claimable,
            unbals,
            del_shape_bs,
        )
    } else {
        None
    };
    DeletionJoints { field, variant }
}

/// Claim the deletion hunks (shape A holds no balanced pair so the
/// caller loop skips it; shape B re-inserts the def hunk, idempotent).
pub(crate) fn claim_deletion_hunks(
    claimed_hunks: &mut std::collections::HashSet<usize>,
    field: &Option<DelJoint>,
    variant: &Option<DelJoint>,
) {
    if let Some(del) = field {
        claimed_hunks.insert(del.hi);
    }
    if let Some(del) = variant {
        claimed_hunks.insert(del.hi);
    }
}

/// D13 probe dispatch (D5: lives here so `item_intent.rs` stays under
/// the 2000-line gate). Insertion/deletion joints probe
/// composed-emission vs claimed-native full-post equality; the plain
/// path probes every claimed site.
pub(crate) fn probe_pre_pass(
    pre: &std::collections::HashMap<String, String>,
    emission: &str,
    claimed_pairs: &[super::item_intent::Pair],
    hunk_sites: &[(String, String)],
    claimed_hunks: &std::collections::HashSet<usize>,
    has_joint: bool,
) -> bool {
    if has_joint {
        let mut claimed_map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (hi, (path, text)) in hunk_sites.iter().enumerate() {
            if claimed_hunks.contains(&hi) {
                claimed_map
                    .entry(path.clone())
                    .or_default()
                    .push(text.clone());
            }
        }
        super::item_joint_field_add::probe_add_joint(pre, emission, &claimed_map)
    } else {
        super::item_intent::probe_reproduces(pre, emission, claimed_pairs)
    }
}

/// A decomposed shape-B deletion hunk: (2 minus, 1 plus) with the def
/// rename at [0] and the field line at minus[1]. `hi` is the hunk-entry
/// index (== the virtual def pair's hunk, already claimed as the def
/// hunk); `point` is the old-side line of the deleted field (the hunk
/// header's old start, mirroring the addition joint); `del_minus` the
/// raw field line.
pub(crate) struct DelShapeB {
    pub(crate) hi: usize,
    pub(crate) sec: usize,
    pub(crate) path: String,
    pub(crate) point: usize,
    pub(crate) del_minus: String,
}

/// The joint claim: `hi` is the deletion's hunk entry (shape A: the
/// pure-deletion hunk; shape B: the def hunk itself, already claimed);
/// `point` the `@@ -P,1 @@` range; `member_minus` the raw field line;
/// `shape_b` selects the virtual-def `claimed_idx` lift.
pub(crate) struct DelJoint {
    pub(crate) hi: usize,
    pub(crate) point: usize,
    pub(crate) member_minus: String,
    pub(crate) shape_b: bool,
}

/// Decompose a shape-B deletion hunk, or `None`. Purely structural:
/// exactly two minuses and one plus; the caller (def discovery via
/// `split_item`, the joint finder via the field classifier + carrier
/// law) decides meaning. `point` is the header's old-side start
/// (mirrors the addition joint: the deleted field's old line for the
/// adjacent shape). Both orders of (def, field) vs (field, def) in the
/// minus side are accepted structurally — the finder requires the
/// field line to parse as a field and the other half to be the def.
pub(crate) fn decompose_del_shape_b(
    minuses: &[String],
    pluses: &[String],
    text: &str,
) -> Option<(String, String, String, usize)> {
    if minuses.len() != 2 || pluses.len() != 1 {
        return None;
    }
    let point = deletion_point(text)?;
    Some((
        minuses[0].clone(),
        pluses[0].clone(),
        minuses[1].clone(),
        point,
    ))
}

/// Old-side start of the first `@@ -start[,len] ... @@` header.
/// Unlike `definition_addition::insertion_point` (pure-insertion only),
/// this accepts any old length — deletions carry `-N` or `-N,L`.
pub(crate) fn deletion_point(text: &str) -> Option<usize> {
    for line in text.lines() {
        if !line.starts_with("@@") {
            continue;
        }
        let first = line.split_whitespace().nth(1)?;
        let range = first.strip_prefix('-')?;
        let start: usize = range.split(',').next()?.parse().ok()?;
        return Some(start.max(1));
    }
    None
}

/// The `struct S` containing `field` that spans 1-based `point`, or
/// `None` under the exactly-one-carrier law.
///
/// This is the deletion mirror of `definition_addition::struct_spanning`
/// (which recovers the container *lacking* the member at an insertion
/// point): a deletion needs the container that *holds* the field.
/// Exactly one struct in the file (top-level or inline-`mod`-nested,
/// mirroring the `carrier` walk) may contain the field, and it must
/// span the deletion point — otherwise the joint declines.
fn del_carrier(pre: &str, field: &str, point: usize) -> Option<String> {
    use syn::spanned::Spanned;
    let file: syn::File = syn::parse_str(pre).ok()?;
    fn walk(items: &[syn::Item], field: &str, point: usize, holders: &mut Vec<String>) {
        for it in items {
            match it {
                syn::Item::Struct(s) => {
                    if let syn::Fields::Named(fields) = &s.fields {
                        if fields
                            .named
                            .iter()
                            .any(|f| f.ident.as_ref().is_some_and(|i| i == field))
                        {
                            let (start, end) = (s.span().start().line, s.span().end().line);
                            if start <= point && point <= end {
                                holders.push(s.ident.to_string());
                            }
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, field, point, holders);
                    }
                }
                _ => {}
            }
        }
    }
    let mut holders = Vec::new();
    walk(&file.items, field, point, &mut holders);
    if holders.len() != 1 {
        return None;
    }
    holders.into_iter().next()
}

/// WI-21 joint search for the field-deletion half: returns the joint
/// claim when the def section holds exactly one attached field
/// deletion (shape-A pure-deletion hunk or shape-B record, same file
/// and section, deletion-point carrier is the renamed struct) plus
/// claimable uses otherwise, with no other non-claimable balanced pair
/// in the def section. Untouched live uses never block: the D13 probe
/// declines touched uses (emission keeps what the native removed), the
/// engine reports untouched uses as `orphans` (`partial`). Struct-only,
/// `fn` excluded, at most one joint per def section (the caller runs
/// this arm only when the attr, evo, and addition arms decline).
/// `None` means no joint — the caller falls back to today's posture.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_field_del_joint(
    pairs: &[super::item_intent::Pair],
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
    unbals: &[super::item_joint_field_add::Unbal],
    del_shape_bs: &[DelShapeB],
) -> Option<DelJoint> {
    if is_fn || def_kind != "struct" {
        return None;
    }
    let def_sec = pairs[def_idx].sec;
    let def_path = pairs[def_idx].path.clone();
    // No stranded balanced delta: every other balanced pair in the def
    // section must be claimable (a co-resident evo the evo arm declined
    // vetoes the joint too — never a half joint).
    for (pi, pair) in pairs.iter().enumerate() {
        if pi == def_idx {
            continue;
        }
        if pair.sec != def_sec || pair.path != def_path {
            continue;
        }
        if !pair_claimable(pair) {
            return None;
        }
    }
    let mut cands: Vec<DelJoint> = Vec::new();
    // Shape A: pure-deletion hunk in the def section.
    for u in unbals {
        if u.sec != def_sec || u.path != def_path {
            continue;
        }
        if u.minuses.len() != 1 || !u.pluses.is_empty() {
            continue;
        }
        let point = deletion_point(&u.text)?;
        let field = super::item_joint_field_add::parse_field_add(&u.minuses[0])?;
        let pre_text = pre.get(&u.path)?;
        if del_carrier(pre_text, &field, point)? != old {
            continue;
        }
        cands.push(DelJoint {
            hi: u.hi,
            point,
            member_minus: u.minuses[0].clone(),
            shape_b: false,
        });
    }
    // Shape B: decomposed record in the def section.
    for b in del_shape_bs {
        if b.sec != def_sec || b.path != def_path {
            continue;
        }
        let field = super::item_joint_field_add::parse_field_add(&b.del_minus)?;
        let pre_text = pre.get(&def_path)?;
        if del_carrier(pre_text, &field, b.point)? != old {
            continue;
        }
        cands.push(DelJoint {
            hi: b.hi,
            point: b.point,
            member_minus: b.del_minus.clone(),
            shape_b: true,
        });
    }
    if cands.len() != 1 {
        return None;
    }
    let joint = cands.into_iter().next()?;
    // Slice-3 law: no whole-word honesty gate. Touched uses fail the
    // D13 probe downstream (the emission keeps the use line the native
    // removed or rewrote); untouched anchored uses surface as engine
    // `orphans` (`partial`); unrelated same-named idents are not
    // anchored uses and fire `verified`.
    Some(joint)
}

/// Shared-rule two-hunk emission: the deletion hunk first (its
/// `struct Old {` context matches the pre tree), the terse rename
/// follow hunk second. No new `PatternKind`: the engine lowers the
/// deletion hunk as `field_deletion` and the rename hunk as
/// `item_rename` with follow; the joint lives in the shared rule name
/// and the single D13 probe.
#[allow(clippy::too_many_arguments)]
pub(crate) fn del_emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
    point: usize,
    member_minus: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    let mut member = member_minus.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    out.push_str(&format!("@@ -{point},1 @@ rule={rule}\n"));
    out.push_str(&format!("  struct {old} {{\n"));
    out.push_str(&format!("- {member}\n"));
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    out
}

/// The `enum E` containing `variant` that spans 1-based `point`, or
/// `None` under the exactly-one-carrier law (the deletion mirror of
/// the addition `enum_spanning`: a deletion needs the container that
/// *holds* the variant).
fn enum_del_carrier(pre: &str, variant: &str, point: usize) -> Option<String> {
    use syn::spanned::Spanned;
    let file: syn::File = syn::parse_str(pre).ok()?;
    fn walk(items: &[syn::Item], variant: &str, point: usize, holders: &mut Vec<String>) {
        for it in items {
            match it {
                syn::Item::Enum(e) => {
                    if e.variants.iter().any(|v| v.ident == variant) {
                        let (start, end) = (e.span().start().line, e.span().end().line);
                        if start <= point && point <= end {
                            holders.push(e.ident.to_string());
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, variant, point, holders);
                    }
                }
                _ => {}
            }
        }
    }
    let mut holders = Vec::new();
    walk(&file.items, variant, point, &mut holders);
    if holders.len() != 1 {
        return None;
    }
    holders.into_iter().next()
}

/// WI-21 slices 2–3 joint search for the variant-deletion half:
/// returns the joint claim when the def section holds exactly one
/// attached variant deletion (shape-A pure-deletion hunk or shape-B
/// record, same file and section, deletion-point carrier is the renamed
/// enum) plus claimable uses otherwise, with no other non-claimable
/// balanced pair in the def section. Untouched live uses never block:
/// the D13 probe declines touched uses, the engine reports untouched
/// uses as `orphans` (`partial`). Enum-only, `fn` excluded, at most one
/// joint per def section (the caller runs this arm only when the attr,
/// evo, addition, and field-deletion arms decline). `None` means no
/// joint.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_variant_del_joint(
    pairs: &[super::item_intent::Pair],
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
    unbals: &[super::item_joint_field_add::Unbal],
    del_shape_bs: &[DelShapeB],
) -> Option<DelJoint> {
    if is_fn || def_kind != "enum" {
        return None;
    }
    let def_sec = pairs[def_idx].sec;
    let def_path = pairs[def_idx].path.clone();
    for (pi, pair) in pairs.iter().enumerate() {
        if pi == def_idx {
            continue;
        }
        if pair.sec != def_sec || pair.path != def_path {
            continue;
        }
        if !pair_claimable(pair) {
            return None;
        }
    }
    let mut cands: Vec<DelJoint> = Vec::new();
    for u in unbals {
        if u.sec != def_sec || u.path != def_path {
            continue;
        }
        if u.minuses.len() != 1 || !u.pluses.is_empty() {
            continue;
        }
        let point = deletion_point(&u.text)?;
        let variant = super::item_joint_field_add::parse_variant_add(&u.minuses[0])?;
        let pre_text = pre.get(&u.path)?;
        if enum_del_carrier(pre_text, &variant, point)? != old {
            continue;
        }
        cands.push(DelJoint {
            hi: u.hi,
            point,
            member_minus: u.minuses[0].clone(),
            shape_b: false,
        });
    }
    for b in del_shape_bs {
        if b.sec != def_sec || b.path != def_path {
            continue;
        }
        let variant = super::item_joint_field_add::parse_variant_add(&b.del_minus)?;
        let pre_text = pre.get(&def_path)?;
        if enum_del_carrier(pre_text, &variant, b.point)? != old {
            continue;
        }
        cands.push(DelJoint {
            hi: b.hi,
            point: b.point,
            member_minus: b.del_minus.clone(),
            shape_b: true,
        });
    }
    if cands.len() != 1 {
        return None;
    }
    let joint = cands.into_iter().next()?;
    // Slice-3 law: no whole-word honesty gate (see the field arm).
    // The D13 probe declines touched uses; the engine reports untouched
    // uses as `orphans` (`partial`).
    Some(joint)
}

/// Shared-rule two-hunk emission for the enum half: the deletion hunk
/// first (its `enum Old {` context matches the pre tree), the terse
/// rename follow hunk second.
#[allow(clippy::too_many_arguments)]
pub(crate) fn variant_del_emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
    point: usize,
    member_minus: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    let mut member = member_minus.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    out.push_str(&format!("@@ -{point},1 @@ rule={rule}\n"));
    out.push_str(&format!("  enum {old} {{\n"));
    out.push_str(&format!("- {member}\n"));
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn infer(section: &str, pre: &str) -> Option<String> {
        super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section.to_string()),
            &std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]),
        )
    }

    #[test]
    fn decompose_matrix() {
        // Shape B deletion: 2 minus, 1 plus.
        let text = "@@ -1,2 +1 @@\n-struct Old {\n-    y: u32,\n+struct New {\n";
        let (dm, dp, del, point) = decompose_del_shape_b(
            &["struct Old {".to_string(), "    y: u32,".to_string()],
            &["struct New {".to_string()],
            text,
        )
        .expect("shape-B deletion must decompose");
        assert_eq!(dm, "struct Old {");
        assert_eq!(dp, "struct New {");
        assert_eq!(del, "    y: u32,");
        assert_eq!(point, 1);
        // Addition shape is not a deletion shape.
        assert!(decompose_del_shape_b(
            &["a".to_string()],
            &["b".to_string(), "c".to_string()],
            "@@ -1 +1,2 @@\n"
        )
        .is_none());
        // Balanced is not shape B.
        assert!(decompose_del_shape_b(
            &["a".to_string()],
            &["b".to_string()],
            "@@ -1,1 +1,1 @@\n-a\n+b\n"
        )
        .is_none());
        // Pure deletion (1 minus, 0 plus) is shape A, not B.
        assert!(
            decompose_del_shape_b(&["    y: u32,".to_string()], &[], "@@ -3 +2,0 @@\n").is_none()
        );
    }

    #[test]
    fn shape_a_joint_fires() {
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -5,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre).expect("shape-A deletion joint must fire");
        assert!(out.contains("rule="), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        assert!(out.contains("+ struct New"), "got:\n{out}");
        assert!(out.contains("- y: u32,"), "got:\n{out}");
        assert!(!out.contains("+ y: u32,"), "got:\n{out}");
        // Shared rule r1 on both hunks.
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["f.rs"].contains("struct New"),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("y: u32"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("s: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn shape_b_joint_fires() {
        // Adjacent def + deletion merge into one unbalanced hunk.
        let pre = "struct Old {\n    y: u32,\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1 @@\n-struct Old {\n-    y: u32,\n+struct New {\n@@ -5 +4 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre).expect("shape-B deletion joint must fire");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        assert!(out.contains("+ struct New"), "got:\n{out}");
        assert!(out.contains("- y: u32,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["f.rs"].contains("struct New"),
            "got: {}",
            files["f.rs"]
        );
        assert!(!files["f.rs"].contains("y: u32"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("s: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn two_deletions_decline() {
        // Two deletions: single-deletion law declines the joint.
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n    z: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -4 +3,0 @@\n-    z: u32,\n@@ -6,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            // Joint ⟺ r1 shared by two hunks; anything else reserves r1
            // at most once per hunk with distinct names.
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "two deletions decline the joint; the rename may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn live_use_declines_joint() {
        // Touched live use: the native deletes the `s.y` line alongside
        // the field, so the joint emission (which keeps `s.y`) fails the
        // D13 probe and declines (rename may still partition alone).
        // Untouched uses fire partial (see the slice-3 pins below).
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nfn f(s: Old) -> u32 {\n    s.y\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -5,3 +4,2 @@\n-fn f(s: Old) -> u32 {\n-    s.y\n+fn f(s: New) -> u32 {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "live-use deletion declines the joint, got:\n{o}"
            );
            assert!(!o.contains("- y: u32,"), "got:\n{o}");
        }
    }

    #[test]
    fn unrelated_same_named_ident_fires_verified() {
        // Slice-3 precision: an unrelated same-named `y` (a fn param
        // here) is not an anchored use of `Old::y`, so the joint fires
        // `verified` with zero orphans (WI-14 siblings never veto). The
        // D13 probe passes byte-identical; the engine orphan scan stays
        // silent on the bare param.
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nfn f(s: Old) {\n}\nfn g(y: u32) -> u32 {\n    y\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -5,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre).expect("unrelated ident must not block the joint");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        assert!(out.contains("- y: u32,"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            report.rules.iter().all(|r| r.orphans.is_empty()),
            "got: {report:?}"
        );
        assert!(
            files["f.rs"].contains("fn g(y: u32)"),
            "got: {}",
            files["f.rs"]
        );
    }

    #[test]
    fn untouched_live_field_use_fires_partial_with_orphan() {
        // Slice-3 headline: the native leaves `s.y` untouched while
        // deleting the field. The joint fires (probe passes — both sides
        // keep `s.y`); the engine reports the stranded `s.y` as an
        // `off-target` orphan, capping the run at `partial`.
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nfn f(s: Old) {\n    let _ = s.y;\n}\nfn g(t: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n@@ -5,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n@@ -8,1 +7,1 @@\n-fn g(t: Old) {\n+fn g(t: New) {\n";
        let out = infer(section, pre).expect("untouched live use must fire partial joint");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        assert!(out.contains("- y: u32,"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["f.rs"].contains("s.y"),
            "orphaned use retained, got: {}",
            files["f.rs"]
        );
        assert!(
            report.rules.iter().any(|r| !r.orphans.is_empty()),
            "deletion hunk must carry the stranded use, got: {report:?}"
        );
        let orphan = report
            .rules
            .iter()
            .flat_map(|r| r.orphans.iter())
            .find(|o| o.text.contains("s.y"))
            .expect("s.y orphan named");
        assert_eq!(orphan.reason, "off-target", "got: {orphan:?}");
    }

    #[test]
    fn another_container_deletion_declines_joint() {
        // The deletion belongs to `Other`: no joint.
        let pre =
            "struct Old {\n    x: u32,\n}\nstruct Other {\n    y: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -5 +4,0 @@\n-    y: u32,\n@@ -7,1 +6,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "another-container deletion never joints, got:\n{o}"
            );
        }
    }

    #[test]
    fn variant_deletion_declines_joint() {
        // An enum-variant deletion alongside a *struct* rename is
        // cross-kind: no deletion joint (the rename may still partition
        // alone; the deletion stays an ordinary leftover).
        let pre =
            "struct Old {\n    x: u32,\n}\nenum E {\n    A,\n    B,\n}\nfn f(s: Old, e: E) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -5 +4,0 @@\n-    B,\n@@ -7,1 +6,1 @@\n-fn f(s: Old, e: E) {\n+fn f(s: New, e: E) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "cross-kind deletion never joints; the rename may still partition alone, got:\n{o}"
            );
            assert!(!o.contains("field deletion"), "got:\n{o}");
        }
    }

    #[test]
    fn zero_use_joint_declines() {
        // Def + deletion with no rename use: the intent needs a real use.
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -3 +2,0 @@\n-    y: u32,\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert!(
                o.matches("rule=r1").count() <= 1,
                "zero-use deletion joint must decline, got:\n{o}"
            );
        }
    }

    #[test]
    fn enum_shape_a_joint_fires() {
        // `enum` rename + single variant deletion (slice 2).
        let pre = "enum Old {\n    A,\n    B,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -5,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre).expect("enum shape-A deletion joint must fire");
        assert!(out.contains("rule="), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("+ enum New"), "got:\n{out}");
        assert!(out.contains("- B,"), "got:\n{out}");
        assert!(!out.contains("+ B,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(files["f.rs"].contains("enum New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains('B'), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("v: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn enum_shape_b_joint_fires() {
        // Adjacent def + deletion merge into one unbalanced hunk.
        let pre = "enum Old {\n    B,\n    A,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1 @@\n-enum Old {\n-    B,\n+enum New {\n@@ -5 +4 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre).expect("enum shape-B deletion joint must fire");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("+ enum New"), "got:\n{out}");
        assert!(out.contains("- B,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(files["f.rs"].contains("enum New"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("v: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn enum_tuple_variant_joint_fires() {
        // Tuple-payload variant deletion fires like the bare shape.
        let pre = "enum Old {\n    A,\n    B(u32),\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B(u32),\n@@ -5,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre).expect("tuple variant deletion joint must fire");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("- B(u32),"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(!files["f.rs"].contains("B(u32)"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("enum New"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn two_variant_deletions_decline() {
        let pre = "enum Old {\n    A,\n    B,\n    C,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -4 +3,0 @@\n-    C,\n@@ -6,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "two variant deletions decline the joint, got:\n{o}"
            );
        }
    }

    #[test]
    fn live_variant_use_declines_joint() {
        // Untouched `Old::B` while the follow rewrites it to `New::B`:
        // the emission and the native disagree, so the D13 probe
        // declines. The partial shape carries the `Old::B → New::B`
        // rewrite (see the slice-3 pin below).
        let pre =
            "enum Old {\n    A,\n    B,\n}\nfn f() -> Old {\n    Old::B\n}\nfn g(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -7,1 +6,1 @@\n-fn g(v: Old) {\n+fn g(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "live-use variant deletion declines the joint, got:\n{o}"
            );
            assert!(!o.contains("- B,"), "got:\n{o}");
        }
    }

    #[test]
    fn unrelated_variant_ident_fires_verified() {
        // Slice-3 precision: an unrelated same-named `B` (a struct name
        // here) is a different namespace from the `Old::B` variant, so
        // the joint fires `verified` with zero orphans. The struct is
        // byte-identical on both sides; the probe passes.
        let pre = "enum Old {\n    A,\n    B,\n}\nstruct B {\n    x: u32,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -8,1 +7,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre).expect("unrelated struct B must not block the joint");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        assert!(out.contains("  enum Old {"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            report.rules.iter().all(|r| r.orphans.is_empty()),
            "got: {report:?}"
        );
        assert!(files["f.rs"].contains("struct B"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn rewritten_variant_use_fires_partial_with_orphan() {
        // Slice-3 variant headline: the native rewrites `Old::B` to
        // `New::B` as a claimable rename-use pair while deleting the
        // variant. The joint fires (probe passes — both sides carry
        // `New::B`); the engine reports the dangling `New::B` as an
        // orphan, capping the run at `partial`.
        let pre =
            "enum Old {\n    A,\n    B,\n}\nfn f() -> Old {\n    Old::B\n}\nfn g(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n@@ -5,1 +4,1 @@\n-fn f() -> Old {\n+fn f() -> New {\n@@ -6,1 +5,1 @@\n-    Old::B\n+    New::B\n@@ -8,1 +7,1 @@\n-fn g(v: Old) {\n+fn g(v: New) {\n";
        let out = infer(section, pre).expect("rewritten variant use must fire partial joint");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        assert!(out.contains("- B,"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        let report = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["f.rs"].contains("New::B"),
            "orphaned use retained, got: {}",
            files["f.rs"]
        );
        assert!(
            report.rules.iter().any(|r| !r.orphans.is_empty()),
            "deletion hunk must carry the stranded variant use, got: {report:?}"
        );
    }

    #[test]
    fn another_enum_container_deletion_declines_joint() {
        let pre = "enum Old {\n    A,\n}\nenum Other {\n    A,\n    B,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -5 +4,0 @@\n-    B,\n@@ -7,1 +6,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "another-container variant deletion never joints, got:\n{o}"
            );
        }
    }

    #[test]
    fn field_deletion_under_enum_rename_declines() {
        // A field-line deletion alongside an *enum* rename is
        // cross-kind: no deletion joint.
        let pre = "enum Old {\n    A,\n    B,\n}\nstruct S {\n    y: u32,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -5 +4,0 @@\n-    y: u32,\n@@ -7,1 +6,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "cross-kind deletion never joints, got:\n{o}"
            );
            assert!(!o.contains("- y: u32,"), "got:\n{o}");
        }
    }

    #[test]
    fn zero_use_variant_joint_declines() {
        let pre = "enum Old {\n    A,\n    B,\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -3 +2,0 @@\n-    B,\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert!(
                o.matches("rule=r1").count() <= 1,
                "zero-use variant joint must decline, got:\n{o}"
            );
        }
    }

    #[test]
    fn variant_emission_applies_jointly() {
        let pre = "enum Old {\n    A,\n    B,\n}\nfn f(v: Old) {\n}\n";
        let emission = variant_del_emit("a.rs", "r1", "enum", "", "Old", "New", 3, "B,");
        assert!(emission.contains("# follow=uses"), "got:\n{emission}");
        assert!(emission.contains("- enum Old\n"), "got:\n{emission}");
        assert!(emission.contains("+ enum New\n"), "got:\n{emission}");
        assert!(emission.contains("- B,"), "got:\n{emission}");
        assert_eq!(emission.matches("rule=r1").count(), 2, "got:\n{emission}");
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("a.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint emission applies");
        assert!(files["a.rs"].contains("enum New"), "got: {}", files["a.rs"]);
        assert!(
            !files["a.rs"].contains("\n    B,"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("fn f(v: New)"),
            "got: {}",
            files["a.rs"]
        );
    }

    #[test]
    fn emission_applies_jointly() {
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nfn f(s: Old) {\n}\n";
        let emission = del_emit("a.rs", "r1", "struct", "", "Old", "New", 3, "y: u32,");
        assert!(emission.contains("# follow=uses"), "got:\n{emission}");
        assert!(emission.contains("- struct Old\n"), "got:\n{emission}");
        assert!(emission.contains("+ struct New\n"), "got:\n{emission}");
        assert!(emission.contains("- y: u32,"), "got:\n{emission}");
        assert_eq!(emission.matches("rule=r1").count(), 2, "got:\n{emission}");
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("a.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint emission applies");
        assert!(
            files["a.rs"].contains("struct New"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["a.rs"].contains("y: u32"), "got: {}", files["a.rs"]);
        assert!(
            files["a.rs"].contains("fn f(s: New)"),
            "got: {}",
            files["a.rs"]
        );
    }
}
