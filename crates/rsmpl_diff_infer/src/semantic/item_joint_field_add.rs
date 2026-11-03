//! WI-5 slice 2b-next2b-first: struct rename + field-addition joint.
//!
//! The detection branch lives in the item-intent pre-pass (it owns the
//! pair/hunk tables); this module holds the pure helpers so the intent
//! file stays under the D5 line gate: the shape-A/shape-B classifiers,
//! the same-container carrier check (the insertion point's pre-image
//! carrier is the renamed struct), the shared-rule two-hunk emission
//! (addition hunk first, rename follow hunk second — both match against
//! original coordinates, so order is reader convention, not engine
//! requirement), and the multi-file D13 probe.
//!
//! Two native shapes raise. Shape A (distant): a balanced def-rename
//! hunk plus a pure-insertion (`@@ -N,0 @@`) field hunk in the same
//! file+section — today this partitions into a rename intent plus a
//! leftover ordinary insertion. Shape B (adjacent): def change and
//! insertion adjacent merge into one unbalanced (1-minus, 2-plus) hunk
//! whose minus/plus[0] is the def rename and plus[1] the field line —
//! today the pair model skips the hunk entirely so the section stays
//! wholly ordinary. Both raise to one shared-rule emission; decline
//! falls back to today's posture (partition for A, ordinary for B).
//!
//! Variant-addition alongside a rename never joints here (deferred to
//! 2b-next2b-next); a member in a different file than its container is
//! impossible by the container law (non-goal, pinned).
//!
//! WI-5 slice 2b-next2b-next: the enum half of the same ladder — an
//! `enum` rename plus exactly one variant-addition line (`B,`,
//! `B(u32),`, `B { x: u32 },`, `B = 3,`) in the same file and section
//! raises to the shared-rule two-hunk emission with the `enum Old {`
//! context. Field and variant finders are mutually exclusive by kind
//! (struct vs enum), so cross-kind members never joint.

/// One unbalanced native hunk entry (built by the item-intent
/// pre-pass; `hi` is the hunk-entry index in its table).
pub(crate) struct Unbal {
    pub(crate) hi: usize,
    pub(crate) sec: usize,
    pub(crate) path: String,
    pub(crate) minuses: Vec<String>,
    pub(crate) pluses: Vec<String>,
    pub(crate) text: String,
}

/// A decomposed shape-B hunk: (1 minus, 2 plus) with the def rename at
/// [0] and the field line at plus[1]. `hi` is the hunk-entry index
/// (== the virtual def pair's hunk, already claimed as the def hunk);
/// `point` is the old-side insertion line (the hunk header's old
/// start); `add_plus` the raw field line.
pub(crate) struct ShapeB {
    pub(crate) hi: usize,
    pub(crate) sec: usize,
    pub(crate) path: String,
    pub(crate) point: usize,
    pub(crate) add_plus: String,
}

/// The joint claim: `hi` is the insertion's hunk entry (shape A: the
/// pure-insertion hunk; shape B: the def hunk itself, already
/// claimed); `point` the `@@ -P,1 @@` range; `member_plus` the raw
/// field line; `shape_b` selects the virtual-def `claimed_idx` lift.
pub(crate) struct AddJoint {
    pub(crate) hi: usize,
    pub(crate) point: usize,
    pub(crate) member_plus: String,
    pub(crate) shape_b: bool,
}

/// Decompose a shape-B hunk, or `None`. Purely structural: exactly one
/// minus and two pluses; the caller (def discovery via `split_item`,
/// the joint finder via the field classifier + carrier law) decides
/// meaning. `point` is the header's old-side start (unbalanced hunks
/// carry no per-line sites).
pub(crate) fn decompose_shape_b(
    minuses: &[String],
    pluses: &[String],
    text: &str,
) -> Option<(String, String, String, usize)> {
    if minuses.len() != 1 || pluses.len() != 2 {
        return None;
    }
    let point = old_start(text)?;
    Some((
        minuses[0].clone(),
        pluses[0].clone(),
        pluses[1].clone(),
        point,
    ))
}

/// Old-side start of the first `@@ -start[,len] ... @@` header.
fn old_start(text: &str) -> Option<usize> {
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

/// Parse a field-addition line to its field name, or `None`. Mirrors
/// the WI-12b field branch (`name: Ty`, top-level colon, `syn::Type`
/// validation, `pub`-prefix tolerance) so the joint and the single
/// agree on what a field is; variant/attr/hetero lines decline.
pub(crate) fn parse_field_add(line: &str) -> Option<String> {
    let mut t = line.trim();
    t = t.strip_suffix(',').map(str::trim).unwrap_or(t);
    t = t.strip_suffix(';').map(str::trim).unwrap_or(t);
    if t.is_empty() || t.starts_with("#[") {
        return None;
    }
    if t.contains('(') || t.contains('{') || t.contains('=') || t.contains('$') {
        return None;
    }
    let head = t.split_whitespace().next().unwrap_or("");
    if matches!(
        head,
        "type" | "const" | "static" | "let" | "fn" | "enum" | "struct" | "union" | "trait" | "impl"
    ) && !head.starts_with("pub")
    {
        return None;
    }
    let at = find_field_colon(t)?;
    let (left, right) = t.split_at(at);
    let ty = right.strip_prefix(':')?.trim();
    if ty.is_empty() {
        return None;
    }
    let _: syn::Type = syn::parse_str(ty).ok()?;
    let left = left.trim();
    let mut parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let name = parts.pop()?.to_string();
    if !is_ident(&name) {
        return None;
    }
    if parts.is_empty() {
        return Some(name);
    }
    let vis = parts.join(" ");
    if vis == "pub" || vis.starts_with("pub(") || vis.starts_with("pub ") {
        return Some(name);
    }
    None
}

/// Single top-level colon (no `::` adjacency), bracket-aware.
fn find_field_colon(t: &str) -> Option<usize> {
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let bytes = t.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] as char {
            '(' => dp += 1,
            ')' => dp -= 1,
            '[' => db += 1,
            ']' => db -= 1,
            '{' => dbr += 1,
            '}' => dbr -= 1,
            '<' => da += 1,
            '>' => {
                if da > 0 {
                    da -= 1;
                }
            }
            ':' if dp == 0 && db == 0 && dbr == 0 && da == 0 => {
                let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
                let next = bytes.get(i + 1).copied();
                if prev != Some(b':') && next != Some(b':') {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && super::ident_len(s) == Some(s.len())
}

/// WI-5 joint search for the field-addition half: returns the joint
/// claim when the def section holds exactly one attached field
/// addition (shape A pure-insertion hunk or shape-B record, same file
/// and section, insertion-point carrier is the renamed struct) plus
/// claimable uses otherwise, with no other non-claimable balanced pair
/// in the def section. Struct-only, `fn` excluded, at most one joint
/// per def section (the caller runs this arm only when the attr and
/// evo arms decline). `None` means no joint — the caller falls back to
/// the pure-intent path (partition for shape A; the shape-B gate
/// declines shape-B sections to today's wholly-ordinary posture).
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_field_add_joint(
    pairs: &[super::item_intent::Pair],
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
    unbals: &[Unbal],
    shape_bs: &[ShapeB],
) -> Option<AddJoint> {
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
    let mut cands: Vec<AddJoint> = Vec::new();
    // Shape A: pure-insertion hunk in the def section.
    for u in unbals {
        if u.sec != def_sec || u.path != def_path {
            continue;
        }
        if !u.minuses.is_empty() || u.pluses.len() != 1 {
            continue;
        }
        let point = super::definition_addition::insertion_point(&u.text)?;
        let field = parse_field_add(&u.pluses[0])?;
        let pre_text = pre.get(&u.path)?;
        if super::definition_addition::struct_spanning(pre_text, &field, point)? != old {
            continue;
        }
        cands.push(AddJoint {
            hi: u.hi,
            point,
            member_plus: u.pluses[0].clone(),
            shape_b: false,
        });
    }
    // Shape B: decomposed record in the def section (its hunk holds
    // the def pair itself when it is the def hunk; a distinct-hunk
    // record rides the same section law as shape A).
    for b in shape_bs {
        if b.sec != def_sec || b.path != def_path {
            continue;
        }
        let field = parse_field_add(&b.add_plus)?;
        let pre_text = pre.get(&def_path)?;
        if super::definition_addition::struct_spanning(pre_text, &field, b.point)? != old {
            continue;
        }
        cands.push(AddJoint {
            hi: b.hi,
            point: b.point,
            member_plus: b.add_plus.clone(),
            shape_b: true,
        });
    }
    if cands.len() != 1 {
        return None;
    }
    cands.into_iter().next()
}

/// Parse a variant-addition line to its variant name, or `None`.
/// Mirrors the WI-12b variant branch (`variant_ident` + `syn::Variant`
/// validation) so the joint and the single agree on what a variant is;
/// field/attr/hetero lines decline (`y: u32` never parses as a Variant,
/// struct-payload colons ride inside the braces).
pub(crate) fn parse_variant_add(line: &str) -> Option<String> {
    let variant = super::definition_addition::variant_ident(line)?;
    if !is_ident(&variant) {
        return None;
    }
    let body = line
        .trim()
        .trim_end_matches(',')
        .trim_end()
        .trim_end_matches(';')
        .trim();
    if body.is_empty() {
        return None;
    }
    let _: syn::Variant = syn::parse_str(body).ok()?;
    Some(variant)
}

/// WI-5 joint search for the variant-addition half: returns the joint
/// claim when the def section holds exactly one attached variant
/// addition (shape A pure-insertion hunk or shape-B record, same file
/// and section, insertion-point carrier is the renamed enum) plus
/// claimable uses otherwise, with no other non-claimable balanced pair
/// in the def section. Enum-only, `fn` excluded, at most one joint
/// per def section (the caller runs this arm only when the attr, evo,
/// and field-add arms decline). `None` means no joint.
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_variant_add_joint(
    pairs: &[super::item_intent::Pair],
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
    unbals: &[Unbal],
    shape_bs: &[ShapeB],
) -> Option<AddJoint> {
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
    let mut cands: Vec<AddJoint> = Vec::new();
    // Shape A: pure-insertion hunk in the def section.
    for u in unbals {
        if u.sec != def_sec || u.path != def_path {
            continue;
        }
        if !u.minuses.is_empty() || u.pluses.len() != 1 {
            continue;
        }
        let point = super::definition_addition::insertion_point(&u.text)?;
        let variant = parse_variant_add(&u.pluses[0])?;
        let pre_text = pre.get(&u.path)?;
        if super::definition_addition::enum_spanning(pre_text, &variant, point)? != old {
            continue;
        }
        cands.push(AddJoint {
            hi: u.hi,
            point,
            member_plus: u.pluses[0].clone(),
            shape_b: false,
        });
    }
    // Shape B: decomposed record in the def section.
    for b in shape_bs {
        if b.sec != def_sec || b.path != def_path {
            continue;
        }
        let variant = parse_variant_add(&b.add_plus)?;
        let pre_text = pre.get(&def_path)?;
        if super::definition_addition::enum_spanning(pre_text, &variant, b.point)? != old {
            continue;
        }
        cands.push(AddJoint {
            hi: b.hi,
            point: b.point,
            member_plus: b.add_plus.clone(),
            shape_b: true,
        });
    }
    if cands.len() != 1 {
        return None;
    }
    cands.into_iter().next()
}
/// Shared-rule two-hunk emission: the WI-12b addition hunk first (its
/// `struct Old {` context matches the pre tree), the terse rename
/// follow hunk second. No new `PatternKind`: the engine lowers the
/// addition hunk as `definition_addition` and the rename hunk as
/// `item_rename` with follow; the joint lives in the shared rule name
/// and the single D13 probe.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
    point: usize,
    member_plus: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    let mut member = member_plus.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    out.push_str(&format!("@@ -{point},1 @@ rule={rule}\n"));
    out.push_str(&format!("  struct {old} {{\n"));
    out.push_str(&format!("+ {member}\n"));
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    out
}

/// Shared-rule two-hunk emission for the enum half: the WI-12b
/// enum-context addition hunk (`enum Old {`) first, the terse rename
/// follow hunk second. Same engine lowering as the struct half
/// (`definition_addition` + `item_rename` with follow).
#[allow(clippy::too_many_arguments)]
pub(crate) fn variant_add_emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
    point: usize,
    member_plus: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    let mut member = member_plus.trim().to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    out.push_str(&format!("@@ -{point},1 @@ rule={rule}\n"));
    out.push_str(&format!("  enum {old} {{\n"));
    out.push_str(&format!("+ {member}\n"));
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    out
}

/// D13 probe for the add joint: the composed emission applied to a
/// `pre` clone must reproduce, on every file, the tree that applying
/// the claimed native hunks (per path, original order) produces.
/// Missed sites fail the plus leg; overwritten twins fail the
/// unchanged leg; zero-match hunks fail at apply time (no `# optional`
/// anywhere in the emission).
pub(crate) fn probe_add_joint(
    pre: &std::collections::HashMap<String, String>,
    emission: &str,
    claimed: &std::collections::HashMap<String, Vec<String>>,
) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(emission) else {
        return false;
    };
    let mut got = pre.clone();
    if rsmpl_engine::apply_patch_to_tree(&mut got, &parsed).is_err() {
        return false;
    }
    for (path, pre_text) in pre {
        let mut exp = std::collections::HashMap::from([(path.clone(), pre_text.clone())]);
        if let Some(texts) = claimed.get(path) {
            if !texts.is_empty() {
                let mut sec = format!("{}\n", crate::git_header(path));
                for t in texts {
                    sec.push_str(t);
                }
                let Ok(native) = rsmpl_patch::parse_patch(&sec) else {
                    return false;
                };
                if rsmpl_engine::apply_patch_to_tree(&mut exp, &native).is_err() {
                    return false;
                }
            }
        }
        if got.get(path) != exp.get(path) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompose_matrix() {
        // Shape B: 1 minus, 2 plus.
        let text = "@@ -1 +1,2 @@\n-struct Old {\n+struct New {\n+    y: u32,\n";
        let (dm, dp, add, point) = decompose_shape_b(
            &["struct Old {".to_string()],
            &["struct New {".to_string(), "    y: u32,".to_string()],
            text,
        )
        .expect("shape B must decompose");
        assert_eq!(dm, "struct Old {");
        assert_eq!(dp, "struct New {");
        assert_eq!(add, "    y: u32,");
        assert_eq!(point, 1);
        // Balanced: not shape B.
        assert!(decompose_shape_b(
            &["a".to_string()],
            &["b".to_string()],
            "@@ -1,1 +1,1 @@\n-a\n+b\n"
        )
        .is_none());
        // Pure insertion: not shape B (no minus).
        assert!(decompose_shape_b(
            &[],
            &["    y: u32,".to_string()],
            "@@ -2,0 +3 @@\n+    y: u32,\n"
        )
        .is_none());
        // Three pluses: not shape B.
        assert!(decompose_shape_b(
            &["a".to_string()],
            &["b".to_string(), "c".to_string(), "d".to_string()],
            "@@ -1 +1,3 @@\n"
        )
        .is_none());
    }

    #[test]
    fn field_add_matrix() {
        assert_eq!(parse_field_add("y: u32").as_deref(), Some("y"));
        assert_eq!(parse_field_add("    y: u32,").as_deref(), Some("y"));
        assert_eq!(parse_field_add("pub y: u32").as_deref(), Some("y"));
        // Variant lines decline.
        assert!(parse_field_add("B,").is_none());
        assert!(parse_field_add("B(u32),").is_none());
        assert!(parse_field_add("A = 1,").is_none());
        // Attr lines decline.
        assert!(parse_field_add("#[derive(Debug)]").is_none());
        // Rename/evo lines decline (two-sided is never an addition).
        assert!(parse_field_add("x: u32").is_some());
        // Item heads decline.
        assert!(parse_field_add("type A = Old;").is_none());
        // Unparseable type declines.
        assert!(parse_field_add("y: (").is_none());
    }

    #[test]
    fn emission_applies_jointly() {
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let emission = add_emit("a.rs", "r1", "struct", "", "Old", "New", 2, "y: u32,");
        assert!(emission.contains("# follow=uses"), "got:\n{emission}");
        assert!(emission.contains("- struct Old\n"), "got:\n{emission}");
        assert!(emission.contains("+ struct New\n"), "got:\n{emission}");
        assert!(emission.contains("+ y: u32,"), "got:\n{emission}");
        // Shared rule across both hunks.
        assert_eq!(emission.matches("rule=r1").count(), 2, "got:\n{emission}");
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("a.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint emission applies");
        assert!(
            files["a.rs"].contains("struct New"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["a.rs"].contains("y: u32"), "got: {}", files["a.rs"]);
        assert!(
            files["a.rs"].contains("fn f(s: New)"),
            "got: {}",
            files["a.rs"]
        );
    }

    fn infer(section: &str, pre: &str) -> Option<String> {
        super::super::infer_semantic_with_pre_images(
            std::slice::from_ref(&section.to_string()),
            &std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]),
        )
    }

    #[test]
    fn shape_a_joint_fires() {
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -2,0 +3 @@\n+    y: u32,\n@@ -4,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre).expect("shape-A joint must fire");
        assert!(out.contains("rule="), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        assert!(out.contains("+ struct New"), "got:\n{out}");
        assert!(out.contains("+ y: u32,"), "got:\n{out}");
        // Shared rule r1 on both hunks (the intent pre-pass reserves
        // first); later detectors keep leftovers, never a second r1.
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["f.rs"].contains("struct New"),
            "got: {}",
            files["f.rs"]
        );
        assert!(files["f.rs"].contains("y: u32"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("s: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn shape_b_joint_fires() {
        // Adjacent def + insertion merge into one unbalanced hunk.
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1,2 @@\n-struct Old {\n+struct New {\n+    y: u32,\n@@ -4 +5 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre).expect("shape-B joint must fire");
        assert!(out.contains("- struct Old"), "got:\n{out}");
        assert!(out.contains("+ struct New"), "got:\n{out}");
        assert!(out.contains("+ y: u32,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["f.rs"].contains("struct New"),
            "got: {}",
            files["f.rs"]
        );
        assert!(files["f.rs"].contains("y: u32"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("s: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn two_additions_decline() {
        // Two insertions: single-addition law declines the joint — the
        // section keeps today's partition (rename intent + ordinary
        // insertions), never a half joint.
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -2,0 +3 @@\n+    y: u32,\n@@ -2,0 +4 @@\n+    z: u32,\n@@ -4,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            // Joint ⟺ r1 shared by two hunks; anything else (pure
            // rename partition, addition singles) reserves r1 at most
            // once per hunk with distinct names.
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "two additions decline the joint; the rename may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn another_container_addition_declines_joint() {
        // The insertion belongs to `Other`: no joint. Today's partition
        // posture holds (pure rename may claim def+use).
        let pre =
            "struct Old {\n    x: u32,\n}\nstruct Other {\n    a: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -5,0 +6 @@\n+    b: u32,\n@@ -7,1 +7,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "another-container addition never joints; the rename may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn variant_insertion_declines_joint() {
        // A variant insertion alongside the rename is the deferred
        // slice: no joint, today's posture (wholly ordinary or
        // partitioned) holds.
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -2,0 +3 @@\n+    B,\n@@ -4,1 +4,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "variant insertion never joints; the rename may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn shape_b_variant_insertion_stays_ordinary() {
        // Adjacent def + variant insertion decomposes structurally, but
        // the joint finder declines the variant line and the shape-B
        // gate keeps the section wholly ordinary (today's posture —
        // the deferred slice owns variant joints).
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1,2 @@\n-struct Old {\n+struct New {\n+    B,\n@@ -4 +5 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("rule="),
            "shape-B variant insertion must stay wholly ordinary, got:\n{}",
            out.unwrap_or_default()
        );
    }

    #[test]
    fn zero_use_joint_declines() {
        // Def + addition with no rename use: one pair only — the intent
        // needs a real use, so no joint (the lone insertion may still
        // raise its own WI-12b single under r1).
        let pre = "struct Old {\n    x: u32,\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -2,0 +3 @@\n+    y: u32,\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert!(
                o.matches("rule=r1").count() <= 1,
                "zero-use joint must decline, got:\n{o}"
            );
        }
    }

    #[test]
    fn fn_def_with_insertion_declines() {
        // `fn` rename + insertion belongs to WI-4: the item joint
        // declines, today's posture holds.
        let pre = "fn foo(x: u32) -> u32 {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-fn foo(x: u32) -> u32 {\n+fn bar(x: u32) -> u32 {\n@@ -2,0 +3 @@\n+    y: u32,\n@@ -5,1 +5,1 @@\n-    foo(1);\n+    bar(1);\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "fn defs never item-add-joint-fire; the fn intent may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn cross_file_use_joint_fires() {
        // Def + addition in one file, the rename use in another: the
        // workspace claim machinery carries it as one joint.
        let sections = [
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -2,0 +3 @@\n+    y: u32,\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-fn f(s: crate::a::Old) {\n+fn f(s: crate::a::New) {\n"
                .to_string(),
        ];
        let pre = std::collections::HashMap::from([
            (
                "a.rs".to_string(),
                "struct Old {\n    x: u32,\n}\n".to_string(),
            ),
            (
                "b.rs".to_string(),
                "fn f(s: crate::a::Old) {\n}\n".to_string(),
            ),
        ]);
        let out = super::super::infer_semantic_with_pre_images(&sections, &pre)
            .expect("cross-file joint must fire");
        assert!(out.contains("+ y: u32,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(
            files["a.rs"].contains("struct New"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["a.rs"].contains("y: u32"), "got: {}", files["a.rs"]);
        assert!(
            files["b.rs"].contains("crate::a::New"),
            "got: {}",
            files["b.rs"]
        );
        assert!(!files["a.rs"].contains("Old"), "got: {}", files["a.rs"]);
    }

    #[test]
    fn variant_add_matrix() {
        assert_eq!(parse_variant_add("B,").as_deref(), Some("B"));
        assert_eq!(parse_variant_add("B(u32),").as_deref(), Some("B"));
        assert_eq!(parse_variant_add("B { x: u32 },").as_deref(), Some("B"));
        assert_eq!(parse_variant_add("B = 3,").as_deref(), Some("B"));
        // Field lines decline the variant half (cross-kind never joints).
        assert!(parse_variant_add("y: u32,").is_none());
        assert!(parse_variant_add("pub y: u32,").is_none());
        // Attr / hetero lines decline.
        assert!(parse_variant_add("#[derive(Debug)]").is_none());
        assert!(parse_variant_add("y + 1,").is_none());
        assert!(parse_variant_add("y: (").is_none());
    }

    #[test]
    fn variant_shape_a_joint_fires() {
        let pre = "enum Old {\n    A,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -2,0 +3 @@\n+    B,\n@@ -4,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre).expect("variant shape-A joint must fire");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("+ enum New"), "got:\n{out}");
        assert!(out.contains("enum Old {"), "got:\n{out}");
        assert!(out.contains("+ B,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(files["f.rs"].contains("enum New"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("B,"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("v: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn variant_shape_b_joint_fires() {
        // Adjacent def + insertion merge into one unbalanced hunk.
        let pre = "enum Old {\n    A,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1,2 @@\n-enum Old {\n+enum New {\n+    B,\n@@ -4 +5 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre).expect("variant shape-B joint must fire");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("+ enum New"), "got:\n{out}");
        assert!(out.contains("+ B,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(files["f.rs"].contains("enum New"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("B,"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn variant_payload_shape_a_fires() {
        // Tuple-payload and discriminant additions ride the same ladder.
        for add in ["B(u32),", "B = 3,"] {
            let pre = "enum Old {\n    A,\n}\nfn f(v: Old) {\n}\n";
            let section = format!(
                "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {{\n+enum New {{\n@@ -2,0 +3 @@\n+    {add}\n@@ -4,1 +4,1 @@\n-fn f(v: Old) {{\n+fn f(v: New) {{\n"
            );
            let out = infer(&section, pre).expect("payload variant must fire");
            assert_eq!(out.matches("rule=r1").count(), 2, "add={add} got:\n{out}");
        }
    }

    #[test]
    fn variant_cross_file_use_joint_fires() {
        // Def + addition in one file, the rename use in another.
        let sections = [
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -2,0 +3 @@\n+    B,\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1,1 +1,1 @@\n-fn f(v: crate::a::Old) {\n+fn f(v: crate::a::New) {\n"
                .to_string(),
        ];
        let pre = std::collections::HashMap::from([
            ("a.rs".to_string(), "enum Old {\n    A,\n}\n".to_string()),
            (
                "b.rs".to_string(),
                "fn f(v: crate::a::Old) {\n}\n".to_string(),
            ),
        ]);
        let out = super::super::infer_semantic_with_pre_images(&sections, &pre)
            .expect("variant cross-file joint must fire");
        assert!(out.contains("+ B,"), "got:\n{out}");
        assert_eq!(out.matches("rule=r1").count(), 2, "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(files["a.rs"].contains("enum New"), "got: {}", files["a.rs"]);
        assert!(files["a.rs"].contains("B,"), "got: {}", files["a.rs"]);
        assert!(
            files["b.rs"].contains("crate::a::New"),
            "got: {}",
            files["b.rs"]
        );
    }

    #[test]
    fn field_line_beside_enum_rename_declines_joint() {
        // Cross-kind member: enum rename + field line never joints (both
        // finders decline — the rename may still partition alone).
        let pre = "enum Old {\n    A,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -2,0 +3 @@\n+    y: u32,\n@@ -4,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "cross-kind member never joints; the rename may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn two_variant_additions_decline() {
        let pre = "enum Old {\n    A,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -2,0 +3 @@\n+    B,\n@@ -2,0 +4 @@\n+    C,\n@@ -4,1 +4,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "two additions decline the joint; the rename may still partition alone, got:\n{o}"
            );
        }
    }

    #[test]
    fn variant_another_container_declines_joint() {
        // The insertion belongs to `Other`: no joint.
        let pre = "enum Old {\n    A,\n}\nenum Other {\n    X,\n}\nfn f(v: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -5,0 +6 @@\n+    Y,\n@@ -7,1 +7,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "another-container addition never joints, got:\n{o}"
            );
        }
    }

    #[test]
    fn variant_zero_use_declines() {
        let pre = "enum Old {\n    A,\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -2,0 +3 @@\n+    B,\n";
        let out = infer(section, pre);
        if let Some(o) = out {
            assert!(
                o.matches("rule=r1").count() <= 1,
                "zero-use variant joint must decline, got:\n{o}"
            );
        }
    }

    #[test]
    fn cross_file_member_never_joints() {
        // A member in another file than its container cannot joint
        // (container law non-goal): the def rename lives in `a.rs` while
        // the insertion's carrier (`Other`) lives in `b.rs` — at most the
        // pure rename partitions, never a shared-rule joint.
        let sections = [
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -1,1 +1,1 @@\n-fn f(s: Old) {\n+fn f(s: New) {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3 @@\n+    b: u32,\n"
                .to_string(),
        ];
        let pre = std::collections::HashMap::from([
            (
                "a.rs".to_string(),
                "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n".to_string(),
            ),
            (
                "b.rs".to_string(),
                "struct Other {\n    a: u32,\n}\n".to_string(),
            ),
        ]);
        let out = super::super::infer_semantic_with_pre_images(&sections, &pre);
        if let Some(o) = out {
            assert_eq!(
                o.matches("rule=r1").count(),
                1,
                "cross-file member never joints; the rename may still partition alone, got:\n{o}"
            );
            assert!(!o.contains("+ b: u32,"), "got:\n{o}");
        }
        // Enum half of the same law.
        let sections = [
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-enum Old {\n+enum New {\n@@ -1,1 +1,1 @@\n-fn f(v: Old) {\n+fn f(v: New) {\n"
                .to_string(),
            "diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -2,0 +3 @@\n+    Y,\n"
                .to_string(),
        ];
        let pre = std::collections::HashMap::from([
            (
                "a.rs".to_string(),
                "enum Old {\n    A,\n}\nfn f(v: Old) {\n}\n".to_string(),
            ),
            ("b.rs".to_string(), "enum Other {\n    X,\n}\n".to_string()),
        ]);
        let out = super::super::infer_semantic_with_pre_images(&sections, &pre);
        if let Some(o) = out {
            // WI-2b: the `+ Y,` insertion is a proved lone single
            // (container recovery + scratch probe), so the normalized
            // span admits it exactly at the boundary (40 <= 24+16) —
            // no shared rule with the rename, hence no cross-file
            // joint, just two distinct singles.
            assert_eq!(
                o.matches("rule=").count(),
                2,
                "rename plus lone insertion single, got:\n{o}"
            );
            assert!(
                o.contains("rule=r1"),
                "rename keeps its own rule, got:\n{o}"
            );
            assert!(
                o.contains("rule=r2"),
                "insertion raises under its own rule, got:\n{o}"
            );
            assert!(
                o.contains("  enum Other {\n+ Y,\n"),
                "lone insertion single shape, got:\n{o}"
            );
        }
    }
}
