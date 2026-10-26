//! `rsmpl-git`: convert an RSmPL-Git patch into an ordinary Git patch and apply it.
//!
//! Agent-centric usage:
//!
//! ```sh
//! # Inline patch:
//! rsmpl-git --patch '<diff text>' --repo ./my-repo
//! # Piped patch (no temp file):
//! cat change.rsmpl | rsmpl-git --patch - --repo ./my-repo
//! # From a file, printing the lowered diff without applying:
//! rsmpl-git --file change.rsmpl --repo ./my-repo --check
//! # Minimal-token review (no context lines, still appliable):
//! rsmpl-git --file change.rsmpl --repo ./my-repo --check --context 0
//! # Machine-readable report:
//! rsmpl-git --file change.rsmpl --repo ./my-repo --format json --check
//! # Discover valid @@ ranges without reading whole files:
//! rsmpl-git --repo ./my-repo --inspect src/lib.rs --format json
//! # Also list call/method/macro sites for tight call-rewrite ranges:
//! rsmpl-git --repo ./my-repo --inspect src/lib.rs --calls
//! # Scan a whole subtree in one call (grep-style `file: kind name s-e`):
//! rsmpl-git --repo ./my-repo --inspect src/ --calls
//! # Bound large scans so they stay token-efficient:
//! rsmpl-git --repo ./my-repo --inspect src --calls --max-items 20
//! ```
//!
//! Flow (SPEC 26.1): parse RSmPL-Git patch -> apply semantic + ordinary hunks
//! to an in-memory tree (original coordinates) -> emit a concrete Git diff ->
//! `git apply` (or `--check`).

#![allow(clippy::result_large_err)]

mod inspect;

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
    // NOTE: stays `Option` (not `default_value_t = 3`) so absent-vs-explicit
    // is detectable: `--inspect` rejects an explicitly passed `--context`
    // (even `--context 3`); the default is applied via `unwrap_or(3)`.
    // Plain `//` (not `///`) keeps this out of `--help` (T46 token budget).
    #[arg(long, value_name = "N")]
    context: Option<usize>,

    /// Report format: text or json (default text).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,

    /// List item spans for PATH (file or dir); no patch needed (SPEC D.3).
    #[arg(long, value_name = "PATH")]
    inspect: Option<PathBuf>,

    /// With --inspect: also list call/method/macro sites (SPEC D.3.1).
    #[arg(long)]
    calls: bool,

    /// With --inspect: cap entries at N (SPEC D.3.3).
    #[arg(long, value_name = "N")]
    max_items: Option<usize>,
}

/// CLI failure: structured patch/IO/inspect-usage errors (JSON-enveloped with
/// `--format json`, SPEC D.2) vs. input-shape usage errors (plain text, same
/// class as clap's own errors; enveloping those is T26). Inspect flag misuse
/// (T47) is `Structured` `RSMPL_IO_CONFLICT`, not `Usage`, so JSON-mode agents
/// get the same `{error:{...}}` envelope as every other checkable failure.
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

    /// Inspect flag-misuse rejection (T47): same `RSMPL_IO_CONFLICT` shape as
    /// other checkable failures. `reason` keeps the historic usage message so
    /// existing text greps keep matching; `suggestion` states the fix.
    fn inspect_usage(message: &str, suggestion: &str) -> Self {
        Self::Structured(
            StructuredError::new(ErrorCode::IoConflict, message.to_string())
                .with_suggestion(suggestion.to_string()),
        )
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
                    let payload = serde_json::json!({ "error": s.to_json() });
                    println!("{payload}");
                    eprintln!("error: {}", s.code.as_str());
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

    if cli.calls && cli.inspect.is_none() {
        return Err(CliError::inspect_usage(
            "--calls requires --inspect <path>",
            "retry with --inspect <path> --calls",
        ));
    }

    if cli.max_items.is_some() && cli.inspect.is_none() {
        return Err(CliError::inspect_usage(
            "--max-items requires --inspect <path>",
            "retry with --inspect <path> --max-items N",
        ));
    }

    if let Some(rel) = cli.inspect.clone() {
        return run_inspect(&cli, &rel);
    }

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
                "provide exactly one of --patch <text> or --file <path> (or use --inspect <path>)"
                    .to_string(),
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
    // `changed` follows section order (`order` + sorted `extra`).
    let context_radius = cli.context.unwrap_or(3);
    let mut changed: Vec<(String, usize, usize)> = Vec::new();
    let mut sections = Vec::new();
    for rel in &order {
        let old = &originals[rel];
        let new = &updated[rel];
        if let Some(s) =
            rsmpl_git::unified_diff_for_file_with_context(rel, old, new, context_radius)
        {
            let (added, removed) = rsmpl_git::file_line_stats(old, new);
            changed.push((rel.clone(), added, removed));
            sections.push(s);
        }
    }
    // Include changed files not in `order` (shouldn't happen, but be safe).
    let mut extra: Vec<String> = updated
        .keys()
        .filter(|k| !originals.contains_key(*k))
        .cloned()
        .collect();
    extra.sort();
    for rel in &extra {
        if let Some(s) =
            rsmpl_git::unified_diff_for_file_with_context(rel, "", &updated[rel], context_radius)
        {
            let (added, removed) = rsmpl_git::file_line_stats("", &updated[rel]);
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
            "check": if cli.check { serde_json::Value::String("ok".to_string()) } else { serde_json::Value::Null },
        });
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
        );
        if cli.check && !lowered.trim().is_empty() {
            eprintln!("check: lowered patch applies cleanly");
        }
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
    eprintln!(
        "applied {} file(s): {}",
        report.files_changed.len(),
        report.files_changed.join(", ")
    );
    Ok(())
}

fn run_inspect(cli: &Cli, rel: &PathBuf) -> Result<(), CliError> {
    if cli.patch.is_some() || cli.file.is_some() {
        return Err(CliError::inspect_usage(
            "--inspect does not take --patch/--file",
            "drop --patch/--file; --inspect needs no patch",
        ));
    }
    if cli.check || cli.output.is_some() {
        return Err(CliError::inspect_usage(
            "--inspect does not take --check/--output",
            "drop --check/--output; --inspect only lists spans",
        ));
    }
    if cli.diff_max_lines.is_some() || cli.context.is_some() {
        return Err(CliError::inspect_usage(
            "--inspect does not take --diff-max-lines/--context",
            "drop --diff-max-lines/--context; budgets shape the apply path only",
        ));
    }
    let rel_str = rel.to_string_lossy().to_string();
    let abs = cli.repo.join(rel);
    if abs.is_dir() {
        return run_inspect_dir(cli, rel, &abs);
    }
    if !abs.exists() && looks_like_dir_request(&rel_str) {
        return Err(CliError::io_file(
            format!(
                "cannot read {}: no such directory (did you mean an existing file or directory?)",
                abs.display()
            ),
            &rel_str,
        ));
    }
    let text = std::fs::read_to_string(&abs)
        .map_err(|e| CliError::io_file(format!("cannot read {}: {e}", abs.display()), &rel_str))?;
    let items = inspect::inspect_source(&text)
        .map_err(|e| CliError::Structured(e.with_file(rel_str.clone())))?;
    let calls = if cli.calls {
        Some(
            inspect::inspect_calls(&text)
                .map_err(|e| CliError::Structured(e.with_file(rel_str.clone())))?,
        )
    } else {
        None
    };
    let cap = cli.max_items.unwrap_or(usize::MAX);
    let total = items.len() + calls.as_ref().map_or(0, Vec::len);
    let truncated = total > cap;
    let omitted = total.saturating_sub(cap.min(total));
    let (items_out, calls_out) = truncate_single(items, calls, cap);
    match cli.format {
        Format::Json => {
            let payload = if let Some(calls) = calls_out {
                serde_json::json!({ "path": rel_str, "items": items_out, "calls": calls, "truncated": truncated })
            } else {
                serde_json::json!({ "path": rel_str, "items": items_out, "truncated": truncated })
            };
            println!(
                "{}",
                serde_json::to_string(&payload)
                    .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
            );
        }
        Format::Text => {
            for i in &items_out {
                println!("{} {} {}-{}", i.kind, i.name, i.start, i.end);
            }
            if let Some(calls) = calls_out {
                for c in &calls {
                    println!("{} {} {}-{}", c.kind, c.name, c.start, c.end);
                }
            }
            if truncated {
                eprintln!(
                    "truncated: {omitted} more entries omitted (use --max-items N or scope to a subtree)"
                );
            }
        }
    }
    Ok(())
}

/// Truncate a single-file inspect result to a global `cap` over
/// `items ++ calls` (items first, today's order). Returns the kept slices.
fn truncate_single(
    items: Vec<inspect::ItemInfo>,
    calls: Option<Vec<inspect::CallInfo>>,
    cap: usize,
) -> (Vec<inspect::ItemInfo>, Option<Vec<inspect::CallInfo>>) {
    let take_items = items.len().min(cap);
    let remaining = cap.saturating_sub(take_items);
    let items_out = items.into_iter().take(take_items).collect();
    let calls_out = calls.map(|c| c.into_iter().take(remaining).collect());
    (items_out, calls_out)
}

/// Directory scan for `--inspect <dir>`: one call lists items (+ calls
/// with `--calls`) for every `.rs` file under the directory.
///
/// Text mode prints grep-style `rel: kind name s-e` lines (files sorted,
/// items-then-calls within each file; files with no entries emit nothing).
/// JSON mode prints `{"path","files":[{"path","items"[, "calls"]}],
/// "truncated": bool}` with one entry per successfully parsed `.rs` file.
/// Unreadable/unparseable files are skipped with a stderr warning so one
/// broken file never blocks the whole scan (single-inspect it for the full
/// parse detail). With `--max-items N` at most `N` entries are emitted
/// globally (text: stderr `truncated:` marker; JSON: `"truncated": true`).
fn run_inspect_dir(
    cli: &Cli,
    rel: &std::path::Path,
    abs: &std::path::Path,
) -> Result<(), CliError> {
    let rel_str = rel.to_string_lossy().to_string();
    let mut abs_files = inspect::collect_rs_files(abs);
    abs_files.sort_by_key(|a| rel_display(&cli.repo, a));
    // Pass 1: parse everything (warn-and-skip broken files), so the global
    // `--max-items` cap can be applied deterministically in sorted order.
    let mut parsed: Vec<(
        String,
        Vec<inspect::ItemInfo>,
        Option<Vec<inspect::CallInfo>>,
    )> = Vec::new();
    for abs_file in &abs_files {
        let rel_file = rel_display(&cli.repo, abs_file);
        let text = match std::fs::read_to_string(abs_file) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("warning: skipping {rel_file}: cannot read: {e}");
                continue;
            }
        };
        let items = match inspect::inspect_source(&text) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("warning: skipping {rel_file}: {}", e.message);
                continue;
            }
        };
        let calls = if cli.calls {
            match inspect::inspect_calls(&text) {
                Ok(v) => Some(v),
                Err(e) => {
                    eprintln!("warning: skipping calls for {rel_file}: {}", e.message);
                    continue;
                }
            }
        } else {
            None
        };
        parsed.push((rel_file, items, calls));
    }
    let cap = cli.max_items.unwrap_or(usize::MAX);
    let total: usize = parsed
        .iter()
        .map(|(_, items, calls)| items.len() + calls.as_ref().map_or(0, Vec::len))
        .sum();
    let truncated = total > cap;
    let omitted = total.saturating_sub(cap.min(total));
    if cli.format == Format::Json {
        let files = truncate_dir_entries(parsed, cap, cli.calls);
        let payload =
            serde_json::json!({ "path": rel_str, "files": files, "truncated": truncated });
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
        );
    } else {
        let mut remaining = cap;
        for (rel_file, items, calls) in &parsed {
            for i in items.iter().take(remaining.min(items.len())) {
                println!("{}: {} {} {}-{}", rel_file, i.kind, i.name, i.start, i.end);
            }
            let used_items = items.len().min(remaining);
            remaining = remaining.saturating_sub(used_items);
            if let Some(calls) = calls {
                for c in calls.iter().take(remaining.min(calls.len())) {
                    println!("{}: {} {} {}-{}", rel_file, c.kind, c.name, c.start, c.end);
                }
                remaining = remaining.saturating_sub(calls.len().min(remaining));
            }
            if remaining == 0 && truncated {
                // Stop at the cap; later files are omitted (empty files emit
                // nothing in text mode anyway).
                break;
            }
        }
        if truncated {
            eprintln!(
                "truncated: {omitted} more entries omitted (use --max-items N or scope to a subtree)"
            );
        }
    }
    Ok(())
}

/// Truncate directory entries to a global `cap` over `items ++ calls`,
/// preserving sorted file order and items-then-calls order within a file.
/// Empty files (no entries) are always kept for coverage; files after the
/// budget is exhausted are omitted.
fn truncate_dir_entries(
    parsed: Vec<(
        String,
        Vec<inspect::ItemInfo>,
        Option<Vec<inspect::CallInfo>>,
    )>,
    cap: usize,
    with_calls: bool,
) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    let mut remaining = cap;
    for (rel_file, items, calls) in parsed {
        if with_calls {
            let calls = calls.unwrap_or_default();
            let entry_total = items.len() + calls.len();
            if entry_total == 0 {
                files.push(serde_json::json!({ "path": rel_file, "items": [], "calls": [] }));
                continue;
            }
            if remaining == 0 {
                continue;
            }
            let take_items = items.len().min(remaining);
            remaining = remaining.saturating_sub(take_items);
            let take_calls = calls.len().min(remaining);
            remaining = remaining.saturating_sub(take_calls);
            let items_out: Vec<_> = items.into_iter().take(take_items).collect();
            let calls_out: Vec<_> = calls.into_iter().take(take_calls).collect();
            files.push(
                serde_json::json!({ "path": rel_file, "items": items_out, "calls": calls_out }),
            );
            if remaining == 0 {
                // Later non-empty files are omitted; trailing empty files
                // would still be appended by later iterations (they cost 0),
                // which is intended for coverage.
            }
        } else {
            if items.is_empty() {
                files.push(serde_json::json!({ "path": rel_file, "items": [] }));
                continue;
            }
            if remaining == 0 {
                continue;
            }
            let take_items = items.len().min(remaining);
            remaining = remaining.saturating_sub(take_items);
            let items_out: Vec<_> = items.into_iter().take(take_items).collect();
            files.push(serde_json::json!({ "path": rel_file, "items": items_out }));
        }
    }
    files
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

/// Repo-relative display path with `/` separators for inspect output.
fn rel_display(repo: &std::path::Path, abs: &std::path::Path) -> String {
    abs.strip_prefix(repo)
        .map_or_else(
            |_| abs.to_string_lossy().into_owned(),
            |r| r.to_string_lossy().into_owned(),
        )
        .replace('\\', "/")
}

fn looks_like_dir_request(rel: &str) -> bool {
    rel.ends_with('/') || rel == "." || rel.is_empty()
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
