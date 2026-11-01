//! WI-5 slice 2b (first) + 2b-next: item rename + member-evolution joint.
//!
//! The detection branch lives in the item-intent pre-pass (it owns the
//! pair/hunk tables); this module holds the pure helpers so the intent
//! file stays under the D5 line gate: the field-evo and variant-evo
//! classifiers (same member, types/payload differ, both parse), the
//! same-container carrier checks (the member's pre-image carrier is the
//! renamed struct/enum), and the joint follow emission (def pair above
//! the member pair, `# sweep` + `# follow=uses` — the rename half owns
//! uses; pure evolutions stay C17).

/// A parsed field-evolution pair: same field, differing concrete types.
pub(crate) struct FieldEvo {
    /// Field name (concrete, preserved).
    pub(crate) field: String,
}

/// Parse one balanced pair as a field-type evolution, or `None`.
pub(crate) fn parse_field_evo_pair(minus: &str, plus: &str) -> Option<FieldEvo> {
    let (m_vis, m_name, m_ty) = split_field(minus)?;
    let (p_vis, p_name, p_ty) = split_field(plus)?;
    if m_vis != p_vis || m_name != p_name {
        return None;
    }
    if m_name.is_empty() || !is_ident(&m_name) {
        return None;
    }
    // Variant-shaped lines never reach here (split_field rejects
    // parens/braces/`=`), alias/const lines carry `=`/`type` heads.
    if m_ty.contains('$') || p_ty.contains('$') {
        return None;
    }
    let m_parsed: syn::Type = syn::parse_str(&m_ty).ok()?;
    let p_parsed: syn::Type = syn::parse_str(&p_ty).ok()?;
    let norm = |t: &syn::Type| {
        quote::quote!(#t)
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let m_norm = norm(&m_parsed);
    if m_norm == norm(&p_parsed) {
        return None;
    }
    Some(FieldEvo { field: m_name })
}

/// The `struct S` carrying field `field` at 1-based `site`, or `None`
/// under the exactly-one-carrier rule (the `definition_evolution`
/// discipline: a second same-named field anywhere vetoes).
pub(crate) fn field_carrier(pre: &str, field: &str, site: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut at_site: Vec<String> = Vec::new();
    let mut carriers = 0usize;
    fn walk(
        items: &[syn::Item],
        field: &str,
        site: usize,
        at_site: &mut Vec<String>,
        carriers: &mut usize,
    ) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Struct(s) => {
                    if let syn::Fields::Named(fields) = &s.fields {
                        let mut has = false;
                        for f in &fields.named {
                            if f.ident.as_ref().is_some_and(|i| i == field) {
                                has = true;
                                if f.span().start().line == site {
                                    at_site.push(s.ident.to_string());
                                }
                            }
                        }
                        if has {
                            *carriers += 1;
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, field, site, at_site, carriers);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, field, site, &mut at_site, &mut carriers);
    if carriers != 1 || at_site.len() != 1 {
        return None;
    }
    at_site.into_iter().next()
}

/// WI-5 joint search over the pre-pass pair/hunk tables: returns the
/// field-evo pair index when the def hunk holds exactly one attached
/// field-evo pair (same file and section, carrier is the renamed
/// struct) plus claimable uses otherwise, with no other joint-grade
/// pair in the def hunk. `pair_claimable` is the caller's gate (it owns
/// the fn/item distinction); `is_fn` excludes `fn` defs outright (WI-4
/// owns fn rename+evolve). `def_kind`/`old` gate the container law
/// (struct-only). `None` means no joint — the caller falls back to the
/// pure-intent path (or declines on a real co-resident veto).
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_field_evo_joint(
    pairs: &[super::item_intent::Pair],
    hunk_pairs: &[Vec<usize>],
    def_hunk: usize,
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
) -> Option<usize> {
    if is_fn || def_kind != "struct" {
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
    // Same file and section as the definition (one native hunk, the
    // flip-target shape; cross-hunk joints are a follow-up).
    if pairs[pi].path != pairs[def_idx].path || pairs[pi].sec != pairs[def_idx].sec {
        return None;
    }
    let evo = parse_field_evo_pair(&pairs[pi].minus, &pairs[pi].plus)?;
    // Same-container law: the field's carrier is the renamed struct.
    let pre_text = pre.get(&pairs[pi].path)?;
    if field_carrier(pre_text, &evo.field, pairs[pi].site)? != old {
        return None;
    }
    Some(pi)
}

/// A parsed variant-evolution pair: same variant, single-region delta.
pub(crate) struct VariantEvo {
    /// Variant name (concrete, preserved).
    pub(crate) variant: String,
}

/// Parse one balanced pair as a variant evolution (tuple payload,
/// struct payload, or discriminant), or `None`. Bare renames,
/// field-type lines, insertions, and hetero shapes decline.
pub(crate) fn parse_variant_evo_pair(minus: &str, plus: &str) -> Option<VariantEvo> {
    let mut m = minus.trim();
    let mut p = plus.trim();
    m = m.strip_suffix(',').map(str::trim).unwrap_or(m);
    p = p.strip_suffix(',').map(str::trim).unwrap_or(p);
    m = m.strip_suffix(';').map(str::trim).unwrap_or(m);
    p = p.strip_suffix(';').map(str::trim).unwrap_or(p);
    if m.is_empty() || p.is_empty() || m.starts_with("#[") || p.starts_with("#[") {
        return None;
    }
    // Field lines are the struct joint's domain (except struct-payload
    // members, which carry both `{` and `:`).
    if (m.contains(':') || p.contains(':')) && !(m.contains('{') && p.contains('{')) {
        return None;
    }
    if m.contains('$') || p.contains('$') {
        return None;
    }
    let (m_ident, m_tail) = split_variant(m)?;
    let (p_ident, p_tail) = split_variant(p)?;
    if m_ident != p_ident || m_ident.is_empty() || !is_ident(&m_ident) {
        return None;
    }
    let mt = m_tail.trim();
    let pt = p_tail.trim();
    // Discriminant: `= expr` both sides, exprs differ, both parse.
    if mt.starts_with('=') && pt.starts_with('=') {
        let me = mt.strip_prefix('=').unwrap_or("").trim();
        let pe = pt.strip_prefix('=').unwrap_or("").trim();
        if me.is_empty() || pe.is_empty() || me == pe {
            return None;
        }
        let _: syn::Expr = syn::parse_str(me).ok()?;
        let _: syn::Expr = syn::parse_str(pe).ok()?;
        return Some(VariantEvo { variant: m_ident });
    }
    // Tuple payload: `(…)` both sides, arity-equal, ≥1 differs.
    if mt.starts_with('(') && pt.starts_with('(') {
        let mi = mt.strip_prefix('(')?.strip_suffix(')')?;
        let pi = pt.strip_prefix('(')?.strip_suffix(')')?;
        let m_tys = split_top_commas(mi)?;
        let p_tys = split_top_commas(pi)?;
        if m_tys.len() != p_tys.len() || m_tys.is_empty() || m_tys == p_tys {
            return None;
        }
        for t in m_tys.iter().chain(p_tys.iter()) {
            let _: syn::Type = syn::parse_str(t).ok()?;
        }
        return Some(VariantEvo { variant: m_ident });
    }
    // Struct payload: `{ f: T, … }` both sides, names equal, ≥1 differs.
    if mt.starts_with('{') && pt.starts_with('{') {
        let mi = mt.strip_prefix('{')?.strip_suffix('}')?;
        let pi = pt.strip_prefix('{')?.strip_suffix('}')?;
        let m_fs = split_struct_fields(mi)?;
        let p_fs = split_struct_fields(pi)?;
        if m_fs.len() != p_fs.len() || m_fs.is_empty() {
            return None;
        }
        let m_names: Vec<&str> = m_fs.iter().map(|(n, _)| n.as_str()).collect();
        let p_names: Vec<&str> = p_fs.iter().map(|(n, _)| n.as_str()).collect();
        if m_names != p_names {
            return None;
        }
        let m_tys: Vec<&str> = m_fs.iter().map(|(_, t)| t.as_str()).collect();
        let p_tys: Vec<&str> = p_fs.iter().map(|(_, t)| t.as_str()).collect();
        if m_tys == p_tys {
            return None;
        }
        for t in m_tys.iter().chain(p_tys.iter()) {
            let _: syn::Type = syn::parse_str(t).ok()?;
        }
        return Some(VariantEvo { variant: m_ident });
    }
    None
}

/// The `enum E` carrying `variant` at 1-based `site`, or `None`
/// under the exactly-one-carrier rule (mirrors `field_carrier`).
pub(crate) fn enum_of_variant(pre: &str, variant: &str, site: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut at_site: Vec<String> = Vec::new();
    let mut carriers = 0usize;
    fn walk(
        items: &[syn::Item],
        variant: &str,
        site: usize,
        at_site: &mut Vec<String>,
        carriers: &mut usize,
    ) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Enum(e) => {
                    let mut has = false;
                    for v in &e.variants {
                        if v.ident == variant {
                            has = true;
                            if v.span().start().line == site {
                                at_site.push(e.ident.to_string());
                            }
                        }
                    }
                    if has {
                        *carriers += 1;
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, variant, site, at_site, carriers);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, variant, site, &mut at_site, &mut carriers);
    if carriers != 1 || at_site.len() != 1 {
        return None;
    }
    at_site.into_iter().next()
}

/// WI-5 joint search for the enum half: returns the variant-evo pair
/// index when the def hunk holds exactly one attached variant-evo pair
/// (same file and section, carrier is the renamed enum) plus claimable
/// uses otherwise. Mirrors `find_field_evo_joint` (`enum`-only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn find_variant_evo_joint(
    pairs: &[super::item_intent::Pair],
    hunk_pairs: &[Vec<usize>],
    def_hunk: usize,
    def_idx: usize,
    is_fn: bool,
    def_kind: &str,
    old: &str,
    pre: &std::collections::HashMap<String, String>,
    pair_claimable: &dyn Fn(&super::item_intent::Pair) -> bool,
) -> Option<usize> {
    if is_fn || def_kind != "enum" {
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
    if pairs[pi].path != pairs[def_idx].path || pairs[pi].sec != pairs[def_idx].sec {
        return None;
    }
    let evo = parse_variant_evo_pair(&pairs[pi].minus, &pairs[pi].plus)?;
    let pre_text = pre.get(&pairs[pi].path)?;
    if enum_of_variant(pre_text, &evo.variant, pairs[pi].site)? != old {
        return None;
    }
    Some(pi)
}

/// Split `Variant` + tail (`(…)`, `{ … }`, `= …`, or bare).
fn split_variant(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    let len = t
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    if len == 0 {
        return None;
    }
    let (ident, tail) = t.split_at(len);
    Some((ident.to_string(), tail.trim().to_string()))
}

/// Top-level comma split (concrete types; `<…>`/`(…)`/`[…]` nest).
fn split_top_commas(s: &str) -> Option<Vec<String>> {
    if s.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '<' | '(' | '[' => {
                depth += 1;
                cur.push(c);
            }
            '>' | ')' | ']' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    Some(out)
}

/// Split `{ f: T, … }` inner into (name, type) pairs.
fn split_struct_fields(s: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let flush = |seg: &str, out: &mut Vec<(String, String)>| -> Option<()> {
        let seg = seg.trim();
        if seg.is_empty() {
            return Some(());
        }
        let colon = seg.find(':')?;
        let (n, t) = seg.split_at(colon);
        let t = t.strip_prefix(':')?.trim();
        if !is_ident(n.trim()) || t.is_empty() {
            return None;
        }
        out.push((n.trim().to_string(), t.to_string()));
        Some(())
    };
    for c in s.chars() {
        match c {
            '<' | '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            '>' | ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                let seg = cur.clone();
                flush(&seg, &mut out)?;
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        let seg = cur.clone();
        flush(&seg, &mut out)?;
    }
    Some(out)
}

/// Joint follow emission: the terse def pair above the member pair.
/// `kind`/`prefix`/`old`/`new` are the `split_item` def parts; the
/// member lines ride raw (trimmed on emit, commas preserved).
#[allow(clippy::too_many_arguments)]
pub(crate) fn joint_emit(
    path: &str,
    rule: &str,
    kind: &str,
    prefix: &str,
    old: &str,
    new: &str,
    member_minus: &str,
    member_plus: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {prefix}{kind} {old}\n+ {prefix}{kind} {new}\n"));
    out.push_str(&format!(
        "- {}\n+ {}\n",
        member_minus.trim(),
        member_plus.trim()
    ));
    out
}

/// Split `vis name: Ty` on the single top-level colon. Returns
/// (vis, name, ty). Rejects variant/alias/const shapes.
fn split_field(line: &str) -> Option<(String, String, String)> {
    let mut t = line.trim();
    t = t.strip_suffix(',').map(str::trim).unwrap_or(t);
    t = t.strip_suffix(';').map(str::trim).unwrap_or(t);
    if t.is_empty() || t.starts_with("#[") {
        return None;
    }
    if t.contains('(') || t.contains('{') || t.contains('=') {
        return None;
    }
    // A `type`/`const`/`static`/`let`/`fn` head is never a field line.
    let head = t.split_whitespace().next().unwrap_or("");
    if matches!(
        head,
        "type" | "const" | "static" | "let" | "fn" | "enum" | "struct" | "union" | "trait" | "impl"
    ) && !head.starts_with("pub")
    {
        return None;
    }
    let colon = find_field_colon(t)?;
    let (left, right) = t.split_at(colon);
    let ty = right.strip_prefix(':')?.trim();
    if ty.is_empty() {
        return None;
    }
    let left = left.trim();
    if left.is_empty() || left.contains(['{', '}', '(', ')', ';', '=']) {
        return None;
    }
    let mut parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let name = parts.pop()?.to_string();
    let vis = if parts.is_empty() {
        String::new()
    } else {
        let v = parts.join(" ");
        if v == "pub" || v.starts_with("pub(") || v.starts_with("pub ") {
            v
        } else {
            return None;
        }
    };
    Some((vis, name, ty.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_pair_matrix() {
        assert!(parse_field_evo_pair("x: u32", "x: u64").is_some());
        assert!(parse_field_evo_pair("    x: u32,", "    x: u64,").is_some());
        assert!(parse_field_evo_pair("pub x: u32", "pub x: u64").is_some());
        // Same type: not an evolution.
        assert!(parse_field_evo_pair("x: u32", "x: u32").is_none());
        // Rename: not an evolution.
        assert!(parse_field_evo_pair("x: u32", "y: u32").is_none());
        // Variant/alias/const shapes decline.
        assert!(parse_field_evo_pair("Old(u32)", "Old(u64)").is_none());
        assert!(parse_field_evo_pair("type A = Vec<u8>;", "type A = Box<[u8]>;").is_none());
        assert!(parse_field_evo_pair("const MAX: i32 = 1;", "const MAX: i32 = 2;").is_none());
        // Vis mismatch declines.
        assert!(parse_field_evo_pair("x: u32", "pub x: u64").is_none());
        // Unparseable type declines.
        assert!(parse_field_evo_pair("x: (", "x: u64").is_none());
    }

    #[test]
    fn carrier_matrix() {
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nstruct Other {\n    x: u32,\n}\n";
        // Two same-named fields: no unique carrier.
        assert_eq!(field_carrier(pre, "x", 2), None);
        // Unique field resolves.
        assert_eq!(field_carrier(pre, "y", 3).as_deref(), Some("Old"));
        // Wrong site: no carrier.
        assert_eq!(field_carrier(pre, "y", 2), None);
    }

    #[test]
    fn emission_applies_jointly() {
        let pre = "struct Old {\n    x: u32,\n}\nfn f(s: Old) {\n}\n";
        let emission = joint_emit("a.rs", "r1", "struct", "", "Old", "New", "x: u32", "x: u64");
        assert!(emission.contains("# follow=uses"), "got:\n{emission}");
        assert!(emission.contains("- struct Old\n"), "got:\n{emission}");
        assert!(emission.contains("+ struct New\n"), "got:\n{emission}");
        assert!(emission.contains("- x: u32"), "got:\n{emission}");
        assert!(emission.contains("+ x: u64"), "got:\n{emission}");
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("a.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint emission applies");
        assert!(
            files["a.rs"].contains("struct New"),
            "got: {}",
            files["a.rs"]
        );
        assert!(files["a.rs"].contains("x: u64"), "got: {}", files["a.rs"]);
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
    fn another_container_evo_stays_ordinary() {
        // The evo belongs to `Other`, not the renamed `Old`: no joint.
        // The stranded-use veto keeps the section wholly ordinary (the
        // evo hunk strands the `Old` use) — never a lying rename.
        let pre =
            "struct Old {\n    x: u32,\n}\nstruct Other {\n    y: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-struct Old {\n+struct New {\n@@ -5,2 +5,2 @@\n-    y: u32,\n+    y: u64,\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("rule="),
            "another-container evo must stay wholly ordinary, got:\n{}",
            out.unwrap_or_default()
        );
    }

    #[test]
    fn two_evos_decline_joint() {
        // Two member deltas with one rename: not this shape (single-evo
        // law) — wholly ordinary, never a half joint.
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nfn f(s: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,5 +1,5 @@\n-struct Old {\n-    x: u32,\n-    y: u32,\n+struct New {\n+    x: u64,\n+    y: u64,\n }\n-fn f(s: Old) {\n+fn f(s: New) {\n";
        let out = infer(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("follow=uses"),
            "two-evo joint must not fire, got:\n{}",
            out.unwrap_or_default()
        );
    }

    #[test]
    fn fn_def_never_joints() {
        // `fn` rename+evolve belongs to WI-4: the item joint declines.
        let pre = "fn foo(x: u32) -> u32 {\n    x\n}\nfn m() {\n    foo(1);\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n-fn foo(x: u32) -> u32 {\n-    x\n+fn bar(x: u64) -> u32 {\n+    x\n@@ -4,1 +4,1 @@\n-    foo(1);\n+    bar(1);\n";
        let out = infer(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("rule="),
            "fn defs must never item-joint-fire, got:\n{}",
            out.unwrap_or_default()
        );
    }

    #[test]
    fn variant_pair_matrix() {
        assert!(parse_variant_evo_pair("A(u32)", "A(u64)").is_some());
        assert!(parse_variant_evo_pair("A { x: u32 }", "A { x: u64 }").is_some());
        assert!(parse_variant_evo_pair("A = 1", "A = 2").is_some());
        // Bare rename: not an evolution.
        assert!(parse_variant_evo_pair("A", "B").is_none());
        // Same payload: not an evolution.
        assert!(parse_variant_evo_pair("A(u32)", "A(u32)").is_none());
        // Field line declines.
        assert!(parse_variant_evo_pair("x: u32", "x: u64").is_none());
        // Arity mismatch declines.
        assert!(parse_variant_evo_pair("A(u32)", "A(u32, u64)").is_none());
    }

    #[test]
    fn variant_carrier_matrix() {
        let pre = "enum Old {\n    A(u32),\n}\nenum Other {\n    A(u32),\n}\n";
        // Two same-named variants: no unique carrier.
        assert_eq!(enum_of_variant(pre, "A", 2), None);
        // Unique variant resolves.
        let pre2 = "enum Old {\n    A(u32),\n    B(u32),\n}\n";
        assert_eq!(enum_of_variant(pre2, "B", 3).as_deref(), Some("Old"));
    }

    #[test]
    fn enum_joint_fires_end_to_end() {
        let pre = "enum Old {\n    A(u32),\n}\nfn f(x: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,4 +1,4 @@\n-enum Old {\n-    A(u32),\n+enum New {\n+    A(u64),\n }\n-fn f(x: Old) {\n+fn f(x: New) {\n";
        let out = infer(section, pre).expect("enum joint must fire");
        assert!(out.contains("rule="), "got:\n{out}");
        assert!(out.contains("# follow=uses"), "got:\n{out}");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("+ enum New"), "got:\n{out}");
        assert!(out.contains("- A(u32)"), "got:\n{out}");
        assert!(out.contains("+ A(u64)"), "got:\n{out}");
        // Real-apply byte-identical.
        let parsed = rsmpl_patch::parse_patch(&out).expect("emits parseable patch");
        let mut files = std::collections::HashMap::from([("f.rs".to_string(), pre.to_string())]);
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("joint applies");
        assert!(files["f.rs"].contains("enum New"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("A(u64)"), "got: {}", files["f.rs"]);
        assert!(files["f.rs"].contains("x: New"), "got: {}", files["f.rs"]);
        assert!(!files["f.rs"].contains("Old"), "got: {}", files["f.rs"]);
    }

    #[test]
    fn enum_joint_discriminant_fires() {
        let pre = "enum Old {\n    A = 1,\n}\nfn f(x: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,4 +1,4 @@\n-enum Old {\n-    A = 1,\n+enum New {\n+    A = 2,\n }\n-fn f(x: Old) {\n+fn f(x: New) {\n";
        let out = infer(section, pre).expect("disc joint must fire");
        assert!(out.contains("- enum Old"), "got:\n{out}");
        assert!(out.contains("A = 2"), "got:\n{out}");
    }

    #[test]
    fn enum_joint_another_container_stays_ordinary() {
        let pre = "enum Old {\n    A(u32),\n}\nenum Other {\n    B(u32),\n}\nfn f(x: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-enum Old {\n+enum New {\n@@ -5,2 +5,2 @@\n-    B(u32),\n+    B(u64),\n-fn f(x: Old) {\n+fn f(x: New) {\n";
        let out = infer(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("follow=uses"),
            "another-container variant evo must not joint-fire, got:\n{}",
            out.unwrap_or_default()
        );
    }

    #[test]
    fn enum_joint_cross_kind_stays_ordinary() {
        // Field line + enum rename: cross-kind, never the enum joint.
        let pre = "enum Old {\n    A(u32),\n}\nfn f(x: Old) {\n}\n";
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,4 +1,4 @@\n-enum Old {\n-    x: u32,\n+enum New {\n+    x: u64,\n }\n-fn f(x: Old) {\n+fn f(x: New) {\n";
        let out = infer(section, pre);
        assert!(
            out.is_none() || !out.as_ref().unwrap().contains("follow=uses"),
            "cross-kind member must not enum-joint-fire, got:\n{}",
            out.unwrap_or_default()
        );
    }
}
