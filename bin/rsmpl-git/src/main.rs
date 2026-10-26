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
//! rsmpl-git --file change.rsmpl --repo ./my-repo --emit-diff --check
//! # Machine-readable report:
//! rsmpl-git --file change.rsmpl --repo ./my-repo --format json --emit-diff
//! # Discover valid @@ ranges without reading whole files:
//! rsmpl-git --repo ./my-repo --inspect src/lib.rs --format json
//! # Also list call/method/macro sites for tight call-rewrite ranges:
//! rsmpl-git --repo ./my-repo --inspect src/lib.rs --calls
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
    /// Inline RSmPL-Git patch text, or `-` to read from stdin.
    #[arg(long, conflicts_with = "file")]
    patch: Option<String>,

    /// Path to a file containing the RSmPL-Git patch, or `-` for stdin.
    #[arg(long, conflicts_with = "patch")]
    file: Option<PathBuf>,

    /// Repository worktree to modify (default: current directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Dry run: lower and run `git apply --check` without modifying files.
    #[arg(long)]
    check: bool,

    /// Print the lowered concrete Git patch to stdout (text mode).
    /// In `--format json` mode the diff is carried in the JSON `diff` field.
    #[arg(long)]
    emit_diff: bool,

    /// Write the lowered concrete Git patch to this file.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Print per-rule match counts to stderr.
    #[arg(long)]
    verbose: bool,

    /// Output format for the result report (`text` default, `json` machine-readable).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,

    /// Inspect a repo-relative Rust file and list its items with line spans
    /// (no patch needed). Useful for authoring `@@ -old,len` ranges.
    #[arg(long, value_name = "PATH")]
    inspect: Option<PathBuf>,

    /// With `--inspect`, also list candidate call/method-call/macro sites
    /// with line spans (for authoring tight call-rewrite ranges without
    /// reading the whole file). Requires `--inspect`.
    #[arg(long)]
    calls: bool,
}

/// CLI failure: structured patch/IO errors (JSON-enveloped with
/// `--format json`, SPEC D.2) vs. flag-usage errors (plain text, same class
/// as clap's own errors; enveloping those is T26).
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
        return Err(CliError::Usage(
            "--calls requires --inspect <path>".to_string(),
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

    if cli.verbose {
        for r in &report.rules {
            eprintln!("rule {} ({}): {} match(es)", r.rule, r.file, r.matches);
        }
    }

    // Lower to a concrete Git patch (SPEC 26.2): only changed files, no RSmPL.
    let mut sections = Vec::new();
    for rel in &order {
        let old = &originals[rel];
        let new = &updated[rel];
        if let Some(s) = rsmpl_git::unified_diff_for_file(rel, old, new) {
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
        if let Some(s) = rsmpl_git::unified_diff_for_file(rel, "", &updated[rel]) {
            sections.push(s);
        }
    }
    let lowered = rsmpl_git::assemble_patch(sections);

    if let Some(out) = &cli.output {
        std::fs::write(out, &lowered).map_err(|e| {
            CliError::io_file(
                format!("cannot write {}: {e}", out.display()),
                &out.to_string_lossy(),
            )
        })?;
    }

    if cli.format == Format::Json {
        let diff_field = if lowered.trim().is_empty() {
            String::new()
        } else {
            lowered.clone()
        };
        if cli.check && !diff_field.is_empty() {
            rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
        }
        let payload = serde_json::json!({
            "rules": report.rules.iter().map(|r| serde_json::json!({
                "rule": r.rule, "file": r.file, "matches": r.matches
            })).collect::<Vec<_>>(),
            "files_changed": report.files_changed,
            "diff": diff_field,
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
        if cli.emit_diff {
            println!("(no changes)");
        } else if cli.verbose {
            eprintln!("(no changes)");
        }
        return Ok(());
    }

    if cli.emit_diff {
        print!("{lowered}");
    }

    if cli.check {
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
        return Err(CliError::Usage(
            "--inspect does not take --patch/--file".to_string(),
        ));
    }
    if cli.check || cli.emit_diff || cli.output.is_some() {
        return Err(CliError::Usage(
            "--inspect does not take --check/--emit-diff/--output".to_string(),
        ));
    }
    let rel_str = rel.to_string_lossy().to_string();
    let abs = cli.repo.join(rel);
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
    match cli.format {
        Format::Json => {
            let payload = if let Some(calls) = calls {
                serde_json::json!({ "path": rel_str, "items": items, "calls": calls })
            } else {
                serde_json::json!({ "path": rel_str, "items": items })
            };
            println!(
                "{}",
                serde_json::to_string(&payload)
                    .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
            );
        }
        Format::Text => {
            for i in &items {
                println!("{} {} {}-{}", i.kind, i.name, i.start, i.end);
            }
            if let Some(calls) = calls {
                for c in &calls {
                    println!("{} {} {}-{}", c.kind, c.name, c.start, c.end);
                }
            }
        }
    }
    Ok(())
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
