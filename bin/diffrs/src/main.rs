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

#![allow(clippy::result_large_err)]

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

    /// Raise from native patch text instead of collecting `git diff`
    /// (e.g. `git diff` output, a saved PR patch) against `--repo`'s
    /// pre-image. `--staged`/`[rev]` still select the pre-image base.
    /// Conflicts with `--file`.
    #[arg(long, value_name = "TEXT", conflicts_with = "file")]
    patch: Option<String>,

    /// Raise from a file holding native patch text (same semantics as
    /// `--patch`; prefer for large patches). Conflicts with `--patch`.
    #[arg(long, value_name = "PATH", conflicts_with = "patch")]
    file: Option<PathBuf>,

    /// Verify the raise in-process (T361 loop): scratch-apply the raised
    /// patch to an in-memory copy of the pre-image and byte-compare
    /// against the true post-tree. JSON gains a `check` object
    /// (`verified`, `files_compared`, `first_divergence`); text prints a
    /// one-line verdict on stderr. Failure exits 2 naming the diverging
    /// file + line — never a bare `false`. Never trusts lowering
    /// validation alone: `--check`-style checks cannot see sweep
    /// over-match.
    #[arg(long)]
    check: bool,

    /// Report format: text (the patch) or json (default text).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,

    /// Single revision forwarded to `git diff [rev]` (e.g. `HEAD`,
    /// `HEAD~1`); omit for pending changes.
    rev: Option<String>,
}

enum CliError {
    Structured(StructuredError),
    /// `--check` found a divergence: the report is already printed, so
    /// exit 2 silently (never a bare `false` — the divergence is named
    /// in the report / on stderr).
    CheckFailed,
}

impl CliError {
    fn msg(message: impl Into<String>) -> Self {
        Self::Structured(
            StructuredError::new(ErrorCode::IoConflict, message)
                .with_suggestion("run `diffrs --help` for usage"),
        )
    }
}

fn main() {
    if let Err(e) = run() {
        match e {
            CliError::Structured(s) => {
                if wants_json() {
                    // Stdout-only JSON envelope, same error object as
                    // `applyrs` via `StructuredError` (SPEC D.2, v4);
                    // text mode keeps stderr.
                    let payload = serde_json::json!({"version": rsmpl_patch::REPORT_VERSION, "error": s.to_json()});
                    println!("{payload}");
                } else {
                    eprintln!("{}", s.render());
                }
            }
            CliError::CheckFailed => {}
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

    // WI-16: patch-text input raises from `--patch`/`--file` text
    // against the same pre-image bases (`--staged`/`[rev]` select the
    // base either way); the git path collects as before. The `--check`
    // loop needs the native text, so the git path re-collects it read-
    // only (no repo mutation between the two reads).
    let (patch, raw_for_check) = match (&cli.patch, &cli.file) {
        (Some(text), None) => (
            rsmpl_diff::compress_raw(&cli.repo, text, cli.staged, cli.rev.as_deref())
                .map_err(map_diff_err)?,
            cli.check.then(|| text.clone()),
        ),
        (None, Some(path)) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                let p = path.to_string_lossy().to_string();
                CliError::Structured(
                    StructuredError::new(
                        ErrorCode::IoConflict,
                        format!("cannot read patch file {}: {e}", path.display()),
                    )
                    .with_file(&p)
                    .with_suggestion(format!(
                        "check that `{p}` exists and is readable, then retry"
                    )),
                )
            })?;
            (
                rsmpl_diff::compress_raw(&cli.repo, &text, cli.staged, cli.rev.as_deref())
                    .map_err(map_diff_err)?,
                cli.check.then_some(text),
            )
        }
        (None, None) => {
            let patch = rsmpl_diff::compress(&cli.repo, cli.staged, cli.rev.as_deref())
                .map_err(map_diff_err)?;
            let raw = if cli.check {
                Some(
                    rsmpl_diff::collect_native(&cli.repo, cli.staged, cli.rev.as_deref())
                        .map_err(map_diff_err)?
                        .raw,
                )
            } else {
                None
            };
            (patch, raw)
        }
        (Some(_), Some(_)) => {
            // Unreachable: clap `conflicts_with` rejects this pair.
            return Err(CliError::msg("`--patch` conflicts with `--file`"));
        }
    };
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
    // WI-10a: a staged/rev conversion whose base lacks a changed `.rs`
    // file silently ran section-visible totality before — name the
    // downgrade so agents know `proven: false` entries are guesses.
    for downgraded in &patch.proven_downgraded {
        eprintln!(
            "warning: {downgraded}: no pre-image at the rev base; provenance downgraded to section-visible"
        );
    }

    // WI-16 `--check`: the T361 loop over the same native text that was
    // raised. Runs before the empty-raise early return so an empty raise
    // still reports its (vacuous) verdict.
    let check = match raw_for_check {
        Some(raw) => Some(
            rsmpl_diff::check_raise(
                &cli.repo,
                cli.staged,
                cli.rev.as_deref(),
                &raw,
                &patch.rsmpl,
            )
            .map_err(map_diff_err)?,
        ),
        None => None,
    };

    if cli.format == Format::Json {
        let mut payload = serde_json::json!({
            "version": rsmpl_patch::REPORT_VERSION,
            "files": patch.files.iter().map(|path| serde_json::json!({"path": path})).collect::<Vec<_>>(),
            "patch": patch.rsmpl,
            // WI-15c: wholly-ordinary sections carry one entry with
            // `detector: "ordinary"` and the chain-level `declined`
            // list (which families were tried and why); semantic hunks
            // never carry `declined` (a firing section explains itself).
            "hunks": patch.hunks.iter().map(|h| {
                let mut entry = serde_json::json!({
                    "rule": h.rule, "file": h.file, "detector": h.detector,
                    "sites": h.sites, "sites_exact": h.sites_exact,
                    "totality": h.totality, "proven": h.proven,
                    "saved_bytes": h.saved_bytes, "group": h.group
                });
                if h.detector == "ordinary" {
                    entry["declined"] = serde_json::json!(h.declined.iter().map(|d| serde_json::json!({
                        "detector": d.detector, "reason": d.reason,
                    })).collect::<Vec<_>>());
                }
                entry
            }).collect::<Vec<_>>(),
        });
        if let Some(report) = &check {
            payload["check"] = check_json(report);
        }
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::msg(format!("cannot serialize JSON report: {e}")))?
        );
        return check_verdict(&check);
    }

    if patch.rsmpl.trim().is_empty() {
        // Stdout is the artifact: empty stays empty so `diffrs |
        // applyrs` never feeds an empty patch (which `applyrs` rejects
        // as an authoring typo, T59). The stderr note signals it.
        eprintln!("(no changes)");
        return check_verdict(&check);
    }
    print!("{}", patch.rsmpl);
    check_verdict(&check)
}

fn map_diff_err(e: rsmpl_diff::DiffError) -> CliError {
    CliError::msg(match e {
        rsmpl_diff::DiffError::Source(s) => s.to_string(),
        rsmpl_diff::DiffError::Infer(i) => i.to_string(),
        rsmpl_diff::DiffError::Verify(v) => match v {
            rsmpl_diff::VerifyError::NativeParse(msg) => {
                format!("{msg} (want `git diff -U0` text, a saved PR patch, or `git diff` output)")
            }
            rsmpl_diff::VerifyError::Raise(msg) => msg,
        },
    })
}

/// WI-16: JSON form of the T361 verdict (SPEC D.2). `first_divergence`
/// is `null` on success — a failure always names file + line, never a
/// bare `false`.
fn check_json(report: &rsmpl_diff::VerifyReport) -> serde_json::Value {
    serde_json::json!({
        "verified": report.verified,
        "files_compared": report.files_compared,
        "first_divergence": report.first_divergence.as_ref().map(|d| serde_json::json!({
            "file": d.file, "line": d.line, "want": d.want, "got": d.got,
        })),
    })
}

/// WI-16: text-mode one-line verdict on stderr; stdout stays the patch
/// artifact. Divergence exits 2 with file + line named.
fn check_verdict(check: &Option<rsmpl_diff::VerifyReport>) -> Result<(), CliError> {
    let Some(report) = check else {
        return Ok(());
    };
    if report.verified {
        eprintln!("verified: true ({} files compared)", report.files_compared);
        return Ok(());
    }
    let div = report
        .first_divergence
        .as_ref()
        .map(|d| format!("{} diverges at line {}", d.file, d.line))
        .unwrap_or_else(|| "no divergence named".to_string());
    eprintln!("verified: false: {div}");
    Err(CliError::CheckFailed)
}
