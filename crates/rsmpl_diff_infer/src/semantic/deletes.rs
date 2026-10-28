//! Whole-file item-deletion sweep (T464).
//!
//! A deleted `.rs` file renders natively as full-minus `-U0` hunks. When
//! the deleted content parses as Rust, every top-level item compresses to
//! one `# scope=file` + `# allow=many` + `# item I` / `- I` hunk (SPEC
//! 18.2 bare-`- I` deletion: the engine deletes every contained item, and
//! the T464 delete+delete union-merge absorbs double-claimed separator
//! blanks), while every non-item line (module docs, comments, stray
//! blanks) stays an ordinary `@@ -s,l +0,0 @@` leftover hunk. The rewrite
//! clears the T375 byte-win guard, else the ordinary passthrough stays.

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
}
