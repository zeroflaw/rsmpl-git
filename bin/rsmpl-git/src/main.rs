//! `rsmpl-git`: convert an RSmPL-Git patch into an ordinary Git patch and apply it.
//!
//! Agent-centric usage:
//!
//! ```sh
//! # Run from the repo root; from elsewhere, add --repo ./my-repo to any command:
//! # Inline patch:
//! rsmpl-git --patch '<diff text>'
//! # Piped patch (no temp file):
//! cat change.rsmpl | rsmpl-git --patch -
//! # From a file, printing the lowered diff without applying:
//! rsmpl-git --file change.rsmpl --check
//! # Minimal-token review (no context lines, still appliable):
//! rsmpl-git --file change.rsmpl --check --context 0
//! # Machine-readable report:
//! rsmpl-git --file change.rsmpl --format json --check
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
    name = "rsmpl-git",
    version,
    about = "Apply RSmPL-Git semantic patches via lowering to Git patches"
)]
struct Cli {
    /// Inline patch text, or `-` for stdin.
    #[arg(long, conflicts_with = "file")]
    patch: Option<String>,

    /// Patch file path, or `-` for stdin.
    #[arg(long, conflicts_with = "patch")]
    file: Option<PathBuf>,

    /// Repo worktree (default: .).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Dry run: lower + `git apply --check`, apply nothing.
    #[arg(long)]
    check: bool,

    /// Write full lowered diff here (ignores report budget; SPEC D.2.1).
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Report-only diff cap N (`0` omits; stats/output stay full; SPEC D.2.1).
    #[arg(long, value_name = "N")]
    diff_max_lines: Option<usize>,

    /// Lowering context radius (default 3, `0` valid; SPEC D.2.2).
    #[arg(long, value_name = "N", default_value_t = 3)]
    context: usize,

    /// Report format: text or json (default text).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

/// CLI failure: structured patch/IO errors (JSON-enveloped with
/// `--format json`, SPEC D.2) vs. input-shape usage errors (plain text, same
/// class as clap's own errors; enveloping those is T26).
enum CliError {
    Structured(StructuredError),
    Usage(String),
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
            CliError::Usage(msg) => {
                eprintln!("{msg}");
            }
            CliError::Structured(s) => {
                if wants_json() {
                    // JSON failures are stdout-only (T89 symmetry with the
                    // T85 success silence): the stdout envelope carries the
                    // machine signal; stderr keeps `warning:` lines only.
                    // Text mode keeps the human stderr diagnostic.
                    let payload = serde_json::json!({ "error": s.to_json() });
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
    let cli = Cli::parse();

    let patch_text = match (&cli.patch, &cli.file) {
        (Some(p), None) if p == "-" => read_stdin()?,
        (Some(p), None) => p.clone(),
        (None, Some(f)) if f.as_os_str() == "-" => read_stdin()?,
        (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| {
            CliError::io_file(
                format!("cannot read patch file {}: {e}", f.display()),
                &f.to_string_lossy(),
            )
        })?,
        (None, None) => {
            return Err(CliError::Usage(
                "provide exactly one of --patch <text> or --file <path>".to_string(),
            ));
        }
        (Some(_), Some(_)) => {
            return Err(CliError::Usage(
                "provide exactly one of --patch <text> or --file <path>".to_string(),
            ));
        }
    };

    let patch = rsmpl_patch::parse_patch(&patch_text).map_err(parse_err)?;

    // Load every targeted file from the repo (missing -> empty, so new files
    // can be created by insertion hunks).
    let mut originals: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for section in &patch.files {
        let rel = section.target_path.clone();
        if originals.contains_key(&rel) {
            continue;
        }
        let abs = cli.repo.join(&rel);
        let text = match std::fs::read_to_string(&abs) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("warning: {rel} not found, treating as new (empty) file");
                String::new()
            }
            Err(e) => {
                return Err(CliError::io_file(
                    format!("cannot read {}: {e}", abs.display()),
                    &rel,
                ));
            }
        };
        order.push(rel.clone());
        originals.insert(rel, text);
    }

    let mut updated = originals.clone();
    let report =
        rsmpl_engine::apply_patch_to_tree(&mut updated, &patch).map_err(CliError::Structured)?;

    // Lower to a concrete Git patch (SPEC 26.2): only changed files, no RSmPL.
    // Per-file stats are computed from the full old/new contents so they stay
    // exact even when the *reported* diff is truncated/omitted (SPEC D.2.1)
    // or the lowering uses a smaller context radius (SPEC D.2.2).
    // `changed` follows patch section order (`order`). The engine only
    // mutates paths already in `order`, so no spillover branch is needed.
    let mut changed: Vec<(String, usize, usize)> = Vec::new();
    let mut sections = Vec::new();
    for rel in &order {
        let old = &originals[rel];
        let new = &updated[rel];
        if let Some(s) = rsmpl_git::unified_diff_for_file_with_context(rel, old, new, cli.context) {
            let (added, removed) = rsmpl_git::file_line_stats(old, new);
            changed.push((rel.clone(), added, removed));
            sections.push(s);
        }
    }
    let lowered = rsmpl_git::assemble_patch(sections);
    // `--diff-max-lines` only bounds the *reported* diff. `--check`,
    // `--output` and apply always use the full `lowered` diff. (`--context`
    // is not a report budget: it shaped `lowered` itself above.)
    let (reported, diff_truncated) = reported_diff(&lowered, cli.diff_max_lines);
    let total_lines = lowered.lines().count();

    // `--output` saves the full lowering for review/debugging. It is written
    // before `--check` runs, so a failing `--check` still leaves the full
    // appliable diff on disk (SPEC D.2.3). An empty lowering writes nothing:
    // no changes means no file (SPEC D.2.3).
    if !lowered.trim().is_empty() {
        if let Some(out) = &cli.output {
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
    }

    if cli.format == Format::Json {
        if cli.check && !lowered.trim().is_empty() {
            rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
        }
        let payload = serde_json::json!({
            "rules": report.rules.iter().map(|r| serde_json::json!({
                "rule": r.rule, "file": r.file, "matches": r.matches
            })).collect::<Vec<_>>(),
            "files": changed.iter().map(|(path, added, removed)| serde_json::json!({
                "path": path, "added": added, "removed": removed
            })).collect::<Vec<_>>(),
            "diff": reported,
            "diff_truncated": diff_truncated,
            "diff_lines": total_lines,
            "diff_shown": reported.lines().count(),
            "check": if cli.check { serde_json::Value::String("ok".to_string()) } else { serde_json::Value::Null },
        });
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
        );
        // No stderr `check:` line in JSON mode: the stdout `"check":"ok"`
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

fn read_stdin() -> Result<String, CliError> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| CliError::io(format!("cannot read stdin: {e}")))?;
    Ok(buf)
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
