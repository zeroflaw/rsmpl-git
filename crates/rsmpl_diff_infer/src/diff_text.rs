//! Native diff text utilities (WI-15 D5 split).
//!
//! Split from the crate root under the 2000-line gate (D5): pure move,
//! no behavior change. Owns the `diff --git` section splitting,
//! binary/gitlink stripping, header collapsing, and ranged-vs-sweep
//! emission helpers. The crate root re-exports the shared names so
//! `crate::b_side_path` and friends keep resolving with no call-site
//! changes.

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
pub(crate) fn strip_unpatchable_sections(raw: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
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

/// WI-1b ranged-vs-sweep emission (SPEC D.4/D.5): `sites` are the
/// 1-based old-side minus lines a detector class covers (the existing
/// `totality::changed_old_lines` output, already threaded as `changed`).
/// Empty (unparseable header), any `0` (new-file `@@ -0,0`), or a span
/// (`max - min + 1`) over 64 lines emits the canonical sweep dummy
/// `@@ -1,1 @@` + `# sweep`; otherwise the tight original range
/// `@@ -min,span @@` + `# allow=many` (no `# scope=file`).
pub(crate) fn emission_header(path: &str, rule: &str, sites: &[usize]) -> String {
    let header = git_header(path);
    if sites.is_empty() || sites.contains(&0) {
        return format!("{header}\n@@ -1,1 @@ rule={rule}\n# sweep\n");
    }
    let min = sites.iter().min().copied().unwrap_or(0);
    let max = sites.iter().max().copied().unwrap_or(0);
    if min < 1 {
        return format!("{header}\n@@ -1,1 @@ rule={rule}\n# sweep\n");
    }
    let span = max.saturating_sub(min).saturating_add(1);
    if span <= 64 {
        format!("{header}\n@@ -{min},{span} @@ rule={rule}\n# allow=many\n")
    } else {
        format!("{header}\n@@ -1,1 @@ rule={rule}\n# sweep\n")
    }
}

/// WI-1b body-only variant for section drivers (`try_call_peel_section`,
/// `try_call_literal_section`, `verbatim::try_verbatim_sweep`): the file
/// header lives in the section prelude, so only the hunk header is
/// emitted. Same ≤64 rule; overlapping groups are resolved by callers
/// (disjoint ranged, else sweep).
pub(crate) fn emission_body(rule: &str, sites: &[usize]) -> String {
    if sites.is_empty() || sites.contains(&0) {
        return format!("@@ -1,1 @@ rule={rule}\n# sweep\n");
    }
    let min = sites.iter().min().copied().unwrap_or(0);
    let max = sites.iter().max().copied().unwrap_or(0);
    if min < 1 {
        return format!("@@ -1,1 @@ rule={rule}\n# sweep\n");
    }
    let span = max.saturating_sub(min).saturating_add(1);
    if span <= 64 {
        format!("@@ -{min},{span} @@ rule={rule}\n# allow=many\n")
    } else {
        format!("@@ -1,1 @@ rule={rule}\n# sweep\n")
    }
}
/// Drop git's trailing `@@ … @@ <title>` section titles on
/// two-sided (native ordinary) hunk headers.
pub(crate) fn strip_hunk_titles(section: &str) -> String {
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
pub(crate) fn strip_index_lines(section: &str) -> String {
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
pub(crate) fn collapse_git_headers(section: &str) -> String {
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
