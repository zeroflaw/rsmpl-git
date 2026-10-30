//! Git-facing source for the inverse mapping (`diffrs`).
//!
//! Shells out to external `git` (the T294/T299 decision binds this: no
//! `gix` dependency for a minimal CLI). Collects the pending native
//! diff — tracked modifications via `git diff`, plus untracked files
//! via `git status` (emitted as new-file ordinary sections, since
//! `git diff` never shows them) — for the infer crate to compress.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("cannot run git diff in {repo}: {inner}")]
    Git { repo: String, inner: String },
    #[error("cannot read untracked file {path}: {inner}")]
    Untracked { path: String, inner: String },
}

/// What to collect: `staged` selects `git diff --cached` (index vs
/// `HEAD`/rev); otherwise the worktree-vs-index diff. `rev` forwards a
/// single `git diff [rev]` revision (e.g. `HEAD`, `HEAD~1`) to also
/// cover history with the same mechanism.
pub struct GitSource {
    pub repo: PathBuf,
    pub staged: bool,
    pub rev: Option<String>,
}

/// Raw native diff plus the file list it covers (tracked and untracked).
pub struct CollectedDiff {
    pub raw: String,
    pub files: Vec<String>,
    /// A top-level `.history/` audit log was present but skipped
    /// (warned on stderr, never embedded).
    pub skipped_history: bool,
    /// Untracked directory entries skipped during collection (nested
    /// repos, empty dirs — T387; warned on stderr, never embedded).
    pub skipped_untracked_dirs: Vec<String>,
}

impl GitSource {
    pub fn collect(&self) -> Result<CollectedDiff, SourceError> {
        let mut cmd = Command::new("git");
        cmd.arg("-C")
            .arg(&self.repo)
            .arg("diff")
            .arg("--no-color")
            .arg("--no-ext-diff")
            .arg("--unified=0");
        if self.staged {
            cmd.arg("--cached");
        }
        if let Some(rev) = &self.rev {
            cmd.arg(rev);
        }
        let out = cmd.output().map_err(|e| SourceError::Git {
            repo: self.repo.display().to_string(),
            inner: format!("failed to run `git diff` (is git installed?): {e}"),
        })?;
        if !out.status.success() {
            return Err(SourceError::Git {
                repo: self.repo.display().to_string(),
                inner: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        // T396: per-section UTF-8 validity instead of whole-output
        // lossy decoding — valid-UTF-8 sections pass through
        // byte-identical (U+FFFD chars in them are genuine text, e.g.
        // string-literal test data), while invalid-UTF-8 sections
        // (binary creates, which native `git diff` dumps raw) become
        // `Binary files … differ` markers for the infer strip.
        let mut raw = tracked_raw(&out.stdout);
        // Tracked/staged `.history/` audit logs (SPEC D.1/D.4) skip the
        // same way as untracked ones: tool output, never a lowering
        // input (warned by the binary, like stripped binaries).
        let (stripped_raw, stripped_tracked_history) = strip_history_sections(&raw);
        raw = stripped_raw;
        // `git diff` detects renames by default (`similarity index`
        // + `rename from/to` headers), which applyrs lowers back to
        // rename sections (T377) -- moves stay moves instead of
        // delete+create pairs. `--unified=0` keeps ordinary passthrough minimal-token:
        // zero-context hunks still round-trip through `applyrs
        // --check`, and inference reads only `-`/`+` lines.

        // Untracked files never appear in `git diff`; emit them as
        // new-file ordinary sections so the patch covers all pending
        // changes (agents create new files constantly). Pending worktree
        // scope only (T392): `git diff --cached` and `git diff [rev]`
        // never show untracked files, so staged/rev conversions must not
        // embed them either (they leaked into both, misrepresenting
        // index/history scope).
        let mut untracked = if self.rev.is_none() && !self.staged {
            untracked_files(&self.repo).map_err(|e| SourceError::Git {
                repo: self.repo.display().to_string(),
                inner: e,
            })?
        } else {
            Vec::new()
        };
        untracked.sort();
        // `.history/` is the `applyrs` audit log (SPEC D.1): tool
        // output, never a lowering input. Old worktrees (whose
        // `.gitignore` predates the T253 entry) still report it as
        // untracked; embedding it would feed previous patches back
        // into the next conversion, so skip it here (warned by the
        // binary, like stripped binaries).
        let (untracked, skipped): (Vec<String>, Vec<String>) =
            untracked.into_iter().partition(|p| !is_history_path(p));
        let skipped_history = !skipped.is_empty() || stripped_tracked_history;
        // Untracked directories (T387): `git status -uall` reports a
        // nested repo as `?? inner/` and an empty dir as `?? dir/` —
        // real `git diff` shows neither, and there is no content to
        // embed (a nested repo's files belong to another repo, an
        // empty dir holds nothing). Reading them as files fails with
        // EISDIR, so skip them here (warned by the binary) instead of
        // failing the whole conversion on unrelated state.
        //
        // T390: `--repo` may be a subdirectory (agents run from one);
        // `git diff`/`git status` paths are already repo-root-relative,
        // so resolve the filesystem base to the worktree top level
        // once — joining them onto the subdir reads `sub/sub/…`.
        // Output paths stay root-relative (check/apply with the root).
        let base = toplevel(&self.repo);
        let mut skipped_untracked_dirs = Vec::new();
        for path in &untracked {
            let abs = base.join(path);
            // Read the link itself, never follow it (T354, mirrors the
            // T353 applyrs read side): a dangling symlink would fail
            // `std::fs::read` with `SourceError` (exit 2) and a
            // link-to-dir with EISDIR; `read_link` serves both.
            let (content, mode) = match std::fs::symlink_metadata(&abs) {
                Ok(md) if md.file_type().is_dir() => {
                    skipped_untracked_dirs.push(path.clone());
                    continue;
                }
                Ok(md) if md.file_type().is_symlink() => match std::fs::read_link(&abs) {
                    Ok(target) => (target.to_string_lossy().into_owned().into_bytes(), "120000"),
                    Err(e) => {
                        return Err(SourceError::Untracked {
                            path: path.clone(),
                            inner: e.to_string(),
                        });
                    }
                },
                Ok(md) => (
                    std::fs::read(&abs).map_err(|e| SourceError::Untracked {
                        path: path.clone(),
                        inner: e.to_string(),
                    })?,
                    // T364: preserve the exec bit so scripts round-trip
                    // with their mode (the old hardcoded 100644 silently
                    // dropped +x).
                    regular_file_mode(&md),
                ),
                Err(e) => {
                    return Err(SourceError::Untracked {
                        path: path.clone(),
                        inner: e.to_string(),
                    });
                }
            };
            // Binary untracked files cannot ride a UTF-8 patch: emit the
            // same `Binary files … differ` marker section `git diff`
            // shows once staged, so the infer stage strips them with a
            // warning (T378) instead of silently dropping them (T364
            // silent-drop class).
            // T396: NUL is git's binary definition; anything else that
            // is not valid UTF-8 (e.g. a no-NUL binary blob, which the
            // old lossy decode turned into U+FFFD text) takes the same
            // marker — U+FFFD itself is genuine text now that the
            // tracked side checks validity at the byte level.
            if content.contains(&0) || std::str::from_utf8(&content).is_err() {
                // T399: C-quote `"`, `\` + control slots (native
                // quotepath shape; plain paths stay raw).
                let (old_slot, new_slot) = (
                    rsmpl_patch::quote_git_path(&format!("a/{path}")),
                    rsmpl_patch::quote_git_path(&format!("b/{path}")),
                );
                raw.push_str(&format!(
                    "diff --git {old_slot} {new_slot}\nBinary files {old_slot} and {new_slot} differ\n"
                ));
                continue;
            }
            let text = String::from_utf8_lossy(&content);
            raw.push_str(&new_file_section(path, &text, mode));
        }

        let mut files = files_in_diff(&raw);
        for path in untracked {
            if skipped_untracked_dirs.contains(&path) {
                continue;
            }
            if !files.contains(&path) {
                files.push(path);
            }
        }
        Ok(CollectedDiff {
            raw,
            files,
            skipped_history,
            skipped_untracked_dirs,
        })
    }
}

/// Repo worktree top level for `--repo` (T390): `git -C <repo>
///
/// rev-parse --show-toplevel`, falling back to `<repo>` itself when
/// unresolvable (bare dir, git missing — callers already fail fast on
/// the real git invocations).
fn toplevel(repo: &Path) -> std::path::PathBuf {
    match Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
    {
        Ok(out) if out.status.success() => {
            let top = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if top.is_empty() {
                repo.to_path_buf()
            } else {
                std::path::PathBuf::from(top)
            }
        }
        _ => repo.to_path_buf(),
    }
}

/// Repo-relative paths of untracked files (`git status --porcelain`).
fn untracked_files(repo: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("status")
        .arg("--porcelain=v1")
        .arg("-uall")
        .output()
        .map_err(|e| format!("failed to run `git status` (is git installed?): {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let mut paths = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `?? <path>`; renames never appear as untracked. Quoted
        // tokens are C-quoted non-ASCII names (T389) and decode to
        // the real filename; unquoted tokens pass through.
        if let Some(path) = line.strip_prefix("?? ") {
            paths.push(rsmpl_patch::decode_git_path_token(path));
        }
    }
    Ok(paths)
}

/// Tool-internal audit paths: `applyrs` archives every run under
/// top-level `.history/` (SPEC D.1); those files are never lowering
/// inputs, so the source skips them even when untracked.
fn is_history_path(path: &str) -> bool {
    path == ".history" || path.starts_with(".history/")
}
/// Strip tracked/staged `.history/` sections from a unified diff
/// (WI-7-history-strip): returns the filtered diff plus whether
/// anything was stripped (warning parity with the untracked skip).
/// Split is line-start `diff --git ` so section bytes pass through
/// untouched; a section strips when its header paths — or either
/// `rename from/to` line when present — hit top-level `.history/`.
/// Single-path headers (`diff --git P`) check `P` directly.
fn strip_history_sections(raw: &str) -> (String, bool) {
    if raw.is_empty() {
        return (String::new(), false);
    }
    let mut starts: Vec<usize> = vec![0];
    let mut idx = 0usize;
    while idx < raw.len() {
        if raw.as_bytes()[idx] == b'\n' && raw[idx + 1..].starts_with("diff --git ") {
            starts.push(idx + 1);
        }
        idx += 1;
    }
    let mut out = String::new();
    let mut stripped = false;
    for (i, start) in starts.iter().enumerate() {
        let end = if i + 1 < starts.len() {
            starts[i + 1]
        } else {
            raw.len()
        };
        let section = &raw[*start..end];
        if section_is_history(section) {
            stripped = true;
            continue;
        }
        out.push_str(section);
    }
    (out, stripped)
}
/// One section is history when its header paths or rename lines point
/// at top-level `.history/` (either side counts: a move out of
/// `.history/` is still audit noise, never a lowering input).
fn section_is_history(section: &str) -> bool {
    let mut lines = section.lines();
    let header = match lines.next() {
        Some(h) => h,
        None => return false,
    };
    if let Some(rest) = header.strip_prefix("diff --git ") {
        if let Some(parts) = rsmpl_patch::split_git_paths(rest) {
            for part in &parts {
                let norm = part
                    .strip_prefix("a/")
                    .or_else(|| part.strip_prefix("b/"))
                    .unwrap_or(part);
                if is_history_path(norm) {
                    return true;
                }
            }
        }
    }
    for line in lines {
        if line.starts_with("diff --git ") {
            break;
        }
        if let Some(from) = line
            .strip_prefix("rename from ")
            .or_else(|| line.strip_prefix("rename to "))
        {
            let decoded = rsmpl_patch::decode_git_path_token(from.trim());
            if is_history_path(&decoded) {
                return true;
            }
        }
    }
    false
}
/// Mode for an untracked regular file (T364): `100755` when any exec
/// bit is set, else `100644`. Unix only -- elsewhere everything is
/// `100644` (no exec-bit concept to preserve).
#[cfg(unix)]
fn regular_file_mode(md: &std::fs::Metadata) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    if md.permissions().mode() & 0o111 != 0 {
        "100755"
    } else {
        "100644"
    }
}
#[cfg(not(unix))]
fn regular_file_mode(_md: &std::fs::Metadata) -> &'static str {
    "100644"
}

/// A new-file ordinary section for untracked `path` with `content` and
/// `mode` (`100644`, `100755`, or `120000` -- exactly what `git diff`
/// shows for a staged new file). The caller selects the mode: symlinks
/// read as `120000` (T354), executables as `100755` (T364, unix only).
/// Empty content emits headers only, exactly what real git shows for
/// an empty new file (T364) -- the old `@@ -0,0 +1,0 @@` shape was
/// never git's and masked the empty-create drop.
fn new_file_section(path: &str, content: &str, mode: &str) -> String {
    // T399: C-quote `"`, `\` + control slots (native quotepath shape;
    // plain paths stay raw — raw interpolation mis-splits quote names).
    let (old_slot, new_slot) = (
        rsmpl_patch::quote_git_path(&format!("a/{path}")),
        rsmpl_patch::quote_git_path(&format!("b/{path}")),
    );
    let mut section = format!("diff --git {old_slot} {new_slot}\nnew file mode {mode}\n");
    if content.is_empty() {
        return section;
    }
    section.push_str(&format!("--- /dev/null\n+++ {new_slot}\n"));
    // T385: split on `\n` (not `str::lines`, which strips `\r`) so
    // CRLF content round-trips byte-exact instead of converting to LF.
    let mut clines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        clines.pop();
    }
    section.push_str(&format!("@@ -0,0 +1,{} @@\n", clines.len()));
    for line in clines {
        section.push('+');
        section.push_str(line);
        section.push('\n');
    }
    // `git diff` ends a section missing the trailing newline with the
    // `\ No newline at end of file` marker; reproduce it so the shape
    // stays `git apply`-compatible.
    if !content.ends_with('\n') {
        section.push_str("\\ No newline at end of file\n");
    }
    section
}

/// Tracked-side raw diff with per-section UTF-8 validity (T396).
/// Valid-UTF-8 sections pass through byte-identical, so any U+FFFD
/// chars in them are genuine text (e.g. string-literal test data),
/// never a binary signal. Binary creates — native `git diff` dumps
/// them raw, since only modifies get the `Binary … differ` marker —
/// become marker sections for the infer strip, instead of
/// lossy-decoding into U+FFFD text that corrupts the round-trip
/// (the T395 shape).
///
/// The split itself is byte-level on line-start `diff --git `, so
/// validity never depends on decoding. A leading chunk that opens
/// no section passes through lossy (the old shape — infer fails it
/// as before, never silently).
fn tracked_raw(stdout: &[u8]) -> String {
    const MARK: &[u8] = b"diff --git ";
    let mut sections: Vec<&[u8]> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < stdout.len() {
        if stdout[i] == b'\n' && stdout[i + 1..].starts_with(MARK) {
            sections.push(&stdout[start..=i]);
            start = i + 1;
        }
        i += 1;
    }
    sections.push(&stdout[start..]);
    let mut raw = String::new();
    for section in sections {
        match std::str::from_utf8(section) {
            Ok(text) => raw.push_str(text),
            Err(_) => raw.push_str(&binary_marker_for(section)),
        }
    }
    raw
}

/// Marker replacing an invalid-UTF-8 tracked section (T396): keeps the
/// real `diff --git` header (lossy-decoded for the path — headers stay
/// ASCII under git's default quotepath) and swaps the body for the
/// `Binary files … differ` line the infer strip already owns. A chunk
/// that opens no section is not a diff at all: it returns lossy so
/// infer fails it exactly as before (no new silent shape).
fn binary_marker_for(section: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(section);
    let rest = match lossy
        .lines()
        .next()
        .unwrap_or("")
        .strip_prefix("diff --git ")
    {
        Some(rest) => rest,
        None => return lossy.into_owned(),
    };
    let path = match rsmpl_patch::split_git_paths(rest) {
        Some(parts) => {
            let side = parts.last().cloned().unwrap_or_default();
            side.strip_prefix("b/").unwrap_or(&side).to_string()
        }
        None => rest.trim().to_string(),
    };
    // T399: C-quote the marker slots (native shape for quoted names;
    // the `diff --git` line above keeps byte-identical native `rest`).
    let (old_slot, new_slot) = (
        rsmpl_patch::quote_git_path(&format!("a/{path}")),
        rsmpl_patch::quote_git_path(&format!("b/{path}")),
    );
    format!("diff --git {rest}\nBinary files {old_slot} and {new_slot} differ\n")
}

/// Repo-relative paths covered by a unified diff: the `b/` side of
/// every `diff --git a/… b/…` header (unquoted spaces rejoined via
/// `split_git_paths`, T379; a single-path header yields that path).
pub fn files_in_diff(raw: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = match rsmpl_patch::split_git_paths(rest) {
                Some(parts) => {
                    let side = parts.last().cloned().unwrap_or_default();
                    side.strip_prefix("b/").unwrap_or(&side).to_string()
                }
                None => continue,
            };
            if !files.contains(&path) {
                files.push(path);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "diff --git a/src/lib.rs b/src/lib.rs\nindex 1..2 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n.";

    #[test]
    fn files_in_diff_lists_both_sides_once() {
        let raw = "diff --git a/a.rs b/a.rs\n@@\n\
                   diff --git a/b.rs b/b.rs\n@@\n\
                   diff --git a/a.rs b/a.rs\n@@\n";
        assert_eq!(files_in_diff(raw), vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn new_file_section_shape() {
        let section = new_file_section("src/new.rs", "fn n() {}\n", "100644");
        assert!(section.starts_with("diff --git a/src/new.rs b/src/new.rs\n"));
        assert!(section.contains("new file mode 100644\n"));
        assert!(section.contains("--- /dev/null\n"));
        assert!(section.contains("@@ -0,0 +1,1 @@\n"));
        assert!(section.contains("+fn n() {}\n"));
    }

    #[test]
    fn new_file_section_symlink_mode() {
        // Link-target strings carry no trailing newline, so the
        // section reproduces git's exact shape (`+target` + marker).
        let section = new_file_section("link.txt", "real.txt", "120000");
        assert!(section.contains("new file mode 120000\n"));
        assert!(section.contains("+real.txt\n"));
        assert!(section.contains("\\ No newline at end of file\n"));
    }

    #[test]
    fn new_file_section_marks_missing_trailing_newline() {
        let section = new_file_section("x.txt", "no-newline", "100644");
        assert!(section.contains("\\ No newline at end of file\n"));
    }

    #[test]
    fn new_file_section_empty_is_headers_only() {
        // T364: an empty untracked file emits exactly what real git
        // shows (headers only, no hunk) -- the old `@@ -0,0 +1,0 @@`
        // shape never occurs in git output.
        let section = new_file_section("empty.txt", "", "100644");
        assert!(section.contains("new file mode 100644\n"));
        assert!(!section.contains("@@"), "got: {section}");
        assert!(!section.contains("--- /dev/null"), "got: {section}");
    }

    #[test]
    fn new_file_section_exec_mode_passes_through() {
        // T364: the caller-selected 100755 for executables reaches
        // the section verbatim (the lowering honors it).
        let section = new_file_section("run.sh", "#!/bin/sh\n", "100755");
        assert!(section.contains("new file mode 100755\n"));
        assert!(section.contains("--- /dev/null\n"));
    }

    #[test]
    fn new_file_section_preserves_crlf() {
        // T385: CRLF content must ride the `+` lines byte-exact —
        // `str::lines` stripped the `\r` and silently converted
        // worktree files to LF on apply.
        let section = new_file_section("win.txt", "a\r\nb\r\n", "100644");
        assert!(section.contains("@@ -0,0 +1,2 @@\n"), "got: {section}");
        assert!(section.contains("+a\r\n+b\r\n"), "got: {section}");
        assert!(!section.contains("\\ No newline"), "got: {section}");
    }

    #[test]
    fn files_in_diff_empty() {
        assert!(files_in_diff("").is_empty());
        assert!(files_in_diff(SAMPLE) == vec!["src/lib.rs"]);
    }

    #[test]
    fn files_in_diff_rejoins_unquoted_spaces() {
        // T379: real git never quotes spaces, so the `b/` side is
        // rejoined via the shared splitter instead of `splitn(2, ' ')`.
        let raw = "diff --git a/my file.txt b/my file.txt\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(files_in_diff(raw), vec!["my file.txt"]);
        // A single-path header (SPEC 6) yields that path, not "".
        assert_eq!(
            files_in_diff("diff --git src/lib.rs\n@@\n"),
            vec!["src/lib.rs"]
        );
    }

    #[test]
    fn history_paths_are_detected() {
        assert!(is_history_path(".history/x.rsmpl"));
        assert!(is_history_path(".history"));
        assert!(!is_history_path("src/.history/x"));
        assert!(!is_history_path("history.rs"));
        assert!(!is_history_path("src/lib.rs"));
    }

    #[test]
    fn strip_history_sections_drops_tracked_history() {
        // WI-7-history-strip: a tracked `.history/` modify strips
        // entirely (flag true); mixed output keeps the real section
        // byte-identical with the flag still set.
        let hist = "diff --git a/.history/a.rsmpl b/.history/a.rsmpl\n--- a/.history/a.rsmpl\n+++ b/.history/a.rsmpl\n@@ -1 +1 @@\n-x\n+y\n";
        let (out, stripped) = strip_history_sections(hist);
        assert_eq!(out, "");
        assert!(stripped);
        let real = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-a\n+b\n";
        let mixed = format!("{real}{hist}");
        let (out, stripped) = strip_history_sections(&mixed);
        assert_eq!(out, real);
        assert!(stripped);
        let (out, stripped) = strip_history_sections(real);
        assert_eq!(out, real);
        assert!(!stripped);
        assert_eq!(strip_history_sections(""), ("".to_string(), false));
    }

    #[test]
    fn strip_history_sections_keeps_nested_and_quoted() {
        // Nested `src/.history/` is not tool output (kept); a quoted
        // top-level `.history/` name still strips; rename lines into
        // `.history/` strip on either side.
        let nested = "diff --git a/src/.history/x b/src/.history/x\n@@ -1 +1 @@\n-a\n+b\n";
        let (out, stripped) = strip_history_sections(nested);
        assert_eq!(out, nested);
        assert!(!stripped);
        let quoted = "diff --git \"a/.history/q\\\"x.rsmpl\" \"b/.history/q\\\"x.rsmpl\"\n@@ -1 +1 @@\n-a\n+b\n";
        let (out, stripped) = strip_history_sections(quoted);
        assert_eq!(out, "");
        assert!(stripped);
        let renamed = "diff --git a/f.rs b/.history/a.rsmpl\nsimilarity index 90%\nrename from f.rs\nrename to .history/a.rsmpl\n";
        let (out, stripped) = strip_history_sections(renamed);
        assert_eq!(out, "");
        assert!(stripped);
        let single = "diff --git .history/a.rsmpl\n@@ -1 +1 @@\n-a\n+b\n";
        let (out, stripped) = strip_history_sections(single);
        assert_eq!(out, "");
        assert!(stripped);
    }

    #[test]
    fn tracked_raw_passes_valid_utf8_byte_identical() {
        // T396: valid-UTF-8 sections (even carrying genuine U+FFFD
        // chars, e.g. string-literal fixtures) pass through untouched;
        // empty and non-diff garbage keep the old lossy shape (failed
        // downstream as before — no new silent shape).
        let section = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-const S: &str = \"�\";\n+const S: &str = \"�!\";\n";
        assert_eq!(tracked_raw(section.as_bytes()), section);
        assert_eq!(tracked_raw(b""), "");
        assert_eq!(
            tracked_raw(b"fatal: not a git repository\n"),
            "fatal: not a git repository\n"
        );
    }

    #[test]
    fn tracked_raw_replaces_invalid_utf8_with_binary_marker() {
        // T396: a binary create (native `git diff` dumps raw bytes —
        // only modifies get the marker) becomes the same `Binary
        // files … differ` section staged modifies get, so infer strips
        // + warns it. The old whole-output lossy decode turned these
        // bytes into U+FFFD text that corrupted the lowering (T395).
        let mut create = b"diff --git a/blob.bin b/blob.bin\nnew file mode 100644\n".to_vec();
        create.extend_from_slice(b"--- /dev/null\n+++ b/blob.bin\n@@ -0,0 +1 @@\n+");
        create.extend_from_slice(&[0x80, 0x81, 0xFE, b'\n']);
        let out = tracked_raw(&create);
        assert!(
            out.contains("Binary files a/blob.bin and b/blob.bin differ\n"),
            "got: {out}"
        );
        assert!(!out.contains('�'), "got: {out}");
        // Mixed output: the text section survives byte-identical while
        // the binary section is replaced.
        let mut mixed = b"diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n".to_vec();
        mixed.extend_from_slice(b"@@ -1 +1 @@\n-a\n+b\n");
        mixed.extend_from_slice(&create);
        let out = tracked_raw(&mixed);
        assert!(out.contains("-a\n+b\n"), "got: {out}");
        assert!(
            out.contains("Binary files a/blob.bin and b/blob.bin differ\n"),
            "got: {out}"
        );
    }

    #[test]
    fn new_file_section_quotes_quote_and_backslash_names() {
        // T399: raw interpolation mis-split quote/backslash names at
        // both ends (silent wrong-file write, rc 0) — C-quote them
        // (native quotepath shape); plain names stay raw.
        let section = new_file_section("q\"ute.txt", "hello\n", "100644");
        assert!(
            section.contains("diff --git \"a/q\\\"ute.txt\" \"b/q\\\"ute.txt\"\n"),
            "got: {section}"
        );
        assert!(
            section.contains("+++ \"b/q\\\"ute.txt\"\n"),
            "got: {section}"
        );
        assert!(
            !section.contains("a/q\"ute.txt b/"),
            "raw unquoted header: {section}"
        );
        let section = new_file_section("back\\slash.txt", "hi\n", "100644");
        assert!(
            section.contains("\"a/back\\\\slash.txt\" \"b/back\\\\slash.txt\""),
            "got: {section}"
        );
        let plain = new_file_section("plain.txt", "hi\n", "100644");
        assert!(
            plain.starts_with("diff --git a/plain.txt b/plain.txt\n"),
            "got: {plain}"
        );
    }

    #[test]
    fn binary_marker_for_quotes_quoted_names() {
        // T399: the marker for a quoted-name binary section keeps the
        // real name (C-quoted slots, native shape for quoted names).
        let mut section =
            b"diff --git \"a/q\\\"bin.txt\" \"b/q\\\"bin.txt\"\nnew file mode 100644\n".to_vec();
        section.extend_from_slice(b"--- /dev/null\n+++ \"b/q\\\"bin.txt\"\n@@ -0,0 +1 @@\n+");
        section.extend_from_slice(&[0x80, 0x81, b'\n']);
        let out = binary_marker_for(&section);
        assert!(
            out.contains("\"a/q\\\"bin.txt\" and \"b/q\\\"bin.txt\""),
            "got: {out}"
        );
    }
}
