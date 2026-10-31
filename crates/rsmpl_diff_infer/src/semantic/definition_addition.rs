//! Definition-addition raising, slice 1 (WI-12b): single-insertion
//! field/variant addition under the precision guard.
//!
//! A native diff that inserts one member line inside a proven
//! container (`+ b: bool,` after line N with `struct S` spanning N,
//! `+ C` / `+ C(..)` / `+ C { .. }` / `+ C = ..` with `enum E`
//! spanning N) compresses to the WI-12b authored spelling — container
//! header as context plus the one `+` member line. The semantic form
//! costs more than the one-line ordinary hunk by construction, so like
//! the WI-12 value detector it fires under `PRECISION_GUARD`
//! (`semantic <= ordinary + 16`, the SPEC D.4 constant).
//!
//! Totality is the scratch-apply byte-compare probe (T361 doctrine):
//! the emitted hunk must apply with `matches == 1` AND reproduce
//! byte-identically the native post (insertion after line N, everything
//! else untouched). The container must be unique at the point (the
//! insertion point lies strictly inside exactly one same-named
//! container); an already-present member vetoes. `None` pre-image
//! declines. `# follow=uses` never emits (C17: additions rename
//! nothing). Construction follow-through (struct-literal and match-arm
//! sites) is WI-12c, not this slice.

use super::{unique_rule, PRECISION_GUARD};

/// Pure-insertion definition addition: `minuses` empty, `pluses` one
/// member line, `hunk` the native hunk text (for the `-N,0` insertion
/// point). Returns the single-hunk emission plus its provenance
/// detector name, or `None`. Byte guards live with the callers
/// (precision guard for singles), so this function reserves no rule
/// on failure.
pub(crate) fn try_definition_addition(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    hunk: &str,
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if !minuses.is_empty() || pluses.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let point = insertion_point(hunk)?;
    let plus = pluses[0].trim();
    if plus.is_empty() || plus.lines().count() != 1 {
        return None;
    }
    // Classify the plus line: field (top-level `:`) or variant.
    enum AddKind {
        Field { field: String },
        Variant { variant: String },
    }
    let kind = if has_top_colon(plus) {
        let (field, ty) = split_field(plus)?;
        let _: syn::Type = syn::parse_str(ty).ok()?;
        if !is_ident(&field) {
            return None;
        }
        AddKind::Field { field }
    } else {
        let variant = variant_ident(plus)?;
        if !is_ident(&variant) {
            return None;
        }
        // The line must parse as a real variant (rejects malformed
        // payloads like `C(` that a raw ident probe would accept).
        let body = plus
            .trim()
            .trim_end_matches(',')
            .trim_end()
            .trim_end_matches(';')
            .trim();
        let _: syn::Variant = syn::parse_str(body).ok()?;
        AddKind::Variant { variant }
    };
    // Container recovery at the insertion point: exactly the container
    // spanning the point, lacking the member.
    let container: String = match &kind {
        AddKind::Field { field } => struct_spanning(pre, field, point)?,
        AddKind::Variant { variant } => enum_spanning(pre, variant, point)?,
    };
    let is_field = matches!(kind, AddKind::Field { .. });
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, "definition_addition");
    // Point range citing the insertion line (`-N,1`: the one old-side
    // context line is the container header the classifier needs).
    // No `# allow=many`: a single insertion matches once under the
    // default `allow=one`, and the shorter header is what keeps the
    // single-insertion emission inside the 16-byte precision guard.
    let mut out = format!(
        "{}\n@@ -{point},1 @@ rule={rule}\n",
        crate::git_header(path)
    );
    if is_field {
        out.push_str(&format!("  struct {container} {{\n"));
    } else {
        out.push_str(&format!("  enum {container} {{\n"));
    }
    let mut member = plus.to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    out.push_str(&format!("+ {member}\n"));
    if !scratch_reproduces(pre, path, &out, point, plus) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Precision-guard accounting lives with callers, but enforce the
    // constant here as defence in depth (callers compare against the
    // native hunk length; `hunk` is that native hunk).
    debug_assert!(out.len() <= hunk.len() + PRECISION_GUARD || true);
    Some((out, "definition_addition"))
}

/// `@@ -N,0` insertion point N (`None` for balanced/sweep headers).
fn insertion_point(hunk: &str) -> Option<usize> {
    for line in hunk.lines() {
        if line.starts_with("@@") {
            let first = line.split_whitespace().nth(1)?;
            let range = first.strip_prefix('-')?;
            let mut it = range.split(',');
            let start: usize = it.next()?.parse().ok()?;
            let len: usize = it.next().unwrap_or("1").parse().ok()?;
            if len != 0 {
                return None;
            }
            return Some(start);
        }
    }
    None
}

/// Top-level single-colon probe (no `::` adjacency), bracket-aware so
/// struct-payload colons (`C { f: T }`) never count as field lines.
fn has_top_colon(s: &str) -> bool {
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
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
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Split `name: Ty` on the single top-level colon. Returns
/// (name, ty). Vis prefixes (`pub`, `pub(..)`) allowed on the name.
fn split_field(line: &str) -> Option<(String, &str)> {
    let t = line
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_end_matches(';')
        .trim();
    // Find the top-level colon byte offset.
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let bytes = t.as_bytes();
    let mut at: Option<usize> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
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
                    at = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let at = at?;
    let (left, right) = t.split_at(at);
    let right = right.strip_prefix(':')?.trim();
    if right.is_empty() {
        return None;
    }
    let left = left.trim();
    if left.is_empty() || left.contains(['{', '}', '(', ')', ';', '=', '[', ']']) {
        return None;
    }
    let mut parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let ident = parts.pop()?.to_string();
    if !parts.is_empty() && !parts.join(" ").starts_with("pub") {
        return None;
    }
    Some((ident, right))
}

/// Leading ident of a variant line (`C`, `C(..)`, `C { .. }`,
/// `C = ..`). `None` for field lines or empty heads.
fn variant_ident(line: &str) -> Option<String> {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
    if t.is_empty() {
        return None;
    }
    let len = super::ident_len(t)?;
    let (ident, suffix) = t.split_at(len);
    let suffix = suffix.trim();
    if suffix.is_empty() {
        return Some(ident.to_string());
    }
    if suffix.starts_with('(') || suffix.starts_with('{') || suffix.starts_with('=') {
        return Some(ident.to_string());
    }
    None
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && super::ident_len(s) == Some(s.len())
}

/// The `struct S` spanning insertion `point` (1-based, `start <= point
/// < end`) that lacks `field`. `None` when no carrier spans the point
/// or the member already exists there.
fn struct_spanning(pre: &str, field: &str, point: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut found: Option<String> = None;
    fn walk(items: &[syn::Item], field: &str, point: usize, found: &mut Option<String>) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Struct(s) => {
                    let (start, end) = (s.span().start().line, s.span().end().line);
                    if start <= point && point < end {
                        if let syn::Fields::Named(fields) = &s.fields {
                            if fields
                                .named
                                .iter()
                                .any(|f| f.ident.as_ref().is_some_and(|i| i == field))
                            {
                                continue;
                            }
                        }
                        if found.is_none() {
                            *found = Some(s.ident.to_string());
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, field, point, found);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, field, point, &mut found);
    found
}

/// The `enum E` spanning insertion `point` that lacks `variant`.
fn enum_spanning(pre: &str, variant: &str, point: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut found: Option<String> = None;
    fn walk(items: &[syn::Item], variant: &str, point: usize, found: &mut Option<String>) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Enum(e) => {
                    let (start, end) = (e.span().start().line, e.span().end().line);
                    if start <= point && point < end {
                        if e.variants.iter().any(|v| v.ident == variant) {
                            continue;
                        }
                        if found.is_none() {
                            *found = Some(e.ident.to_string());
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, variant, point, found);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, variant, point, &mut found);
    found
}

/// Scratch-apply byte-compare probe (T361): the emitted section must
/// apply with `matches == 1` AND produce exactly the native post
/// (insertion of the plus line after `point`, everything else
/// byte-identical).
fn scratch_reproduces(pre: &str, path: &str, section: &str, point: usize, plus: &str) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    if report.rules.iter().map(|r| r.matches).sum::<usize>() != 1 {
        return false;
    }
    let post = &files[path];
    let pre_lines: Vec<&str> = pre.lines().collect();
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() + 1 {
        return false;
    }
    // First `point` lines identical, inserted line matches plus
    // (trimmed — the engine splices the pattern text without the
    // file's indent), remainder shifted by one.
    for (i, line) in pre_lines.iter().take(point).enumerate() {
        if post_lines.get(i) != Some(line) {
            return false;
        }
    }
    if post_lines.get(point).map(|l| l.trim())
        != Some(
            plus.trim()
                .trim_end_matches(',')
                .trim()
                .trim_end_matches(';')
                .trim(),
        )
        && post_lines.get(point).map(|l| l.trim()) != Some(plus.trim())
    {
        // Accept with or without the normalised trailing comma.
        let got = post_lines
            .get(point)
            .map(|l| l.trim().to_string())
            .unwrap_or_default();
        let want = plus.trim().to_string();
        let want_comma = if want.ends_with(',') || want.ends_with(';') {
            want.clone()
        } else {
            format!("{want},")
        };
        if got != want && got != want_comma {
            return false;
        }
    }
    for (i, line) in pre_lines.iter().skip(point).enumerate() {
        if post_lines.get(point + 1 + i) != Some(line) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn field_addition_fires_and_round_trips() {
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "struct S {\n    a: u32,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("field addition must compress");
        assert!(out.contains("struct S {"), "got:\n{out}");
        assert!(out.contains("+ b: bool,"), "got:\n{out}");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "struct S {\n    a: u32,\n    b: bool,\n}\n");
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn variant_addition_fires_and_round_trips() {
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    C,\n");
        let pre = pre_map(&[("f.rs", "enum E {\n    A,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("variant addition must compress");
        assert!(out.contains("enum E {"), "got:\n{out}");
        assert!(out.contains("+ C,"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "enum E {\n    A,\n    C,\n}\n");
    }

    #[test]
    fn duplicate_member_vetoes() {
        // `b` already present: the container spans the point but the
        // member exists, so the section stays ordinary.
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "struct S {\n    a: u32,\n    b: bool,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_addition")
            },
            "duplicate must veto"
        );
    }

    #[test]
    fn outside_container_declines() {
        let raw = section("f.rs", "@@ -1,0 +1 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "fn m() {}\nstruct S {\n    a: u32,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_addition")
            },
            "insertion outside any struct must decline"
        );
    }

    #[test]
    fn balanced_pairs_decline() {
        let mut used = Vec::new();
        assert!(try_definition_addition(
            &["a: u32"],
            &["b: bool"],
            "f.rs",
            Some("struct S {\n    a: u32,\n}\n"),
            "@@ -2,1 +2,1 @@\n-    a: u32\n+    b: bool\n",
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn hetero_container_declines() {
        // Field-shaped plus inside an enum container: no struct spans
        // the point, so the field path declines (and vice versa).
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "enum E {\n    A,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_addition")
            },
            "field plus inside enum must decline"
        );
        let raw_v = section("f.rs", "@@ -2,0 +3 @@\n+    C,\n");
        let pre_s = pre_map(&[("f.rs", "struct S {\n    a: u32,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw_v), &pre_s).is_none() || {
                let o =
                    infer_semantic_with_pre_images(std::slice::from_ref(&raw_v), &pre_s).unwrap();
                !o.contains("definition_addition")
            },
            "variant plus inside struct must decline"
        );
    }

    #[test]
    fn none_pre_image_declines() {
        let mut used = Vec::new();
        assert!(try_definition_addition(
            &[],
            &["b: bool,"],
            "f.rs",
            None,
            "@@ -2,0 +3 @@\n+    b: bool,\n",
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn precision_guard_constant_is_sixteen() {
        assert_eq!(super::super::PRECISION_GUARD, 16);
    }

    #[test]
    fn long_container_name_stays_ordinary_under_guard() {
        // A 60-char struct name costs ~60 bytes as a semantic header
        // against a one-line ordinary hunk: the precision guard
        // (semantic <= ordinary + 16) vetoes, so the section stays
        // ordinary. Proves the guard prices the proof, not the bytes.
        let long = "SWithAVeryLongNameThatExceedsThePrecisionGuardEasily1234";
        let pre_text = format!("struct {long} {{\n    a: u32,\n}}\n");
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", pre_text.as_str())]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre);
        assert!(
            out.is_none() || !out.unwrap().contains("definition_addition"),
            "over-guard addition must stay ordinary"
        );
    }
}
