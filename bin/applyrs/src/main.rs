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

    /// Extra verification after a successful apply: only `build` is
    /// accepted (`cargo check --quiet` in `--repo`). Never runs under
    /// `--check`, never by default (SPEC D.2).
    #[arg(long, value_name = "MODE")]
    verify: Option<String>,

    /// Archive the raw patch to `<repo>/.history/{id}.rsmpl` on `--check`
    /// runs too (SPEC D.1). Real applies always archive; `--check` skips
    /// the archive unless this flag is passed (clean measurement).
    #[arg(long)]
    history: bool,
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
            err: StructuredError::new(ErrorCode::IoConflict, message),
            applied,
        }
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
            let mut payload = serde_json::json!({"version": 3, "error": e.err().to_json()});
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
            return Err(CliError::io_file(reason, &rel));
        }
        // T377: a rename section's hunks cite old-side coordinates, so
        // seed the new-side entry from the OLD file's content (the
        // engine then applies ordinary and RSmPL hunks transparently).
        let (read_rel, is_rename) = if section.is_rename() {
            let old_rel = rsmpl_patch::stripped_path(&section.old_path);
            if let Some(reason) = reject_escaped_target(&old_rel) {
                return Err(CliError::io_file(reason, &old_rel));
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

    // WI-7c: orphan context for `--verify build` failures (first
    // diagnostic names the breakage the report already lists).
    let orphans_hint = orphan_hint(&report);
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
                        orphans_hint.as_deref(),
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
                run_verify_build_with_orphans(&cli.repo, orphans_hint.as_deref())?;
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
            "version": 3,
            "rules": report.rules.iter().map(|r| {
                let mut entry = serde_json::json!({
                    "rule": r.rule, "file": r.file, "matches": r.matches, "spans": r.spans.iter().map(|(s, e)| serde_json::json!([s, e])).collect::<Vec<_>>(),
                    "semantics": r.semantics.as_str(),
                });
                // WI-2b/2c: per-site evidence, one entry per emitted edit
                // in span order. Omitted when no site binds (renames,
                // insertions, deletes, use-prefix carry no evidence).
                if !r.sites.is_empty() {
                    let (sites_val, sites_trunc, bindings_cut) = sites_json(&r.sites);
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
                // `# follow=uses` skip notes stay hunk-level (no span).
                if let Some(skipped) = &r.follow_skipped {
                    entry["follow_skipped"] = serde_json::json!(skipped);
                }
                // WI-7c: report-only orphan evidence, capped like bindings.
                if !r.orphans.is_empty() {
                    let (orphans_val, orphans_trunc, orphans_cut) = orphans_json(&r.orphans);
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
                orphans_hint.as_deref(),
            )?;
            eprintln!("check: lowered patch applies cleanly; semantics {semantics}; build check passed (temp copy, worktree untouched)");
        } else {
            eprintln!("check: lowered patch applies cleanly; semantics {semantics}");
        }
        if let Some(line) = orphan_text_line(&report) {
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
        run_verify_build_with_orphans(&cli.repo, orphans_hint.as_deref())?;
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

/// WI-1b: global `checked.semantics` (SPEC 24.6, D.2). `verified` iff
/// every rule is `instantiated`/`postcondition` and no ordinary hunk is
/// present; otherwise `partial` (insertions, cuts, pastes, follow-only
/// extras, optional zero-edits, and ordinary hunks all force `partial`).
/// Empty patches verify vacuously.
fn global_semantics(report: &rsmpl_engine::ApplyReport) -> &'static str {
    if report.has_ordinary {
        return "partial";
    }
    for r in &report.rules {
        match r.semantics.as_str() {
            "instantiated" | "postcondition" => {}
            _ => return "partial",
        }
    }
    "verified"
}

/// WI-2b/2c + WI-1a: token-bounded per-site evidence (SPEC D.2). One entry
/// per emitted edit in span order — `[{span: [s, e], bindings: {...}}]` —
/// with 8-entry / 80-char per-site truncation. A value cut at 80 chars
/// carries a trailing `…` (U+2026) so agents can tell a truncated binding
/// from a complete one. Returns the JSON array plus the summed
/// omitted-entry count (`sites_truncated`) plus the number of cut values
/// (`bindings_truncated_chars`).
fn sites_json(sites: &[rsmpl_engine::SiteReport]) -> (serde_json::Value, usize, usize) {
    const MAX_ENTRIES: usize = 8;
    const MAX_CHARS: usize = 80;
    let mut arr = Vec::with_capacity(sites.len());
    let mut omitted_total = 0usize;
    let mut cut_total = 0usize;
    for site in sites {
        let mut map = serde_json::Map::new();
        for (k, v) in site.bindings.iter().take(MAX_ENTRIES) {
            let cut = v.chars().count() > MAX_CHARS;
            let mut truncated: String = v.chars().take(MAX_CHARS).collect();
            if cut {
                truncated.push('…');
                cut_total += 1;
            }
            map.insert(k.clone(), serde_json::Value::String(truncated));
        }
        omitted_total += site.bindings.len().saturating_sub(map.len());
        arr.push(serde_json::json!({
            "span": [site.span.0, site.span.1],
            "bindings": serde_json::Value::Object(map),
        }));
    }
    (serde_json::Value::Array(arr), omitted_total, cut_total)
}

/// WI-2a + WI-7c `--verify build`: `cargo check --quiet` in `repo`
/// after a successful apply, with orphan context — the first diagnostic
/// names the `file:line` breakage the report already lists. Failure
/// exits 2 (the patch stays applied — the agent decides the next step;
/// JSON carries `applied: true`).
fn run_verify_build_with_orphans(
    repo: &std::path::Path,
    orphans: Option<&str>,
) -> Result<(), CliError> {
    match run_verify_build_in(repo) {
        Ok(()) => Ok(()),
        Err(short) => {
            let ctx = orphans.map_or(String::new(), |o| format!(" [{o}]"));
            Err(CliError::verify(
                format!("--verify build: {short}{ctx} (patch stays applied)"),
                true,
            ))
        }
    }
}

/// WI-7c + WI-1a: token-bounded orphan evidence (SPEC D.2). Max 8 entries
/// per rule, each `text` truncated to 80 chars with a trailing `…` when
/// cut. Returns the JSON array plus the omitted-entry count
/// (`orphans_truncated`) plus the number of cut texts
/// (`orphans_truncated_chars`).
fn orphans_json(orphans: &[rsmpl_engine::OrphanReport]) -> (serde_json::Value, usize, usize) {
    const MAX_ORPHANS: usize = 8;
    const MAX_CHARS: usize = 80;
    let mut arr = Vec::with_capacity(orphans.len().min(MAX_ORPHANS));
    let mut cut_total = 0usize;
    for o in orphans.iter().take(MAX_ORPHANS) {
        let cut = o.text.chars().count() > MAX_CHARS;
        let mut text: String = o.text.chars().take(MAX_CHARS).collect();
        if cut {
            text.push('…');
            cut_total += 1;
        }
        arr.push(serde_json::json!({"file": o.file, "line": o.line, "text": text}));
    }
    let omitted = orphans.len().saturating_sub(arr.len());
    (serde_json::Value::Array(arr), omitted, cut_total)
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

/// WI-7c + WI-1a: one stderr line for text mode (same 8-entry / 80-char
/// caps; cut texts carry a trailing `…`).
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
            parts.push(format!("{}:{}: {text}", o.file, o.line));
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

/// Core `cargo check --quiet` probe in `dir`: `Ok` on success, `Err(short)`
/// with the first `error` diagnostic line (300 chars) on failure.
fn run_verify_build_in(dir: &std::path::Path) -> Result<(), String> {
    let out = std::process::Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run `cargo check`: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr
        .lines()
        .find(|l| l.contains("error"))
        .map(str::trim)
        .unwrap_or("cargo check failed");
    Err(first.chars().take(300).collect())
}

/// WI-2a: verify the in-memory post-images in a temp copy of `repo` so
/// `--check --verify build` answers "would this build?" without touching
/// the worktree. Copies the tree (minus `.git`/`target`/`.history`),
/// overlays `updated` contents for every `changed` path (plus rename
/// cleanup), runs `cargo check --quiet` there, then removes the temp dir.
/// Failure is `applied: false` (worktree untouched by construction).
#[allow(clippy::too_many_arguments)]
fn verify_in_temp_copy(
    repo: &std::path::Path,
    updated: &std::collections::HashMap<String, String>,
    changed: &[(String, usize, usize, String)],
    rename_from: &std::collections::HashMap<String, String>,
    is_symlink: &std::collections::HashSet<String>,
    patch: &rsmpl_patch::Patch,
    orphans: Option<&str>,
) -> Result<(), CliError> {
    let tmp = materialize_verify_tree(repo, updated, changed, rename_from, is_symlink, patch)?;
    let result = run_verify_build_in(&tmp);
    // Cleanup always; a removal fault warns but never masks the verdict.
    if let Err(e) = std::fs::remove_dir_all(&tmp) {
        eprintln!(
            "warning: verify: cannot remove temp copy {}: {e}",
            tmp.display()
        );
    }
    match result {
        Ok(()) => Ok(()),
        Err(short) => {
            let ctx = orphans.map_or(String::new(), |o| format!(" [{o}]"));
            Err(CliError::verify(
                format!("--verify build: {short}{ctx} (worktree untouched; verified in temp copy)"),
                false,
            ))
        }
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
        // Bare patch-structure faults with no rule/file context.
        rsmpl_patch::ParseError::Message(m) => {
            CliError::Structured(StructuredError::new(ErrorCode::ParseConflict, m))
        }
    }
}
