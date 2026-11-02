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
//! # Inline, no temp file (applies archive to `<repo>/.history/{id}.rsmpl`; `--check` needs `--history`):
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

mod report;
mod verify_build;

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

    /// Inline patch text (no temp file needed; applies archive to
    /// `<repo>/.history/{id}.rsmpl`, SPEC D.1; `--check` archives only
    /// with `--history`). Conflicts with `--file`.
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

    /// Presentation context lines for the reported/`--output` diff
    /// (default 0, SPEC D.2.2). `--check`/apply always verify the
    /// 0-context lowering; N only renders context around the same
    /// changes so `--output` artifacts apply with vanilla `git apply`.
    #[arg(long, default_value_t = 0)]
    context: usize,

    /// Report format: text or json (default text).
    #[arg(long, value_enum, default_value = "text")]
    format: Format,

    // SPEC D.2 is normative for `--verify build`; keep the sentence below
    // verbatim in agreement with SPEC D.2 (WI-17 single story).
    /// Extra verification after a successful apply: only `build` is
    /// accepted (`cargo check --quiet` in `--repo`). Under `--check`,
    /// verifies in a temp copy and reports `applied: false` on failure
    /// (SPEC D.2); never by default.
    #[arg(long, value_name = "MODE")]
    verify: Option<String>,

    /// Archive the raw patch to `<repo>/.history/{id}.rsmpl` on `--check`
    /// runs too (SPEC D.1). Real applies always archive; `--check` skips
    /// the archive unless this flag is passed (clean measurement).
    #[arg(long)]
    history: bool,

    /// Binding evidence budget (SPEC D.2): default truncates each binding
    /// value at 80 chars (with `binding_meta` digests for cut values);
    /// `full` lifts the cut for that run only (token-expensive path).
    #[arg(long, value_name = "MODE")]
    bindings: Option<String>,
}

/// CLI failure: structured patch/IO errors (JSON-enveloped with
/// `--format json`, SPEC D.2). Clap's own parse errors are enveloped at the
/// `try_parse` site below. Both print the stdout-only JSON envelope in
/// json mode (SPEC D.2).
///
/// WI-2a: `--verify build` failures additionally carry whether the patch
/// landed (`applied: true` post-apply, `false` for `--check` temp-copy
/// verification), emitted as a top-level additive JSON key so agents can
/// assert without parsing prose.
enum CliError {
    Structured(StructuredError),
    Verify { err: StructuredError, applied: bool },
}

impl CliError {
    fn io(message: String) -> Self {
        Self::Structured(StructuredError::new(ErrorCode::IoConflict, message))
    }

    fn io_file(message: String, file: &str) -> Self {
        Self::Structured(StructuredError::new(ErrorCode::IoConflict, message).with_file(file))
    }

    fn verify(message: String, applied: bool) -> Self {
        Self::Verify {
            err: StructuredError::new(ErrorCode::IoConflict, message).with_suggestion(
                "check that the repo has a Cargo.toml and cargo is installed, then retry",
            ),
            applied,
        }
    }

    /// WI-2: post-lowering build breakage (SPEC 25.10). `file`/`range`
    /// name the first diagnostic (or orphan fallback); `repair` is the
    /// `read_file` payload, or null on the double-miss allow-list entry.
    fn verify_build(
        message: String,
        file: Option<String>,
        range: Option<String>,
        repair: Option<serde_json::Value>,
        applied: bool,
    ) -> Self {
        let suggestion = match (&file, &range) {
            (Some(f), Some(r)) => {
                format!("read `{f}` at `{r}` and fix the build breakage, then retry")
            }
            _ => "fix the build breakage, then retry".to_string(),
        };
        let mut err =
            StructuredError::new(ErrorCode::BuildConflict, message).with_suggestion(suggestion);
        if let Some(f) = file {
            err = err.with_file(f);
        }
        if let Some(r) = range {
            err = err.with_range(&r);
        }
        if let Some(rep) = repair {
            err = err.with_repair(rep);
        }
        Self::Verify { err, applied }
    }

    fn err(&self) -> &StructuredError {
        match self {
            Self::Structured(s) => s,
            Self::Verify { err, .. } => err,
        }
    }

    fn applied(&self) -> Option<bool> {
        match self {
            Self::Structured(_) => None,
            Self::Verify { applied, .. } => Some(*applied),
        }
    }
}

fn main() {
    if let Err(e) = run() {
        if wants_json() {
            // JSON failures are stdout-only (T89 symmetry with the
            // T85 success silence): the stdout envelope carries the
            // machine signal; stderr keeps `warning:` lines only.
            // Text mode keeps the human stderr diagnostic.
            let mut payload = serde_json::json!({"version": rsmpl_patch::REPORT_VERSION, "error": e.err().to_json()});
            if let Some(applied) = e.applied() {
                payload["applied"] = serde_json::json!(applied);
            }
            println!("{payload}");
        } else {
            eprintln!("{}", e.err().render());
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

    // WI-2 `--verify`: only `build` exists in v0.1; anything else is a
    // usage error (never silently ignored, SPEC 2.6).
    if let Some(mode) = &cli.verify {
        if mode != "build" {
            let s = StructuredError::new(
                ErrorCode::IoConflict,
                format!("unsupported --verify mode `{mode}` (only `--verify build` is supported in v0.1)"),
            )
            .with_suggestion("use `--verify build` or drop the flag");
            return Err(CliError::Structured(s));
        }
    }

    // WI-2 `--bindings`: only `full` lifts the 80-char cut; anything else
    // is a usage error (never silently the default, SPEC 2.6).
    let bindings_full = match &cli.bindings {
        None => false,
        Some(mode) if mode == "full" => true,
        Some(mode) => {
            let s = StructuredError::new(
                ErrorCode::IoConflict,
                format!("unsupported --bindings mode `{mode}` (only `--bindings full` is supported in v0.1)"),
            )
            .with_suggestion("use `--bindings full` or drop the flag");
            return Err(CliError::Structured(s));
        }
    };

    let patch_text = match (&cli.patch, &cli.file) {
        (Some(p), _) => p.clone(),
        (None, Some(f)) if f.as_os_str() == "-" => read_stdin()?,
        (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| {
            // WI-3 allow-list: the unreadable patch file stays
            // repair-null (environmental — no patch spelling fixes
            // it), but the suggestion names the path.
            let path = f.to_string_lossy().to_string();
            CliError::Structured(
                StructuredError::new(
                    ErrorCode::IoConflict,
                    format!("cannot read patch file {}: {e}", f.display()),
                )
                .with_file(&path)
                .with_suggestion(format!(
                    "check that `{path}` exists and is readable, then retry"
                )),
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

    // T253 + WI-1c: real applies archive every raw patch input to
    // `<repo>/.history/{id}.rsmpl` (last 128 kept) and report the path;
    // `--check` skips the archive unless `--history` is passed (clean
    // measurement). Runs before parsing so even authoring failures leave
    // audit when archiving. History IO never fails the run: write/prune
    // faults are `warning:` lines on stderr.
    let history_path = if !cli.check || cli.history {
        save_history(&cli.repo, &patch_text)
    } else {
        None
    };
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
    // T377: rename targets seeded from old-side content (`new -> old`).
    let mut rename_from: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for section in &patch.files {
        let rel = section.target_path.clone();
        if originals.contains_key(&rel) {
            continue;
        }
        if let Some(reason) = reject_escaped_target(&rel) {
            // WI-3: the escape stays `RSMPL_IO_CONFLICT` (T94 pin) but
            // carries the mechanical patch-side fix, so the enumeration
            // stays null-free.
            let s = StructuredError::new(ErrorCode::IoConflict, reason)
                .with_file(&rel)
                .with_repair(StructuredError::fix_pattern_repair(
                    "paths",
                    "use a repo-relative `diff --git a/<path> b/<path>` target without leading `/` or `..`",
                ));
            return Err(CliError::Structured(s));
        }
        // T377: a rename section's hunks cite old-side coordinates, so
        // seed the new-side entry from the OLD file's content (the
        // engine then applies ordinary and RSmPL hunks transparently).
        let (read_rel, is_rename) = if section.is_rename() {
            let old_rel = rsmpl_patch::stripped_path(&section.old_path);
            if let Some(reason) = reject_escaped_target(&old_rel) {
                // WI-3: same patch-side repair as the target-path escape.
                let s = StructuredError::new(ErrorCode::IoConflict, reason)
                    .with_file(&old_rel)
                    .with_repair(StructuredError::fix_pattern_repair(
                        "paths",
                        "use a repo-relative `diff --git a/<path> b/<path>` target without leading `/` or `..`",
                    ));
                return Err(CliError::Structured(s));
            }
            (old_rel, true)
        } else {
            (rel.clone(), false)
        };
        let abs = cli.repo.join(&read_rel);
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
                    eprintln!("warning: {read_rel} not found, treating as new (empty) file");
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
        originals.insert(rel.clone(), text);
        // T377: record the rename source only when the old side was
        // actually read; a missing old side stays the existing
        // warn-and-create leniency (no rename marking).
        if is_rename && !is_new.contains(&rel) {
            rename_from.insert(rel, read_rel);
        }
    }

    // WI-6b1 + WI-7c: a patch with `# follow=uses` or a plain fn/item
    // rename searches the whole workspace. Follow rewrites cross-file
    // uses; orphan scans only report them. Unreadable files warn and are
    // skipped (the engine tripwire still vetoes when an unparseable file
    // may name the ident).
    let mut extra_order: Vec<String> = Vec::new();
    let needs_workspace = patch.files.iter().flat_map(|s| &s.hunks).any(|h| match h {
        rsmpl_patch::Hunk::Rsmpl(rh) => {
            rh.follows_uses()
                || rh
                    .directives
                    .iter()
                    .any(|d| matches!(d, rsmpl_patch::Directive::Symbol { .. }))
                || matches!(
                    rsmpl_pattern::compile(rh).map(|c| c.kind),
                    Ok(rsmpl_pattern::PatternKind::Fn { .. }
                        | rsmpl_pattern::PatternKind::ItemRename { .. })
                )
        }
        rsmpl_patch::Hunk::Ordinary(_) => false,
    });
    let mut follow_report: Option<(usize, &'static str)> = None;
    if needs_workspace {
        let (workspace_files, source) = workspace_rs_files_with_source(&cli.repo);
        follow_report = Some((workspace_files.len(), source));
        for rel in workspace_files {
            if originals.contains_key(&rel) {
                continue;
            }
            match std::fs::read_to_string(cli.repo.join(&rel)) {
                Ok(text) => {
                    originals.insert(rel.clone(), text);
                    extra_order.push(rel);
                }
                Err(e) => eprintln!("warning: cannot read {rel} for follow=uses search: {e}"),
            }
        }
    }

    let mut updated = originals.clone();
    let report =
        rsmpl_engine::apply_patch_to_tree(&mut updated, &patch).map_err(CliError::Structured)?;

    // Lower to a concrete Git patch (SPEC 26.2): only changed files, no RSmPL.
    // The verified lowering is always 0-context (SPEC D.2.2): minimal tokens
    // by default, still appliable via the `--unidiff-zero` wrapper below.
    // `--context N` additionally renders the same changes with N context
    // lines for the reported/`--output` diff (vanilla-`git apply`
    // compatible); `--check`, apply, and verification always use the
    // 0-context lowering. Per-file stats are computed from the full
    // old/new contents so they stay exact under any report budget (D.2.1).
    // `changed` follows patch section order (`order`); WI-6b1 cross-file
    // follow rewrites may spill into workspace extras, lowered after.
    let (changed, lowered) = lower_tree(
        &patch,
        &originals,
        &updated,
        &order,
        &extra_order,
        &rename_from,
        &is_new,
        &is_symlink,
        0,
    );
    // Presentation rendering: same contents, N context lines around each
    // change. Stats are context-independent (`file_line_stats`), so only
    // the diff text is rebuilt.
    let displayed = if cli.context == 0 {
        lowered.clone()
    } else {
        lower_tree(
            &patch,
            &originals,
            &updated,
            &order,
            &extra_order,
            &rename_from,
            &is_new,
            &is_symlink,
            cli.context,
        )
        .1
    };
    // `--diff-max-lines` only bounds the *reported* diff. `--check` and
    // apply always use the full 0-context `lowered` diff; `--output`
    // saves the full `displayed` diff (SPEC D.2.2).
    let (reported, diff_truncated) = reported_diff(&displayed, cli.diff_max_lines);
    let total_lines = displayed.lines().count();

    // WI-10a: render the in-memory post-images as a concrete Git patch
    // (SPEC 26.2) with `context` context lines. Stats are
    // context-independent (`file_line_stats`), so only the diff text
    // varies with the radius.
    #[allow(clippy::too_many_arguments)]
    fn lower_tree(
        patch: &rsmpl_patch::Patch,
        originals: &HashMap<String, String>,
        updated: &HashMap<String, String>,
        order: &[String],
        extra_order: &[String],
        rename_from: &HashMap<String, String>,
        is_new: &std::collections::HashSet<String>,
        is_symlink: &std::collections::HashSet<String>,
        context: usize,
    ) -> (Vec<(String, usize, usize, String)>, String) {
        let mut changed: Vec<(String, usize, usize, String)> = Vec::new();
        let mut sections = Vec::new();
        for rel in order {
            let old = &originals[rel];
            let new = &updated[rel];
            // T377: rename sections lower back to `rename from/to`
            // (seeded from old-side content, so the diff spans the move).
            // Runs first -- including the content-equal case, where the
            // ladder below would emit nothing (silent drop).
            if let Some(old_rel) = rename_from.get(rel) {
                let modes = section_modes(patch, rel);
                let (om, nm) = match &modes {
                    Some((o, n)) => (Some(o.as_str()), Some(n.as_str())),
                    None => (None, None),
                };
                let section = rsmpl_git::unified_diff_for_rename_with_context(
                    old_rel, rel, old, new, om, nm, context,
                );
                let (added, removed) = rsmpl_git::file_line_stats(old, new);
                changed.push((rel.clone(), added, removed, post_sha_hex(new)));
                sections.push(section);
                continue;
            }
            // Created files (missing before the patch) lower with `new file
            // mode` headers so `git apply` creates them (SPEC 27.6); an
            // existing-but-empty file keeps the ordinary headers.
            // Deleted files (input targets `/dev/null`, engine emptied the
            // content) lower with `deleted file mode` headers so `git apply`
            // removes them instead of truncating (T350); an ordinary
            // all-minus hunk truncates and keeps the file.
            // T357: same-path delete+create pair (git's file<->symlink
            // typechange shape). When the declared modes differ, lower as
            // the pair so `git apply` swaps the type; same-type pairs
            // stay a single plain modify (minimal tokens).
            if !is_new.contains(rel) {
                let has_delete = patch
                    .files
                    .iter()
                    .any(|s| s.target_path == *rel && s.is_deleted_file);
                let create_mode = patch
                    .files
                    .iter()
                    .find(|s| s.target_path == *rel && s.is_new_file)
                    .and_then(|s| {
                        s.metadata
                            .iter()
                            .find_map(|m| m.strip_prefix("new file mode "))
                    })
                    .filter(|m| m.len() == 6 && m.bytes().all(|b| b.is_ascii_digit()));
                if has_delete && create_mode.is_some() {
                    let new_mode = create_mode.unwrap_or("100644");
                    let old_symlink = is_symlink.contains(rel);
                    let new_symlink = new_mode == "120000";
                    if old_symlink != new_symlink {
                        let pair = rsmpl_git::unified_diff_for_typechange_with_context(
                            rel,
                            old,
                            new,
                            old_symlink,
                            new_mode,
                            context,
                        );
                        if !pair.is_empty() {
                            let (added, removed) = rsmpl_git::file_line_stats(old, new);
                            changed.push((rel.clone(), added, removed, post_sha_hex(new)));
                            sections.extend(pair);
                        }
                        continue;
                    }
                }
            }
            let section = if is_new.contains(rel) {
                // Honor the author's declared `new file mode` (T355):
                // `diffrs` emits `120000` for symlink creates; anything
                // else (or absent) stays the `100644` default.
                let mode = patch
                    .files
                    .iter()
                    .find(|s| s.target_path == *rel)
                    .and_then(|s| {
                        s.metadata
                            .iter()
                            .find_map(|m| m.strip_prefix("new file mode "))
                    })
                    .filter(|m| m.len() == 6 && m.bytes().all(|b| b.is_ascii_digit()))
                    .unwrap_or("100644");
                rsmpl_git::unified_diff_for_new_file_with_context(rel, new, context, mode)
            } else if patch
                .files
                .iter()
                .any(|s| s.target_path == *rel && s.is_deleted_file)
                && new.is_empty()
            {
                rsmpl_git::unified_diff_for_deleted_file_with_context(
                    rel,
                    old,
                    context,
                    is_symlink.contains(rel),
                )
            } else {
                // T364: carry `old mode`/`new mode` into the lowering. A
                // content change that also flips the mode keeps both; a pure
                // mode change (old == new) lowers as a headers-only flip
                // section (exactly what real git emits) instead of today's
                // silent drop.
                let content = rsmpl_git::unified_diff_for_file_with_context(rel, old, new, context);
                match (content, section_modes(patch, rel)) {
                    (Some(s), Some((old_mode, new_mode))) => {
                        Some(rsmpl_git::with_mode_lines(&s, &old_mode, &new_mode))
                    }
                    (None, Some((old_mode, new_mode))) => {
                        Some(rsmpl_git::mode_change_section(rel, &old_mode, &new_mode))
                    }
                    (content, None) => content,
                }
            };
            if let Some(s) = section {
                let (added, removed) = rsmpl_git::file_line_stats(old, new);
                changed.push((rel.clone(), added, removed, post_sha_hex(new)));
                sections.push(s);
            }
        }
        // WI-6b1: changed workspace extras (cross-file `# follow=uses`
        // rewrites in files the patch never targeted) lower as plain content
        // modifies — regular files by construction, never renames/modes.
        for rel in extra_order {
            let old = &originals[rel];
            let new = &updated[rel];
            if old == new {
                continue;
            }
            if let Some(s) = rsmpl_git::unified_diff_for_file_with_context(rel, old, new, context) {
                let (added, removed) = rsmpl_git::file_line_stats(old, new);
                changed.push((rel.clone(), added, removed, post_sha_hex(new)));
                sections.push(s);
            }
        }
        let lowered = rsmpl_git::assemble_patch(sections);
        (changed, lowered)
    } // fn lower_tree

    // `--output` saves the full `displayed` diff for review/debugging. It is
    // written before `--check` runs, so a failing `--check` still leaves the
    // full appliable diff on disk (SPEC D.2.3). An empty diff writes nothing:
    // no changes means no file (SPEC D.2.3).
    if displayed.trim().is_empty() {
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
        std::fs::write(out, &displayed).map_err(|e| {
            CliError::io_file(
                format!("cannot write {}: {e}", out.display()),
                &out.to_string_lossy(),
            )
        })?;
    }

    // WI-2: verify-build classification reads the report directly
    // (first diagnostic or orphan fallback, SPEC 25.10).
    if cli.format == Format::Json {
        // WI-2a: tracks whether `--verify build` actually ran so the
        // success payload can carry additive `"build": "pass"`.
        let mut build_passed = false;
        if cli.check {
            // `--check` verifies without touching the worktree; an empty
            // lowering skips git entirely (SPEC D.2.3).
            if !lowered.trim().is_empty() {
                rsmpl_git::check_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
                // WI-2a `--verify build` under `--check`: verify the
                // post-images in a temp copy; the worktree stays clean.
                if cli.verify.as_deref() == Some("build") {
                    verify_in_temp_copy(
                        &cli.repo,
                        &updated,
                        &changed,
                        &rename_from,
                        &is_symlink,
                        &patch,
                        &report,
                    )?;
                    build_passed = true;
                }
            }
        } else if !lowered.trim().is_empty() {
            // JSON apply mirrors the text apply below: `--format` chooses
            // the report language, never whether the patch lands (T151).
            rsmpl_git::apply_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
            // WI-2a `--verify build`: post-apply run; failure names the
            // applied state explicitly (`applied: true` in JSON).
            if cli.verify.as_deref() == Some("build") {
                run_verify_build_with_orphans(&cli.repo, &report)?;
                build_passed = true;
            }
        }
        // WI-1b v3 envelope: `checked.semantics` is `"verified"|"partial"`
        // (never boolean); each rule carries its own `semantics`
        // (`instantiated|postcondition|reparse-only|not-applicable`).
        // `build` is `pass` iff `--verify build` ran, else `skipped`.
        let build_status = if build_passed { "pass" } else { "skipped" };
        let global_semantics = global_semantics(&report);
        let mut payload = serde_json::json!({
            "version": rsmpl_patch::REPORT_VERSION,
            "rules": report.rules.iter().map(|r| {
                let mut entry = serde_json::json!({
                    "rule": r.rule, "file": r.file, "matches": r.matches, "spans": r.spans.iter().map(|(s, e)| serde_json::json!([s, e])).collect::<Vec<_>>(),
                    "semantics": r.semantics.as_str(),
                });
                // WI-16: symbol resolution mode, additive beside
                // `semantics` (absent on non-symbol hunks).
                if let Some(symbol) = &r.symbol {
                    entry["symbol"] = serde_json::json!(symbol);
                }
                // WI-2b/2c: per-site evidence, one entry per emitted edit
                // in span order. Omitted when no site binds (renames,
                // insertions, deletes, use-prefix carry no evidence).
                if !r.sites.is_empty() {
                    let (sites_val, sites_trunc, bindings_cut) =
                        report::sites_json(&r.sites, &r.span_labels, bindings_full);
                    entry["sites"] = sites_val;
                    if sites_trunc > 0 {
                        entry["sites_truncated"] = serde_json::json!(sites_trunc);
                    }
                    // WI-1a: marked truncation — a cut binding value carries
                    // a trailing `…`, counted here (additive within v2).
                    if bindings_cut > 0 {
                        entry["bindings_truncated_chars"] =
                            serde_json::json!(bindings_cut);
                    }
                }
                // WI-15a: full pre-truncation counts always (additive,
                // display-independent — the verdict reads the same engine
                // counts via `is_truncated_rule`, never these keys).
                entry["orphans_total"] = serde_json::json!(r.orphans.len());
                entry["follow_skipped_total"] =
                    serde_json::json!(r.follow_skipped_details.len());
                entry["sites_total"] =
                    serde_json::json!(report::sites_binding_total(&r.sites));
                // WI-2 span edit labels: parallel to `spans`, emitted only
                // for multi-edit rules with something to say (single-edit
                // rules omit the key; `null` = unlabeled slot).
                if r.spans.len() > 1 && r.span_labels.iter().any(|l| l.is_some()) {
                    entry["span_edits"] = serde_json::json!(r
                        .span_labels
                        .iter()
                        .map(|l| l
                            .as_deref()
                            .map(serde_json::Value::from)
                            .unwrap_or(serde_json::Value::Null))
                        .collect::<Vec<_>>());
                }
                // WI-20 v4: the `follow_skipped` prose string is
                // removed; structured skips live in
                // `follow_skipped_details` alone (SPEC D.2).
                if !r.follow_skipped_details.is_empty() {
                    entry["follow_skipped_details"] = serde_json::json!(r
                        .follow_skipped_details
                        .iter()
                        .map(|s| serde_json::json!({
                            "file": s.file, "line": s.line, "reason": s.reason,
                        }))
                        .collect::<Vec<_>>());
                }
                // WI-7c: report-only orphan evidence, capped like bindings.
                if !r.orphans.is_empty() {
                    let (orphans_val, orphans_trunc, orphans_cut) =
                        report::orphans_json(&r.orphans, bindings_full);
                    entry["orphans"] = orphans_val;
                    if orphans_trunc > 0 {
                        entry["orphans_truncated"] = serde_json::json!(orphans_trunc);
                    }
                    // WI-1a: marked truncation for orphan `text` (same `…`
                    // posture as bindings; additive within v2).
                    if orphans_cut > 0 {
                        entry["orphans_truncated_chars"] =
                            serde_json::json!(orphans_cut);
                    }
                }
                entry
            }).collect::<Vec<_>>(),
            "files": changed.iter().map(|(path, added, removed, post_sha)| serde_json::json!({
                "path": path, "added": added, "removed": removed, "post_sha": post_sha
            })).collect::<Vec<_>>(),
            "diff": reported,
            "diff_truncated": diff_truncated,
            "diff_lines": total_lines,
            "diff_shown": reported.lines().count(),
            "checked": {"lowering": true, "semantics": global_semantics, "build": build_status},
            "history": history_display,
        });
        // WI-7a + WI-7c: workspace provenance whenever the workspace
        // was searched (follow rewrites or orphan scans; additive
        // within v2). Absent when the patch needed no workspace search.
        if let Some((n, source)) = follow_report {
            payload["follow_files"] = serde_json::json!(n);
            payload["follow_source"] = serde_json::json!(source);
        }
        // WI-10b drift evidence (SPEC 10.8, D.2): one entry per drifted
        // hunk, in application order. Absent when no hunk drifted.
        if !report.drifts.is_empty() {
            payload["drift"] = serde_json::json!(report
                .drifts
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "rule": d.rule, "file": d.file,
                        "cited": [d.cited.0, d.cited.1],
                        "actual": [d.actual.0, d.actual.1],
                    })
                })
                .collect::<Vec<_>>());
        }
        println!(
            "{}",
            serde_json::to_string(&payload)
                .map_err(|e| CliError::io(format!("cannot serialize JSON report: {e}")))?
        );
        // No stderr `check:` line in JSON mode: the stdout
        // `"checked"` payload is the machine signal (SPEC D.2.3). Text
        // mode keeps the human line below. `warning:` lines stay
        // stderr-only in both modes.
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
                for (path, added, removed, _) in &changed {
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
        // WI-2a `--verify build` under `--check`: temp-copy verification.
        // WI-1b: the `check:` line carries the global semantics word.
        let semantics = global_semantics(&report);
        if cli.verify.as_deref() == Some("build") {
            verify_in_temp_copy(
                &cli.repo,
                &updated,
                &changed,
                &rename_from,
                &is_symlink,
                &patch,
                &report,
            )?;
            eprintln!("check: lowered patch applies cleanly; semantics {semantics}; build check passed (temp copy, worktree untouched)");
        } else {
            eprintln!("check: lowered patch applies cleanly; semantics {semantics}");
        }
        if let Some(line) = orphan_text_line(&report) {
            eprintln!("{line}");
        }
        for line in follow_skipped_text_lines(&report) {
            eprintln!("{line}");
        }
        for line in incomplete_text_lines(&report) {
            eprintln!("{line}");
        }
        for d in &report.drifts {
            eprintln!(
                "drift: {} {} cited [{},{}] -> actual [{},{}]",
                d.rule, d.file, d.cited.0, d.cited.1, d.actual.0, d.actual.1
            );
        }
        return Ok(());
    }

    // Default: apply the lowered patch with `git apply`, exactly as SPEC 26.1
    // prescribes (convert first, then apply the normal Git patch).
    rsmpl_git::apply_with_git(&cli.repo, &lowered).map_err(CliError::Structured)?;
    // WI-2a `--verify build`: post-apply run; failure keeps the patch
    // applied and reports `applied: true` in JSON.
    if cli.verify.as_deref() == Some("build") {
        run_verify_build_with_orphans(&cli.repo, &report)?;
    }
    // Single source of truth (T65): diff-based `changed` (old-vs-new
    // contents differ), the same set JSON reports as `files[].path`.
    // WI-1b: the applied line carries the global semantics word.
    let names: Vec<&str> = changed.iter().map(|(p, _, _, _)| p.as_str()).collect();
    let semantics = global_semantics(&report);
    eprintln!(
        "applied {} file(s): {}; semantics {semantics}",
        names.len(),
        names.join(", ")
    );
    if let Some(line) = orphan_text_line(&report) {
        eprintln!("{line}");
    }
    for line in follow_skipped_text_lines(&report) {
        eprintln!("{line}");
    }
    for line in incomplete_text_lines(&report) {
        eprintln!("{line}");
    }
    for d in &report.drifts {
        eprintln!(
            "drift: {} {} cited [{},{}] -> actual [{},{}]",
            d.rule, d.file, d.cited.0, d.cited.1, d.actual.0, d.actual.1
        );
    }
    Ok(())
}

/// WI-7a: workspace file set for `# follow=uses` plus its source for the
/// JSON report. Inside a git worktree the set is tracked +
/// untracked-but-not-ignored `.rs` files (`git ls-files --cached
/// --others --exclude-standard`), so ignored/generated files are neither
/// searched nor rewritten; outside a worktree it falls back to the
/// lexical walk and reports `"walk"`.
fn workspace_rs_files_with_source(repo: &std::path::Path) -> (Vec<String>, &'static str) {
    if let Some(files) = git_rs_files(repo) {
        return (files, "git");
    }
    (walk_rs_files(repo), "walk")
}

/// `git ls-files` branch of the workspace set. `None` when git fails
/// (not a worktree, git missing) so the caller falls back to the walk.
fn git_rs_files(repo: &std::path::Path) -> Option<Vec<String>> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut files = Vec::new();
    for part in out.stdout.split(|b| *b == 0) {
        if part.is_empty() {
            continue;
        }
        let rel = String::from_utf8_lossy(part).replace('\\', "/");
        if !rel.ends_with(".rs") {
            continue;
        }
        if rel.starts_with(".git/") || rel.starts_with("target/") || rel.starts_with(".history/") {
            continue;
        }
        // Symlink parity with the walk: never search linked files.
        // `is_file` parity too: `--cached` may list staged-but-deleted
        // paths the walk would never yield; dropping them here beats a
        // read-warning downstream.
        let abs = repo.join(&rel);
        if abs.is_symlink() || !abs.is_file() {
            continue;
        }
        files.push(rel);
    }
    files.sort();
    files.dedup();
    Some(files)
}

/// Lexical-walk branch of the workspace set (fallback outside a git
/// worktree): every repo `.rs` file outside `.git/`, `target/`,
/// `.history/` (std only; symlinked files/dirs are skipped, SPEC 14.8).
/// Sorted for deterministic reports.
fn walk_rs_files(repo: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![repo.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if name == ".git" || name == "target" || name == ".history" {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file() && name.ends_with(".rs") {
                if let Ok(rel) = path.strip_prefix(repo) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Declared `old mode`/`new mode` for `rel` (the T364 mode-flip input:
/// real git's pure-mode-change shape), or `None` when the section
/// carries no mode pair (content-only, today's behavior) or both
/// modes agree (no flip). Verbatim passthrough: a bogus pair fails
/// loud in `git apply`, never silently (SPEC 2.6).
fn section_modes(patch: &rsmpl_patch::Patch, rel: &str) -> Option<(String, String)> {
    let section = patch.files.iter().find(|s| s.target_path == rel)?;
    let old_mode = section
        .metadata
        .iter()
        .find_map(|m| m.strip_prefix("old mode "))?;
    let new_mode = section
        .metadata
        .iter()
        .find_map(|m| m.strip_prefix("new mode "))?;
    if old_mode == new_mode {
        return None;
    }
    Some((old_mode.to_string(), new_mode.to_string()))
}

/// WI-2: SHA-256 hex of one materialized post-image (SPEC D.2 `post_sha`).
fn post_sha_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// WI-1b + round-5 WI-1: global `checked.semantics` (SPEC 24.6, D.2).
/// `verified` iff every rule is `instantiated`/`postcondition`, no ordinary
/// hunk is present, every rule's engine orphan count is zero, every rule's
/// follow-skip count is zero, and no rule's evidence was truncated
/// (orphan cap 8 / orphan text 80 chars / site bindings cap 8 / binding
/// value 80 chars all unhit — the same caps `sites_json`/`orphans_json`
/// render with). Counts are read from the engine `ApplyReport` before JSON
/// truncation. Rule words describe the edit proof; the global word adds
/// completeness, so a definition-only rename with live uses, a follow
/// rename with skipped sites, or a wide rename with truncated evidence
/// reports `partial` naming its blockers. Empty patches verify vacuously.
/// (`optional` zero-edit `not-applicable`, insertions, cuts, pastes,
/// follow-only extras, and ordinary hunks all force `partial` as before.)
fn global_semantics(report: &rsmpl_engine::ApplyReport) -> &'static str {
    if report.has_ordinary {
        return "partial";
    }
    for r in &report.rules {
        match r.semantics.as_str() {
            "instantiated" | "postcondition" => {}
            _ => return "partial",
        }
        if !r.orphans.is_empty() || !r.follow_skipped_details.is_empty() {
            return "partial";
        }
        if is_truncated_rule(r) {
            return "partial";
        }
    }
    "verified"
}

/// Round-5 WI-1: evidence-truncation probe (SPEC D.2 caps). True when the
/// default report would cut this rule's evidence: more than 8 orphans, any
/// orphan text over 80 chars, more than 8 bindings on any site, or any
/// binding value over 80 chars. Mirrors the `MAX_*` constants in
/// `sites_json`/`orphans_json`; `--bindings full` lifts display only, the
/// verdict still sees the caps (completeness is display-independent).
fn is_truncated_rule(r: &rsmpl_engine::RuleReport) -> bool {
    const MAX_ENTRIES: usize = 8;
    const MAX_CHARS: usize = 80;
    if r.orphans.len() > MAX_ENTRIES {
        return true;
    }
    if r.orphans.iter().any(|o| o.text.chars().count() > MAX_CHARS) {
        return true;
    }
    for s in &r.sites {
        if s.bindings.len() > MAX_ENTRIES {
            return true;
        }
        if s.bindings
            .iter()
            .any(|(_, v)| v.chars().count() > MAX_CHARS)
        {
            return true;
        }
    }
    false
}

/// Round-5 WI-1 (D12): one stderr line per blocking rule in text mode
/// (`incomplete: rule <name> (<file>): <n> orphans, <m> follow-skipped[, truncated]`).
/// Rules whose word is not `instantiated`/`postcondition` block as
/// `semantics <word>`; rules with leftovers block with their counts.
/// JSON mode stays stdout-only and carries no such lines.
fn incomplete_text_lines(report: &rsmpl_engine::ApplyReport) -> Vec<String> {
    let mut out = Vec::new();
    if report.has_ordinary {
        out.push("incomplete: ordinary hunks present".to_string());
    }
    for r in &report.rules {
        let word_ok = matches!(r.semantics.as_str(), "instantiated" | "postcondition");
        let orphans = r.orphans.len();
        let skipped = r.follow_skipped_details.len();
        let truncated = is_truncated_rule(r);
        if word_ok && orphans == 0 && skipped == 0 && !truncated {
            continue;
        }
        if word_ok {
            let mut tail = format!("{orphans} orphans, {skipped} follow-skipped");
            if truncated {
                tail.push_str(", truncated");
            }
            out.push(format!("incomplete: rule {} ({}): {tail}", r.rule, r.file));
        } else {
            out.push(format!(
                "incomplete: rule {} ({}): semantics {}",
                r.rule,
                r.file,
                r.semantics.as_str()
            ));
        }
    }
    out
}

/// WI-2 + WI-7c `--verify build`: `cargo check --quiet` in `repo`
/// after a successful apply, with orphan context — the first diagnostic
/// names the `file:line` breakage the report already lists. A real
/// breakage fails as `RSMPL_BUILD_CONFLICT` with a `read_file` repair
/// (diagnostic location, else first-orphan fallback); environmental
/// failures (missing cargo/manifest) keep `RSMPL_IO_CONFLICT`. Failure
/// exits 2 (the patch stays applied — the agent decides the next step;
/// JSON carries `applied: true`).
fn run_verify_build_with_orphans(
    repo: &std::path::Path,
    report: &rsmpl_engine::ApplyReport,
) -> Result<(), CliError> {
    match verify_build::run_verify_build_in(repo) {
        Ok(()) => Ok(()),
        Err(outcome) => Err(verify_build_error(
            outcome,
            report,
            "(patch stays applied)",
            true,
        )),
    }
}
/// WI-2: flatten every rule's orphans (report order) for the repair fallback.
fn flat_orphans(report: &rsmpl_engine::ApplyReport) -> Vec<rsmpl_engine::OrphanReport> {
    report
        .rules
        .iter()
        .flat_map(|r| r.orphans.iter().cloned())
        .collect()
}
/// WI-2: map a classified build outcome to the CLI failure (SPEC 25.10).
fn verify_build_error(
    outcome: verify_build::VerifyBuildOutcome,
    report: &rsmpl_engine::ApplyReport,
    tail: &str,
    applied: bool,
) -> CliError {
    let hint = orphan_hint(report);
    let ctx = hint.map_or(String::new(), |o| format!(" [{o}]"));
    match outcome {
        verify_build::VerifyBuildOutcome::Environmental(msg) => {
            CliError::verify(format!("--verify build: {msg}{ctx} {tail}"), applied)
        }
        verify_build::VerifyBuildOutcome::BuildFailed { short, stderr } => {
            let orphans = flat_orphans(report);
            let repair = verify_build::build_repair_for(&stderr, &orphans);
            let (file, range) = repair.as_ref().map_or((None, None), |r| {
                (
                    r["file"].as_str().map(str::to_string),
                    r["range"].as_str().map(str::to_string),
                )
            });
            CliError::verify_build(
                format!("--verify build: {short}{ctx} {tail}"),
                file,
                range,
                repair,
                applied,
            )
        }
    }
}

/// WI-7c: short `file:line` hint (first 3 orphans) for `--verify build`
/// failure reasons.
fn orphan_hint(report: &rsmpl_engine::ApplyReport) -> Option<String> {
    let mut items: Vec<String> = Vec::new();
    for r in &report.rules {
        for o in &r.orphans {
            items.push(format!("{}:{}", o.file, o.line));
            if items.len() >= 3 {
                break;
            }
        }
        if items.len() >= 3 {
            break;
        }
    }
    if items.is_empty() {
        None
    } else {
        Some(format!("orphans: {}", items.join(", ")))
    }
}

/// WI-2: one stderr line per skipped follow site in text mode
/// (`follow-skipped: {file}:{line}:{reason}` triples, document order).
/// Absent when nothing was skipped.
fn follow_skipped_text_lines(report: &rsmpl_engine::ApplyReport) -> Vec<String> {
    let mut out = Vec::new();
    for r in &report.rules {
        for s in &r.follow_skipped_details {
            out.push(format!(
                "follow-skipped: {}:{}:{}",
                s.file, s.line, s.reason
            ));
        }
    }
    out
}

/// WI-7c + WI-1a: one stderr line for text mode (same 8-entry / 80-char
/// caps; cut texts carry a trailing `…`). WI-2: each entry names its
/// reason triple (`file:line [reason]`).
fn orphan_text_line(report: &rsmpl_engine::ApplyReport) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut total = 0usize;
    for r in &report.rules {
        for o in r.orphans.iter().take(8) {
            let cut = o.text.chars().count() > 80;
            let mut text: String = o.text.chars().take(80).collect();
            if cut {
                text.push('…');
            }
            parts.push(format!("{}:{} [{}]: {text}", o.file, o.line, o.reason));
            total += 1;
        }
    }
    if parts.is_empty() {
        return None;
    }
    let extra: usize = report
        .rules
        .iter()
        .map(|r| r.orphans.len())
        .sum::<usize>()
        .saturating_sub(total);
    let tail = if extra > 0 {
        format!(" (+{extra} more)")
    } else {
        String::new()
    };
    Some(format!(
        "orphans: {} use(s) left behind: {}{tail}",
        total + extra,
        parts.join("; ")
    ))
}

/// WI-2a: verify the in-memory post-images in a temp copy of `repo` so
/// `--check --verify build` answers "would this build?" without touching
/// the worktree (the cargo probe lives in `verify_build`, SPEC 25.10). Copies the tree (minus `.git`/`target`/`.history`),
/// overlays `updated` contents for every `changed` path (plus rename
/// cleanup), runs `cargo check --quiet` there, then removes the temp dir.
/// Failure is `applied: false` (worktree untouched by construction); the
/// code/repair split mirrors the post-apply path (SPEC 25.10).
#[allow(clippy::too_many_arguments)]
fn verify_in_temp_copy(
    repo: &std::path::Path,
    updated: &std::collections::HashMap<String, String>,
    changed: &[(String, usize, usize, String)],
    rename_from: &std::collections::HashMap<String, String>,
    is_symlink: &std::collections::HashSet<String>,
    patch: &rsmpl_patch::Patch,
    report: &rsmpl_engine::ApplyReport,
) -> Result<(), CliError> {
    let tmp = materialize_verify_tree(repo, updated, changed, rename_from, is_symlink, patch)?;
    let result = verify_build::run_verify_build_in(&tmp);
    // Cleanup always; a removal fault warns but never masks the verdict.
    if let Err(e) = std::fs::remove_dir_all(&tmp) {
        eprintln!(
            "warning: verify: cannot remove temp copy {}: {e}",
            tmp.display()
        );
    }
    match result {
        Ok(()) => Ok(()),
        Err(outcome) => Err(verify_build_error(
            outcome,
            report,
            "(worktree untouched; verified in temp copy)",
            false,
        )),
    }
}

/// Materialize the post-image tree for temp-copy verification (std only).
fn materialize_verify_tree(
    repo: &std::path::Path,
    updated: &std::collections::HashMap<String, String>,
    changed: &[(String, usize, usize, String)],
    rename_from: &std::collections::HashMap<String, String>,
    is_symlink: &std::collections::HashSet<String>,
    patch: &rsmpl_patch::Patch,
) -> Result<std::path::PathBuf, CliError> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static CTR: AtomicU32 = AtomicU32::new(0);
    let ctr = CTR.fetch_add(1, Ordering::Relaxed);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!(
        "applyrs-verify-{}-{ctr}-{secs}",
        std::process::id()
    ));
    copy_tree_excluding(repo, &tmp)?;
    let deleted: std::collections::HashSet<&str> = patch
        .files
        .iter()
        .filter(|s| s.is_deleted_file)
        .map(|s| s.target_path.as_str())
        .collect();
    for (rel, _, _, _) in changed {
        let dst = tmp.join(rel);
        if deleted.contains(rel.as_str()) {
            // Deleted files lower as removals: drop the temp copy so the
            // check sees absence, not an empty file.
            match std::fs::remove_file(&dst) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return Err(CliError::io_file(
                        format!("verify: cannot remove {}: {e}", dst.display()),
                        rel,
                    ));
                }
            }
            continue;
        }
        let new_text = updated.get(rel).cloned().unwrap_or_default();
        if is_symlink.contains(rel) {
            let _ = std::fs::remove_file(&dst);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CliError::io_file(
                        format!("verify: cannot create {}: {e}", parent.display()),
                        rel,
                    )
                })?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&new_text, &dst).map_err(|e| {
                let _ = std::fs::remove_dir_all(&tmp);
                CliError::io_file(format!("verify: cannot link {}: {e}", dst.display()), rel)
            })?;
            #[cfg(not(unix))]
            {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(CliError::io(
                    "verify: symlink targets unsupported on this platform".to_string(),
                ));
            }
        } else {
            if let Some(parent) = dst.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        CliError::io_file(
                            format!("verify: cannot create {}: {e}", parent.display()),
                            rel,
                        )
                    })?;
                }
            }
            if let Err(e) = std::fs::write(&dst, new_text) {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(CliError::io_file(
                    format!("verify: cannot write {}: {e}", dst.display()),
                    rel,
                ));
            }
        }
    }
    // Renames: the old-side file was copied with the tree; drop it so the
    // temp check sees the move, not a duplicate definition.
    for (new_rel, old_rel) in rename_from {
        if changed.iter().any(|(p, _, _, _)| p == new_rel) {
            let _ = std::fs::remove_file(tmp.join(old_rel));
        }
    }
    Ok(tmp)
}

/// Recursive tree copy minus `.git`/`target`/`.history` (lexical, std
/// only). Symlinks are recreated as symlinks (never followed); unreadable
/// entries warn and are skipped so verification degrades to the readable
/// tree instead of failing on unrelated files.
fn copy_tree_excluding(repo: &std::path::Path, tmp: &std::path::Path) -> Result<(), CliError> {
    let mut stack = vec![(repo.to_path_buf(), tmp.to_path_buf())];
    while let Some((src_dir, dst_dir)) = stack.pop() {
        std::fs::create_dir_all(&dst_dir).map_err(|e| {
            CliError::io(format!("verify: cannot create {}: {e}", dst_dir.display()))
        })?;
        let entries = match std::fs::read_dir(&src_dir) {
            Ok(rd) => rd,
            Err(e) => {
                eprintln!("warning: verify: cannot list {}: {e}", src_dir.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == "target" || name == ".history" {
                continue;
            }
            let src = entry.path();
            let dst = dst_dir.join(&name);
            let meta = match std::fs::symlink_metadata(&src) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("warning: verify: cannot stat {}: {e}", src.display());
                    continue;
                }
            };
            if meta.file_type().is_symlink() {
                match std::fs::read_link(&src) {
                    Ok(target) => {
                        #[cfg(unix)]
                        if let Err(e) = std::os::unix::fs::symlink(&target, &dst) {
                            eprintln!("warning: verify: cannot link {}: {e}", dst.display());
                        }
                        #[cfg(not(unix))]
                        eprintln!("warning: verify: skipping symlink {}", src.display());
                    }
                    Err(e) => eprintln!("warning: verify: cannot read link {}: {e}", src.display()),
                }
            } else if meta.is_dir() {
                stack.push((src, dst));
            } else if meta.is_file() {
                if let Err(e) = std::fs::copy(&src, &dst) {
                    eprintln!("warning: verify: cannot copy {}: {e}", src.display());
                }
            }
        }
    }
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
                eprintln!("warning: history: cannot prune {}: {e}", stale.display());
            }
        }
    }
}

fn parse_err(e: rsmpl_patch::ParseError) -> CliError {
    match e {
        rsmpl_patch::ParseError::Structured(s) => CliError::Structured(s),
        // Bare patch-structure faults with no rule/file context (WI-3:
        // defensive — `Message` is never constructed today, but any
        // future site joins the null-free enumeration by construction).
        rsmpl_patch::ParseError::Message(m) => CliError::Structured(
            StructuredError::new(ErrorCode::ParseConflict, m).with_repair(
                StructuredError::fix_pattern_repair(
                    "patch",
                    "provide a `diff --git a/.. b/..` file section with hunks (see SPEC 6 minimal recipe)",
                ),
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // WI-15: builder pins call the moved `report::` helpers.
    use super::report::{orphans_json, sites_binding_total, sites_json};

    fn test_report(
        semantics: rsmpl_engine::RuleSemantics,
        orphans: Vec<rsmpl_engine::OrphanReport>,
        skips: Vec<rsmpl_engine::FollowSkipReport>,
        sites: Vec<rsmpl_engine::SiteReport>,
    ) -> rsmpl_engine::ApplyReport {
        rsmpl_engine::ApplyReport {
            rules: vec![rsmpl_engine::RuleReport {
                rule: "r".to_string(),
                file: "src/lib.rs".to_string(),
                matches: 1,
                spans: vec![(1, 1)],
                span_labels: vec![None],
                sites,
                follow_skipped_details: skips,
                orphans,
                semantics,
                symbol: None,
            }],
            has_ordinary: false,
            drifts: Vec::new(),
        }
    }

    #[test]
    fn clean_instantiated_verifies() {
        let rep = test_report(
            rsmpl_engine::RuleSemantics::Instantiated,
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(global_semantics(&rep), "verified");
        assert!(incomplete_text_lines(&rep).is_empty());
    }

    #[test]
    fn orphaned_blocks_with_counts() {
        let rep = test_report(
            rsmpl_engine::RuleSemantics::Instantiated,
            vec![rsmpl_engine::OrphanReport {
                file: "src/lib.rs".to_string(),
                line: 2,
                text: "foo()".to_string(),
                reason: "off-target".to_string(),
            }],
            vec![],
            vec![],
        );
        assert_eq!(global_semantics(&rep), "partial");
        let lines = incomplete_text_lines(&rep);
        assert_eq!(lines.len(), 1, "got: {lines:?}");
        assert!(lines[0].contains("1 orphans"), "got: {lines:?}");
    }

    #[test]
    fn skipped_blocks_with_counts() {
        let rep = test_report(
            rsmpl_engine::RuleSemantics::Postcondition,
            vec![],
            vec![rsmpl_engine::FollowSkipReport {
                file: "src/lib.rs".to_string(),
                line: 3,
                reason: "shadowed".to_string(),
            }],
            vec![],
        );
        assert_eq!(global_semantics(&rep), "partial");
        let lines = incomplete_text_lines(&rep);
        assert_eq!(lines.len(), 1, "got: {lines:?}");
        assert!(lines[0].contains("1 follow-skipped"), "got: {lines:?}");
    }

    #[test]
    fn truncated_binding_blocks() {
        let long = "x".repeat(81);
        let rep = test_report(
            rsmpl_engine::RuleSemantics::Postcondition,
            vec![],
            vec![],
            vec![rsmpl_engine::SiteReport {
                span: (1, 1),
                bindings: vec![("E".to_string(), long)],
            }],
        );
        assert_eq!(global_semantics(&rep), "partial");
        assert!(
            incomplete_text_lines(&rep)[0].contains("truncated"),
            "got: {:?}",
            incomplete_text_lines(&rep)
        );
    }

    #[test]
    fn reparse_only_still_blocks() {
        let rep = test_report(
            rsmpl_engine::RuleSemantics::ReparseOnly,
            vec![],
            vec![],
            vec![],
        );
        assert_eq!(global_semantics(&rep), "partial");
        assert!(
            incomplete_text_lines(&rep)[0].contains("reparse-only"),
            "got: {:?}",
            incomplete_text_lines(&rep)
        );
    }

    fn many_orphans(n: usize) -> Vec<rsmpl_engine::OrphanReport> {
        (0..n)
            .map(|i| rsmpl_engine::OrphanReport {
                file: "src/lib.rs".to_string(),
                line: i as u32 + 2,
                text: format!("old({i})"),
                reason: "off-target".to_string(),
            })
            .collect()
    }

    #[test]
    fn wi15a_totals_count_pre_truncation() {
        // WI-15a: every rule entry carries pre-truncation counts —
        // `orphans_total` (engine len), `follow_skipped_total`
        // (engine len), `sites_total` (binding entries across sites).
        let rep = test_report(
            rsmpl_engine::RuleSemantics::Postcondition,
            many_orphans(12),
            vec![rsmpl_engine::FollowSkipReport {
                file: "src/lib.rs".to_string(),
                line: 3,
                reason: "shadowed".to_string(),
            }],
            vec![rsmpl_engine::SiteReport {
                span: (1, 1),
                bindings: vec![
                    ("E".to_string(), "a".to_string()),
                    ("F".to_string(), "b".to_string()),
                ],
            }],
        );
        let r = &rep.rules[0];
        assert_eq!(r.orphans.len(), 12);
        assert_eq!(r.follow_skipped_details.len(), 1);
        assert_eq!(sites_binding_total(&r.sites), 2);
        // Display-independence: the verdict reads engine counts, so a
        // 12-orphan rule blocks identically under either budget.
        assert_eq!(global_semantics(&rep), "partial");
        assert!(is_truncated_rule(r));
    }

    #[test]
    fn wi15b_full_lifts_entry_caps() {
        // WI-15b: `--bindings full` lifts the 8-entry caps (orphan
        // entries and per-site binding entries), not just the char cut.
        let orphans = many_orphans(12);
        let (val, trunc, _) = orphans_json(&orphans, false);
        assert_eq!(val.as_array().unwrap().len(), 8);
        assert_eq!(trunc, 4);
        let (val, trunc, _) = orphans_json(&orphans, true);
        assert_eq!(val.as_array().unwrap().len(), 12);
        assert_eq!(trunc, 0);
        let sites = vec![rsmpl_engine::SiteReport {
            span: (1, 1),
            bindings: (0..10)
                .map(|i| (format!("B{i}"), "v".to_string()))
                .collect(),
        }];
        let (val, trunc, _) = sites_json(&sites, &[None], false);
        assert_eq!(
            val.as_array().unwrap()[0]["bindings"]
                .as_object()
                .unwrap()
                .len(),
            8
        );
        assert_eq!(trunc, 2);
        let (val, trunc, _) = sites_json(&sites, &[None], true);
        assert_eq!(
            val.as_array().unwrap()[0]["bindings"]
                .as_object()
                .unwrap()
                .len(),
            10
        );
        assert_eq!(trunc, 0);
    }

    #[test]
    fn wi15d_origin_marks_follow_spans() {
        // WI-15d: carried provenance — `follow`-labelled spans read
        // `inferred`, everything else `authored` (never inferred from
        // `semantics`).
        let sites = vec![
            rsmpl_engine::SiteReport {
                span: (1, 1),
                bindings: vec![],
            },
            rsmpl_engine::SiteReport {
                span: (2, 2),
                bindings: vec![],
            },
        ];
        let labels = vec![Some("rename".to_string()), Some("follow".to_string())];
        let (val, _, _) = sites_json(&sites, &labels, false);
        let arr = val.as_array().unwrap();
        assert_eq!(arr[0]["origin"], serde_json::json!("authored"));
        assert_eq!(arr[1]["origin"], serde_json::json!("inferred"));
        // Labelless rules (single-edit) read `authored`.
        let (val, _, _) = sites_json(&sites, &[], true);
        let arr = val.as_array().unwrap();
        assert_eq!(arr[0]["origin"], serde_json::json!("authored"));
        assert_eq!(arr[1]["origin"], serde_json::json!("authored"));
    }
}
