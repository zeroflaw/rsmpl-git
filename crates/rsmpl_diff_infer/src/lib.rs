//! Pure inference for the inverse mapping (`diffrs`).
//!
//! Turns a native unified diff into RSmPL-Git: v1 compresses
//! fn/item renames (one ident delta, identical bodies), N-site
//! heterogeneous fn/method renames (one ident delta → one slot-sweep
//! `V Q fn … G (…) W` hunk), N-site method-to-function fan-outs
//! (`recv.old(args)` → `new_fn(recv, args)`), N-site method-to-method
//! renames (`recv.old(args)` → `recv.new(args)`), N-site use-path
//! prefix renames (`use P::Old::T` → `use P::New::T`), and N-site
//! call rewrites (same callee/arity delta → one `# allow=many`
//! hunk) into semantic hunks,
//! whole-file first, then per balanced hunk (T349); the rest stays ordinary. Binary file sections are stripped (a patch is
//! UTF-8; `applyrs` cannot read binary worktree files), `index` lines
//! are dropped (optional metadata the lowering never reproduces —
//! T374), and the
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
    /// Submodule (gitlink, mode 160000) paths stripped during
    /// inference (T386; warned like binaries).
    pub stripped_gitlink: Vec<String>,
}

pub fn infer_rsmpl(raw: &str) -> Result<InferredPatch, InferError> {
    // No changes is the legitimate empty shape here (the binary, not
    // infer, decides how to report it); an empty patch is only an
    // authoring typo on the `applyrs` side (T59).
    if raw.trim().is_empty() {
        return Ok(InferredPatch {
            rsmpl: String::new(),
            stripped_binary: Vec::new(),
            stripped_gitlink: Vec::new(),
        });
    }
    let (kept, stripped, stripped_gitlink) = strip_unpatchable_sections(raw);
    if kept.is_empty() && (!stripped.is_empty() || !stripped_gitlink.is_empty()) {
        // All-binary / all-gitlink change (T378, T386): nothing
        // expressible in RSmPL-Git (a patch is UTF-8; `applyrs` cannot
        // read submodule directories), so report empty + the stripped
        // names (the binary warns) instead of failing the empty
        // passthrough below as an authoring typo. Section-less
        // non-binary input still falls through to the parse error.
        return Ok(InferredPatch {
            rsmpl: String::new(),
            stripped_binary: stripped,
            stripped_gitlink,
        });
    }
    // Native hunks often carry git's `@@ -a,b +c,d @@ <funcname>`
    // section title; RSmPL hunk headers admit only `rule=` tags
    // (SPEC 8.3), so ordinary titles must go — exactly like `git
    // apply`, which ignores them. Two-sided (native) headers are
    // truncated after the closing `@@`; one-sided (semantic, never
    // present in native diffs) lines are left alone. `index <sha>…`
    // lines go too (T374): `applyrs` parses them as optional file
    // metadata and the lowering never reproduces them, so they are
    // pure overhead in ordinary passthrough. Two-path `diff --git a/P
    // b/P` headers collapse to single-path `diff --git P` (SPEC 6
    // equivalence, T381 — 6 + len(P) bytes saved per section; renames
    // and quoted headers keep two-path, `---`/`+++` markers stay for
    // bare-`git apply` compat). Mode lines (`new/deleted file mode`,
    // `old/new mode`) stay — they feed the create/delete flags and the
    // mode splice.
    let kept: Vec<String> = kept
        .iter()
        .map(|s| collapse_git_headers(&strip_index_lines(&strip_hunk_titles(s))))
        .collect();
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
            stripped_gitlink,
        });
    }
    match rsmpl_patch::parse_patch(&candidate) {
        Ok(_) => Ok(InferredPatch {
            rsmpl: candidate,
            stripped_binary: stripped,
            stripped_gitlink,
        }),
        Err(_) => Ok(InferredPatch {
            rsmpl: passthrough,
            stripped_binary: stripped,
            stripped_gitlink,
        }),
    }
}

fn describe_parse_error(e: &rsmpl_patch::ParseError) -> String {
    match e {
        rsmpl_patch::ParseError::Structured(s) => s.render(),
        rsmpl_patch::ParseError::Message(m) => m.clone(),
    }
}

/// Lines of `s` without stripping trailing `\r` (`str::lines`
/// drops it, corrupting CRLF file content — T385). The trailing empty
/// segment of a `\n`-terminated input is dropped so callers re-append
/// `\n` per line and round-trip byte-exact.
fn raw_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut v: Vec<&str> = s.split('\n').collect();
    if s.ends_with('\n') {
        v.pop();
    }
    v
}

/// Split `raw` into `diff --git` sections; drop sections holding a
/// `Binary files … differ` line or a mode-160000 (gitlink/submodule)
/// declaration. Returns (kept sections, stripped `b/`-side binary
/// paths, stripped `b/`-side gitlink paths).
fn strip_unpatchable_sections(raw: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut stripped = Vec::new();
    let mut stripped_gitlink = Vec::new();
    // `raw` always ends sections with `\n` (git output); a missing
    // trailing section is still flushed below.
    let mut current = String::new();
    let mut current_path: Option<String> = None;
    let mut current_binary = false;
    let mut current_gitlink = false;
    let mut seen_any = false;
    for line in raw_lines(raw) {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if seen_any {
                flush(
                    &mut kept,
                    &mut stripped,
                    &mut stripped_gitlink,
                    &mut current,
                    &current_path,
                    current_binary,
                    current_gitlink,
                );
            }
            seen_any = true;
            current_binary = false;
            current_gitlink = false;
            current_path = b_side_path(rest);
        } else if line.starts_with("Binary files ") {
            current_binary = true;
        } else if is_gitlink_mode_line(line) {
            current_gitlink = true;
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
            &mut stripped_gitlink,
            &mut current,
            &current_path,
            current_binary,
            current_gitlink,
        );
    }
    (kept, stripped, stripped_gitlink)
}

/// A mode line declaring a gitlink (mode 160000): submodule add
/// (`new file mode`), delete (`deleted file mode`), or bump (`old /
/// `new mode`, `index … 160000`). Restricted to metadata prefixes —
/// a content line mentioning 160000 (e.g. `+const X = 160000;`) must
/// never strip its section.
fn is_gitlink_mode_line(line: &str) -> bool {
    (line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("old mode ")
        || line.starts_with("new mode ")
        || line.starts_with("index "))
        && line.split_whitespace().any(|w| w == "160000")
}

fn flush(
    kept: &mut Vec<String>,
    stripped: &mut Vec<String>,
    stripped_gitlink: &mut Vec<String>,
    current: &mut String,
    path: &Option<String>,
    binary: bool,
    gitlink: bool,
) {
    if binary {
        if let Some(p) = path {
            stripped.push(p.clone());
        }
    } else if gitlink {
        if let Some(p) = path {
            stripped_gitlink.push(p.clone());
        }
    } else {
        kept.push(std::mem::take(current));
        return;
    }
    current.clear();
}

fn b_side_path(rest: &str) -> Option<String> {
    // Unquoted spaces rejoined via the shared splitter (T379); a
    // single-path header yields that path.
    rsmpl_patch::split_git_paths(rest).map(|parts| {
        let side = parts.last().cloned().unwrap_or_default();
        side.strip_prefix("b/")
            .unwrap_or(&side)
            .trim_matches('"')
            .to_string()
    })
}

/// Drop git's trailing `@@ … @@ <title>` section titles on
/// two-sided (native ordinary) hunk headers.
fn strip_hunk_titles(section: &str) -> String {
    let mut out = String::new();
    for line in raw_lines(section) {
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

/// Drop git's per-file `index <sha>..<sha> <mode>` lines (T374):
/// optional file metadata for `applyrs` (parsed, never lowered), so
/// ordinary passthrough stays valid without them. Only the `index `
/// prefix matches — `new/deleted file mode` and `old/new mode` lines
/// are load-bearing and never touched.
fn strip_index_lines(section: &str) -> String {
    let mut out = String::new();
    for line in raw_lines(section) {
        if line.starts_with("index ") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Collapse the section's `diff --git` header to single-path (T381).
/// Only the header line is rewritten (`---`/`+++` markers stay);
/// rename, `/dev/null`, and quoted headers pass through via
/// `rsmpl_patch::collapse_git_header`.
fn collapse_git_headers(section: &str) -> String {
    let mut out = String::new();
    for line in raw_lines(section) {
        if line.starts_with("diff --git ") {
            out.push_str(&rsmpl_patch::collapse_git_header(line));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_SECTION: &str = "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-a\n+b\n";
    const TEXT_STRIPPED: &str = "diff --git f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-a\n+b\n";
    const BINARY_SECTION: &str = "diff --git a/img.bin b/img.bin\nindex 1..2 100644\nBinary files a/img.bin and b/img.bin differ\n";

    #[test]
    fn text_only_passes_through_validated() {
        let out = infer_rsmpl(TEXT_SECTION).unwrap();
        assert_eq!(out.rsmpl, TEXT_STRIPPED);
        assert!(out.stripped_binary.is_empty());
    }

    #[test]
    fn binary_section_is_stripped_and_named() {
        let raw = format!("{TEXT_SECTION}{BINARY_SECTION}");
        let out = infer_rsmpl(&raw).unwrap();
        assert_eq!(out.rsmpl, TEXT_STRIPPED);
        assert_eq!(out.stripped_binary, vec!["img.bin"]);
    }

    #[test]
    fn all_binary_change_is_empty_not_an_error() {
        // T378: a binary-only change has nothing expressible in RSmPL-Git
        // (a patch is UTF-8) — report empty + the stripped names (the
        // binary warns) instead of failing the empty passthrough as an
        // authoring typo. Mixed input still keeps the text side.
        let out = infer_rsmpl(BINARY_SECTION).unwrap();
        assert!(out.rsmpl.is_empty(), "got: {}", out.rsmpl);
        assert_eq!(out.stripped_binary, vec!["img.bin"]);
        let two = format!(
            "{BINARY_SECTION}diff --git a/other.bin b/other.bin\nBinary files a/other.bin and b/other.bin differ\n"
        );
        let out = infer_rsmpl(&two).unwrap();
        assert!(out.rsmpl.is_empty());
        assert_eq!(out.stripped_binary, vec!["img.bin", "other.bin"]);
        rsmpl_patch::parse_patch(TEXT_STRIPPED).expect("sanity");
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
        assert!(!out.rsmpl.contains("index "), "got: {}", out.rsmpl);
    }

    #[test]
    fn index_lines_are_stripped_mode_lines_kept() {
        // T374: `index …` is optional metadata (parsed, never lowered)
        // so it goes; `new file mode` feeds the create flag and stays.
        let raw = "diff --git a/new.rs b/new.rs\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1,1 @@\n+fn n() {}\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("index "), "got: {}", out.rsmpl);
        assert!(
            out.rsmpl.contains("new file mode 100644\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(out.rsmpl.contains("+fn n() {}\n"), "got: {}", out.rsmpl);
        rsmpl_patch::parse_patch(&out.rsmpl).expect("stripped patch must parse");
    }

    #[test]
    fn section_headers_collapse_to_single_path() {
        // T381: two-path `diff --git a/P b/P` headers collapse to
        // single-path `diff --git P` (SPEC 6); `---`/`+++` markers
        // stay (bare `git apply` needs them), renames and creates
        // keep their two-path shape.
        let out = infer_rsmpl(TEXT_SECTION).unwrap();
        assert!(
            out.rsmpl.contains("diff --git f.rs\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(!out.rsmpl.contains("diff --git a/"), "got: {}", out.rsmpl);
        assert!(out.rsmpl.contains("--- a/f.rs\n"), "got: {}", out.rsmpl);
        let rename = "diff --git a/old.rs b/new.rs\nsimilarity index 89%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n";
        let out = infer_rsmpl(rename).unwrap();
        assert!(
            out.rsmpl.contains("diff --git a/old.rs b/new.rs\n"),
            "got: {}",
            out.rsmpl
        );
        let create = "diff --git /dev/null b/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1,1 @@\n+n\n";
        let out = infer_rsmpl(create).unwrap();
        assert!(
            out.rsmpl.contains("diff --git /dev/null b/new.rs\n"),
            "got: {}",
            out.rsmpl
        );
    }

    #[test]
    fn crlf_content_survives_title_and_index_strips() {
        // T385: the passthrough rewrites (title/index/header) must not
        // drop the `\r` that CRLF file content carries — the old
        // `str::lines` loops converted worktree files to LF.
        let raw = "diff --git a/win.txt b/win.txt\nindex 1..2 100644\n--- a/win.txt\n+++ b/win.txt\n@@ -1,1 +1,1 @@ title\r\n-line2\r\n+CHANGED\r\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("index "), "got: {}", out.rsmpl);
        assert!(
            out.rsmpl.contains("@@ -1,1 +1,1 @@\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(out.rsmpl.contains("-line2\r\n"), "got: {}", out.rsmpl);
        assert!(out.rsmpl.contains("+CHANGED\r\n"), "got: {}", out.rsmpl);
    }

    #[test]
    fn crlf_rs_section_stays_ordinary() {
        // T385: semantic detectors decline `\r`-carrying sections —
        // a `\r`-less hunk could never match the CRLF worktree file.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,2 @@\n-fn a() {}\r\n-fn b() {}\r\n+fn c() {}\r\n+fn d() {}\r\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
        assert!(out.rsmpl.contains("-fn a() {}\r\n"), "got: {}", out.rsmpl);
    }

    #[test]
    fn gitlink_sections_are_stripped_and_named() {
        // T386: submodule sections (mode 160000) cannot round-trip —
        // `applyrs` cannot read the directory on the pre-image side —
        // so they strip with a warning, like binaries (T378). The
        // content-line guard keeps `+const X = 160000;` sections.
        let add = "diff --git a/sub b/sub\nnew file mode 160000\nindex 0000000..e7d842f\n--- /dev/null\n+++ b/sub\n@@ -0,0 +1 @@\n+Subproject commit e7d842fc138fa5a8e7899a15cfd554d44dc82d3a\n";
        let bump = "diff --git a/sub b/sub\nindex e7d842f..2dac80d 160000\n--- a/sub\n+++ b/sub\n@@ -1 +1 @@\n-Subproject commit e7d842fc138fa5a8e7899a15cfd554d44dc82d3a\n+Subproject commit 2dac80d76a2a7aa7ba3494066ff8fb1cf8896cff\n";
        let delete = "diff --git a/sub b/sub\ndeleted file mode 160000\nindex e7d842f..0000000\n--- a/sub\n+++ /dev/null\n@@ -1 +0,0 @@\n-Subproject commit e7d842fc138fa5a8e7899a15cfd554d44dc82d3a\n";
        let text = format!("{TEXT_SECTION}{add}{bump}{delete}");
        let out = infer_rsmpl(&text).unwrap();
        assert_eq!(out.rsmpl, TEXT_STRIPPED);
        assert_eq!(out.stripped_gitlink, vec!["sub", "sub", "sub"]);
        assert!(out.stripped_binary.is_empty());
        let plain_160000 =
            "diff --git a/n.rs b/n.rs\n--- a/n.rs\n+++ b/n.rs\n@@ -1 +1 @@\n-a\n+const X = 160000;\n";
        let out = infer_rsmpl(plain_160000).unwrap();
        assert!(
            out.rsmpl.contains("+const X = 160000;"),
            "got: {}",
            out.rsmpl
        );
        assert!(out.stripped_gitlink.is_empty());
    }

    #[test]
    fn all_gitlink_change_is_empty_not_an_error() {
        // T386: mirrors the T378 all-binary rule — nothing expressible,
        // so empty + names instead of a parse error.
        let add = "diff --git a/sub b/sub\nnew file mode 160000\n--- /dev/null\n+++ b/sub\n@@ -0,0 +1 @@\n+Subproject commit e7d842f\n";
        let out = infer_rsmpl(add).unwrap();
        assert!(out.rsmpl.is_empty(), "got: {}", out.rsmpl);
        assert_eq!(out.stripped_gitlink, vec!["sub"]);
    }
}
