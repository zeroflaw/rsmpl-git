//! `applyrs`: convert an RSmPL-Git patch into an ordinary Git patch and apply it.
//!
//! Agent-centric usage (usually via the `gitrs apply` dispatcher, which
//! invokes this binary):
//!
//! ```sh
//! # Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
//! # Piped patch, no temp file (no input flags means stdin):
//! cat change.rsmpl | applyrs --check
//! # From a file, printing the lowered diff without applying:
//! applyrs --file change.rsmpl --check
//! # Inline, no temp file (archived to `<repo>/.history/{id}.rsmpl`):
//! applyrs --patch "$(cat change.rsmpl)" --check
//! # Machine-readable report:
//! applyrs --file change.rsmpl --format json --check
//! ```
//!
//! Flow (SPEC 26.1): parse RSmPL-Git patch -> apply semantic + ordinary hunks
//! to an in-memory tree (original coordinates) -> emit a concrete Git diff ->
//! `git apply` (or `--check`).

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use rsmpl_patch::{ErrorCode, StructuredError};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Parser, Debug)]
#[command(
    name = "applyrs",
    version,
    about = "Apply RSmPL-Git semantic patches via lowering to Git patches"
)]
struct Cli {
    /// Patch file path, or `-` for stdin (no input flags also reads stdin).
    /// Conflicts with `--patch`.
    #[arg(long, conflicts_with = "patch")]
    file: Option<PathBuf>,

    /// Inline patch text (no temp file needed; archived to
    /// `<repo>/.history/{id}.rsmpl`, SPEC D.1). Conflicts with `--file`.
    /// Prefer `--file` for large patches (shell quoting/arg-length limits).
    #[arg(long, value_name = "PATCH")]
    patch: Option<String>,

    /// Repo worktree (default: .).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Dry run: lower + `git apply --check`, apply nothing.
    #[arg(long)]
    check: bool,

    /// Write full lowered diff here (cwd-relative, not --repo-joined; ignores report budget; SPEC D.2.1).
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Report-only diff cap N (`0` omits; stats/output stay full; SPEC D.2.1).
    #[arg(long, value_name = "N")]
    diff_max_lines: Option<usize>,

    /// Report format: text or json (default text).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

/// CLI failure: structured patch/IO errors (JSON-enveloped with
/// `--format json`, SPEC D.2). Clap's own parse errors are enveloped at the
/// `try_parse` site below. Both print the stdout-only JSON envelope in
/// json mode (SPEC D.2).
enum CliError {
    Structured(StructuredError),
}

impl CliError {
    fn io(message: String) -> Self {
        Self::Structured(StructuredError::new(ErrorCode::IoConflict, message))
    }

    fn io_file(message: String, file: &str) -> Self {
        Self::Structured(StructuredError::new(ErrorCode::IoConflict, message).with_file(file))
    }
}

fn main() {
    if let Err(e) = run() {
        match e {
            CliError::Structured(s) => {
                if wants_json() {
                    // JSON failures are stdout-only (T89 symmetry with the
                    // T85 success silence): the stdout envelope carries the
                    // machine signal; stderr keeps `warning:` lines only.
                    // Text mode keeps the human stderr diagnostic.
                    let payload = serde_json::json!({"version": 1, "error": s.to_json()});
                    println!("{payload}");
                } else {
                    eprintln!("{}", s.render());
                }
            }
        }
        std::process::exit(2);
    }
}

/// Detect `--format json` (`--format json` or `--format=json`) without
/// running clap, so failure rendering never depends on a successful parse.
fn wants_json() -> bool {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--format" {
            if args.next().is_some_and(|v| v == "json") {
                return true;
            }
        } else if let Some(v) = a.strip_prefix("--format=") {
            if v == "json" {
                return true;
            }
        }
    }
    false
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), CliError> {
    // Usage errors (unknown flag, bad value) are enveloped as
    // `RSMPL_IO_CONFLICT` JSON on stdout when `--format json` was requested
    // (SPEC D.2); otherwise clap's own text error goes to stderr.
    // `wants_json` hand-scans argv because a failed parse leaves no `Cli`
    // to inspect (T99 pin: both spellings, any position; a missing
    // `--format` value can't be known → text).
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            if wants_json() {
                let reason = e
                    .to_string()
                    .lines()
                    .next()
                    .unwrap_or("invalid command-line arguments")
                    .trim()
                    .to_string();
                let s = StructuredError::new(ErrorCode::IoConflict, reason)
                    .with_suggestion("run `applyrs --help` for usage");
                return Err(CliError::Structured(s));
            }
            e.exit();
        }
    };

    let patch_text = match (&cli.patch, &cli.file) {
        (Some(p), _) => p.clone(),
        (None, Some(f)) if f.as_os_str() == "-" => read_stdin()?,
        (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| {
            CliError::io_file(
                format!("cannot read patch file {}: {e}", f.display()),
                &f.to_string_lossy(),
            )
        })?,
        (None, None) => {
            // Unix filter default (T129): no input flags means stdin, so
            // `cat change.rsmpl | gitrs --check` just works. Empty stdin
            // surfaces as the T59 empty-patch `RSMPL_PARSE_CONFLICT`, not
            // silent success. Like any filter, a bare interactive call
            // blocks on stdin — pipe or pass `--file`/`--patch` instead.
            read_stdin()?
        }
    };

    // T253: archive every raw patch input to `<repo>/.history/{id}.rsmpl`
    // (last 128 kept) and report the path. Runs before parsing so even
    // authoring failures leave audit. History IO never fails the run:
    // write/prune faults are `warning:` lines on stderr.
    let history_path = save_history(&cli.repo, &patch_text);
    let history_display = history_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    if let Some(h) = &history_display {
        eprintln!("history: {h}");
    }

    let patch = rsmpl_patch::parse_patch(&patch_text).map_err(parse_err)?;

    // Load every targeted file from the repo (missing -> empty, so new files
    // can be created by insertion hunks). Patch targets must stay inside
    // the worktree (SPEC 27.6): absolute or `..`-escaping paths are
    // rejected before any read (lexical check, symlinks unresolved).
    let mut originals: HashMap<String, String> = HashMap::new();
    let mut is_new: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut is_symlink: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut order: Vec<String> = Vec::new();
    for section in &patch.files {
        let rel = section.target_path.clone();
        if originals.contains_key(&rel) {
            continue;
        }
        if let Some(reason) = reject_escaped_target(&rel) {
            return Err(CliError::io_file(reason, &rel));
        }
        let abs = cli.repo.join(&rel);
        // Git stores a symlink's content as its target string (mode
        // 120000 blobs); `git diff` shows the target as the line
        // content. Read the link itself, never follow it: following a
        // link-to-dir fails with EISDIR and a link-to-file would
        // verify against the wrong bytes (T353 history dogfood:
        // 53372c4 deletes a discovery symlink into a directory).
        let text = match std::fs::symlink_metadata(&abs) {
            Ok(md) if md.file_type().is_symlink() => match std::fs::read_link(&abs) {
                Ok(target) => {
                    is_symlink.insert(rel.clone());
                    target.to_string_lossy().into_owned()
                }
                Err(e) => {
                    return Err(CliError::io_file(
                        format!("cannot read {}: {e}", abs.display()),
                        &rel,
                    ));
                }
            },
            _ => match std::fs::read_to_string(&abs) {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("warning: {rel} not found, treating as new (empty) file");
                    is_new.insert(rel.clone());
                    String::new()
                }
                Err(e) => {
                    return Err(CliError::io_file(
                        format!("cannot read {}: {e}", abs.display()),
                        &rel,
                    ));
                }
            },
        };
        order.push(rel.clone());
        originals.insert(rel, text);
    }

    let mut updated = originals.clone();
    let report =
        rsmpl_engine::apply_patch_to_tree(&mut updated, &patch).map_err(CliError::Structured)?;

    // Lower to a concrete Git patch (SPEC 26.2): only changed files, no RSmPL.
    // The lowering is always 0-context (SPEC D.2.2): minimal tokens by
    // default, still appliable via the `--unidiff-zero` wrapper below.
    // Per-file stats are computed from the full old/new contents so they stay
    // exact even when the *reported* diff is truncated/omitted (SPEC D.2.1).
    // `changed` follows patch section order (`order`). The engine only
    // mutates paths already in `order`, so no spillover branch is needed.
    let mut changed: Vec<(String, usize, usize)> = Vec::new();
    let mut sections = Vec::new();
    for rel in &order {
        let old = &originals[rel];
        let new = &updated[rel];
        // Created files (missing before the patch) lower with `new file
        // mode` headers so `git apply` creates them (SPEC 27.6); an
        // existing-but-empty file keeps the ordinary headers.
        // Deleted files (input targets `/dev/null`, engine emptied the
        // content) lower with `deleted file mode` headers so `git apply`
        // removes them instead of truncating (T350); an ordinary
        // all-minus hunk truncates and keeps the file.
        let section = if is_new.contains(rel) {
            rsmpl_git::unified_diff_for_new_file_with_context(rel, new, 0)
        } else if patch
            .files
            .iter()
            .any(|s| s.target_path == *rel && s.is_deleted_file)
            && new.is_empty()
        {
            rsmpl_git::unified_diff_for_deleted_file_with_context(
                rel,
                old,
                0,
                is_symlink.contains(rel),
            )
        } else {
            rsmpl_git::unified_diff_for_file_with_context(rel, old, new, 0)
        };
        if let Some(s) = section {
            let (added, removed) = rsmpl_git::file_line_stats(old, new);
            changed.push((rel.clone(), added, removed));
            sections.push(s);
        }
    }
    let lowered = rsmpl_git::assemble_patch(sections);
    // `--diff-max-lines` only bounds the *reported* diff. `--check`,
    // `--output` and apply always use the full `lowered` diff.
    let (reported, diff_truncated) = reported_diff(&lowered, cli.diff_max_lines);
    let total_lines = lowered.lines().count();

    // `--output` saves the full lowering for review/debugging. It is written
    // before `--check` runs, so a failing `--check` still leaves the full
    // appliable diff on disk (SPEC D.2.3). An empty lowering writes nothing:
    // no changes means no file (SPEC D.2.3).
    if lowered.trim().is_empty() {
        // Mirror invariant (SPEC D.2.3): `--output` reflects the current
        // lowering, so an empty lowering removes a stale file left by a
        // previous run instead of leaving a phantom diff behind. No
        // pre-existing file stays the no-op success. Removal is silent:
        // `(no changes)` / `files: []` already signal emptiness.
        // An unremovable file fails loud (SPEC 2.6), like write errors.
        if let Some(out) = &cli.output {
            match std::fs::remove_file(out) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(CliError::io_file(
                        format!(
                            "cannot remove stale {} (empty lowering): {e}",
                            out.display()
                        ),
                        &out.to_string_lossy(),
                    ));
                }
            }
        }
    } else if let Some(out) = &cli.output {
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CliError::io_file(
                        format!("cannot write {}: {e}", out.display()),
                        &out.to_string_lossy(),
                    )
                })?;
            }
        }
        std::fs::write(out, &lowered).map_err(|e| {
            CliError::io_file(
                format!("cannot write {}: {e}", out.display()),
                &out.to_string_lossy(),
            )
        })?;
    }

    if cli.format == Format::Json {
        if cli.check {
            // `--check` verifies without touching the worktree; an empty
            // lowering skips git entirely (SPEC D.2.3).
            if !lowered.trim().is_empty() {
                rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
            }
        } else if !lowered.trim().is_empty() {
            // JSON apply mirrors the text apply below: `--format` chooses
            // the report language, never whether the patch lands (T151).
            rsmpl_git::apply_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
        }
        let payload = serde_json::json!({
            "version": 1,
            "rules": report.rules.iter().map(|r| serde_json::json!({
                "rule": r.rule, "file": r.file, "matches": r.matches, "spans": r.spans.iter().map(|(s, e)| serde_json::json!([s, e])).collect::<Vec<_>>()
            })).collect::<Vec<_>>(),
            "files": changed.iter().map(|(path, added, removed)| serde_json::json!({
                "path": path, "added": added, "removed": removed
            })).collect::<Vec<_>>(),
            "diff": reported,
            "diff_truncated": diff_truncated,
            "diff_lines": total_lines,
            "diff_shown": reported.lines().count(),
            "check": cli.check,
            "history": history_display,
        });
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
        );
        // No stderr `check:` line in JSON mode: the stdout `"check":true`
        // payload is the machine signal (SPEC D.2.3). Text mode keeps the
        // human line below. `warning:` lines stay stderr-only in both modes.
        return Ok(());
    }

    if lowered.trim().is_empty() {
        println!("(no changes)");
        return Ok(());
    }

    // Text `--check` always previews the (possibly budget-bounded) diff on
    // stdout; truncation/omit markers go to stderr. A truncated report is
    // review-only: `--check`, `--output` and apply use the full diff.
    // Plain apply stays quiet on stdout (token-efficient default).
    if cli.check {
        if !reported.is_empty() {
            print!("{reported}");
        }
        if diff_truncated {
            if reported.is_empty() {
                eprintln!(
                    "diff omitted (--diff-max-lines 0): {} file(s) changed",
                    changed.len()
                );
                for (path, added, removed) in &changed {
                    eprintln!("  {path}: +{added} -{removed}");
                }
            } else {
                let shown = reported.lines().count();
                eprintln!(
                    "diff truncated: showing {shown} of {total_lines} lines (full diff in --output file if given, else re-run without --diff-max-lines)"
                );
            }
        }
        rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
        eprintln!("check: lowered patch applies cleanly");
        return Ok(());
    }

    // Default: apply the lowered patch with `git apply`, exactly as SPEC 26.1
    // prescribes (convert first, then apply the normal Git patch).
    rsmpl_git::apply_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
    // Single source of truth (T65): diff-based `changed` (old-vs-new
    // contents differ), the same set JSON reports as `files[].path`.
    let names: Vec<&str> = changed.iter().map(|(p, _, _)| p.as_str()).collect();
    eprintln!("applied {} file(s): {}", names.len(), names.join(", "));
    Ok(())
}

/// Bound the *reported* diff for token budget (SPEC D.2.1).
///
/// Returns `(reported, truncated)` where `truncated` is true iff the lowered
/// diff is non-empty and the reported string hides any of it.
/// `--diff-max-lines N` keeps the first N lines (truncated iff lines were
/// cut; N == 0 omits with changes). A truncated diff is review-only, never
/// appliable: `--check`, `--output` and apply always use the full diff.
fn reported_diff(full: &str, max_lines: Option<usize>) -> (String, bool) {
    if full.trim().is_empty() {
        return (String::new(), false);
    }
    if let Some(n) = max_lines {
        let total = full.lines().count();
        if total > n {
            if n == 0 {
                return (String::new(), true);
            }
            let mut out: String = full.lines().take(n).collect::<Vec<_>>().join("\n");
            out.push('\n');
            return (out, true);
        }
    }
    (full.to_string(), false)
}

/// Lexical worktree-containment check for patch `target_path` values
/// (SPEC 27.6). Returns `Some(reason)` when the path must be rejected:
/// absolute paths (which would discard `--repo` under `Path::join`) or
/// `..` segments that escape the repo root. Purely lexical: `.`/empty
/// segments are ignored, symlinks are not resolved.
fn reject_escaped_target(rel: &str) -> Option<String> {
    if std::path::Path::new(rel).is_absolute() {
        return Some(format!(
            "target path is absolute ({rel}); use a repo-relative path without leading /"
        ));
    }
    let mut depth: i32 = 0;
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return Some(format!(
                        "target path escapes the repo ({rel}); use a repo-relative path without ../"
                    ));
                }
            }
            _ => depth += 1,
        }
    }
    None
}

fn read_stdin() -> Result<String, CliError> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| CliError::io(format!("cannot read stdin: {e}")))?;
    Ok(buf)
}

/// T253: archive raw patch text to `<repo>/.history/{id}.rsmpl`, prune to
/// the last 128 entries (lexicographic id order == chronological), and
/// return the path. Never fatal: every IO fault is a stderr `warning:`
/// and yields `None` so the patch still checks/applies.
fn save_history(repo: &std::path::Path, patch_text: &str) -> Option<PathBuf> {
    let dir = repo.join(".history");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("warning: history: cannot create {}: {e}", dir.display());
        return None;
    }
    let id = history_id();
    let path = dir.join(format!("{id}.rsmpl"));
    if let Err(e) = std::fs::write(&path, patch_text) {
        eprintln!("warning: history: cannot write {}: {e}", path.display());
        return None;
    }
    prune_history(&dir);
    Some(path)
}

/// Monotonic-ish unique id without extra deps: zero-padded
/// `{unix_secs:010}-{nanos:09}-{pid:06}-{ctr:04}` sorts chronologically.
fn history_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(0);
    let ctr = CTR.fetch_add(1, Ordering::Relaxed);
    let (secs, nanos) = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0));
    format!("{secs:010}-{nanos:09}-{:06}-{ctr:04}", std::process::id())
}

/// Keep the last 128 `.rsmpl` entries by id order; failures warn only.
fn prune_history(dir: &std::path::Path) {
    const KEEP: usize = 128;
    let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "rsmpl"))
            .collect(),
        Err(e) => {
            eprintln!("warning: history: cannot list {}: {e}", dir.display());
            return;
        }
    };
    entries.sort();
    if entries.len() > KEEP {
        for stale in entries.iter().take(entries.len() - KEEP) {
            if let Err(e) = std::fs::remove_file(stale) {
                eprintln!(
                    "warning: history: cannot prune {}: {e}",
                    stale.display()
                );
            }
        }
    }
}

fn parse_err(e: rsmpl_patch::ParseError) -> CliError {
    match e {
        rsmpl_patch::ParseError::Structured(s) => CliError::Structured(s),
        // Bare patch-structure faults with no rule/file context.
        rsmpl_patch::ParseError::Message(m) => {
            CliError::Structured(StructuredError::new(ErrorCode::ParseConflict, m))
        }
    }
}
