//! Whole-file item-deletion sweep (T464) plus partial-file
//! item-deletion sweep (T465: minus-heavy unbalanced hunks whose minus
//! lines are whole top-level items).
//!
//! A deleted `.rs` file renders natively as full-minus `-U0` hunks. When
//! the deleted content parses as Rust, every top-level item compresses to
//! one `# scope=file` + `# allow=many` + `# item I` / `- I` hunk (SPEC
//! 18.2 bare-`- I` deletion: the engine deletes every contained item, and
//! the T464 delete+delete union-merge absorbs double-claimed separator
//! blanks), while every non-item line (module docs, comments, stray
//! blanks) stays an ordinary `@@ -s,l +0,0 @@` leftover hunk. The rewrite
//! clears the T375 byte-win guard, else the ordinary passthrough stays.
//!
//! A partial deletion (file kept) renders as minus-heavy unbalanced
//! hunks. When every minus line of such a hunk is a whole top-level item
//! (or an absorbed separator blank) in the pre-image, the hunk compresses
//! to one real-range `# allow=many` + `# item I` / `- I` hunk (default
//! Contained scope: the engine deletes exactly the items in the range)
//! plus one ordinary insertion leftover for the plus lines (with a
//! compensating trailing `+` blank when the pre-image line after the
//! range is blank — the last delete absorbs it). Balanced hunks keep
//! their rename/call detectors (strictly minus-heavy only). EOF-touching
//! ranges fire too when some contained item reaches EOF and the absorbed
//! preceding blank is BOF-absent, non-blank, or itself a minus line (T472;
//! a kept preceding blank, or trailing blanks no item reaches, stays vetoed).

use super::unique_rule;
use syn::spanned::Spanned;

/// Try to compress a whole-file deletion `section` (single-path `diff
/// --git`, `+++ /dev/null` prelude, `-U0` minus-only hunks) into one
/// many-delete hunk plus ordinary leftovers for non-item lines. Returns
/// `None` when the section is not a deletion, carries plus lines, fails
/// the contiguity tilt (minus lines != whole file), has unparseable
/// content, holds no items, or loses the byte-win guard.
pub(crate) fn try_delete_sweep(section: &str, used_rules: &mut Vec<String>) -> Option<String> {
    let checkpoint = used_rules.len();
    let lines: Vec<&str> = section.lines().collect();
    let hunk_at = lines.iter().position(|l| l.starts_with("@@"))?;
    let prelude = &lines[..hunk_at];
    if !prelude.contains(&"+++ /dev/null") {
        return None;
    }
    // Collect hunks: old start plus minus lines. Any plus, context, or
    // unknown body line vetoes — `-U0` deletions carry minus lines only.
    // (old_len is implied by the tilt check below, so only the new side
    // is range-checked here.)
    let mut hunks: Vec<(u32, Vec<String>, bool)> = Vec::new();
    let mut cur: Option<(u32, Vec<String>, bool)> = None;
    for line in &lines[hunk_at..] {
        if let Some(rest) = line.strip_prefix("@@") {
            let (old_start, _, new_start, new_len) = parse_unified_header(rest)?;
            if new_start != 0 || new_len != 0 {
                return None;
            }
            if let Some(h) = cur.take() {
                hunks.push(h);
            }
            cur = Some((old_start, Vec::new(), false));
            continue;
        }
        let h = cur.as_mut()?;
        if line.starts_with('+') && !line.starts_with("+++") {
            return None;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            h.1.push(line[1..].to_string());
        } else if *line == "\\ No newline at end of file" {
            h.2 = true;
        } else {
            // Context line, `---`/`+++` markers, or anything else: not a
            // pure deletion section.
            return None;
        }
    }
    if let Some(h) = cur.take() {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return None;
    }
    // Tilt check: minus lines must tile the whole file from line 1.
    let mut expected: u32 = 1;
    let mut minus: Vec<String> = Vec::new();
    let mut trailing_newline = true;
    for (i, (start, body, no_newline)) in hunks.iter().enumerate() {
        if *start != expected {
            return None;
        }
        expected += body.len() as u32;
        minus.extend(body.iter().cloned());
        if *no_newline {
            if i + 1 != hunks.len() {
                return None;
            }
            trailing_newline = false;
        }
    }
    if minus.is_empty() {
        return None;
    }
    let mut content = minus.join("\n");
    if trailing_newline {
        content.push('\n');
    }
    let file: syn::File = syn::parse_str(&content).ok()?;
    if file.items.is_empty() {
        return None;
    }
    let n = minus.len();
    let mut covered = vec![false; n];
    for it in &file.items {
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > n {
            return None;
        }
        covered[(s - 1)..e].fill(true);
    }
    // Leftover runs: maximal uncovered line runs (1-based, inclusive).
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if !covered[i] {
            let s = i;
            while i < n && !covered[i] {
                i += 1;
            }
            runs.push((s + 1, i));
        } else {
            i += 1;
        }
    }
    let rule = unique_rule(used_rules, "delete_items");
    let mut rewritten = String::new();
    for line in prelude {
        rewritten.push_str(line);
        rewritten.push('\n');
    }
    rewritten.push_str(&format!(
        "@@ -1,1 @@ rule={rule}\n# scope=file\n# allow=many\n# item I\n- I\n"
    ));
    for (s, e) in &runs {
        let len = e - s + 1;
        rewritten.push_str(&format!("@@ -{s},{len} +0,0 @@\n"));
        for line in &minus[(s - 1)..*e] {
            rewritten.push('-');
            rewritten.push_str(line);
            rewritten.push('\n');
        }
        if *e == n && !trailing_newline {
            rewritten.push_str("\\ No newline at end of file\n");
        }
    }
    if rewritten.len() < section.len() {
        Some(rewritten)
    } else {
        used_rules.truncate(checkpoint);
        None
    }
}

/// Try to compress one ordinary native hunk whose minus lines are whole
/// top-level items (T465 partial-file deletion, file kept): emits one
/// real-range `# allow=many` + `# item I` / `- I` hunk (default Contained
/// scope deletes exactly the items in the range) plus one ordinary
/// insertion leftover for the plus lines. Returns `None` (caller keeps
/// the ordinary hunk) when the hunk is already semantic, balanced or
/// single-minus, carries context/no-newline lines, lacks a pre-image,
/// has a minus line outside whole contained items, touches EOF with a kept
/// preceding blank (or trailing in-range blanks no item reaches), or loses
/// the per-hunk T375 byte-win guard.
pub(crate) fn try_partial_delete_hunk(
    hunk: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let checkpoint = used_rules.len();
    let mut lines = hunk.lines();
    let header = lines.next()?;
    let rest = header.strip_prefix("@@")?;
    if header.contains("rule=") {
        return None;
    }
    let (old_start, old_len, new_start, _new_len) = parse_unified_header(rest)?;
    // Old-side line numbers per minus line; plus lines ride the leftover.
    let mut minus: Vec<(u32, &str)> = Vec::new();
    let mut plus: Vec<&str> = Vec::new();
    let mut old_ln = old_start;
    // `---` file markers live in the prelude, never in a hunk body, so
    // a `---x` body line is deleted `--x` content (strip one). Context,
    // `\ No newline`, or anything else vetoes (the source runs `-U0`).
    for line in lines {
        match (line.strip_prefix('-'), line.strip_prefix('+')) {
            (Some(text), _) => {
                minus.push((old_ln, text));
                old_ln += 1;
            }
            (_, Some(text)) => plus.push(text),
            _ => return None,
        }
    }
    // Strictly minus-heavy multi-line hunks only: balanced hunks keep
    // their rename/call chain, singles never win T375 (and stranded
    // singles keep their join).
    if minus.len() < 2 || minus.len() <= plus.len() {
        return None;
    }
    let old_text = file_old?;
    let file: syn::File = syn::parse_str(old_text).ok()?;
    let old_lines: Vec<&str> = old_text.lines().collect();
    let last = old_start + old_len.saturating_sub(1);
    let nlines = old_lines.len();
    // Item spans (1-based, inclusive) from the pre-image; syn item spans
    // cover outer attrs, so attr lines need no separate accounting.
    let mut spans: Vec<(u32, u32)> = Vec::new();
    for it in &file.items {
        let s = it.span().start().line as u32;
        let e = it.span().end().line as u32;
        if s < 1 || e < s {
            return None;
        }
        // Any item overlapping the range must be fully contained —
        // else the hunk edits (not deletes) that item.
        if s <= last && e >= old_start && (s < old_start || e > last) {
            return None;
        }
        spans.push((s, e));
    }
    // EOF-touching ranges: the EOF-reaching delete absorbs the blank line
    // preceding its item (delete_lines EOF rule). Sound iff no kept blank
    // is absorbed: some contained item must reach EOF (else trailing
    // in-range blanks would survive the engine delete), and that item's
    // preceding line must be BOF-absent, non-blank, or itself a minus
    // line inside the range. An item starting exactly at the range start
    // with a kept preceding blank stays vetoed (nothing could restore it).
    if last as usize >= nlines {
        let eof_start = spans
            .iter()
            .filter(|(s, e)| *s >= old_start && *e <= last && *e as usize >= nlines)
            .map(|(s, _)| *s)
            .max();
        // The EOF-touching delete absorbs the preceding blank line; only
        // a kept separator blank vetoes (its restore position is gone).
        let s0 = eof_start?;
        if s0 > 1 {
            let prev = old_lines[(s0 as usize) - 2];
            if prev.trim().is_empty() && s0 <= old_start {
                return None;
            }
        }
    }
    if !spans.iter().any(|(s, e)| *s >= old_start && *e <= last) {
        return None;
    }
    // Every non-blank minus line must sit inside a contained item;
    // separator blanks ride delete absorption (union-merged).
    for (ln, text) in &minus {
        if text.trim().is_empty() {
            continue;
        }
        if !spans
            .iter()
            .any(|(s, e)| *s >= old_start && *e <= last && *s <= *ln && *ln <= *e)
        {
            return None;
        }
    }
    // The last delete absorbs the following blank when one exists; the
    // insertion leftover restores it so the tree stays byte-exact.
    let compensate = old_lines
        .get(last as usize)
        .is_some_and(|l| l.trim().is_empty());
    let rule = unique_rule(used_rules, "delete_items");
    let mut out = String::new();
    out.push_str(&format!(
        "@@ -{old_start},{old_len} +{new_start},0 @@ rule={rule}\n# allow=many\n# item I\n- I\n"
    ));
    if !plus.is_empty() || compensate {
        let count = plus.len() + usize::from(compensate);
        let anchor = old_start.checked_sub(1)?;
        out.push_str(&format!("@@ -{anchor},0 +{new_start},{count} @@\n"));
        for text in &plus {
            out.push('+');
            out.push_str(text);
            out.push('\n');
        }
        if compensate {
            out.push_str("+\n");
        }
    }
    if out.len() < hunk.len() {
        Some(out)
    } else {
        used_rules.truncate(checkpoint);
        None
    }
}

/// Parse a two-sided unified header rest (`-s,l +ns,nl @@ ...`) into
/// `(old_start, old_len, new_start, new_len)`; bare lengths default to 1
/// (git omits `,1`). Returns `None` when the shape is not unified.
fn parse_unified_header(rest: &str) -> Option<(u32, u32, u32, u32)> {
    let end = rest.find("@@")?;
    let ranges = rest[..end].trim();
    let mut parts = ranges.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    if parts.next().is_some() {
        return None;
    }
    let (os, ol) = split_range(old)?;
    let (ns, nl) = split_range(new)?;
    Some((os, ol, ns, nl))
}

/// Split an old/new range (`s,l` or bare `s`).
fn split_range(part: &str) -> Option<(u32, u32)> {
    match part.split_once(',') {
        Some((s, l)) => Some((s.parse().ok()?, l.parse().ok()?)),
        None => Some((part.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Parse + engine-apply the emitted patch over synthetic pre-images
    /// (T436 rule — never parse-only), then hand the tree back for state
    /// asserts.
    fn validated_live(patch: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        files
    }

    fn deleted_section(content: &str) -> String {
        let n = content.lines().count();
        let mut body = String::new();
        for line in content.lines() {
            body.push('-');
            body.push_str(line);
            body.push('\n');
        }
        let trailing = if content.ends_with('\n') {
            String::new()
        } else {
            String::from("\\ No newline at end of file\n")
        };
        format!(
            "diff --git a.rs\ndeleted file mode 100644\n--- a/a.rs\n+++ /dev/null\n@@ -1,{n} +0,0 @@\n{body}{trailing}"
        )
    }

    /// A long-enough fn body to clear the T375 byte-win guard (separator
    /// blanks ride ordinary; the sweep still wins on item-heavy files).
    fn big_fn(name: &str) -> String {
        let mut s = format!("/// Docs for {name}.\nfn {name}() {{\n");
        for i in 0..30 {
            s.push_str(&format!("    let v{i} = {i};\n"));
        }
        s.push_str("}\n");
        s
    }

    #[test]
    fn delete_sweep_fires_and_empties_file() {
        let content = format!(
            "//! Crate docs.\n\nuse std::io;\n\n{}struct Thing;\n",
            big_fn("helper")
        );
        let section = deleted_section(&content);
        let mut used = Vec::new();
        let out = try_delete_sweep(&section, &mut used).expect("must fire");
        assert!(
            out.contains(
                "@@ -1,1 @@ rule=delete_items\n# scope=file\n# allow=many\n# item I\n- I\n"
            ),
            "got:\n{out}"
        );
        // Non-item lines (docs, separator blanks) stay ordinary.
        assert!(
            out.contains("@@ -1,2 +0,0 @@\n-//! Crate docs.\n-\n"),
            "got:\n{out}"
        );
        assert!(out.len() < section.len());
        let files = validated_live(&out, &pre_map(&[("a.rs", &content)]));
        assert_eq!(files.get("a.rs").map(String::as_str), Some(""));
    }

    #[test]
    fn delete_sweep_tiny_file_stays_ordinary() {
        // T375: one 3-line item costs more as a sweep hunk than ordinary.
        let content = "fn tiny() {}\n";
        let section = deleted_section(content);
        let mut used = Vec::new();
        assert!(try_delete_sweep(&section, &mut used).is_none());
    }

    #[test]
    fn delete_sweep_plus_side_vetoes() {
        let section =
            "diff --git a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-fn a() {}\n+fn b() {}\n";
        let mut used = Vec::new();
        assert!(try_delete_sweep(section, &mut used).is_none());
    }

    #[test]
    fn delete_sweep_unparseable_content_stays_ordinary() {
        let section = "diff --git a.rs\ndeleted file mode 100644\n--- a/a.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-this is not rust ((\n-that either\n";
        let mut used = Vec::new();
        assert!(try_delete_sweep(section, &mut used).is_none());
    }

    #[test]
    fn delete_sweep_no_trailing_newline_round_trips() {
        // Trailing comment without a newline: the leftover hunk carries
        // the `\ No newline` marker and the tree still ends file-gone.
        let content = format!("{}{}// trailing note", big_fn("a"), big_fn("b"));
        let content = content.trim_end_matches('\n').to_string();
        let section = deleted_section(&content);
        let mut used = Vec::new();
        let out = try_delete_sweep(&section, &mut used).expect("must fire");
        assert!(
            out.contains("\\ No newline at end of file\n"),
            "got:\n{out}"
        );
        let files = validated_live(&out, &pre_map(&[("a.rs", &content)]));
        assert_eq!(files.get("a.rs").map(String::as_str), Some(""));
    }

    /// 7945da1 miniature: two uses + two doc'd fns deleted mid-file,
    /// replaced by two lines; the blank after the range is absorbed by
    /// the last delete and restored by the insertion leftover.
    fn partial_pre() -> String {
        format!(
            "//! Docs.\n\nuse std::fs;\nuse std::path::Path;\nuse std::process::Command;\n\n{}\n{}\n#[test]\nfn kept() {{}}\n",
            big_fn("aaa"),
            big_fn("bbb")
        )
    }

    /// Native hunk deleting lines `start..start+len` (1-based) with the
    /// given plus lines, and the expected post-image (plus lines spliced
    /// in, absorbed blank restored by the caller-supplied tail).
    fn partial_native(start: u32, minus: &[&str], plus: &[&str], new_start: u32) -> String {
        let mut hunk = format!(
            "@@ -{start},{} +{new_start},{} @@\n",
            minus.len(),
            plus.len()
        );
        for line in minus {
            hunk.push('-');
            hunk.push_str(line);
            hunk.push('\n');
        }
        for line in plus {
            hunk.push('+');
            hunk.push_str(line);
            hunk.push('\n');
        }
        hunk
    }

    /// Full one-section patch around one emitted hunk pair, live-applied
    /// over `old` (T436 rule — never parse-only).
    fn partial_live(emitted: &str, old: &str) -> String {
        let patch = format!("diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n{emitted}");
        let parsed = rsmpl_patch::parse_patch(&patch).expect("emitted patch must parse");
        let path = parsed.files[0].target_path.clone();
        let mut files = HashMap::new();
        files.insert(path.clone(), old.to_string());
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        files.remove(&path).unwrap()
    }

    #[test]
    fn partial_delete_hunk_fires_with_insertion_and_compensation() {
        let old = partial_pre();
        let old_lines: Vec<&str> = old.lines().collect();
        // Deleted span: lines 4..=end of big_fn bbb (uses + doc'd fns).
        let start = 4u32;
        let span_end = old_lines
            .iter()
            .position(|l| *l == "#[test]")
            .expect("kept tail") as u32
            + 1
            - 2; // blank line before #[test]
        let minus: Vec<&str> = old_lines[(start as usize - 1)..(span_end as usize)].to_vec();
        let plus = ["mod common;", "use common::*;"];
        let hunk = partial_native(start, &minus, &plus, start);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert!(
            out.contains("rule=delete_items\n# allow=many\n# item I\n- I\n"),
            "got:\n{out}"
        );
        assert!(
            out.contains("@@ -3,0 +4,3 @@\n+mod common;\n+use common::*;\n+\n"),
            "got:\n{out}"
        );
        assert!(out.len() < hunk.len());
        // Expected: kept head + plus lines + the absorbed blank + tail.
        let mut expect = old_lines[..(start as usize - 1)].join("\n");
        expect.push('\n');
        expect.push_str("mod common;\nuse common::*;\n");
        expect.push_str(&old_lines[(span_end as usize)..].join("\n"));
        expect.push('\n');
        assert_eq!(partial_live(&out, &old), expect);
    }

    #[test]
    fn partial_delete_pure_deletion_needs_no_insertion() {
        // No plus lines and a non-blank line after the range: just the
        // delete hunk, no leftover.
        let old = format!("use std::fs;\n{}\nfn kept() {{}}\n", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let len = old_lines.len() - 2; // uses..end of big_fn, tail kept
        let minus: Vec<&str> = old_lines[1..(1 + len)].to_vec();
        let hunk = partial_native(2, &minus, &[], 2);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert_eq!(
            out.lines().filter(|l| l.starts_with("@@")).count(),
            1,
            "got:\n{out}"
        );
        let expect = String::from("use std::fs;\nfn kept() {}\n");
        assert_eq!(partial_live(&out, &old), expect);
    }

    #[test]
    fn partial_delete_pure_deletion_restores_absorbed_blank() {
        // Minus-only with a blank after the range: one `+` line restores
        // the separator the last delete absorbs.
        let old = format!("{}\nfn kept() {{}}\n", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let len = old_lines.len() - 2; // big_fn only, blank + tail kept
        let minus: Vec<&str> = old_lines[..len].to_vec();
        let hunk = partial_native(1, &minus, &[], 1);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert!(out.contains("@@ -0,0 +1,1 @@\n+\n"), "got:\n{out}");
        assert_eq!(partial_live(&out, &old), "\nfn kept() {}\n");
    }

    #[test]
    fn partial_delete_bof_anchor_fires() {
        // Hunk at line 1 with plus lines: insertion anchors at 0.
        let old = format!("{}\nfn kept() {{}}\n", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let len = old_lines.len() - 2; // big_fn only, tail kept
        let minus: Vec<&str> = old_lines[..len].to_vec();
        let plus = ["mod common;"];
        let hunk = partial_native(1, &minus, &plus, 1);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert!(
            out.contains("@@ -0,0 +1,2 @@\n+mod common;\n+\n"),
            "got:\n{out}"
        );
        assert_eq!(partial_live(&out, &old), "mod common;\n\nfn kept() {}\n");
    }

    #[test]
    fn partial_delete_single_minus_vetoes() {
        // cli_e2e's `@@ -4 +3,0 @@` one-line use deletion: never wins.
        let old = "use std::fs;\nuse std::path::Path;\nfn kept() {}\n";
        let hunk = "@@ -2 +1,0 @@\n-use std::path::Path;\n";
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(hunk, Some(old), &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_balanced_vetoes() {
        // Balanced hunks keep their rename/call chain (never stolen).
        let old = format!("{}fn kept() {{}}\n", big_fn("gone"));
        let minus: Vec<&str> = old.lines().take(2).collect();
        let hunk = partial_native(2, &minus, &["use std::fs;", "use std::io;"], 2);
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_mid_item_cut_vetoes() {
        // stdin_e2e shape: minus lines inside (not covering) a fn body.
        let old = "fn big() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\nfn kept() {}\n";
        let hunk = "@@ -2,2 +2,0 @@\n-    let a = 1;\n-    let b = 2;\n";
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(hunk, Some(old), &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_non_item_line_vetoes() {
        // A top-level comment line is no item: the hunk stays ordinary.
        let old = format!("// leading note\n{}\nfn kept() {{}}\n", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let len = old_lines.len() - 2;
        let minus: Vec<&str> = old_lines[..len].to_vec();
        let hunk = partial_native(1, &minus, &[], 1);
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_eof_touch_vetoes() {
        // The last delete would absorb the preceding blank (EOF rule).
        let old = format!("fn kept() {{}}\n\n{}", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let start = 4u32;
        let minus: Vec<&str> = old_lines[(start as usize - 1)..].to_vec();
        let hunk = partial_native(start, &minus, &[], start);
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_tiny_vetoes() {
        // Two short uses cost more as a sweep hunk than ordinary (T375).
        let old = "use a;\nuse b;\nfn kept() {}\n";
        let hunk = "@@ -1,2 +1,0 @@\n-use a;\n-use b;\n";
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(hunk, Some(old), &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_no_pre_vetoes() {
        // Legacy no-pre-image path stays ordinary (moves precedent).
        let hunk = "@@ -1,4 +1,0 @@\n-use a;\n-use b;\n-fn gone() {}\n-}\n";
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(hunk, None, &mut used).is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn partial_delete_eof_touch_in_range_blank_fires() {
        // T472 (3baecc1 miniature): the range starts with the separator
        // blank, so the EOF-absorbed preceding blank is itself a minus
        // line — the engine delete lands exactly on the native deletion.
        let old = format!("fn kept() {{}}\n\n{}", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let minus: Vec<&str> = old_lines[1..].to_vec();
        let hunk = partial_native(2, &minus, &[], 2);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert_eq!(
            out.lines().filter(|l| l.starts_with("@@")).count(),
            1,
            "got:\n{out}"
        );
        assert_eq!(partial_live(&out, &old), "fn kept() {}\n");
    }

    #[test]
    fn partial_delete_eof_touch_with_plus_lines_fires() {
        // Same EOF shape with plus lines: the insertion leftover anchors
        // at old_start - 1, no compensation owed (nothing follows EOF).
        let old = format!("fn kept() {{}}\n\n{}", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let minus: Vec<&str> = old_lines[1..].to_vec();
        let hunk = partial_native(2, &minus, &["mod tail;"], 2);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert!(out.contains("@@ -1,0 +2,1 @@\n+mod tail;\n"), "got:\n{out}");
        assert_eq!(partial_live(&out, &old), "fn kept() {}\nmod tail;\n");
    }

    #[test]
    fn partial_delete_eof_touch_non_blank_preceding_fires() {
        // Item starts exactly at the range start but the preceding line is
        // code: the EOF rule absorbs nothing, so the delete is exact.
        let old = format!("use std::fs;\n{}", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let minus: Vec<&str> = old_lines[1..].to_vec();
        let hunk = partial_native(2, &minus, &[], 2);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert_eq!(partial_live(&out, &old), "use std::fs;\n");
    }

    #[test]
    fn partial_delete_bof_to_eof_fires() {
        // Single item spanning the whole kept file: no preceding line
        // exists, so the EOF rule absorbs nothing.
        let old = big_fn("gone");
        let old_lines: Vec<&str> = old.lines().collect();
        let minus: Vec<&str> = old_lines.to_vec();
        let hunk = partial_native(1, &minus, &[], 1);
        let mut used = Vec::new();
        let out = try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).expect("must fire");
        assert_eq!(partial_live(&out, &old), "");
    }

    #[test]
    fn partial_delete_eof_touch_trailing_blanks_vetoes() {
        // No contained item reaches EOF (trailing in-range blanks): the
        // engine delete would leave them behind, so the hunk stays ordinary.
        let old = format!("{}\n", big_fn("gone"));
        let old_lines: Vec<&str> = old.lines().collect();
        let minus: Vec<&str> = old_lines.to_vec();
        let hunk = partial_native(1, &minus, &[], 1);
        let mut used = Vec::new();
        assert!(try_partial_delete_hunk(&hunk, Some(old.as_ref()), &mut used).is_none());
        assert!(used.is_empty());
    }
}
