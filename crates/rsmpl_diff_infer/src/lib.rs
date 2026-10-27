//! Pure inference for the inverse mapping (`diffrs`).
//!
//! Turns a native unified diff into RSmPL-Git: v1 compresses whole-file
//! fn/item renames (one ident delta, identical bodies), N-site
//! heterogeneous fn/method renames (one ident delta → one slot-sweep
//! `V Q fn … G (…) W` hunk), N-site method-to-function fan-outs
//! (`recv.old(args)` → `new_fn(recv, args)`), N-site method-to-method
//! renames (`recv.old(args)` → `recv.new(args)`), and N-site call rewrites (same
//! callee/arity delta → one `# allow=many` hunk) into semantic hunks,
//! and passes everything else through as ordinary hunks. Binary file sections are stripped (a patch is
//! UTF-8; `applyrs` cannot read binary worktree files) and the
//! remainder is validated by parsing it as RSmPL-Git, so every
//! `diffrs` output is applyrs-consumable by construction. A failed
//! semantic candidate falls back to the ordinary passthrough —
//! never an unparseable patch.

pub mod semantic;

#[derive(Debug, thiserror::Error)]
pub enum InferError {
    #[error("native diff is not valid RSmPL-Git: {0}")]
    Unparseable(String),
}

/// The v1 inference result: the RSmPL patch plus which binary paths
/// were stripped (the binary reports them as `warning:` lines).
pub struct InferredPatch {
    pub rsmpl: String,
    pub stripped_binary: Vec<String>,
}

pub fn infer_rsmpl(raw: &str) -> Result<InferredPatch, InferError> {
    // No changes is the legitimate empty shape here (the binary, not
    // infer, decides how to report it); an empty patch is only an
    // authoring typo on the `applyrs` side (T59).
    if raw.trim().is_empty() {
        return Ok(InferredPatch {
            rsmpl: String::new(),
            stripped_binary: Vec::new(),
        });
    }
    let (kept, stripped) = strip_binary_sections(raw);
    // Native hunks often carry git's `@@ -a,b +c,d @@ <funcname>`
    // section title; RSmPL hunk headers admit only `rule=` tags
    // (SPEC 8.3), so ordinary titles must go — exactly like `git
    // apply`, which ignores them. Two-sided (native) headers are
    // truncated after the closing `@@`; one-sided (semantic, never
    // present in native diffs) lines are left alone.
    let kept: Vec<String> = kept.iter().map(|s| strip_hunk_titles(s)).collect();
    let passthrough = kept.join("");
    rsmpl_patch::parse_patch(&passthrough)
        .map_err(|e| InferError::Unparseable(describe_parse_error(&e)))?;
    // Semantic compression first; any validation failure falls back
    // to the (already validated) ordinary passthrough.
    let candidate = semantic::infer_semantic(&kept).unwrap_or_default();
    if candidate.is_empty() {
        return Ok(InferredPatch {
            rsmpl: passthrough,
            stripped_binary: stripped,
        });
    }
    match rsmpl_patch::parse_patch(&candidate) {
        Ok(_) => Ok(InferredPatch {
            rsmpl: candidate,
            stripped_binary: stripped,
        }),
        Err(_) => Ok(InferredPatch {
            rsmpl: passthrough,
            stripped_binary: stripped,
        }),
    }
}

fn describe_parse_error(e: &rsmpl_patch::ParseError) -> String {
    match e {
        rsmpl_patch::ParseError::Structured(s) => s.render(),
        rsmpl_patch::ParseError::Message(m) => m.clone(),
    }
}

/// Split `raw` into `diff --git` sections; drop sections holding a
/// `Binary files … differ` line. Returns (kept sections, stripped
/// `b/`-side paths).
fn strip_binary_sections(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut stripped = Vec::new();
    // `raw` always ends sections with `\n` (git output); a missing
    // trailing section is still flushed below.
    let mut current = String::new();
    let mut current_path: Option<String> = None;
    let mut current_binary = false;
    let mut seen_any = false;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if seen_any {
                flush(
                    &mut kept,
                    &mut stripped,
                    &mut current,
                    &current_path,
                    current_binary,
                );
            }
            seen_any = true;
            current_binary = false;
            current_path = b_side_path(rest);
        } else if line.starts_with("Binary files ") {
            current_binary = true;
        }
        if seen_any {
            current.push_str(line);
            current.push('\n');
        }
    }
    if seen_any {
        flush(
            &mut kept,
            &mut stripped,
            &mut current,
            &current_path,
            current_binary,
        );
    }
    (kept, stripped)
}

fn flush(
    kept: &mut Vec<String>,
    stripped: &mut Vec<String>,
    current: &mut String,
    path: &Option<String>,
    binary: bool,
) {
    if binary {
        if let Some(p) = path {
            stripped.push(p.clone());
        }
    } else {
        kept.push(std::mem::take(current));
        return;
    }
    current.clear();
}

fn b_side_path(rest: &str) -> Option<String> {
    let mut parts = rest.splitn(2, ' ');
    parts.nth(1).map(|b| {
        b.strip_prefix("b/")
            .unwrap_or(b)
            .trim_matches('"')
            .to_string()
    })
}

/// Drop git's trailing `@@ … @@ <title>` section titles on
/// two-sided (native ordinary) hunk headers.
fn strip_hunk_titles(section: &str) -> String {
    let mut out = String::new();
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("@@") {
            if let Some(end) = rest.find("@@") {
                let ranges = &rest[..end];
                if ranges.contains('-') && ranges.contains('+') {
                    out.push_str("@@");
                    out.push_str(ranges);
                    out.push_str("@@\n");
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_SECTION: &str = "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-a\n+b\n";
    const BINARY_SECTION: &str = "diff --git a/img.bin b/img.bin\nindex 1..2 100644\nBinary files a/img.bin and b/img.bin differ\n";

    #[test]
    fn text_only_passes_through_validated() {
        let out = infer_rsmpl(TEXT_SECTION).unwrap();
        assert_eq!(out.rsmpl, TEXT_SECTION);
        assert!(out.stripped_binary.is_empty());
    }

    #[test]
    fn binary_section_is_stripped_and_named() {
        let raw = format!("{TEXT_SECTION}{BINARY_SECTION}");
        let out = infer_rsmpl(&raw).unwrap();
        assert_eq!(out.rsmpl, TEXT_SECTION);
        assert_eq!(out.stripped_binary, vec!["img.bin"]);
    }

    #[test]
    fn empty_diff_is_an_empty_patch() {
        // Empty input is an authoring typo for `applyrs` but the
        // legitimate no-changes shape here; the binary (not infer)
        // decides how to report it.
        let out = infer_rsmpl("").unwrap();
        assert!(out.rsmpl.is_empty());
        assert!(out.stripped_binary.is_empty());
    }

    #[test]
    fn hunk_section_titles_are_stripped() {
        // git appends the enclosing `fn` signature after the closing
        // `@@`; RSmPL headers admit only `rule=` tags (SPEC 8.3), so
        // infer drops titles like `git apply` does (T343). A body
        // edit (not a rename shape) passes through ordinarily.
        let raw = "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,3 @@ fn foo() {\n fn foo() {\n-    1\n+    2\n }\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,3 +1,3 @@\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(!out.rsmpl.contains("fn foo() {\n fn"), "got: {}", out.rsmpl);
        assert!(out.rsmpl.contains("-    1\n+    2\n"), "got: {}", out.rsmpl);
    }
}
