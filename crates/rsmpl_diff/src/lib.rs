//! Facade for the inverse mapping (`diffrs`): git source → infer →
//! emit RSmPL.
//!
//! The pub API boundary the `diffrs` binary and T341 tests pin:
//! `compress` collects the native diff and returns a validated
//! RSmPL-Git patch (v1: fn/item renames + slot-sweep renames +
//! method-to-function + method-to-method + call-sweep fan-out compressed inside `rsmpl_diff_infer`, rest
//! ordinary passthrough).

use std::path::Path;

use std::collections::HashMap;
use std::process::Command;

pub use rsmpl_diff_infer::{
    declined_weight, sorted_declined, summary_counts, totals_line, DeclinedTotals, HunkDeclined,
    HunkProvenance, InferError, InferredPatch,
};
pub use rsmpl_diff_source::{CollectedDiff, GitSource, SourceError};
pub use verify::{FirstDivergence, VerifyError, VerifyReport};

pub mod verify;

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Infer(#[from] InferError),
    #[error(transparent)]
    Verify(#[from] VerifyError),
}

/// Pre-image file contents for the T403 totality check (verbatim
/// sweeps only): `.rs` paths in the diff mapped to the old text the
/// hunks apply against — the index blob for pending worktree diffs
/// (`git show :path`, exact whether or not the index is layered), the
/// `rev` blob for staged and rev diffs. Fetch failures omit the entry
/// (that section falls back to section-visible evidence); the omitted
/// paths return alongside so the caller can warn that `proven`
/// downgraded (WI-10a).
fn pre_images(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
    files: &[String],
    raw: &str,
) -> (HashMap<String, String>, Vec<String>) {
    // Index blob (`:path`) for pending worktree diffs — exact whether or
    // not the index is layered; the `rev` blob otherwise. A revision
    // *range* (`A..B` / `A...B`) diffs against its endpoints, but the
    // hunks apply against the left (`A`) side — `git show A..B:path`
    // never resolves, so the base is the range's left side (WI-10a).
    let base = if staged || rev.is_some() {
        rev_base(rev.unwrap_or("HEAD"))
    } else {
        String::new()
    };
    let mut out = HashMap::new();
    let mut missing = Vec::new();
    for path in files {
        if !path.ends_with(".rs") {
            continue;
        }
        let spec = format!("{base}:{path}");
        let got = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show", &spec])
            .output();
        if let Ok(o) = got {
            if o.status.success() {
                if let Ok(text) = String::from_utf8(o.stdout) {
                    out.insert(path.clone(), text);
                }
            }
        }
        // WI-7a: rename sections cite the new path in `files` (b-side), but
        // the new path does not exist at base — the hunks apply against the
        // OLD file's content (applyrs seeds the new-side entry from it).
        // Fetch `base:<old>` under the NEW-path key so detectors get
        // pre-image totality instead of section-visible fallback. The
        // `rename from/to` lines are authoritative (the parser prefers them
        // over the `diff --git` halves for spaced/quoted paths).
        let mut rename_old: Option<String> = None;
        let mut rename_new: Option<String> = None;
        let mut flush_rename = |old: &mut Option<String>, new: &mut Option<String>| {
            if let (Some(o), Some(n)) = (old.take(), new.take()) {
                if n.ends_with(".rs") && !out.contains_key(&n) {
                    let spec = format!("{base}:{o}");
                    if let Ok(got) = Command::new("git")
                        .arg("-C")
                        .arg(repo)
                        .args(["show", &spec])
                        .output()
                    {
                        if got.status.success() {
                            if let Ok(text) = String::from_utf8(got.stdout) {
                                out.insert(n, text);
                            }
                        }
                    }
                }
            }
        };
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("diff --git ") {
                flush_rename(&mut rename_old, &mut rename_new);
                let _ = rest;
            } else if let Some(from) = line.strip_prefix("rename from ") {
                // T398: C-quoted values decode via the shared quotepath
                // rule (same as with_ab_prefix); verbatim quotes miss
                // git show and the .rs suffix gate (section-visible fallback).
                rename_old = Some(rsmpl_patch::decode_git_path_token(from.trim()));
            } else if let Some(to) = line.strip_prefix("rename to ") {
                rename_new = Some(rsmpl_patch::decode_git_path_token(to.trim()));
            }
        }
        flush_rename(&mut rename_old, &mut rename_new);
        // WI-10a: a staged/rev conversion fetches every changed `.rs`
        // file from a base blob that must exist — a miss means the
        // section silently downgrades to section-visible totality, so
        // the caller warns. Pending worktree conversions (`base` empty)
        // skip this: untracked files have no index blob by construction.
        if !base.is_empty() && !out.contains_key(path) && !missing.contains(path) {
            missing.push(path.clone());
        }
    }
    (out, missing)
}

/// WI-10a: pre-image base for staged/rev conversions. A revision range
/// (`A..B` / `A...B`) diffs endpoint-vs-endpoint, but raised hunks
/// apply against the left side — the range string itself never
/// resolves in `git show`, so the base is the text before the first
/// `..` (`...` included: split on `..` covers both spellings).
/// A single rev (or bare `HEAD`) is its own base.
fn rev_base(rev: &str) -> String {
    rev.split("..").next().unwrap_or(rev).to_string()
}

/// One blob fetch: `git show <base>:<path>` (pending worktree diffs use
/// the index blob `:path`, staged/rev diffs the `rev` blob). Returns
/// `None` on any failure or when the blob is not text (NUL or invalid
/// UTF-8 — git's binary definition, T396): binary blobs have no
/// text post to prove and both sides strip them already.
fn show_blob(repo: &Path, base: &str, path: &str) -> Option<String> {
    let spec = format!("{base}:{path}");
    let got = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show", &spec])
        .output()
        .ok()?;
    if !got.status.success() {
        return None;
    }
    if got.stdout.contains(&0) || std::str::from_utf8(&got.stdout).is_err() {
        return None;
    }
    // Safe: validity checked above.
    Some(String::from_utf8(got.stdout).unwrap_or_default())
}

/// WI-16: pre-image text for *every* path a native text touches (not
/// just `.rs` — the T361 loop must derive the true post for ordinary
/// passthrough files too). Rename sections seed the new path from the
/// old-side blob (same T377/WI-7a rule as [`pre_images`]). Missing
/// blobs are simply absent (the loop skips underivable files); no
/// `missing` list — infer keeps its own `.rs`-gated downgrade warnings,
/// and this map never feeds inference.
fn pre_images_text(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
    files: &[String],
    raw: &str,
) -> (HashMap<String, String>, Vec<String>) {
    let base = if staged || rev.is_some() {
        rev_base(rev.unwrap_or("HEAD"))
    } else {
        String::new()
    };
    // Rename old-sides first (single pass over the raw, not per file).
    let mut rename_pairs: Vec<(String, String)> = Vec::new();
    let mut rename_old: Option<String> = None;
    for line in raw.lines() {
        if line.starts_with("diff --git ") {
            rename_old = None;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            rename_old = Some(rsmpl_patch::decode_git_path_token(from.trim()));
        } else if let Some(to) = line.strip_prefix("rename to ") {
            if let Some(old) = rename_old.take() {
                rename_pairs.push((old, rsmpl_patch::decode_git_path_token(to.trim())));
            }
        }
    }
    let mut out = HashMap::new();
    for path in files {
        if let Some(text) = show_blob(repo, &base, path) {
            out.insert(path.clone(), text);
        }
    }
    for (old, new) in &rename_pairs {
        if !out.contains_key(new) {
            if let Some(text) = show_blob(repo, &base, old) {
                out.insert(new.clone(), text);
            }
        }
    }
    (out, Vec::new())
}

/// A validated RSmPL-Git patch covering the pending native changes.
pub struct CompressedPatch {
    pub rsmpl: String,
    pub files: Vec<String>,
    /// Binary paths stripped during inference (warned on stderr).
    pub stripped_binary: Vec<String>,
    /// Submodule (gitlink) paths stripped during inference (warned on
    /// stderr).
    pub stripped_gitlink: Vec<String>,
    /// A `.history/` audit log was skipped during collection
    /// (warned on stderr, never embedded).
    pub skipped_history: bool,
    /// Untracked directory entries skipped during collection
    /// (warned on stderr, never embedded).
    pub skipped_untracked_dirs: Vec<String>,
    /// WI-2 per-hunk provenance (one entry per emitted semantic hunk;
    /// empty when fully ordinary), backing `diffrs --format json`.
    pub hunks: Vec<HunkProvenance>,
    /// WI-4 slice 1: patch-level pre-pass declines (one per declined
    /// pre-pass stage). Additive (v5 stays).
    pub pre_pass_declined: Vec<HunkDeclined>,
    /// WI-10a: changed `.rs` paths with no pre-image in a staged/rev
    /// conversion (the binary warns: `proven` downgraded to
    /// section-visible). Empty for pending worktree conversions.
    pub proven_downgraded: Vec<String>,
}

pub fn compress(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
) -> Result<CompressedPatch, DiffError> {
    let source = GitSource {
        repo: repo.to_path_buf(),
        staged,
        rev: rev.map(str::to_string),
    };
    let collected: CollectedDiff = source.collect()?;
    compress_collected(repo, staged, rev, &collected)
}

/// WI-16 follow-up: raise from an already-collected native diff so one
/// `git diff`/`git status` collection serves both the raise and the
/// `--check` raw text (the CLI collects once, then calls this and
/// reuses `collected.raw` for `check_raise` instead of collecting
/// twice). Byte-identical to [`compress`] by construction: `compress`
/// is this function over a fresh collection.
pub fn compress_collected(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
    collected: &CollectedDiff,
) -> Result<CompressedPatch, DiffError> {
    let (pre, proven_downgraded) = pre_images(repo, staged, rev, &collected.files, &collected.raw);
    compress_closed_with_pre(
        repo,
        staged,
        rev,
        &collected.raw,
        pre,
        RaiseMeta {
            proven_downgraded,
            skipped_history: collected.skipped_history,
            skipped_untracked_dirs: collected.skipped_untracked_dirs.clone(),
        },
    )
}

/// WI-16: collect the native diff without inferring (the CLI's git
/// path collects once, raises via [`compress_collected`], and reuses
/// the collected raw for the `--check` T361 loop).
pub fn collect_native(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
) -> Result<CollectedDiff, DiffError> {
    let source = GitSource {
        repo: repo.to_path_buf(),
        staged,
        rev: rev.map(str::to_string),
    };
    Ok(source.collect()?)
}

/// WI-16: raise from native patch *text* (e.g. `git diff` output, a
/// saved PR patch) against `--repo`'s pre-image. `staged`/`rev` select
/// the pre-image base exactly as in [`compress`] (pending worktree
/// blobs by default); the repo supplies file contents, the text
/// supplies the delta. Nothing is read from or written to the worktree
/// besides blob fetches, so this never mutates the repo.
pub fn compress_raw(
    repo: &Path,
    raw: &str,
    staged: bool,
    rev: Option<&str>,
) -> Result<CompressedPatch, DiffError> {
    let files = rsmpl_diff_source::files_in_diff(raw);
    let (pre, proven_downgraded) = pre_images(repo, staged, rev, &files, raw);
    compress_closed_with_pre(
        repo,
        staged,
        rev,
        raw,
        pre,
        RaiseMeta {
            proven_downgraded,
            skipped_history: false,
            skipped_untracked_dirs: Vec::new(),
        },
    )
}

/// Pass-through collection metadata for the closed raise (keeps the
/// raised-patch assembly under the argument-count gate).
struct RaiseMeta {
    proven_downgraded: Vec<String>,
    skipped_history: bool,
    skipped_untracked_dirs: Vec<String>,
}

/// Closed raising: inference over a `pre` map widened to cover
/// off-diff workspace files that mention a followed ident (the
/// DISCOVERED off-diff gap). Phase 1 infers as today; when the emission
/// carries `# follow=uses`, mentioning files outside `pre` join the D13
/// domain and inference re-runs, so an intent whose site set exceeds the
/// native diff declines to ordinary instead of silently rewriting beyond
/// it. Bounded: at most 3 re-inferences; a final honest inference on the
/// fullest map settles the loop. Diffs whose phase-1 emission has no
/// follow intent pay nothing.
fn compress_closed_with_pre(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
    raw: &str,
    pre: HashMap<String, String>,
    meta: RaiseMeta,
) -> Result<CompressedPatch, DiffError> {
    let mut pre = pre;
    for _ in 0..3 {
        let inferred: InferredPatch = rsmpl_diff_infer::infer_rsmpl_with_pre_images(raw, &pre)?;
        let idents = follow_old_idents(&inferred.rsmpl);
        if idents.is_empty() {
            return Ok(finish(&inferred, meta));
        }
        let extra = off_diff_pre(repo, staged, rev, &pre, &idents);
        if extra.is_empty() {
            return Ok(finish(&inferred, meta));
        }
        pre.extend(extra);
    }
    // Three extensions without settling (a new follow ident per round):
    // one final inference on the fullest map; its probes saw every
    // mentioning file, so whatever it emits is honestly proven.
    let inferred: InferredPatch = rsmpl_diff_infer::infer_rsmpl_with_pre_images(raw, &pre)?;
    Ok(finish(&inferred, meta))
}

fn finish(inferred: &InferredPatch, meta: RaiseMeta) -> CompressedPatch {
    let files = rsmpl_diff_source::files_in_diff(&inferred.rsmpl);
    CompressedPatch {
        rsmpl: inferred.rsmpl.clone(),
        files,
        stripped_binary: inferred.stripped_binary.clone(),
        stripped_gitlink: inferred.stripped_gitlink.clone(),
        skipped_history: meta.skipped_history,
        skipped_untracked_dirs: meta.skipped_untracked_dirs,
        hunks: inferred.hunks.clone(),
        pre_pass_declined: inferred.pre_pass_declined.clone(),
        proven_downgraded: meta.proven_downgraded,
    }
}

/// Old idents claimed by `# follow=uses` hunks in a raised emission
/// (deduped, sorted). Unparseable emissions yield empty — the guard
/// no-ops exactly like the unwidened path.
fn follow_old_idents(rsmpl: &str) -> Vec<String> {
    let Ok(parsed) = rsmpl_patch::parse_patch(rsmpl) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for section in &parsed.files {
        for hunk in &section.hunks {
            let rsmpl_patch::Hunk::Rsmpl(rh) = hunk else {
                continue;
            };
            if !rh.follows_uses() {
                continue;
            }
            if let Some(old) = rsmpl_engine::follow_old_ident(rh) {
                if !out.contains(&old) {
                    out.push(old);
                }
            }
        }
    }
    out.sort();
    out
}

/// Pre-image content for off-diff workspace files that mention one of
/// `idents`: the D13-domain widening. Tracked files come from one
/// `git grep` (ident-token occurrences via `-w`; string/comment hits
/// are harmless — the exact probe absorbs them); untracked files
/// (pending scope only, matching collection) are scanned in-process.
/// Files already in `pre`, non-`.rs` files, and tool/internal paths
/// never join. Failures yield empty (today's behavior, never a new
/// error); unreadable files are skipped like `applyrs`.
fn off_diff_pre(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
    pre: &HashMap<String, String>,
    idents: &[String],
) -> HashMap<String, String> {
    if idents.is_empty() {
        return HashMap::new();
    }
    let with_base = staged || rev.is_some();
    let base = if with_base {
        rev_base(rev.unwrap_or("HEAD"))
    } else {
        String::new()
    };
    let mut mentioning: Vec<String> = Vec::new();
    // All git searches run at the worktree top (T390: `--repo` may be a
    // subdirectory): `git diff` paths are root-relative by git's own
    // behavior, so the guard's hits must be too — a subdir-CWD pathspec
    // would otherwise match nothing and silently skip the domain.
    let top = repo_top(repo);
    // Tracked files: one `git grep` over the worktree (pending) or the
    // base blobs (staged/rev). Exit 1 with no output means no hits.
    // Options precede the rev: `git grep <rev>` parses a late option
    // as a revision. Blob hits carry a `<base>:` prefix to strip.
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(&top)
        .arg("grep")
        .arg("-l")
        .arg("-F")
        .arg("-w")
        .arg("-z")
        .arg("--full-name");
    for ident in idents {
        cmd.arg("-e").arg(ident);
    }
    if with_base {
        cmd.arg(&base);
    }
    cmd.arg("--").arg("*.rs");
    if let Ok(out) = cmd.output() {
        if out.status.success() {
            for part in out.stdout.split(|b| *b == 0) {
                if part.is_empty() {
                    continue;
                }
                let mut rel = String::from_utf8_lossy(part)
                    .trim_end_matches('\n')
                    .replace('\\', "/");
                if with_base {
                    if let Some(stripped) = rel.strip_prefix(&format!("{base}:")) {
                        rel = stripped.to_string();
                    } else {
                        continue;
                    }
                }
                if rel.is_empty()
                    || !rel.ends_with(".rs")
                    || rel.starts_with(".git/")
                    || rel.starts_with("target/")
                    || rel.starts_with(".history/")
                {
                    continue;
                }
                if !pre.contains_key(&rel) && !mentioning.contains(&rel) {
                    mentioning.push(rel);
                }
            }
        }
    }
    // Untracked files (pending scope only): listed like `applyrs`,
    // scanned in-process (no `--untracked` version reliance).
    if !with_base {
        if let Ok(out) = Command::new("git")
            .arg("-C")
            .arg(&top)
            .args([
                "ls-files",
                "--others",
                "--exclude-standard",
                "--full-name",
                "-z",
            ])
            .output()
        {
            if out.status.success() {
                for part in out.stdout.split(|b| *b == 0) {
                    if part.is_empty() {
                        continue;
                    }
                    let rel = String::from_utf8_lossy(part).replace('\\', "/");
                    if !rel.ends_with(".rs")
                        || rel.starts_with("target/")
                        || rel.starts_with(".history/")
                        || pre.contains_key(&rel)
                        || mentioning.contains(&rel)
                    {
                        continue;
                    }
                    let abs = top.join(&rel);
                    if abs.is_symlink() || !abs.is_file() {
                        continue;
                    }
                    if let Ok(text) = std::fs::read_to_string(&abs) {
                        if idents.iter().any(|id| contains_word(&text, id)) {
                            mentioning.push(rel);
                        }
                    }
                }
            }
        }
    }
    mentioning.sort();
    mentioning.dedup();
    let mut out = HashMap::new();
    for path in &mentioning {
        if with_base {
            if let Some(text) = show_blob(&top, &base, path) {
                out.insert(path.clone(), text);
            }
        } else if let Ok(text) = std::fs::read_to_string(top.join(path)) {
            // Off-diff ⇒ worktree == index: the worktree read is the
            // exact pending pre-image (zero forks).
            out.insert(path.clone(), text);
        }
    }
    out
}

/// Worktree top level for guard searches (T390 shape): `git -C <repo>
/// rev-parse --show-toplevel`, falling back to `<repo>` itself when
/// unresolvable. Single-sourcing with `rsmpl_diff_source`'s private
/// copy is WI-24 territory.
fn repo_top(repo: &Path) -> std::path::PathBuf {
    match Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--show-toplevel"])
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

/// Ident-token occurrence (word-char boundaries `[A-Za-z0-9_]`):
/// a superset of ident-token sites that never misses one. Matches
/// `git grep -w` for ASCII text; the D13 probe (exact) decides.
fn contains_word(text: &str, ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }
    let mut start = 0;
    while start + ident.len() <= text.len() {
        let Some(pos) = text[start..].find(ident) else {
            return false;
        };
        let s = start + pos;
        let e = s + ident.len();
        let before = text[..s].chars().next_back().is_some_and(is_word_char);
        let after = text[e..].chars().next().is_some_and(is_word_char);
        if !before && !after {
            return true;
        }
        start = s + 1;
    }
    false
}

fn is_word_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// WI-16: the T361 loop against `--repo` — fetch the pre-image for the
/// files `raw_native` touches (same base selection as [`compress`])
/// and [`verify`] the raised patch against it. Changed-files bounded:
/// no worktree copy, no `git apply`; blobs only.
pub fn check_raise(
    repo: &Path,
    staged: bool,
    rev: Option<&str>,
    raw_native: &str,
    rsmpl: &str,
) -> Result<VerifyReport, DiffError> {
    let files = rsmpl_diff_source::files_in_diff(raw_native);
    let (pre, _) = pre_images_text(repo, staged, rev, &files, raw_native);
    Ok(verify::verify(&pre, raw_native, rsmpl)?)
}

#[cfg(test)]
mod tests {
    use super::{rev_base, CollectedDiff};

    #[test]
    fn rev_base_strips_ranges_to_their_left_side() {
        assert_eq!(rev_base("HEAD"), "HEAD");
        assert_eq!(rev_base("HEAD~1"), "HEAD~1");
        assert_eq!(rev_base("HEAD~1..HEAD"), "HEAD~1");
        assert_eq!(rev_base("HEAD~1...HEAD"), "HEAD~1");
        assert_eq!(rev_base("abc..def"), "abc");
    }

    #[test]
    fn compress_collected_reuses_one_collection_without_git_forks() {
        // WI-16 follow-up: the CLI collects once and raises from the
        // same `CollectedDiff`. A non-`.rs` section needs no `git show`
        // blob fetches (`pre_images` only fetches `.rs` paths), so this
        // runs with no repo and proves the seam forwards the collected
        // raw + skip flags through the shared closed-raise path.
        let raw = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n";
        let collected = CollectedDiff {
            raw: raw.to_string(),
            files: vec!["f.txt".to_string()],
            skipped_history: true,
            skipped_untracked_dirs: vec!["empty-dir/".to_string()],
        };
        let patch = super::compress_collected(std::path::Path::new("."), false, None, &collected)
            .expect("ordinary text section must raise");
        assert!(
            patch.rsmpl.contains("diff --git f.txt"),
            "got: {}",
            patch.rsmpl
        );
        assert_eq!(patch.files, vec!["f.txt".to_string()]);
        assert!(patch.skipped_history);
        assert_eq!(patch.skipped_untracked_dirs, vec!["empty-dir/".to_string()]);
        // Empty collection raises empty (the binary reports `(no changes)`).
        let empty = CollectedDiff {
            raw: String::new(),
            files: Vec::new(),
            skipped_history: false,
            skipped_untracked_dirs: Vec::new(),
        };
        let patch = super::compress_collected(std::path::Path::new("."), false, None, &empty)
            .expect("empty collection must raise");
        assert!(patch.rsmpl.is_empty());
        assert!(patch.files.is_empty());
    }

    #[test]
    fn follow_old_idents_collects_follow_claims() {
        // fn + item follows name their minus idents (deduped, sorted);
        // follow-free emissions (ordinary, plain call sweep) yield empty.
        let emission = "diff --git src/a.rs\n\
            @@ -1,1 @@ rule=r1\n\
            # follow=uses\n\
            - fn old(x: i32) -> i32\n\
            + fn new(x: i32) -> i32\n\
            diff --git src/b.rs\n\
            @@ -1,1 @@ rule=r2\n\
            # sweep\n\
            # follow=uses\n\
            - pub struct Old\n\
            + pub struct New\n\
            diff --git src/b.rs\n\
            @@ -2,1 @@ rule=r2\n\
            # sweep\n\
            # follow=uses\n\
            - pub struct Old\n\
            + pub struct New\n";
        assert_eq!(
            super::follow_old_idents(emission),
            vec!["Old".to_string(), "old".to_string()]
        );
        let sweep = "diff --git src/a.rs\n\
            @@ -1,1 @@ rule=s\n\
            # allow=many\n\
            - foo(E)\n\
            + bar(E)\n";
        assert!(super::follow_old_idents(sweep).is_empty());
        assert!(super::follow_old_idents("").is_empty());
        assert!(super::follow_old_idents("not a patch").is_empty());
    }

    #[test]
    fn contains_word_matches_ident_boundaries() {
        assert!(super::contains_word("use crate::Old;\n", "Old"));
        assert!(super::contains_word("x: Old,", "Old"));
        assert!(super::contains_word("\"Old\"", "Old"));
        assert!(!super::contains_word("struct Older;\n", "Old"));
        assert!(!super::contains_word("Bold move\n", "old"));
        assert!(!super::contains_word("fn f() {}\n", "Old"));
        assert!(!super::contains_word("anything", ""));
    }
}
