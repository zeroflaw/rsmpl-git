//! Line-oriented parser for RSmPL-Git patch files.
//!
//! Grammar (SPEC section 30):
//! `patch ::= file_section*`, file sections start with `diff --git`
//! (one path suffices, SPEC 6), hunks start with `@@ -a,b (+c,d)? @@ tail`
//! (new-side elision is RSmPL-only, SPEC 8). A hunk is RSmPL iff the
//! header carries `rule=` or at least one unprefixed `#` directive line
//! follows the header (SPEC 7.1); `# lang=rust` is accepted but never
//! required (Rust is the default).

// WI-3c: git-path helpers live in `gitpaths`, whole-patch validation in
// `validate` (both re-exported here so `crate::parser::X` paths keep
// working for `canonical.rs`, `lib.rs`, and the tests below).
pub use super::gitpaths::{
    collapse_git_header, decode_git_path_token, quote_git_path, split_git_paths,
};
use super::gitpaths::{parse_diff_git_header, target_path, with_ab_prefix};
use super::validate::{
    validate_no_broken_raw_quote_directive, validate_no_broken_raw_quote_pattern, validate_patch,
};
use crate::errors::{ErrorCode, StructuredError};
use crate::model::*;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Structured(#[from] StructuredError),
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, ParseError>;

/// Parse a complete patch document.
pub fn parse_patch(text: &str) -> Result<Patch> {
    let mut patch = Patch::default();
    // T385: no CRLF normalization — a trailing `\r` on a hunk body
    // line is CRLF file content and must survive byte-exact (the
    // engine compares against `split('\n')` file bytes and inserts
    // `text + '\n'`). Structural lines tolerate one trailing `\r` at
    // their own consumption points (whitespace-split parses, `trim`
    // checks, and explicit strips below), so CRLF-authored patch
    // files keep parsing as before.
    let lines: Vec<&str> = text.split('\n').collect();

    // Handle optional top-level version marker `# gitrs=0.1` (SPEC 33.1).
    let mut idx = 0;
    // Skip leading blank lines.
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx < lines.len() && is_version_marker(lines[idx]) {
        patch.version = Some(lines[idx].trim().trim_start_matches('#').trim().to_string());
        idx += 1;
    }

    let mut current: Option<FileSection> = None;

    while idx < lines.len() {
        let line = lines[idx];
        if line.trim().is_empty() {
            // Blank line between hunks/files: skip (tolerated, not a body line).
            // A truly empty diff body line would be "" which git treats as
            // context-empty; we treat lone "" inside hunks via hunk parsing.
            idx += 1;
            continue;
        }
        if let Some((old, new)) = parse_diff_git_header(line) {
            if let Some(sec) = current.take() {
                patch.files.push(sec);
            }
            // T377: differing real paths are a file rename section
            // (SPEC 6) -- accepted and lowered back to `rename
            // from/to`. `/dev/null` halves stay the create/delete
            // spelling. Genuinely different paths were rejected
            // before v0.1 rename support (T29).
            current = Some(FileSection {
                old_path: old.clone(),
                new_path: new.clone(),
                target_path: target_path(&new, &old),
                is_new_file: old == "/dev/null" || old == "dev/null",
                is_deleted_file: new == "/dev/null" || new == "dev/null",
                metadata: Vec::new(),
                hunks: Vec::new(),
            });
            idx += 1;
            continue;
        }
        if line.starts_with("@@ ") || line.starts_with("@@-") || line.starts_with("@@") {
            let section = current.as_mut().ok_or_else(|| {
                StructuredError::new(ErrorCode::ParseConflict, "hunk header outside file section")
                    .with_repair(StructuredError::fix_pattern_repair(
                        "section",
                        "start the patch with a `diff --git a/.. b/..` file section before the first `@@` header",
                    ))
            })?;
            let target = section.target_path.clone();
            let (hunk, next) = parse_hunk(&lines, idx, &target)?;
            section.hunks.push(hunk);
            idx = next;
            continue;
        }
        // File metadata lines (---, +++, index, new/deleted file mode, etc.)
        if line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("index ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity")
            || line.starts_with("rename ")
            || line.starts_with("Binary ")
        {
            // Real git deletions/creations keep same-path `diff --git`
            // halves and mark the side in `---`/`+++` instead (T350):
            // mirror the markers into the create/delete flags so they
            // hold regardless of header spelling.
            //
            // T385: tolerate one trailing `\r` (CRLF-authored patch
            // file) — the exact `/dev/null` comparisons and the stored
            // metadata (re-emitted into the lowered diff) must not
            // carry it. Hunk body content lines keep their `\r` in
            // `parse_body_lines` below; this strip is metadata-only.
            let line = line.strip_suffix('\r').unwrap_or(line);
            if let Some(sec) = current.as_mut() {
                if line == "+++ /dev/null" || line == "+++ dev/null" {
                    sec.is_deleted_file = true;
                } else if line == "--- /dev/null" || line == "--- dev/null" {
                    sec.is_new_file = true;
                } else if line.starts_with("deleted file mode") {
                    // T364: real git omits `---`/`+++` for empty files,
                    // so mirror the mode lines too -- a `deleted file
                    // mode` section is a delete even with no `+++` line,
                    // and `new file mode` a create even with no `---`.
                    sec.is_deleted_file = true;
                } else if line.starts_with("new file mode") {
                    sec.is_new_file = true;
                } else if let Some(from) = line.strip_prefix("rename from ") {
                    // T397: bare git resolves renames from these lines
                    // (not the `diff --git` split), so they override the
                    // halves — a no-op when both agree (the T377 shapes).
                    // Uniform is kept: halves wear `a/`/`b/` downstream.
                    sec.old_path = with_ab_prefix(from, 'a');
                    sec.target_path = target_path(&sec.new_path, &sec.old_path);
                } else if let Some(to) = line.strip_prefix("rename to ") {
                    sec.new_path = with_ab_prefix(to, 'b');
                    sec.target_path = target_path(&sec.new_path, &sec.old_path);
                }
                sec.metadata.push(line.to_string());
                idx += 1;
                continue;
            }
            return Err(StructuredError::new(
                ErrorCode::ParseConflict,
                format!("file metadata outside file section: {line}"),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "section",
                "move the metadata line below its `diff --git a/.. b/..` header",
            ))
            .into());
        }
        // Unknown stray line.
        if current.is_none() {
            // Tolerate trailing version marker duplicates? No — error.
            if line.trim().is_empty() {
                idx += 1;
                continue;
            }
            return Err(StructuredError::new(
                ErrorCode::ParseConflict,
                format!("unexpected line outside file section: {line}"),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "section",
                "start the patch with a `diff --git a/.. b/..` file section (see SPEC 6 minimal recipe)",
            ))
            .into());
        }
        // Stray line inside file section but outside hunk: be lenient and
        // record as metadata (e.g. `diff --git` extensions).
        if let Some(sec) = current.as_mut() {
            // T365: a hunk header with leading whitespace is a valid
            // unified-diff context line, so without this gate it lands
            // here as metadata and the section lowers to an empty no-op
            // (exit 0, `check: true`). Headers must start at column 0.
            let trimmed = line.trim_start();
            if trimmed.starts_with("@@") && parse_hunk_header(trimmed).is_ok() {
                return Err(StructuredError::new(
                    ErrorCode::ParseConflict,
                    format!(
                        "hunk header with leading whitespace (headers must start at column 0): {line}"
                    ),
                )
                .with_file(sec.target_path.clone())
                .with_range(format!("line {}", idx + 1))
                .with_suggestion("remove the leading whitespace so the header starts at column 0")
                .with_repair(StructuredError::fix_pattern_repair(
                    "header",
                    "remove the leading whitespace so the header starts at column 0",
                ))
                .into());
            }
            sec.metadata.push(line.to_string());
            idx += 1;
            continue;
        }
        idx += 1;
    }

    if let Some(sec) = current.take() {
        patch.files.push(sec);
    }

    if patch.files.is_empty() {
        // T59: an empty patch (zero file sections) is an authoring typo,
        // not a no-op — fail fast per SPEC 2.6 instead of reporting success.
        return Err(StructuredError::new(
            ErrorCode::ParseConflict,
            "empty patch: no `diff --git` file sections",
        )
        .with_suggestion(
            "provide at least one `diff --git a/.. b/..` file section with a hunk \
              (see SPEC 6 minimal recipe); check for an empty patch input \
              (stdin, --file, or --patch)",
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "empty",
            "provide at least one `diff --git a/.. b/..` file section with a hunk (see SPEC 6 minimal recipe)",
        ))
        .into());
    }

    validate_patch(&patch)?;

    Ok(patch)
}

fn is_version_marker(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("# gitrs=") || t.starts_with("#gitrs=")
}

fn parse_hunk(lines: &[&str], at: usize, file: &str) -> Result<(Hunk, usize)> {
    let header_line = lines[at];
    let header = parse_hunk_header(header_line).map_err(|e: StructuredError| {
        ParseError::Structured(e.with_range(format!("line {}", at + 1)))
    })?;
    let mut idx = at + 1;
    // Collect directive block: consecutive unprefixed `#` lines.
    let mut raw_directives: Vec<String> = Vec::new();
    while idx < lines.len() {
        let l = lines[idx];
        if l.starts_with('#') {
            raw_directives.push(l.to_string());
            idx += 1;
        } else {
            break;
        }
    }
    let is_rsmpl_candidate = !raw_directives.is_empty() || header.rule.is_some();
    // Validate backtick quoting early for RSmPL candidates (SPEC 24.1.5).
    if is_rsmpl_candidate {
        for d in &raw_directives {
            validate_no_broken_raw_quote_directive(d)?;
        }
    }

    if is_rsmpl_candidate {
        // Per SPEC 7.1 a hunk is RSmPL iff the header carries `rule=`
        // or at least one unprefixed `#` directive block follows it
        // (Rust is the default language); `# lang=rust` may be
        // included for explicitness, `# lang=<other>` is rejected by
        // `parse_directives`. A `rule=`-only hunk carries zero
        // directives (diet form, SPEC WI-1); unknown/typo'd keys are
        // rejected there too, so omitting the lang line never silently
        // accepts a typo.
        let directives = parse_directives(&raw_directives)?;
        // Parse body lines until next hunk/file header or EOF/blank separator.
        let (body, next) = parse_body_lines(lines, idx, true)?;
        if body.is_empty() {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "RSmPL hunk has no pattern body lines",
            )
            .with_rule(header.rule.clone().unwrap_or_default())
            .with_repair(StructuredError::fix_pattern_repair(
                "body",
                "add at least one `-`/`+`/` ` pattern line below the header",
            ))
            .into());
        }
        // Directive must precede body (SPEC 7.3) — guaranteed by construction.
        // Also reject directives appearing after body start: that would have
        // been parsed as body; but an unprefixed `#` inside body region is
        // a hard error (it looks like a misplaced directive).
        Ok((
            Hunk::Rsmpl(RsmplHunk {
                header,
                directives,
                raw_directives,
                lines: body,
            }),
            next,
        ))
    } else {
        // New-side elision (`@@ -old,len @@`) is RSmPL-only (SPEC 8):
        // ordinary hunks keep full Git two-range semantics.
        if header.new_elided {
            // WI-3: `fix_header` names the exact header the agent pastes
            // (new side mirrors the old side; counts verified at apply).
            let expected_header = format!(
                "@@ -{},{} +{},{} @@",
                header.old_start, header.old_len, header.old_start, header.old_len
            );
            return Err(StructuredError::new(
                ErrorCode::ParseConflict,
                format!(
                    "ordinary hunk requires both -old,len +new,len ranges: {}",
                    lines[at]
                ),
            )
            .with_suggestion(
                "write both ranges (e.g. `@@ -1,2 +1,2 @@`) or add `rule=` or a `#` directive line to make it an RSmPL hunk",
            )
            .with_repair(serde_json::json!({
                "action": "fix_header",
                "expected_old": header.old_len,
                "expected_new": header.old_len,
                "claimed_old": header.old_len,
                "claimed_new": null,
                "expected_header": expected_header,
            }))
            .into());
        }
        let (mut body, mut next) = parse_body_lines(lines, idx, false)?;
        // T367: ordinary hunk headers must agree with the body: old-side
        // lines (` ` + `-`) equal `-old,len`, new-side lines (` ` + `+`)
        // equal `+new,len` (`\ No newline` markers are neither side).
        // The engine applies from the body alone, so a miscounted header
        // used to surface downstream as a misleading `GIT_HUNK_CONFLICT`.
        // RSmPL hunks are exempt (counts are search ranges there, bodies
        // are patterns, SPEC 10.4).
        // T431: trailing blank-context repair (see `trailing_blank` module,
        // kept out of line to hold the 2000-line hygiene limit).
        crate::trailing_blank::repair_trailing_blank_context(
            &mut body,
            &mut next,
            lines,
            header.old_len,
            header.new_len,
        );
        let old_count = body
            .iter()
            .filter(|b| b.prefix == ' ' || b.prefix == '-')
            .count() as u32;
        let new_count = body
            .iter()
            .filter(|b| b.prefix == ' ' || b.prefix == '+')
            .count() as u32;
        if old_count != header.old_len || new_count != header.new_len {
            let mut err = StructuredError::new(
                ErrorCode::ParseConflict,
                format!(
                    "ordinary hunk body has {old_count} old-side and {new_count} new-side lines but header claims -{},{} +{},{}: {}",
                    header.old_start, header.old_len, header.new_start, header.new_len, lines[at]
                ),
            )
            .with_file(file)
            .with_range(format!("line {}", at + 1))
            .with_suggestion("fix the header counts to match the body lines (old-side = ` ` + `-` lines, new-side = ` ` + `+` lines)")
            .with_repair(serde_json::json!({
                "action": "fix_header",
                "expected_old": old_count,
                "expected_new": new_count,
                "claimed_old": header.old_len,
                "claimed_new": header.new_len,
                "expected_header": format!(
                    "@@ -{},{} +{},{} @@",
                    header.old_start, old_count, header.new_start, new_count
                ),
            }));
            if let Some(rule) = &header.rule {
                err = err.with_rule(rule.clone());
            }
            return Err(err.into());
        }
        Ok((
            Hunk::Ordinary(OrdinaryHunk {
                header,
                lines: body,
            }),
            next,
        ))
    }
}

fn is_body_start(s: &str) -> bool {
    matches!(s.chars().next(), Some(' ' | '+' | '-' | '\\'))
}

fn parse_body_lines(
    lines: &[&str],
    mut idx: usize,
    is_rsmpl: bool,
) -> Result<(Vec<BodyLine>, usize)> {
    let mut out = Vec::new();
    while idx < lines.len() {
        let l = lines[idx];
        if l.starts_with("diff --git ") || l.starts_with("@@ ") || l.starts_with("@@-") {
            break;
        }
        if l.is_empty() || l.trim().is_empty() {
            // A blank line is ambiguous: git represents an empty context
            // line as "" (zero-length) or " " — while a truly blank line
            // separates hunks. Look ahead past the whole consecutive
            // blank run: if the first non-blank after the run is a body
            // line, the entire run counts as empty context lines;
            // otherwise the run is a separator and the hunk ends here.
            // The lookahead alone decides, even for the first body line:
            // a hunk header is always followed by body lines, so a leading
            // blank run followed by body lines belongs to the hunk (T212).
            // Run-aware: a single-blank check (immediate next line only)
            // splits hunks holding ≥2 consecutive blank file lines.
            let mut j = idx;
            while j < lines.len() && (lines[j].is_empty() || lines[j].trim().is_empty()) {
                j += 1;
            }
            let next_is_body = j < lines.len() && is_body_start(lines[j]);
            if next_is_body {
                while idx < j {
                    let bl = lines[idx];
                    // T388: preserve content byte-exact. A leading `' '`
                    // is the context prefix (git emits `" " + content`,
                    // so `"    "` is a 3-space file line, not an empty
                    // one). Any other leading whitespace (including the
                    // empty line) has no diff-prefix meaning; the empty
                    // line reads as empty context.
                    let text = if bl.is_empty() {
                        String::new()
                    } else {
                        bl.strip_prefix(' ').unwrap_or_default().to_string()
                    };
                    out.push(BodyLine { prefix: ' ', text });
                    idx += 1;
                }
                continue;
            }
            break;
        }
        if l.starts_with('#') {
            // Unprefixed `#` after body start: misplaced directive (SPEC 7.3).
            if is_rsmpl {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    "directive line must appear immediately after hunk header, before body lines",
                )
                .with_suggestion("move the `#` line above the first `-`/`+`/` ` line")
                .with_repair(StructuredError::fix_pattern_repair(
                    "directive",
                    "move the `#` line above the first `-`/`+`/` ` line",
                ))
                .into());
            }
            break;
        }
        let first = l.chars().next().unwrap_or(' ');
        if matches!(first, ' ' | '+' | '-' | '\\') {
            let text = if l.is_empty() {
                String::new()
            } else {
                l[1..].to_string()
            };
            // Early raw-quote validation for RSmPL bodies.
            if is_rsmpl && matches!(first, ' ' | '+' | '-') {
                validate_no_broken_raw_quote_pattern(&text)?;
            }
            out.push(BodyLine {
                prefix: first,
                text,
            });
            idx += 1;
        } else {
            break;
        }
    }
    Ok((out, idx))
}

fn parse_hunk_header(line: &str) -> std::result::Result<HunkHeader, StructuredError> {
    // `@@ -old[,len] (+new[,len])? @@ tail`; the `+new` side may be elided
    // for RSmPL hunks (SPEC 8), defaulting to the old side (cf. SPEC 10.4).
    // WI-3: header-shape repairs name the exact header template.
    let err = |msg: String| {
        StructuredError::new(ErrorCode::ParseConflict, msg).with_repair(
            StructuredError::fix_pattern_repair(
                "header",
                "write the header as `@@ -old[,len] [+new[,len]] @@ [rule=name]` (e.g. `@@ -1,3 @@ rule=r`)",
            ),
        )
    };
    if !line.starts_with("@@") {
        return Err(err(format!("invalid hunk header: {line}")));
    }
    // Find closing `@@`.
    let inner_end = line[2..]
        .find("@@")
        .ok_or_else(|| err(format!("invalid hunk header (missing closing @@): {line}")))?;
    let ranges = line[2..2 + inner_end].trim();
    let tail = line[2 + inner_end + 2..].trim().to_string();
    let mut parts = ranges.split_whitespace();
    let old_s = parts
        .next()
        .ok_or_else(|| err(format!("invalid hunk header ranges: {line}")))?;
    let new_s = parts.next();
    if parts.next().is_some() {
        return Err(err(format!("invalid hunk header ranges: {line}")));
    }
    let (old_start, old_len) = parse_range(old_s, '-')?;
    let (new_start, new_len, new_elided) = match new_s {
        Some(s) => {
            let (ns, nl) = parse_range(s, '+')?;
            (ns, nl, false)
        }
        None => (old_start, old_len, true),
    };
    let rule = parse_header_tags(&tail)?;
    Ok(HunkHeader {
        old_start,
        old_len,
        new_start,
        new_len,
        new_elided,
        rule,
        raw_tail: tail,
    })
}

fn parse_range(s: &str, expect: char) -> std::result::Result<(u32, u32), StructuredError> {
    // WI-3: range-shape repairs name the exact range template.
    let range_err = |s: &str| {
        StructuredError::new(ErrorCode::ParseConflict, format!("invalid hunk range: {s}"))
            .with_repair(StructuredError::fix_pattern_repair(
                "header",
                "write the range as `-start[,len]` / `+start[,len]` (e.g. `@@ -1,3 +1,3 @@`)",
            ))
    };
    let body = s.strip_prefix(expect).ok_or_else(|| range_err(s))?;
    if let Some((a, b)) = body.split_once(',') {
        let start: u32 = a.parse().map_err(|_| range_err(s))?;
        let len: u32 = b.parse().map_err(|_| range_err(s))?;
        // Git allows `start=0` only with `len=0`.
        Ok((start, len))
    } else {
        let start: u32 = body.parse().map_err(|_| range_err(s))?;
        Ok((start, 1))
    }
}

/// Parse the trailing `rule=<name>` tag (SPEC 8.2/8.3). Any other `k=v`
/// token is rejected with `RSMPL_PATTERN_CONFLICT`: header tags other
/// than `rule=` do nothing in v0.1 (T107), so accepting them would be a
/// silent no-op against SPEC 2.6. Bare words without `=` are human
/// section text and stay ignored.
fn parse_header_tags(tail: &str) -> std::result::Result<Option<String>, StructuredError> {
    let mut rule = None;
    for tok in tail.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            // Strip trailing commas (e.g. `rule=x,`).
            let v = v.trim_end_matches(',').to_string();
            if k == "rule" {
                rule = Some(v);
            } else {
                // WI-3: machine repair. Only `rule=` is legal in a header
                // tail, so the only close spelling is `rule` itself
                // (same first character, distance ≤ 2, e.g. `rul=x`);
                // anything else repairs to a removal the agent applies
                // verbatim.
                let corrected =
                    if k.starts_with('r') && crate::directives::edit_distance(k, "rule") <= 2 {
                        Some(format!("rule={v}"))
                    } else {
                        None
                    };
                let write_instead = match &corrected {
                    Some(c) => {
                        format!("write `{c}` in the header tail (only `rule=` is supported)")
                    }
                    None => format!("remove `{k}={v}` (only `rule=` is supported in v0.1)"),
                };
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("unknown header tag `{k}={v}` (only `rule=` is supported in v0.1; remove it, see SPEC 8.3)"),
                )
                .with_repair(StructuredError::fix_directive_repair(
                    format!("{k}={v}").as_str(),
                    corrected.as_deref(),
                    write_instead.as_str(),
                )));
            }
        }
    }
    Ok(rule)
}

pub use crate::directives::parse_directives;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordinary_hunk() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,4 @@\n mod old;\n+mod new;\n\n fn main() {}\n";
        let p = parse_patch(text).unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].target_path, "src/lib.rs");
        assert_eq!(p.files[0].hunks.len(), 1);
        assert!(!p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn accepts_differing_diff_git_paths_as_rename() {
        // T377: differing real paths are a file rename section (SPEC
        // 6) -- accepted, target resolves to the new side. Replaces
        // the T29 fail-fast rejection.
        let text = "diff --git a/src/old.rs b/src/new.rs\nrename from src/old.rs\nrename to src/new.rs\n--- a/src/old.rs\n+++ b/src/new.rs\n@@ -1,1 +1,1 @@\n-foo\n+bar\n";
        let p = parse_patch(text).unwrap();
        assert_eq!(p.files.len(), 1);
        assert!(p.files[0].is_rename());
        assert_eq!(p.files[0].target_path, "src/new.rs");
        assert!(!p.files[0].is_new_file);
        assert!(!p.files[0].is_deleted_file);
        // Pure renames (headers only, no hunks) parse too.
        let pure = parse_patch(
            "diff --git a/src/old.rs b/src/new.rs\nsimilarity index 100%\nrename from src/old.rs\nrename to src/new.rs\n",
        )
        .unwrap();
        assert!(pure.files[0].is_rename());
        assert!(pure.files[0].hunks.is_empty());
    }

    #[test]
    fn splits_equal_sides_and_prefers_rename_lines() {
        // T397: a path containing an interior `b/…` segment must split
        // where the stripped sides are EQUAL (bare-git parity), not at
        // the first `b/`-led part (which silently mis-targeted the file).
        let p = parse_patch(
            "diff --git a/a b/c d/e f.txt b/a b/c d/e f.txt\n--- /dev/null\n+++ b/a b/c d/e f.txt\n@@ -0,0 +1,1 @@\n+deep\n",
        )
        .unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].target_path, "a b/c d/e f.txt");
        assert!(!p.files[0].is_rename());
        // Spaced renames: halves alone mis-split, but the `rename
        // from/to` lines (what bare git resolves) win.
        let r = parse_patch("diff --git a/old b/c b/new d\nrename from old b/c\nrename to new d\n")
            .unwrap();
        assert!(r.files[0].is_rename());
        assert_eq!(r.files[0].target_path, "new d");
        assert_eq!(r.files[0].old_path, "a/old b/c");
        // Plain space paths still split at the single `b/` part.
        let s = parse_patch(
            "diff --git a/my file.txt b/my file.txt\n--- a/my file.txt\n+++ b/my file.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n",
        )
        .unwrap();
        assert_eq!(s.files[0].target_path, "my file.txt");
    }

    #[test]
    fn allows_dev_null_create_and_delete_halves() {
        // T29: `/dev/null` halves stay the create/delete spelling.
        let created =
            parse_patch("diff --git /dev/null b/src/new.rs\n@@ -0,0 +1,1 @@\n+foo\n").unwrap();
        assert!(created.files[0].is_new_file);
        let deleted =
            parse_patch("diff --git a/src/old.rs /dev/null\n@@ -1,1 +0,0 @@\n-foo\n").unwrap();
        assert!(deleted.files[0].is_deleted_file);
    }

    #[test]
    fn same_path_dev_null_markers_set_create_delete_flags() {
        // T350: real git keeps same-path `diff --git` halves for
        // deletions/creations and marks the side in `---`/`+++` — the
        // flags must mirror the markers, not just the header halves.
        let deleted = parse_patch("diff --git a/src/old.rs b/src/old.rs\ndeleted file mode 100644\n--- a/src/old.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-foo\n").unwrap();
        assert!(deleted.files[0].is_deleted_file);
        assert_eq!(deleted.files[0].target_path, "src/old.rs");
        let created = parse_patch("diff --git a/src/new.rs b/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,1 @@\n+foo\n").unwrap();
        assert!(created.files[0].is_new_file);
        assert_eq!(created.files[0].target_path, "src/new.rs");
    }

    #[test]
    fn bare_mode_lines_set_create_delete_flags() {
        // T364: real git omits `---`/`+++` for empty files, so the
        // mode lines alone must mark the section (headers-only input
        // lowers to a headers-only section).
        let created =
            parse_patch("diff --git a/empty.txt b/empty.txt\nnew file mode 100644\n").unwrap();
        assert!(created.files[0].is_new_file);
        assert!(!created.files[0].is_deleted_file);
        let deleted =
            parse_patch("diff --git a/gone.txt b/gone.txt\ndeleted file mode 100644\n").unwrap();
        assert!(deleted.files[0].is_deleted_file);
        assert!(!deleted.files[0].is_new_file);
    }
    #[test]
    fn detects_rsmpl_hunk() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=rename_foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn parses_langless_hunk_as_rsmpl() {
        // T69: `# lang=rust` is optional (Rust is the default); any `#`
        // directive block makes the hunk RSmPL.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=x\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }

    #[test]
    fn rejects_non_rust_lang() {
        // T69: omitting the lang line defaults to Rust, but naming a
        // different language is still an error.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=x\n# lang=python\n- foo(E)\n+ bar(E)\n";
        assert!(parse_patch(text).is_err());
    }

    #[test]
    fn rejects_misplaced_directive() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,7 +10,7 @@ rule=x\n# lang=rust\n- foo(E)\n# expr E\n+ bar(E)\n";
        assert!(parse_patch(text).is_err());
    }

    #[test]
    fn parses_directives() {
        let d = parse_directives(&[
            "# lang=rust".to_string(),
            "# allow=many".to_string(),
            "# expr E".to_string(),
            "# symbol Old=crate::old_api".to_string(),
        ])
        .unwrap();
        assert!(d.iter().any(|x| x.is_lang_rust()));
    }

    #[test]
    fn parses_repeat_flag() {
        // T20: bare `# repeat` parses; any value form is rejected.
        let d = parse_directives(&["# repeat".to_string()]).unwrap();
        assert!(d.iter().any(|x| matches!(x, Directive::Repeat)));
        assert!(parse_directives(&["# repeat=true".to_string()]).is_err());
    }

    #[test]
    fn rejects_repeat_with_bind() {
        // T20: `# repeat` folds rewrites; captures move verbatim.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,0 @@ rule=cut\n# repeat\n# item I\n# bind=H\n- I\n";
        let err = parse_patch(text).unwrap_err();
        assert!(err.to_string().contains("bind"), "got: {err}");
    }

    #[test]
    fn rejects_repeat_with_deletion() {
        // T20: deletions already remove nested matches; repeat is vacuous.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=del\n# repeat\n# expr E\n- Box::new(E)\n";
        let err = parse_patch(text).unwrap_err();
        assert!(err.to_string().contains("both `-` and `+`"), "got: {err}");
    }

    #[test]
    fn rejects_repeat_with_insertion() {
        // T20: insertions have no minus side to re-apply.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,0 @@ rule=ins\n# repeat\n+ use crate::x;\n";
        let err = parse_patch(text).unwrap_err();
        assert!(err.to_string().contains("both `-` and `+`"), "got: {err}");
    }

    #[test]
    fn rejects_broken_raw_quote() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,1 +10,1 @@ rule=x\n# lang=rust\n# type T\n- Result<T, r#`type`>\n+ X\n";
        assert!(parse_patch(text).is_err());
    }

    #[test]
    fn parses_zero_length_insertion_anchor() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -5,0 +5,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => {
                assert_eq!(rh.header.old_start, 5);
                assert_eq!(rh.header.old_len, 0);
                assert_eq!(rh.header.insertion_anchor(), Some(5));
            }
            _ => panic!("expected rsmpl"),
        }
    }

    #[test]
    fn crlf_body_content_survives_byte_exact() {
        // T385: a trailing `\r` on hunk body lines is CRLF file
        // content — the old global `\r\n` normalization silently
        // converted worktree files to LF on apply. The engine compares
        // against `split('\n')` bytes, so the `\r` must reach it.
        let text = "diff --git a/win.txt b/win.txt\n--- a/win.txt\n+++ b/win.txt\n@@ -1,2 +1,2 @@\n-line1\r\n-line2\r\n+line1\r\n+CHANGED\r\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => {
                let texts: Vec<&str> = oh.lines.iter().map(|l| l.text.as_str()).collect();
                assert_eq!(texts, vec!["line1\r", "line2\r", "line1\r", "CHANGED\r"]);
            }
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn crlf_authored_metadata_still_parses() {
        // T385: a CRLF-authored patch file (every line ends `\r\n`)
        // keeps parsing — structural lines tolerate one trailing
        // `\r`, only body content keeps it.
        let text = "diff --git a/new.txt b/new.txt\r\nnew file mode 100644\r\n--- /dev/null\r\n+++ b/new.txt\r\n@@ -0,0 +1,1 @@\r\n+hi\r\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].is_new_file);
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => {
                // Body content keeps its `\r` even in a CRLF-authored
                // file; only structural lines strip it.
                assert_eq!(oh.lines[0].text, "hi\r");
            }
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn parses_no_newline_marker() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-foo\n+bar\n\\ No newline at end of file\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => {
                assert!(oh.lines.iter().any(|l| l.prefix == '\\'));
            }
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn parses_version_marker() {
        let text = "# gitrs=0.1\ndiff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-foo\n+bar\n";
        let p = parse_patch(text).unwrap();
        assert!(p.version.is_some());
    }

    #[test]
    fn rejects_typo_directive_key() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -10,1 +10,1 @@ rule=x\n# lang=rust\n# alow=many\n- foo(E)\n+ bar(E)\n";
        let e = parse_patch(text).unwrap_err();
        let msg = format!("{e}");
        assert!(msg.contains("alow") || msg.contains("allow"));
    }

    #[test]
    fn rejects_unknown_and_unsupported_directives() {
        // T113: silent no-op directives fail fast (SPEC 2.6).
        for directive in [
            "# foo",
            "# foo=bar",
            "# edition=2021",
            "# format_scope=node",
            "# frobnicate many things",
        ] {
            let text = format!(
                "diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# lang=rust\n{directive}\n- fn foo(...) {{\n+ fn bar(...) {{\n      ...\n  }}\n"
            );
            let e = parse_patch(&text).unwrap_err();
            let msg = format!("{e}");
            assert!(
                msg.contains("RSMPL_PATTERN_CONFLICT"),
                "directive {directive} got: {msg}"
            );
        }
    }

    #[test]
    fn rejects_unknown_header_tags() {
        // T107: dead header tags fail fast (SPEC 2.6/8.3); only `rule=`
        // is supported. Bare prose tails (no `=`) stay ignored.
        for tail in ["id=x", "depends=y", "foo=bar"] {
            let text = format!(
                "diff --git src/lib.rs\n@@ -1,1 @@ {tail}\n# lang=rust\n- fn foo(...) {{\n+ fn bar(...) {{\n      ...\n  }}\n"
            );
            let e = parse_patch(&text).unwrap_err();
            let msg = format!("{e}");
            assert!(
                msg.contains("RSMPL_PATTERN_CONFLICT") && msg.contains("unknown header tag"),
                "tail {tail} got: {msg}"
            );
        }
        // `rule=` plus bare prose still parses.
        let p = parse_patch(
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=x fix foo\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n",
        )
        .unwrap();
        assert_eq!(p.files[0].hunks[0].header().rule.as_deref(), Some("x"));
    }

    #[test]
    fn empty_line_between_body_lines_is_context() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n a\n\n b\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => {
                assert_eq!(oh.lines.len(), 3);
            }
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn consecutive_blank_lines_are_empty_context() {
        // Tooling: a run of bare empties between body lines is a run of
        // empty contexts (git parity), not a hunk separator. The old
        // immediate-next lookahead split `a\n\n\nb` into a 1-line hunk.
        for (b1, b2) in [("", ""), (" ", ""), ("", " "), (" ", " ")] {
            let text = format!(
                "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,4 @@\n a\n{b1}\n{b2}\n b\n"
            );
            let p = parse_patch(&text).unwrap();
            match &p.files[0].hunks[0] {
                Hunk::Ordinary(oh) => {
                    assert_eq!(oh.lines.len(), 4, "blanks={b1:?}/{b2:?}");
                    assert_eq!(oh.lines[1].prefix, ' ');
                    assert_eq!(oh.lines[1].text, "");
                    assert_eq!(oh.lines[2].prefix, ' ');
                    assert_eq!(oh.lines[2].text, "");
                }
                _ => panic!("expected ordinary"),
            }
        }
    }

    #[test]
    fn empty_patch_rejected() {
        for text in ["", "  \n\n", "# gitrs=0.1\n"] {
            let e = parse_patch(text).unwrap_err();
            let msg = format!("{e}");
            assert!(msg.contains("RSMPL_PARSE_CONFLICT"), "got: {msg}");
            assert!(msg.contains("diff --git"), "got: {msg}");
        }
    }

    #[test]
    fn single_path_header_parses() {
        // T68: `diff --git <path>` ≡ `diff --git a/X b/X`; `b/`-style
        // prefixes are stripped and quoted space paths keep working.
        let p = parse_patch("diff --git src/lib.rs\n@@ -1,3 +1,3 @@\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n")
            .unwrap();
        assert_eq!(p.files[0].target_path, "src/lib.rs");
        assert!(p.files[0].hunks[0].is_rsmpl());
        let p = parse_patch(
            "diff --git \"a/my file.rs\" \"b/my file.rs\"\n@@ -1,1 +1,1 @@\n-foo\n+bar\n",
        )
        .unwrap();
        assert_eq!(p.files[0].target_path, "my file.rs");
    }

    #[test]
    fn c_quoted_octal_paths_decode() {
        // T389: default `core.quotepath=true` C-quotes non-ASCII names
        // (`"caf\303\251.txt"`); the splitter decodes the octal escapes
        // to the real filename instead of dropping the backslashes.
        let p = parse_patch(
            "diff --git \"a/caf\\303\\251.txt\" \"b/caf\\303\\251.txt\"\n@@ -1,1 +1,1 @@\n-a\n+b\n",
        )
        .unwrap();
        assert_eq!(p.files[0].target_path, "caf\u{e9}.txt");
        assert_eq!(
            decode_git_path_token("\"caf\\303\\251.txt\""),
            "caf\u{e9}.txt"
        );
        assert_eq!(decode_git_path_token("plain file.txt"), "plain file.txt");
    }

    #[test]
    fn unquoted_space_paths_rejoin() {
        // T379: real git never quotes spaces, so `diff --git` headers
        // arrive unquoted (`a/my file.txt b/my file.txt`) — the shared
        // splitter rejoins at the `b/`-led part.
        let p = parse_patch("diff --git a/my file.txt b/my file.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n")
            .unwrap();
        assert_eq!(p.files[0].target_path, "my file.txt");
        // `/dev/null` create/delete halves split at the marker.
        let p = parse_patch("diff --git a/my new.txt /dev/null\n@@ -1,1 +0,0 @@\n-a\n").unwrap();
        assert_eq!(p.files[0].target_path, "my new.txt");
        assert!(p.files[0].is_deleted_file);
        let p =
            parse_patch("diff --git a/gone file.txt b/gone file.txt\ndeleted file mode 100644\n")
                .unwrap();
        assert_eq!(p.files[0].target_path, "gone file.txt");
        assert!(p.files[0].is_deleted_file);
        // A genuinely renamed space path is a rename section (T377).
        let p = parse_patch("diff --git a/my file.txt b/other file.txt\n@@ -1,1 +1,1 @@\n-a\n+b\n")
            .unwrap();
        assert!(p.files[0].is_rename());
        assert_eq!(p.files[0].target_path, "other file.txt");
        // Garbage with no valid split still fails fast.
        assert!(parse_diff_git_header("diff --git a/x y z").is_none());
    }

    #[test]
    fn single_path_with_space_parses_as_one_path() {
        // T381: `diff --git my file.rs` is the single-path (SPEC 6)
        // spelling of one file with a space — not a rename of `my`
        // into `file.rs` (wrong-file hazard: the old split targeted
        // `file.rs`). Bare two-word remainders that are not git-shaped
        // (e.g. `diff --git foo bar`) are likewise one path.
        let p = parse_patch(
            "diff --git my file.rs\n--- a/my file.rs\n+++ b/my file.rs\n@@ -1,1 +1,1 @@\n-a\n+b\n",
        )
        .unwrap();
        assert_eq!(p.files[0].target_path, "my file.rs");
        assert!(!p.files[0].is_rename());
        let (old, new) = parse_diff_git_header("diff --git foo bar").unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("foo bar", "foo bar"));
        // Genuinely git-shaped pairs still split in two.
        let (old, new) = parse_diff_git_header("diff --git a/x b/y").unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("a/x", "b/y"));
        // Identical duplicated bare paths collapse to the single path.
        let (old, new) = parse_diff_git_header("diff --git src/lib.rs src/lib.rs").unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("src/lib.rs", "src/lib.rs"));
        let (old, new) = parse_diff_git_header("diff --git foo foo").unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("foo", "foo"));
        let (old, new) = parse_diff_git_header("diff --git my file.rs my file.rs").unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("my file.rs", "my file.rs"));
        // Quoted duplication stays literal (escape hatch for `A A` files).
        let (old, new) = parse_diff_git_header("diff --git \"foo foo\"").unwrap();
        assert_eq!((old.as_str(), new.as_str()), ("foo foo", "foo foo"));
    }

    #[test]
    fn multi_word_single_path_without_git_shape_parses() {
        // T382: the T381 rule generalized — three or more words with no
        // git-shaped part (`a/`/`b/`/`/dev/null`) are one single path.
        // Anything claiming a git shape but malformed still fails fast.
        let (old, new) = parse_diff_git_header("diff --git my weird file.rs").unwrap();
        assert_eq!(old.as_str(), "my weird file.rs");
        assert_eq!(new.as_str(), "my weird file.rs");
        assert!(parse_diff_git_header("diff --git a/x y z").is_none());
        assert!(parse_diff_git_header("diff --git x b/y z").is_none());
    }

    #[test]
    fn collapse_git_header_keeps_only_round_trippable_singles() {
        // T381: `a/P b/P` collapses to `diff --git P` only when `P`
        // re-parses as the same file.
        assert_eq!(
            collapse_git_header("diff --git a/f.rs b/f.rs"),
            "diff --git f.rs"
        );
        assert_eq!(
            collapse_git_header("diff --git a/my file.rs b/my file.rs"),
            "diff --git my file.rs"
        );
        // Renames, creates/deletes, quotes, and already-single or
        // non-header lines pass through.
        assert_eq!(
            collapse_git_header("diff --git a/old.rs b/new.rs"),
            "diff --git a/old.rs b/new.rs"
        );
        assert_eq!(
            collapse_git_header("diff --git /dev/null b/new.rs"),
            "diff --git /dev/null b/new.rs"
        );
        assert_eq!(
            collapse_git_header("diff --git \"a/q f.rs\" \"b/q f.rs\""),
            "diff --git \"a/q f.rs\" \"b/q f.rs\""
        );
        assert_eq!(collapse_git_header("diff --git f.rs"), "diff --git f.rs");
        assert_eq!(collapse_git_header("--- a/f.rs"), "--- a/f.rs");
        // A file literally named `a/x b/y` would re-parse as a rename;
        // a file named `a/b` would strip to `b` — both keep two-path.
        assert_eq!(
            collapse_git_header("diff --git a/a/x b/y b/a/x b/y"),
            "diff --git a/a/x b/y b/a/x b/y"
        );
        assert_eq!(
            collapse_git_header("diff --git a/a/b b/a/b"),
            "diff --git a/a/b b/a/b"
        );
    }

    #[test]
    fn single_range_rsmpl_parses_new_eq_old() {
        // T64: `@@ -old,len @@` elides the ignored new side (RSmPL-only).
        let p = parse_patch("diff --git src/lib.rs\n@@ -1,3 @@\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n")
            .unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => {
                assert!(rh.header.new_elided);
                assert_eq!((rh.header.new_start, rh.header.new_len), (1, 3));
            }
            _ => panic!("expected rsmpl"),
        }
    }

    #[test]
    fn scope_file_parses() {
        // T61: `# scope=file` searches the whole file with a dummy range.
        let p = parse_patch("diff --git src/lib.rs\n@@ -1,1 @@ rule=x\n# scope=file\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n")
            .unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Rsmpl(rh) => assert_eq!(rh.scope(), Scope::File),
            _ => panic!("expected rsmpl"),
        }
    }

    #[test]
    fn leading_blank_context_is_hunk_body() {
        // T212: a leading blank context line belongs to the hunk (git
        // parses hunk bodies by count); it must not end the hunk early
        // with zero body lines.
        for blank in [" ", ""] {
            let text = format!("diff --git f.md\n@@ -2,3 +2,2 @@\n{blank}\n-l3\n l4\n");
            let p = parse_patch(&text).unwrap();
            assert_eq!(p.files[0].hunks.len(), 1);
            match &p.files[0].hunks[0] {
                Hunk::Ordinary(oh) => {
                    assert_eq!(oh.lines.len(), 3);
                    assert_eq!(oh.lines[0].prefix, ' ');
                    assert_eq!(oh.lines[0].text, "");
                    assert_eq!(oh.lines[1].prefix, '-');
                    assert_eq!(oh.lines[1].text, "l3");
                }
                _ => panic!("expected ordinary, blank={blank:?}"),
            }
        }
    }

    #[test]
    fn ws_only_context_preserves_content() {
        // T388: a whitespace-only context line (`" " + content`) keeps
        // its content byte-exact — `"    "` is a 3-space file line,
        // not an empty one (the T212 branch used to record `""`,
        // failing `applyrs --check` with `context mismatch` while
        // `git apply` succeeded).
        let text = "diff --git f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1,5 +1,5 @@\n line1\n    \n-line3\n+line3b\n line4\n line5\n";
        let p = parse_patch(text).unwrap();
        assert_eq!(p.files[0].hunks.len(), 1);
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => {
                assert_eq!(oh.lines.len(), 6);
                assert_eq!(oh.lines[1].prefix, ' ');
                assert_eq!(oh.lines[1].text, "   ");
            }
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn single_range_ordinary_rejected() {
        // T64: ordinary hunks keep full Git two-range semantics.
        let e = parse_patch("diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,2 @@\n-foo\n+bar\n")
            .unwrap_err();
        let msg = format!("{e}");
        assert!(
            msg.contains("RSmpl") || msg.contains("RSMPL") || msg.contains("both"),
            "got: {msg}"
        );
    }

    #[test]
    fn parses_table_gap_directives() {
        // T246: SPEC 9.3 names every implemented directive family.
        let d = parse_directives(&[
            "# returns=none".to_string(),
            "# root=expr".to_string(),
            "# fallback=syntax".to_string(),
            "# symbol Old=crate::old_api".to_string(),
            "# metavars=sigiled".to_string(),
        ])
        .unwrap();
        assert_eq!(d.len(), 5);
    }

    #[test]
    fn indented_hunk_header_rejected() {
        // T365: a hunk header with leading whitespace is a valid
        // unified-diff context line; without this gate it lands in
        // metadata and the section lowers to an empty no-op (exit 0).
        // Headers must start at column 0.
        for indented in [
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n @@ -1,1 +1,1 @@\n-foo\n+bar\n",
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n\t@@ -1,1 +1,1 @@\n-foo\n+bar\n",
        ] {
            let err = parse_patch(indented).unwrap_err();
            let ParseError::Structured(s) = &err else {
                panic!("expected structured PARSE_CONFLICT, got: {err}");
            };
            assert_eq!(s.code, ErrorCode::ParseConflict);
            assert!(
                s.message.contains("leading whitespace"),
                "got: {}",
                s.message
            );
            assert_eq!(s.file.as_deref(), Some("src/lib.rs"));
            assert_eq!(s.range.as_deref(), Some("line 4"));
        }
    }

    #[test]
    fn indented_header_inside_body_stays_context() {
        // Guard against overreach: an indented `@@` line inside a hunk
        // body is a context line (git semantics), not an error.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n @@ -9,9 +9,9 @@\n real\n";
        let p = parse_patch(text).unwrap();
        match &p.files[0].hunks[0] {
            Hunk::Ordinary(oh) => {
                assert_eq!(oh.lines.len(), 2);
                assert_eq!(oh.lines[0].text, "@@ -9,9 +9,9 @@");
            }
            _ => panic!("expected ordinary"),
        }
    }

    #[test]
    fn ordinary_hunk_count_mismatch_rejected() {
        // T367: ordinary header counts must agree with the body
        // (old-side = ` ` + `-`, new-side = ` ` + `+`; `\ No newline`
        // markers are neither side).
        let old_short = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,3 @@\n a\n-b\n+c\n a\n";
        let err = parse_patch(old_short).unwrap_err();
        let ParseError::Structured(s) = &err else {
            panic!("expected structured PARSE_CONFLICT, got: {err}");
        };
        assert_eq!(s.code, ErrorCode::ParseConflict);
        assert!(
            s.message.contains(
                "ordinary hunk body has 3 old-side and 3 new-side lines but header claims -1,5 +1,3"
            ),
            "got: {}",
            s.message
        );
        assert_eq!(s.file.as_deref(), Some("src/lib.rs"));
        assert_eq!(s.range.as_deref(), Some("line 4"));
        assert!(s
            .suggestion
            .as_deref()
            .unwrap()
            .contains("fix the header counts"));
        // WI-10 consideration: the mismatch carries machine repair —
        // exact counts plus a drop-in corrected header line.
        let repair = s.repair.clone().expect("count mismatch must carry repair");
        assert_eq!(repair["action"], serde_json::json!("fix_header"));
        assert_eq!(repair["expected_old"], serde_json::json!(3));
        assert_eq!(repair["expected_new"], serde_json::json!(3));
        assert_eq!(repair["claimed_old"], serde_json::json!(5));
        assert_eq!(repair["claimed_new"], serde_json::json!(3));
        assert_eq!(
            repair["expected_header"],
            serde_json::json!("@@ -1,3 +1,3 @@")
        );
        // New-side-only mismatch.
        let new_short = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,9 @@\n a\n-b\n+c\n a\n";
        let err = parse_patch(new_short).unwrap_err();
        let ParseError::Structured(s) = &err else {
            panic!("expected structured PARSE_CONFLICT, got: {err}");
        };
        assert_eq!(s.code, ErrorCode::ParseConflict);
        assert!(
            s.message.contains("header claims -1,3 +1,9"),
            "got: {}",
            s.message
        );
        let repair = s.repair.clone().expect("count mismatch must carry repair");
        assert_eq!(
            repair["expected_header"],
            serde_json::json!("@@ -1,3 +1,3 @@")
        );
        // `\ No newline` markers are neither side: exact counts pass.
        let marker_ok = "diff --git a/link b/link\n--- /dev/null\n+++ b/link\n@@ -0,0 +1,1 @@\n+real.txt\n\\ No newline at end of file\n";
        parse_patch(marker_ok).unwrap();
    }

    #[test]
    fn rsmpl_hunk_counts_not_validated() {
        // SPEC 10.4: RSmPL counts are search ranges, bodies are patterns;
        // no correspondence is required.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,99 @@ rule=x\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
        let p = parse_patch(text).unwrap();
        assert!(p.files[0].hunks[0].is_rsmpl());
    }
}
