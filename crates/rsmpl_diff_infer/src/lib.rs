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
//! hunk), and whole-item 1-to-1 moves (byte-identical fn deleted here,
//! added there → `# item I` + `# bind` cut / `# use` paste, T430) into
//! semantic hunks,
//! whole-file first, then per balanced hunk (T349); the rest stays ordinary. Binary file sections are stripped (a patch is
//! UTF-8; `applyrs` cannot read binary worktree files), `index` lines
//! are dropped (optional metadata the lowering never reproduces —
//! T374), and the
//! remainder is validated by parsing it as RSmPL-Git, so every
//! `diffrs` output is applyrs-consumable by construction. A failed
//! semantic candidate falls back to the ordinary passthrough —
//! never an unparseable patch.

pub mod semantic;
pub(crate) mod totality;

use std::collections::HashMap;

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
    infer_inner(raw, &HashMap::new())
}

/// Inference with pre-image file contents (T403 totality): `pre` maps
/// collapsed `diff --git` b-side paths to the old file text the diff
/// applies against (the index blob for pending worktree diffs, the
/// `rev` blob for staged and rev diffs — assembled by the caller, which
/// owns the repo). Sections without an entry fall back to
/// section-visible evidence (best-effort, same residual risk as the
/// other sweep detectors); unparseable entries veto verbatim sweeps
/// for that section. Legacy `infer_rsmpl` passes an empty map.
pub fn infer_rsmpl_with_pre_images(
    raw: &str,
    pre: &HashMap<String, String>,
) -> Result<InferredPatch, InferError> {
    infer_inner(raw, pre)
}

fn infer_inner(raw: &str, pre: &HashMap<String, String>) -> Result<InferredPatch, InferError> {
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
    let candidate = semantic::infer_semantic_with_pre_images(&kept, pre).unwrap_or_default();
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
/// `Binary files … differ` line, binary-content `+`/`-` body lines
/// (T395), or a mode-160000 (gitlink/submodule) declaration. Returns
/// (kept sections, stripped `b/`-side binary paths, stripped `b/`-side
/// gitlink paths).
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
        } else if is_binary_body_line(line) {
            // NUL body line: git's own binary definition. (T396: the
            // T395 U+FFFD arm is gone — invalid-UTF-8 sections never
            // reach infer as text because the source swaps them for
            // `Binary` markers at the byte level, while genuine U+FFFD
            // chars in valid-UTF-8 text pass through.)
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

/// A hunk body `+`/`-` line carrying binary content: NUL, git's own
/// binary definition (T396: U+FFFD is genuine text — the source
/// detects invalid UTF-8 at the byte level and sends markers, so
/// valid-UTF-8 sections keep their U+FFFD chars here). `+++`/`---`
/// file markers are excluded (paths never carry these signals).
fn is_binary_body_line(line: &str) -> bool {
    (line.starts_with('+') || line.starts_with('-'))
        && !(line.starts_with("+++") || line.starts_with("---"))
        && line.contains('\0')
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

pub(crate) fn b_side_path(rest: &str) -> Option<String> {
    // Unquoted spaces rejoined via the shared splitter (T379); a
    // single-path header yields that path.
    rsmpl_patch::split_git_paths(rest).map(|parts| {
        let side = parts.last().cloned().unwrap_or_default();
        // T399: no `trim_matches('"')` — the splitter already strips
        // only the outer C-quote pair, so edge quotes that belong to
        // the filename survive (trimming corrupted `"lead`/trail"`).
        side.strip_prefix("b/").unwrap_or(&side).to_string()
    })
}

/// `diff --git` header line for `path` (T399): C-quote `"`, `\` +
/// control slots via `quote_git_path` (native quotepath shape; plain
/// paths stay raw), then the T381 single-path collapse. Plain
/// `a/{p} b/{p}` interpolation mis-splits quote/backslash names.
pub(crate) fn git_header(path: &str) -> String {
    let raw = format!(
        "diff --git {} {}",
        rsmpl_patch::quote_git_path(&format!("a/{path}")),
        rsmpl_patch::quote_git_path(&format!("b/{path}"))
    );
    rsmpl_patch::collapse_git_header(&raw)
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
/// `rsmpl_patch::collapse_git_header`. Headers-only sections (mode
/// flips, empty creates: no `---`/`+++` markers) keep two-path — bare
/// `git apply` resolves filenames from the markers and fails on a
/// collapsed single path without them (T393).
fn collapse_git_headers(section: &str) -> String {
    // T393: gate the collapse on marker presence. A headers-only
    // section has no body lines, so a `--- `/`+++ ` hit can only come
    // from real file markers; sections that already collapse today
    // (hunks always ship markers) are unaffected.
    let collapsible = raw_lines(section)
        .iter()
        .any(|l| l.starts_with("--- ") || l.starts_with("+++ "));
    let mut out = String::new();
    for line in raw_lines(section) {
        if collapsible && line.starts_with("diff --git ") {
            out.push_str(&rsmpl_patch::collapse_git_header(line));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// One balanced hunk through the shared detector chain (the per-hunk
/// fallback's core): item rename for singles, else slot / method /
/// strip / use-prefix / call sweeps in order. Detectors return full
/// sections; only the hunk body is kept, and only when it beats the
/// ordinary hunk (the T375 per-hunk guard, prelude shared either way).
/// `None` on any failure — reserved rule names are released, so later
/// hunks keep tight names.
pub(crate) fn compress_balanced_pair_body(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    hunk_len: usize,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.is_empty() {
        return None;
    }
    let checkpoint = used_rules.len();
    let full = if minuses.len() == 1 {
        semantic::try_item_rename(minuses[0], pluses[0], path, used_rules)
    } else if let Some(slot) =
        semantic::try_fn_slot_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(slot)
    } else if let Some(m2f) =
        semantic::try_method_to_fn_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(m2f)
    } else if let Some(m2m) =
        semantic::try_method_to_method_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(m2m)
    } else if let Some(same) = semantic::try_method_same_literal_sweep(
        minuses, pluses, path, file_old, changed, used_rules,
    ) {
        Some(same)
    } else if let Some(strip) =
        semantic::try_ref_method_strip_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(strip)
    } else if let Some(uses) =
        semantic::try_use_prefix_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(uses)
    } else if let Some(call) =
        semantic::try_call_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(call)
    } else if let Some(call) =
        semantic::try_call_arg_add_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(call)
    } else if let Some(call) =
        semantic::try_call_literal_sweep(minuses, pluses, path, file_old, changed, used_rules)
    {
        Some(call)
    } else {
        semantic::try_call_peel_sweep(minuses, pluses, path, file_old, changed, used_rules)
    };
    match full {
        Some(f) => {
            let body = f.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
            if body.len() < hunk_len {
                Some(body.to_string())
            } else {
                used_rules.truncate(checkpoint);
                None
            }
        }
        None => {
            used_rules.truncate(checkpoint);
            None
        }
    }
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
    fn mode_flip_call_sweep_keeps_modes_per_hunk() {
        // T401: whole-section semantic dropped old/new mode (exec-bit lost).
        // Mode sections go per-hunk: prelude keeps modes, sweep compresses.
        let raw = "diff --git a/f.rs b/f.rs\nold mode 100644\nnew mode 100755\n--- a/f.rs\n+++ b/f.rs\n@@ -1,5 +1,5 @@\n fn m() {\n-    old_handler_name(a1);\n-    old_handler_name(a2);\n-    old_handler_name(a3);\n+    new_handler_name(a1);\n+    new_handler_name(a2);\n+    new_handler_name(a3);\n }\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(
            out.rsmpl.contains("old mode 100644\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("new mode 100755\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
        rsmpl_patch::parse_patch(&out.rsmpl).expect("mode sweep must parse");
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
    fn headers_only_sections_keep_two_path_headers() {
        // T393: mode-only and empty-create sections carry no
        // `---`/`+++` markers, and bare `git apply` resolves
        // filenames from those markers — collapsing to single-path
        // breaks bare-git compat (`lacks filename information`), so
        // these sections keep `diff --git a/P b/P`.
        let mode = "diff --git a/f.sh b/f.sh\nold mode 100644\nnew mode 100755\n";
        let out = infer_rsmpl(mode).unwrap();
        assert!(
            out.rsmpl.contains("diff --git a/f.sh b/f.sh\n"),
            "got: {}",
            out.rsmpl
        );
        let empty = "diff --git a/empty.txt b/empty.txt\nnew file mode 100644\n";
        let out = infer_rsmpl(empty).unwrap();
        assert!(
            out.rsmpl.contains("diff --git a/empty.txt b/empty.txt\n"),
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("kept patch must parse");
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

    #[test]
    fn binary_create_with_literal_bytes_is_stripped_and_named() {
        // T395: a staged no-NUL binary create reaches infer as body
        // lines (native `git diff` dumps literal bytes for creates),
        // lossy-decoded with an interior `\r` — the exact shape that
        // used to leak through with no warning and corrupt the
        // lowering (`--check` failed `corrupt patch`).
        let create = "diff --git a/blob.bin b/blob.bin\nnew file mode 100644\n--- /dev/null\n+++ b/blob.bin\n@@ -0,0 +1,2 @@\n+C�q�;\u{0}1�\n+]\u{FFFD}`[=\u{8}:\u{FFFD}05\r�\n";
        let out = infer_rsmpl(create).unwrap();
        assert!(out.rsmpl.is_empty(), "got: {}", out.rsmpl);
        assert_eq!(out.stripped_binary, vec!["blob.bin"]);
        let mixed = format!("{TEXT_SECTION}{create}");
        let out = infer_rsmpl(&mixed).unwrap();
        assert_eq!(out.rsmpl, TEXT_STRIPPED);
        assert_eq!(out.stripped_binary, vec!["blob.bin"]);
    }

    #[test]
    fn genuine_replacement_char_text_is_not_binary() {
        // T396: a literal U+FFFD char in valid-UTF-8 text (e.g. a
        // string-literal fixture) is genuine content — the old U+FFFD
        // arm stripped the whole section, silently dropping a text
        // file's changes (caught on 62fae28's own lib.rs, whose T395
        // test strings carry U+FFFD). Only NUL still strips here;
        // invalid-UTF-8 sections arrive as `Binary` markers instead.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-const S: &str = \"�\";\n+const S: &str = \"�!\";\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(
            out.stripped_binary.is_empty(),
            "got: {:?}",
            out.stripped_binary
        );
        assert!(
            out.rsmpl.contains("+const S: &str = \"�!\";"),
            "got: {}",
            out.rsmpl
        );
    }

    #[test]
    fn interior_cr_text_is_not_binary() {
        // T395 boundary: a lone interior `\r` in *text* content is
        // not a binary signal — the section stays ordinary (diffrs
        // output matches native U0 here, and bare git fails both
        // identically, so the outcome is faithful).
        let raw =
            "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-aaa\n+a\rb\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(out.stripped_binary.is_empty());
        assert!(out.rsmpl.contains("+a\rb\n"), "got: {}", out.rsmpl);
    }

    #[test]
    fn quoted_binary_marker_strips_with_real_name() {
        // T399: the header decode used to drop the interior `"` (the
        // source emitted it raw and the splitter mis-read it), so the
        // stripped name lost the quote. Quoted emission round-trips it.
        let raw = "diff --git \"a/q\\\"bin.txt\" \"b/q\\\"bin.txt\"\nBinary files \"a/q\\\"bin.txt\" and \"b/q\\\"bin.txt\" differ\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(out.rsmpl.is_empty(), "got: {}", out.rsmpl);
        assert_eq!(out.stripped_binary, vec!["q\"bin.txt"]);
    }

    #[test]
    fn quoted_b_side_path_keeps_edge_quotes() {
        // T399: `trim_matches('"')` stripped genuine edge quotes from
        // the decoded name (`"lead.txt` became `lead.txt`). The
        // splitter already removes only the outer C-quote pair.
        let rest = "\"a/\\\"lead.txt\" \"b/\\\"lead.txt\"";
        assert_eq!(b_side_path(rest).as_deref(), Some("\"lead.txt"));
    }

    #[test]
    fn verbatim_sweep_compresses_identical_pairs_across_hunks() {
        // T403 miniature of the 50d4a49 shape: two identical (1,1)
        // pairs split by a pure addition (which vetoes whole-section
        // balance) compress to one literal hunk; the addition stays.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -9,1 +10,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -20,0 +21,1 @@\n+let cached = state.parsed;\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("- &state.parsed\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(out.rsmpl.contains("+ parsed\n"), "got: {}", out.rsmpl);
        assert!(
            out.rsmpl.contains("+let cached = state.parsed;\n"),
            "got: {}",
            out.rsmpl
        );
        assert_eq!(
            out.rsmpl.matches("@@ -1,1 @@").count(),
            1,
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim hunk must parse");
    }

    #[test]
    fn verbatim_sweep_tiny_pairs_stay_passthrough() {
        // T375: two 2-char pairs cost more as a rule hunk (rule +
        // scope + allow overhead) than the two short ordinary hunks.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-&a,\n+a,\n@@ -9,1 +10,1 @@\n-&a,\n+a,\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_control_flow_stays_passthrough() {
        // eacce0e shape: for-loop headers parse as statements, not
        // expressions — no engine shape matches them (SPEC 18.6 R1).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-for line in section.lines() {\n+for line in raw_lines(section) {\n@@ -9,1 +10,1 @@\n-for line in section.lines() {\n+for line in raw_lines(section) {\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_bare_ident_stays_passthrough() {
        // f52d59a shape: a bare-ident minus would match every mention
        // (declarations included) — the precision gate rejects it.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    wide,\n+    ZeroMatchHint { wide, preview },\n@@ -9,1 +10,1 @@\n-    wide,\n+    ZeroMatchHint { wide, preview },\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_literal_minus_stays_passthrough() {
        // String-literal-only deltas would match every equal literal —
        // the precision gate rejects them.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    \"old\",\n+    \"new\",\n@@ -9,1 +10,1 @@\n-    \"old\",\n+    \"new\",\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_contained_minus_stays_passthrough() {
        // The stripped minus also occurs inside a leftover hunk's minus
        // line — sweeping would rewrite it and break that hunk's match.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -9,1 +10,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -20,2 +21,1 @@\n-        f(&state.parsed, x);\n-        g(y);\n+        f(parsed, x);\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_leftover_hunk_stays_ordinary_in_place() {
        // T403 leftover path: the verbatim group compresses while the
        // balanced but non-identical hunk (no detector shape fits it)
        // stays ordinary in place — identical to the per-hunk fallback.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -9,1 +10,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -20,1 +21,1 @@\n-        timeout_secs,\n+        timeout_secs.max(1),\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains(
                "@@ -20,1 +21,1 @@\n-        timeout_secs,\n+        timeout_secs.max(1),\n"
            ),
            "got: {}",
            out.rsmpl
        );
        assert_eq!(
            out.rsmpl.matches("@@ -1,1 @@").count(),
            1,
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("mixed output must parse");
    }

    #[test]
    fn verbatim_sweep_stranded_unbalanced_hunk_joins() {
        // T441 slice (c52441a miniature): two balanced (1,1) identical
        // pairs plus the same pair stranded in an unbalanced hunk (1
        // minus + extras, converted line first). The stranded site
        // joins the group (totality 3 == 3) instead of vetoing it; the
        // extras stay as a pure-insertion leftover. Byte-exact
        // round-trip through the engine pins the split.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n@@ -9,1 +9,1 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n@@ -20,1 +21,4 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n+    let done = extra();\n+\n+    done();\n";
        let pre = "fn m() {\n    validated(&text);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated(&text);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated(&text);\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("- validated(&text)\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("+ validated_live(&text, &pre)\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl
                .contains("@@ -20,0 +22,3 @@\n+    let done = extra();\n+\n+    done();\n"),
            "leftover insertion missing, got: {}",
            out.rsmpl
        );
        assert!(
            !out.rsmpl.contains("-    validated(&text);"),
            "native minus must be consumed, got: {}",
            out.rsmpl
        );
        assert!(
            !out.rsmpl.contains("+    validated_live(&text, &pre);"),
            "native plus must be consumed, got: {}",
            out.rsmpl
        );
        assert_eq!(
            out.rsmpl.matches("@@ -1,1 @@").count(),
            1,
            "got: {}",
            out.rsmpl
        );
        let parsed = rsmpl_patch::parse_patch(&out.rsmpl).expect("sweep must parse");
        let mut files = map.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("sweep must apply cleanly");
        let post = "fn m() {\n    validated_live(&text, &pre);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated_live(&text, &pre);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated_live(&text, &pre);\n    let done = extra();\n\n    done();\n}\n";
        assert_eq!(files["f.rs"], post, "round-trip must be byte-exact");
    }

    #[test]
    fn verbatim_sweep_stranded_join_vetoes_first_plus_mismatch() {
        // The stranded hunk's first plus differs from the group plus
        // (the converted line is not leading) — the leftover would
        // split in two, so the hunk cannot join and totality (2 != 3)
        // vetoes the group: fully ordinary.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n@@ -9,1 +9,1 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n@@ -20,1 +21,2 @@\n-    validated(&text);\n+    extra();\n+    validated_live(&text, &pre);\n";
        let pre = "fn m() {\n    validated(&text);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated(&text);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated(&text);\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_stranded_join_vetoes_extra_pre_image_site() {
        // Joining makes the group n=3, but the pre-image holds a
        // fourth untouched `validated(&text)` — totality 3 != 4
        // vetoes: sweeping would over-match the guard site.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n@@ -9,1 +9,1 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n@@ -20,1 +21,4 @@\n-    validated(&text);\n+    validated_live(&text, &pre);\n+    let done = extra();\n+\n+    done();\n";
        let pre = "fn m() {\n    validated(&text);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated(&text);\n    validated(&text);\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    x();\n    validated(&text);\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_yields_to_hetero_call_sweep() {
        // Hetero-arg pairs never group (count 1 each), so the
        // whole-section call sweep still wins with its metavars.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n }\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(out.rsmpl.contains("- foo(E0)\n"), "got: {}", out.rsmpl);
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }

    #[test]
    fn verbatim_sweep_let_statements_compress() {
        // T440 (flips the old 05de4d1 veto): `let` scrutinees parse as
        // `Expr::Let`, but complete single-line `let` statements take
        // the `Stmt` path — the engine matches concrete lets
        // structurally (probe-verified `applyrs --check` clean), so the
        // identical pair compresses. T449: the `let` unwrap in
        // `parse_call_expr` promotes this hetero-callee pair to the
        // call sweep (same shape class as bare statements now; chain
        // order tries call sweeps before verbatim — sizes tie within
        // a byte); verbatim keeps pairs no call shape can express.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let out = run(a);\n+    let out = run2(a);\n@@ -9,1 +10,1 @@\n-    let out = run(a);\n+    let out = run2(a);\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=calls_run_to_run2\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(out.rsmpl.contains("- run(E0)\n"), "got: {}", out.rsmpl);
        assert!(out.rsmpl.contains("+ run2(E0)\n"), "got: {}", out.rsmpl);
        rsmpl_patch::parse_patch(&out.rsmpl).expect("call sweep hunk must parse");
    }
    #[test]
    fn verbatim_sweep_macro_minus_stays_passthrough() {
        // 33a8cbb shape: plain patterns never enter macro bodies (SPEC 21);
        // the engine rejects the shape outright (loud PATTERN_CONFLICT).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    h = collapse(&format!(\"a/{p}\"));\n+    h = mk(p);\n@@ -9,1 +10,1 @@\n-    h = collapse(&format!(\"a/{p}\"));\n+    h = mk(p);\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_sweep_try_over_closure_stays_passthrough() {
        // a2fc9b3 shape: Try alone and closures alone match fine, but a
        // `?`-applied call wrapping a closure matches nothing (loud
        // MATCH_CONFLICT) — probe-bisected, so the gate is exactly this
        // combination.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    c(&x).map_err(|e| g(&e))?;\n+    c(&x).map_err(H)?;\n@@ -9,1 +10,1 @@\n-    c(&x).map_err(|e| g(&e))?;\n+    c(&x).map_err(H)?;\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule="), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_sweep_closure_without_try_compresses() {
        // Gate precision: closures alone match fine (probe-verified), so a
        // closure-bearing pair without `?` compresses once totality holds.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    wrap(map_err(|e| g(&e)));\n+    wrap(map2(|e| g(&e)));\n@@ -9,1 +10,1 @@\n-    wrap(map_err(|e| g(&e)));\n+    wrap(map2(|e| g(&e)));\n";
        let pre = "fn f() {\n    wrap(map_err(|e| g(&e)));\n    wrap(map_err(|e| g(&e)));\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(!out.rsmpl.contains("rule=verbatim_2"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_sweep_bare_call_literal_repeat_compresses() {
        // T422 (closed, no detector): same-callee literal-arg repeats
        // (`run("old")` -> `run("new")` xN) already compress via the
        // verbatim sweep - hetero receivers (the T420 gap) cannot exist
        // for bare calls, and a dedicated `calls_run_literal` hunk would
        // only rename the rule (`verbatim` is shorter) for byte-identical
        // bodies. A same-callee/different-arg neighbor must not veto
        // (subterm totality keys on the full needle).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,6 +1,6 @@\n fn m() {\n-    run(\"old_literal_value_long_enough_for_token_win\");\n-    run(\"old_literal_value_long_enough_for_token_win\");\n-    run(\"old_literal_value_long_enough_for_token_win\");\n+    run(\"new_literal_value_long_enough_for_token_win\");\n+    run(\"new_literal_value_long_enough_for_token_win\");\n+    run(\"new_literal_value_long_enough_for_token_win\");\n     run(\"something_else_entirely\");\n }\n";
        let pre = "fn m() {\n    run(\"old_literal_value_long_enough_for_token_win\");\n    run(\"old_literal_value_long_enough_for_token_win\");\n    run(\"old_literal_value_long_enough_for_token_win\");\n    run(\"something_else_entirely\");\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl
                .contains("- run(\"old_literal_value_long_enough_for_token_win\")\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl
                .contains("+ run(\"new_literal_value_long_enough_for_token_win\")\n"),
            "got: {}",
            out.rsmpl
        );
        assert_eq!(
            out.rsmpl.matches("@@ -1,1 @@").count(),
            1,
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim hunk must parse");
    }
    #[test]
    fn verbatim_sweep_totality_fires_past_substring_false_positive() {
        // 50d4a49 precision: `&state.parsed` inside `&state.parsed.items` is
        // a different subterm (probe-verified the engine leaves it alone),
        // so totality holds and the group compresses. String literals here
        // carry no needle (opaque-text veto would fire otherwise).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -9,1 +10,1 @@\n-        &state.parsed,\n+        parsed,\n";
        let pre = "fn f(s: S) {\n    g(&state.parsed, 1);\n    h(&state.parsed);\n    k(&state.parsed.items, 2);\n    l(\"see docs\");\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("- &state.parsed\n"),
            "got: {}",
            out.rsmpl
        );
    }
    #[test]
    fn verbatim_sweep_totality_vetoes_unchanged_occurrence() {
        // 712ad9d soundness: the needle occurs as a subterm in an unchanged
        // guard line, so totality fails and the section stays ordinary —
        // without this veto the sweep would silently rewrite the guard.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n@@ -9,1 +10,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n";
        let pre = "fn f(c: C) {\n    g(&cand.body_text);\n    g(&cand.body_text);\n    if x != &cand.body_text {}\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_sweep_totality_fires_past_unrelated_string_twin() {
        // T411 line-aware: the literal twin sits on line 4 while the hunks
        // change old lines 2 and 9 — it cannot hide a diff site, so the
        // group compresses (the engine never matches inside literals).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n@@ -9,1 +10,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n";
        let pre = "fn f(c: C) {\n    g(&cand.body_text);\n    g(&cand.body_text);\n    l(\"use &cand.body_text here\");\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
    }
    #[test]
    fn verbatim_sweep_totality_vetoes_string_twin_on_changed_line() {
        // T411: the literal twin sits on old line 9, changed by the second
        // hunk — it could hide a diff site, so the group stays ordinary.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n@@ -9,1 +10,1 @@\n-        &cand.body_text,\n+        slice(&cand.body_text),\n";
        let pre = "fn f(c: C) {\n    g(&cand.body_text);\n    g(&cand.body_text);\n    h();\n    h();\n    h();\n    h();\n    h();\n    l(\"use &cand.body_text here\");\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_sweep_totality_vetoes_unparseable_pre_image() {
        // A pre-image that does not parse cannot be verified — veto rather
        // than emit a patch `applyrs` would fail (the rewritten-file
        // reparse still guards the apply side).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        &state.parsed,\n+        parsed,\n@@ -9,1 +10,1 @@\n-        &state.parsed,\n+        parsed,\n";
        let pre = "this is not rust {{{\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_sweep_return_shape_compresses() {
        // T418 (found by the full-history sweep on b9c00d5): `return ...`
        // parses as `Expr::Return`, which the engine general matcher now
        // handles structurally (T412 gate lifted for Return only), so the
        // pair compresses with exact totality — history_e2e pins the live
        // `--check` + byte-compare on b9c00d5 itself.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    return compress(section, &path, used);\n+    return compress(section, &path, extra, used);\n@@ -9,1 +10,1 @@\n-    return compress(section, &path, used);\n+    return compress(section, &path, extra, used);\n";
        let pre = "fn f() {\n    if x {\n        return compress(section, &path, used);\n    }\n    return compress(section, &path, used);\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim"),
            "got: {}",
            out.rsmpl
        );
    }
    #[test]
    fn verbatim_sweep_leading_bang_compresses() {
        // T415 (unblocked by the unary router fix; b208847 compresses):
        // leading-`!` bypasses to `Raw` and compiles as a general unary
        // expr (mirrors the T252 `&` bypass), the engine matches `Unary`
        // structurally, and the pre-image holds exactly two subterms.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    !hit(&a, &b)\n+    !hit(&a, &b, c)\n@@ -9,1 +10,1 @@\n-    !hit(&a, &b)\n+    !hit(&a, &b, c)\n";
        let pre = "fn f() {\n    if !hit(&a, &b) {}\n    if !hit(&a, &b) {}\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim hunk must parse");
    }
    #[test]
    fn verbatim_sweep_leading_neg_compresses() {
        // T415 symmetric case: leading-`-` (`-f(...)` parses as
        // `Expr::Unary::Neg`) bypasses the call router to `Raw` exactly
        // like leading-`!`, so the identical pair compresses.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    -neg(x)\n+    -pos(x)\n@@ -9,1 +10,1 @@\n-    -neg(x)\n+    -pos(x)\n";
        let pre = "fn f(x: i32) -> i32 {\n    -neg(x)\n}\nfn g(x: i32) -> i32 {\n    -neg(x)\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim hunk must parse");
    }
    #[test]
    fn verbatim_sweep_field_label_strips_and_compresses() {
        // T416 (mined from c48a6e6): struct-literal `label: expr` lines
        // fail `Expr` parse on the full line — the sweep strips the label
        // and compresses on the inner expression (the engine visits field
        // values as sub-expressions; probe-verified `--check` clean).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        text: normalize(&sub),\n+        text: normalize(&wrap(&sub)),\n@@ -9,1 +10,1 @@\n-        text: normalize(&sub),\n+        text: normalize(&wrap(&sub)),\n";
        let pre = "struct S { text: String }\nfn f(sub: &str) -> S {\n    S { text: normalize(&sub) }\n}\nfn g(sub: &str) -> S {\n    S { text: normalize(&sub) }\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("- normalize(&sub)\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("+ normalize(&wrap(&sub))\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(!out.rsmpl.contains("- text:"), "got: {}", out.rsmpl);
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim hunk must parse");
    }
    #[test]
    fn verbatim_sweep_mismatched_labels_stay_passthrough() {
        // T416 boundary: the label itself changed (`text:` -> `name:`) —
        // the inner-expression rewrite would leave the wrong label, so
        // the pair stays ordinary.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-        text: normalize(&sub),\n+        name: normalize(&wrap(&sub)),\n@@ -9,1 +10,1 @@\n-        text: normalize(&sub),\n+        name: normalize(&wrap(&sub)),\n";
        let pre = "struct S { text: String }\nfn f(sub: &str) -> S {\n    S { text: normalize(&sub) }\n}\nfn g(sub: &str) -> S {\n    S { text: normalize(&sub) }\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_let_sweep_fires_across_hunks() {
        // T440 miniature of the 05de4d1 shape: two identical complete
        // `let` migration statements split by a pure addition (which
        // vetoes whole-section balance) compress to one literal hunk
        // with the `;`-terminated lines; the addition stays ordinary.
        // The engine `Stmt` shape matches concrete lets structurally
        // (probe-verified `applyrs --check` clean).
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let out = run(dir.path(), &[\"--patch\", patch]);\n+    let out = run_stdin(dir.path(), patch, &[]);\n@@ -9,1 +10,1 @@\n-    let out = run(dir.path(), &[\"--patch\", patch]);\n+    let out = run_stdin(dir.path(), patch, &[]);\n@@ -20,0 +21,1 @@\n+let cached = state.parsed;\n";
        let pre = "fn f() {\n    let out = run(dir.path(), &[\"--patch\", patch]);\n    let out = run(dir.path(), &[\"--patch\", patch]);\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl
                .contains("- let out = run(dir.path(), &[\"--patch\", patch]);\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl
                .contains("+ let out = run_stdin(dir.path(), patch, &[]);\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("+let cached = state.parsed;\n"),
            "got: {}",
            out.rsmpl
        );
        assert_eq!(
            out.rsmpl.matches("@@ -1,1 @@").count(),
            1,
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim let hunk must parse");
    }
    #[test]
    fn verbatim_let_sweep_twin_vetoes() {
        // T440 soundness: an unchanged twin of the minus `let` in the
        // pre-image breaks statement totality, so the group stays
        // ordinary — without this veto the sweep would rewrite the twin.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let out = run_old(dir.path());\n+    let out = run_new(dir.path());\n@@ -9,1 +10,1 @@\n-    let out = run_old(dir.path());\n+    let out = run_new(dir.path());\n";
        let pre = "fn f() {\n    let out = run_old(dir.path());\n    let out = run_old(dir.path());\n    let out = run_old(dir.path());\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_let_multiline_fragment_stays_passthrough() {
        // T440 boundary (b849869 "failed loud" negative pin): a `let`
        // declarator line without init/semicolon never parses as `Stmt`
        // (the engine has no fragment shape), so the pair stays ordinary.
        // Continuation lines differ per site (n=1 each) so no expression
        // group fires either — the hunk is genuinely uncovered.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,2 +2,2 @@\n-    let lenient =\n-        run(dir.path(), patch, &[\"--check\"]);\n+    let mismatched =\n+        run_stdin(dir.path(), patch, &[\"--check\"]);\n@@ -9,2 +10,2 @@\n-    let lenient =\n-        run(dir.path(), other, &[\"--check\"]);\n+    let mismatched =\n+        run_stdin(dir.path(), other, &[\"--check\"]);\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_let_else_stays_passthrough() {
        // T440 boundary: `let-else` diverges — the engine never moves into
        // let-else targets, so the pair stays ordinary.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let Some(x) = opt_old() else { return; };\n+    let Some(x) = opt_new() else { return; };\n@@ -9,1 +10,1 @@\n-    let Some(x) = opt_old() else { return; };\n+    let Some(x) = opt_new() else { return; };\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_let_macro_init_stays_passthrough() {
        // T440 boundary: a macro invocation in the init matches nothing
        // (the engine never enters macro bodies), so the pair stays
        // ordinary rather than failing loud.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let v = vec_old!(1, 2, 3);\n+    let v = vec_new!(1, 2, 3);\n@@ -9,1 +10,1 @@\n-    let v = vec_old!(1, 2, 3);\n+    let v = vec_new!(1, 2, 3);\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_let_init_mismatch_stays_passthrough() {
        // T440 boundary: init-less `let z;` vs initialized `let z = 1;`
        // never matches in the engine (`(Some, None) => false`), so the
        // pair stays ordinary.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let z;\n+    let z = 1;\n@@ -9,1 +10,1 @@\n-    let z;\n+    let z = 1;\n";
        let out = infer_rsmpl(raw).unwrap();
        assert!(!out.rsmpl.contains("rule=verbatim"), "got: {}", out.rsmpl);
    }
    #[test]
    fn verbatim_let_initless_pair_compresses() {
        // T440 gate symmetry: init-less on both sides matches in the
        // engine (`(None, None) => true`, probe-verified `applyrs
        // --check` clean), so the identical pair compresses when the
        // byte-guard clears.
        let raw = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -2,1 +2,1 @@\n-    let pending_watcher_item;\n+    let active_watcher_item;\n@@ -9,1 +10,1 @@\n-    let pending_watcher_item;\n+    let active_watcher_item;\n";
        let pre = "fn f() {\n    let pending_watcher_item;\n    let pending_watcher_item;\n}\n";
        let mut map = std::collections::HashMap::new();
        map.insert("f.rs".to_string(), pre.to_string());
        let out = infer_rsmpl_with_pre_images(raw, &map).unwrap();
        assert!(
            out.rsmpl.contains("@@ -1,1 @@ rule=verbatim\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("- let pending_watcher_item;\n"),
            "got: {}",
            out.rsmpl
        );
        assert!(
            out.rsmpl.contains("+ let active_watcher_item;\n"),
            "got: {}",
            out.rsmpl
        );
        rsmpl_patch::parse_patch(&out.rsmpl).expect("verbatim let hunk must parse");
    }
}
