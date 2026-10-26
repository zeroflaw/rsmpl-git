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
//! ```
//!
//! Flow (SPEC 26.1): parse RSmPL-Git patch -> apply semantic + ordinary hunks
//! to an in-memory tree (original coordinates) -> emit a concrete Git diff ->
//! `git apply` (or `--check`).

mod inspect;

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use rsmpl_patch::StructuredError;

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
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(2);
    }
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let cli = Cli::parse();

    if let Some(rel) = cli.inspect.clone() {
        return run_inspect(&cli, &rel);
    }

    let patch_text = match (&cli.patch, &cli.file) {
        (Some(p), None) if p == "-" => read_stdin()?,
        (Some(p), None) => p.clone(),
        (None, Some(f)) if f.as_os_str() == "-" => read_stdin()?,
        (None, Some(f)) => std::fs::read_to_string(f)
            .map_err(|e| format!("cannot read patch file {}: {e}", f.display()))?,
        (None, None) => {
            return Err(
                "provide exactly one of --patch <text> or --file <path> (or use --inspect <path>)"
                    .to_string(),
            );
        }
        (Some(_), Some(_)) => {
            return Err("provide exactly one of --patch <text> or --file <path>".to_string());
        }
    };

    let patch = rsmpl_patch::parse_patch(&patch_text).map_err(render_parse_error)?;

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
            Err(e) => return Err(format!("cannot read {}: {e}", abs.display())),
        };
        order.push(rel.clone());
        originals.insert(rel, text);
    }

    let mut updated = originals.clone();
    let report = rsmpl_engine::apply_patch_to_tree(&mut updated, &patch)
        .map_err(|e| render_engine_error(&e))?;

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
        std::fs::write(out, &lowered)
            .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    }

    if cli.format == Format::Json {
        let diff_field = if lowered.trim().is_empty() {
            String::new()
        } else {
            lowered.clone()
        };
        if cli.check && !diff_field.is_empty() {
            rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(|e| render_engine_error(&e))?;
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
            serde_json::to_string(&payload).map_err(|e| e.to_string())?
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
        rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(|e| render_engine_error(&e))?;
        eprintln!("check: lowered patch applies cleanly");
        return Ok(());
    }

    // Default: apply the lowered patch with `git apply`, exactly as SPEC 26.1
    // prescribes (convert first, then apply the normal Git patch).
    rsmpl_git::apply_with_git(&cli.repo, &lowered).map_err(|e| render_engine_error(&e))?;
    eprintln!(
        "applied {} file(s): {}",
        report.files_changed.len(),
        report.files_changed.join(", ")
    );
    Ok(())
}

fn run_inspect(cli: &Cli, rel: &PathBuf) -> Result<(), String> {
    if cli.patch.is_some() || cli.file.is_some() {
        return Err("--inspect does not take --patch/--file".to_string());
    }
    if cli.check || cli.emit_diff || cli.output.is_some() {
        return Err("--inspect does not take --check/--emit-diff/--output".to_string());
    }
    let rel_str = rel.to_string_lossy().to_string();
    let abs = cli.repo.join(rel);
    let text =
        std::fs::read_to_string(&abs).map_err(|e| format!("cannot read {}: {e}", abs.display()))?;
    let items = inspect::inspect_source(&text).map_err(|e| format!("{e} file: {rel_str}"))?;
    match cli.format {
        Format::Json => {
            let payload = serde_json::json!({ "path": rel_str, "items": items });
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|e| e.to_string())?
            );
        }
        Format::Text => {
            for i in &items {
                println!("{} {} {}-{}", i.kind, i.name, i.start, i.end);
            }
        }
    }
    Ok(())
}

fn read_stdin() -> Result<String, String> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    Ok(buf)
}

fn render_parse_error(e: rsmpl_patch::ParseError) -> String {
    match e {
        rsmpl_patch::ParseError::Structured(s) => s.render(),
        rsmpl_patch::ParseError::Message(m) => m,
    }
}

fn render_engine_error(e: &StructuredError) -> String {
    e.render()
}
