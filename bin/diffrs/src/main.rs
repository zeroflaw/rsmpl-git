//! `diffrs`: compress native git changes into an RSmPL-Git patch.
//!
//! Agent-centric usage (usually via the `gitrs diff` dispatcher, which
//! invokes this binary):
//!
//! ```sh
//! # Pending unstaged changes as a semantic patch (stdout):
//! diffrs --repo ./my-repo
//! # Staged changes, or history since a revision:
//! diffrs --staged
//! diffrs HEAD~1
//! # Machine-readable report:
//! diffrs --format json
//! ```
//!
//! v1 compresses fn/item renames, N-site slot-sweep renames, N-site
//! call sweeps, N-site method-to-function fan-outs, N-site
//! method-to-method renames, N-site same-method literal-arg
//! sweeps, and whole-item moves (SPEC D.4) and passes
//! everything else through as ordinary hunks. The output always parses as RSmPL-Git and
//! round-trips through `applyrs --check` against the pre-change tree.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
}

#[derive(Parser, Debug)]
#[command(
    name = "diffrs",
    version,
    about = "Compress git changes into an RSmPL-Git semantic patch"
)]
struct Cli {
    /// Repo worktree (default: .).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Diff the index against HEAD/rev instead of the worktree.
    #[arg(long)]
    staged: bool,

    /// Report format: text (the patch) or json (default text).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,

    /// Single revision forwarded to `git diff [rev]` (e.g. `HEAD`,
    /// `HEAD~1`); omit for pending changes.
    rev: Option<String>,
}

enum CliError {
    Message(String),
}

impl CliError {
    fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

fn main() {
    if let Err(e) = run() {
        match e {
            CliError::Message(m) => {
                if wants_json() {
                    // Stdout-only JSON envelope, same family as
                    // `applyrs` (SPEC D.2); text mode keeps stderr.
                    let payload = serde_json::json!({
                        "version": 1,
                        "error": {
                            "code": "RSMPL_IO_CONFLICT",
                            "rule": null,
                            "file": null,
                            "range": null,
                            "reason": m,
                            "suggestion": "run `diffrs --help` for usage",
                            "repair": null,
                        }
                    });
                    println!("{payload}");
                } else {
                    eprintln!("error: {m}");
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

fn run() -> Result<(), CliError> {
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
                return Err(CliError::msg(reason));
            }
            e.exit();
        }
    };

    let patch = rsmpl_diff::compress(&cli.repo, cli.staged, cli.rev.as_deref()).map_err(|e| {
        CliError::msg(match e {
            rsmpl_diff::DiffError::Source(s) => s.to_string(),
            rsmpl_diff::DiffError::Infer(i) => i.to_string(),
        })
    })?;
    for stripped in &patch.stripped_binary {
        eprintln!("warning: {stripped}: binary file stripped from semantic patch");
    }
    for stripped in &patch.stripped_gitlink {
        eprintln!("warning: {stripped}: submodule (gitlink) stripped from semantic patch");
    }
    if patch.skipped_history {
        eprintln!("warning: .history/: tool audit log skipped in semantic patch");
    }
    for skipped in &patch.skipped_untracked_dirs {
        eprintln!("warning: {skipped}: untracked directory skipped in semantic patch");
    }

    if cli.format == Format::Json {
        let payload = serde_json::json!({
            "version": 1,
            "files": patch.files.iter().map(|path| serde_json::json!({"path": path})).collect::<Vec<_>>(),
            "patch": patch.rsmpl,
            "hunks": patch.hunks.iter().map(|h| serde_json::json!({
                "rule": h.rule, "file": h.file, "detector": h.detector,
                "sites": h.sites, "sites_exact": h.sites_exact,
                "totality": h.totality, "proven": h.proven,
                "saved_bytes": h.saved_bytes, "group": h.group
            })).collect::<Vec<_>>(),
        });
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::msg(format!("cannot serialize JSON report: {e}")))?
        );
        return Ok(());
    }

    if patch.rsmpl.trim().is_empty() {
        // Stdout is the artifact: empty stays empty so `diffrs |
        // applyrs` never feeds an empty patch (which `applyrs` rejects
        // as an authoring typo, T59). The stderr note signals it.
        eprintln!("(no changes)");
        return Ok(());
    }
    print!("{}", patch.rsmpl);
    Ok(())
}
