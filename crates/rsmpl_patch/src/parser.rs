//! Line-oriented parser for RSmPL-Git patch files.
//!
//! Grammar (SPEC section 30):
//! `patch ::= file_section*`, file sections start with `diff --git`
//! (one path suffices, SPEC 6), hunks start with `@@ -a,b (+c,d)? @@ tail`
//! (new-side elision is RSmPL-only, SPEC 8). A hunk is RSmPL iff the
//! first line after the header is an unprefixed `#` directive (SPEC
//! 7.1); `# lang=rust` is accepted but optional (Rust is the default).

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
                }
                sec.metadata.push(line.to_string());
                idx += 1;
                continue;
            }
            return Err(StructuredError::new(
                ErrorCode::ParseConflict,
                format!("file metadata outside file section: {line}"),
            )
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
        .into());
    }

    validate_patch(&patch)?;

    Ok(patch)
}

fn is_version_marker(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("# gitrs=") || t.starts_with("#gitrs=")
}

fn parse_diff_git_header(line: &str) -> Option<(String, String)> {
    // T385: tolerate a CRLF-authored header line — the trailing `\r`
    // would otherwise embed into a single-path fallback (whitespace
    // splitting already tolerates it for shaped headers).
    let line = line.strip_suffix('\r').unwrap_or(line);
    let rest = line.strip_prefix("diff --git ")?;
    // A single path (`diff --git <path>`, SPEC 6) is equivalent to the
    // duplicated two-path form; the two-path form stays for renames/compat.
    let parts = split_git_paths(rest)?;
    if parts.len() == 1 {
        return Some((parts[0].clone(), parts[0].clone()));
    }
    if parts.len() != 2 {
        return None;
    }
    Some((parts[0].clone(), parts[1].clone()))
}

/// Split the rest of a `diff --git <old> <new>` header into paths,
/// honoring quotes. Real git never quotes spaces, so unquoted paths may
/// contain them (`diff --git a/my file.txt b/my file.txt`, T379): when
/// whitespace-splitting yields more than two parts, rejoin at the first
/// `b/`-led part (the joined old side must start with `a/`), else split
/// at a `/dev/null` create/delete half, else fail (error as before).
pub fn split_git_paths(rest: &str) -> Option<Vec<String>> {
    let parts = split_paths(rest)?;
    if parts.len() == 1 {
        return Some(parts);
    }
    if parts.len() == 2 {
        // T381: a two-part split is only a two-path header when it wears
        // git's `a/`/`b/` (or `/dev/null`) uniform. Otherwise the whole
        // remainder is one single-path (SPEC 6) file with a space —
        // previously misread as a rename of the first word into the
        // second (wrong-file hazard: `diff --git my file.rs` targeted
        // `file.rs`). Bare two-word remainders that are not git-shaped
        // (e.g. `diff --git foo bar`) are likewise one path — real git
        // always emits the `a/`/`b/` uniform for pairs.
        let old_git = parts[0].starts_with("a/") || is_dev_null(&parts[0]);
        let new_git = parts[1].starts_with("b/") || is_dev_null(&parts[1]);
        if old_git && new_git {
            return Some(parts);
        }
        return Some(vec![rest.to_string()]);
    }
    if let Some(i) = parts.iter().position(|p| p.starts_with("b/")) {
        if i > 0 && parts[..i].join(" ").starts_with("a/") {
            return Some(vec![parts[..i].join(" "), parts[i..].join(" ")]);
        }
        return None;
    }
    if parts.len() > 2 && is_dev_null(&parts[parts.len() - 1]) {
        return Some(vec![
            parts[..parts.len() - 1].join(" "),
            parts[parts.len() - 1].clone(),
        ]);
    }
    if is_dev_null(&parts[0]) {
        return Some(vec![parts[0].clone(), parts[1..].join(" ")]);
    }
    // T382: the T381 rule generalized — a multi-part remainder with no
    // git-shaped part at all (no `a/`/`b/` uniform, no `/dev/null` half)
    // is one single path with spaces (SPEC 6). Anything claiming a git
    // shape but malformed (e.g. `diff --git a/x y z`) still fails fast.
    if parts
        .iter()
        .any(|p| p.starts_with("a/") || p.starts_with("b/") || is_dev_null(p))
    {
        return None;
    }
    Some(vec![rest.to_string()])
}

/// Collapse a `diff --git a/P b/P` header line to the single-path
/// `diff --git P` spelling (SPEC 6 equivalence, T381) when `P` re-parses
/// as the same target; renames, `/dev/null` halves, quoted headers, and
/// paths whose single form would re-parse differently keep the two-path
/// spelling (e.g. a file literally named `a/x b/y`, or `a/b` where the
/// single form's `a/` prefix would strip to a different file).
/// Non-header lines pass through unchanged.
pub fn collapse_git_header(line: &str) -> String {
    let Some(rest) = line.strip_prefix("diff --git ") else {
        return line.to_string();
    };
    if rest.contains('"') || rest.contains('\'') {
        return line.to_string();
    }
    let parts = match split_git_paths(rest) {
        Some(p) if p.len() == 2 => p,
        _ => return line.to_string(),
    };
    if is_dev_null(&parts[0]) || is_dev_null(&parts[1]) {
        return line.to_string();
    }
    let old = strip_ab_prefix(&parts[0]);
    if old != strip_ab_prefix(&parts[1]) || is_dev_null(&old) {
        return line.to_string();
    }
    // Round-trip guard: the single form must parse back to the same
    // file (same split, non-rename, same target).
    let single = format!("diff --git {old}");
    match parse_diff_git_header(&single) {
        Some((o, n)) if o == n && target_path(&n, &o) == old => single,
        _ => line.to_string(),
    }
}

fn is_dev_null(p: &str) -> bool {
    p == "/dev/null" || p == "dev/null"
}

fn split_paths(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut q = '"';
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == q {
                in_q = false;
            } else if c == '\\' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' || c == '\'' {
            in_q = true;
            q = c;
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn strip_ab_prefix(p: &str) -> String {
    if let Some(s) = p.strip_prefix("a/") {
        return s.to_string();
    }
    if let Some(s) = p.strip_prefix("b/") {
        return s.to_string();
    }
    if let Some(s) = p.strip_prefix('"') {
        let _ = s;
    }
    p.to_string()
}

fn target_path(new_path: &str, old_path: &str) -> String {
    if new_path == "/dev/null" || new_path == "dev/null" {
        return strip_ab_prefix(old_path);
    }
    strip_ab_prefix(new_path)
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
    let is_rsmpl_candidate = !raw_directives.is_empty();
    // Validate backtick quoting early for RSmPL candidates (SPEC 24.1.5).
    if is_rsmpl_candidate {
        for d in &raw_directives {
            validate_no_broken_raw_quote_directive(d)?;
        }
    }

    if is_rsmpl_candidate {
        // Per SPEC 7.1 any unprefixed `#` directive block makes the hunk
        // RSmPL (Rust is the default language); `# lang=rust` may be
        // included for explicitness, `# lang=<other>` is rejected by
        // `parse_directives`. Unknown/typo'd keys are rejected there too,
        // so omitting the lang line never silently accepts a typo.
        let directives = parse_directives(&raw_directives)?;
        // Parse body lines until next hunk/file header or EOF/blank separator.
        let (body, next) = parse_body_lines(lines, idx, true)?;
        if body.is_empty() {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "RSmPL hunk has no pattern body lines",
            )
            .with_rule(header.rule.clone().unwrap_or_default())
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
            return Err(StructuredError::new(
                ErrorCode::ParseConflict,
                format!(
                    "ordinary hunk requires both -old,len +new,len ranges: {}",
                    lines[at]
                ),
            )
            .with_suggestion(
                "write both ranges (e.g. `@@ -1,2 +1,2 @@`) or add a `#` directive line to make it an RSmPL hunk",
            )
            .into());
        }
        let (body, next) = parse_body_lines(lines, idx, false)?;
        // T367: ordinary hunk headers must agree with the body: old-side
        // lines (` ` + `-`) equal `-old,len`, new-side lines (` ` + `+`)
        // equal `+new,len` (`\ No newline` markers are neither side).
        // The engine applies from the body alone, so a miscounted header
        // used to surface downstream as a misleading `GIT_HUNK_CONFLICT`.
        // RSmPL hunks are exempt (counts are search ranges there, bodies
        // are patterns, SPEC 10.4).
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
            .with_suggestion("fix the header counts to match the body lines (old-side = ` ` + `-` lines, new-side = ` ` + `+` lines)");
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
        if l.is_empty() {
            // An empty line is ambiguous: git represents an empty context
            // line as "" (zero-length) or " " — while a truly blank line
            // separates hunks. Look ahead: if the next line is a body line,
            // treat "" as an empty context line; otherwise end the hunk.
            // The lookahead alone decides, even for the first body line:
            // a hunk header is always followed by body lines, so a leading
            // blank followed by body lines belongs to the hunk (T212).
            let next_is_body = idx + 1 < lines.len() && is_body_start(lines[idx + 1]);
            if next_is_body {
                out.push(BodyLine {
                    prefix: ' ',
                    text: String::new(),
                });
                idx += 1;
                continue;
            }
            break;
        }
        if l.trim().is_empty() {
            // Whitespace-only line: same lookahead rule (T212).
            let next_is_body = idx + 1 < lines.len() && is_body_start(lines[idx + 1]);
            if next_is_body {
                out.push(BodyLine {
                    prefix: ' ',
                    text: String::new(),
                });
                idx += 1;
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
    let err = |msg: String| StructuredError::new(ErrorCode::ParseConflict, msg);
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
    let body = s.strip_prefix(expect).ok_or_else(|| {
        StructuredError::new(ErrorCode::ParseConflict, format!("invalid hunk range: {s}"))
    })?;
    if let Some((a, b)) = body.split_once(',') {
        let start: u32 = a.parse().map_err(|_| {
            StructuredError::new(ErrorCode::ParseConflict, format!("invalid hunk range: {s}"))
        })?;
        let len: u32 = b.parse().map_err(|_| {
            StructuredError::new(ErrorCode::ParseConflict, format!("invalid hunk range: {s}"))
        })?;
        // Git allows `start=0` only with `len=0`.
        Ok((start, len))
    } else {
        let start: u32 = body.parse().map_err(|_| {
            StructuredError::new(ErrorCode::ParseConflict, format!("invalid hunk range: {s}"))
        })?;
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
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("unknown header tag `{k}={v}` (only `rule=` is supported in v0.1; remove it, see SPEC 8.3)"),
                ));
            }
        }
    }
    Ok(rule)
}

/// Parse `# key=value` / `# flag` / `# kind Name` directives (SPEC 9).
pub fn parse_directives(raw: &[String]) -> Result<Vec<Directive>> {
    let mut out = Vec::new();
    for line in raw {
        out.push(parse_directive(line)?);
    }
    Ok(out)
}

fn parse_directive(line: &str) -> Result<Directive> {
    let body = line
        .strip_prefix('#')
        .ok_or_else(|| {
            StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid directive: {line}"),
            )
        })?
        .trim();
    if body.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("empty directive: {line}"),
        )
        .into());
    }
    // `key=value` form first.
    if let Some((k, v)) = split_kv(body) {
        return parse_kv_directive(k.trim(), v.trim(), line);
    }
    // Multi-token: could be `# flag`, `# kind Name`, `# kind Name = constraint`,
    // `# symbol Name=path`, `# expr E1, E2` comma form.
    let tokens: Vec<&str> = body.split_whitespace().collect();
    match tokens.as_slice() {
        [] => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("empty directive: {line}"),
        )
        .into()),
        ["optional"] => Ok(Directive::Optional(true)),
        ["repeat"] => Ok(Directive::Repeat),
        ["lang=rust"] => Ok(Directive::LangRust),
        [single] => {
            // Bare flag or `kind` with missing name.
            if MetavarKind::parse(single).is_some() {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("metavariable directive missing name: {line}"),
                )
                .into());
            }
            // T113: unknown bare directives fail fast (SPEC 2.6) — no
            // forward-compat silent ignore.
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("unknown directive `#{single}`: {line}"),
            )
            .with_suggestion("check SPEC 9.3 for the supported directive list".to_string())
            .into())
        }
        [kind, rest @ ..] => {
            if let Some(mk) = MetavarKind::parse(kind) {
                // `symbol` has special `Name=path` form handled in kv? No:
                // `# symbol Old=crate::foo` tokenizes as kind + `Old=crate::foo`.
                if mk == MetavarKind::Symbol {
                    if rest.len() != 1 {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("invalid symbol directive (want `# symbol Name=path`): {line}"),
                        )
                        .into());
                    }
                    let (n, p) = rest[0].split_once('=').ok_or_else(|| {
                        StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("invalid symbol directive (want `# symbol Name=path`): {line}"),
                        )
                    })?;
                    validate_metavar_name(n)?;
                    if p.is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("symbol path must not be empty: {line}"),
                        )
                        .into());
                    }
                    return Ok(Directive::Symbol {
                        name: n.to_string(),
                        path: p.to_string(),
                    });
                }
                // General metavariable: `# kind N1, N2` or `# kind N = constraint`.
                let joined = rest.join(" ");
                // Split off `= constraint` if present.
                let names_part = joined.split_once('=').map(|(a, _)| a).unwrap_or(&joined);
                let mut names = Vec::new();
                for n in names_part.split(',') {
                    let n = n.trim();
                    if n.is_empty() {
                        continue;
                    }
                    // Names must be single identifiers (may be backticked? no).
                    if n.contains(char::is_whitespace) {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("invalid metavariable name `{n}` in: {line}"),
                        )
                        .into());
                    }
                    validate_metavar_name(n)?;
                    names.push(n.to_string());
                }
                if names.is_empty() {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        format!("metavariable directive missing name: {line}"),
                    )
                    .into());
                }
                return Ok(Directive::MetaVar { kind: mk, names });
            }
            // T113: unknown multi-token directives fail fast (SPEC 2.6).
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("unknown directive `#{}`: {line}", tokens[0]),
            )
            .with_suggestion("check SPEC 9.3 for the supported directive list".to_string())
            .into())
        }
    }
}

/// Split `key=value` where key is a single identifier-ish token with no spaces.
fn split_kv(body: &str) -> Option<(&str, &str)> {
    let (k, v) = body.split_once('=')?;
    // Key must not contain whitespace (else it's `# kind Name = constraint`).
    if k.contains(char::is_whitespace) {
        // Exception: `# symbol Name=path` contains no space before `=`... it
        // has a space after `symbol`, so split_once gives k=`symbol Old`.
        // Fall through to multi-token handling by returning None.
        return None;
    }
    // Key must be non-empty identifier-ish.
    if k.trim().is_empty() {
        return None;
    }
    Some((k, v))
}

/// Known `key=value` directive keys (SPEC 9.3). Unknown keys are rejected
/// with `RSMPL_PATTERN_CONFLICT` (T113 fail-fast, SPEC 2.6); only
/// edit-distance-1 near-misses get the did-you-mean suggestion, every other
/// unknown key gets the SPEC 9.3 pointer.
const KNOWN_KV_KEYS: &[&str] = &[
    "lang", "allow", "optional", "scope", "bind", "use", "attrs", "mode", "macro", "format",
    "returns", "root", "fallback", "metavars", "symbol",
];

fn edit_distance_one(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() == b.len() {
        return a.iter().zip(b.iter()).filter(|(x, y)| x != y).count() == 1;
    }
    if a.len() + 1 == b.len() || b.len() + 1 == a.len() {
        let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
        let mut i = 0;
        let mut j = 0;
        let mut skipped = false;
        while i < short.len() && j < long.len() {
            if short[i] == long[j] {
                i += 1;
                j += 1;
            } else if !skipped {
                skipped = true;
                j += 1;
            } else {
                return false;
            }
        }
        return true;
    }
    false
}

#[allow(clippy::too_many_lines)]
fn parse_kv_directive(k: &str, v: &str, line: &str) -> Result<Directive> {
    match k {
        "lang" => {
            if v == "rust" {
                Ok(Directive::LangRust)
            } else {
                Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("unsupported language `{v}` (only `rust` supported): {line}"),
                )
                .into())
            }
        }
        "allow" => match v {
            "one" => Ok(Directive::Allow(Allow::One)),
            "many" => Ok(Directive::Allow(Allow::Many)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid allow value `{v}` (want one|many): {line}"),
            )
            .into()),
        },
        "optional" => match v {
            "true" => Ok(Directive::Optional(true)),
            "false" => Ok(Directive::Optional(false)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid optional value `{v}`: {line}"),
            )
            .into()),
        },
        "repeat" => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid repeat value `{v}` (write bare `# repeat`): {line}"),
        )
        .with_suggestion("write bare `# repeat` with no value".to_string())
        .into()),
        "scope" => match v {
            "contained" => Ok(Directive::Scope(Scope::Contained)),
            "anchor" => Ok(Directive::Scope(Scope::Anchor)),
            "overlap" => Ok(Directive::Scope(Scope::Overlap)),
            "exact" => Ok(Directive::Scope(Scope::Exact)),
            "file" => Ok(Directive::Scope(Scope::File)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid scope `{v}`: {line}"),
            )
            .into()),
        },
        "bind" => {
            validate_metavar_name(v)?;
            Ok(Directive::Bind(v.to_string()))
        }
        "use" => {
            validate_metavar_name(v)?;
            Ok(Directive::Use(v.to_string()))
        }
        "attrs" => match v {
            "preserve" => Ok(Directive::Attrs(AttrsPolicy::Preserve)),
            "absent" => Ok(Directive::Attrs(AttrsPolicy::Absent)),
            "explicit" => Ok(Directive::Attrs(AttrsPolicy::Explicit)),
            "strip" => Ok(Directive::Attrs(AttrsPolicy::Strip)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid attrs value `{v}`: {line}"),
            )
            .into()),
        },
        "mode" => match v {
            "ast" => Ok(Directive::Mode(ParseMode::Ast)),
            "tokens" => Ok(Directive::Mode(ParseMode::Tokens)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid mode `{v}`: {line}"),
            )
            .into()),
        },
        "macro" => match v {
            "invocation" => Ok(Directive::MacroMode(MacroMode::Invocation)),
            "token_tree" => Ok(Directive::MacroMode(MacroMode::TokenTree)),
            "expanded" => Ok(Directive::MacroMode(MacroMode::Expanded)),
            _ => {
                // Could be `# macro M` declaring a macro-name metavariable —
                // but that form has no `=`. With `=`, an unknown macro value.
                Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!("invalid macro mode `{v}`: {line}"),
                )
                .into())
            }
        },
        "format" => match v {
            "preserve" => Ok(Directive::Format(FormatMode::Preserve)),
            "rustfmt" => Ok(Directive::Format(FormatMode::Rustfmt)),
            "canonical" => Ok(Directive::Format(FormatMode::Canonical)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid format `{v}`: {line}"),
            )
            .into()),
        },
        "format_scope" => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unsupported directive `# format_scope={v}` (only `# format=preserve` is supported in v0.1): {line}"),
        )
        .with_suggestion(
            "remove the directive — only `# format=preserve` is supported".to_string(),
        )
        .into()),
        "returns" => match v {
            "none" => Ok(Directive::ReturnsNone(true)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid returns `{v}` (only `none` supported): {line}"),
            )
            .into()),
        },
        "root" => Ok(Directive::Root(v.to_string())),
        "fallback" => match v {
            "syntax" => Ok(Directive::FallbackSyntax(true)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid fallback `{v}`: {line}"),
            )
            .into()),
        },
        "metavars" => match v {
            "sigiled" => Ok(Directive::SigiledMetavars(true)),
            "bare" => Ok(Directive::SigiledMetavars(false)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid metavars `{v}`: {line}"),
            )
            .into()),
        },
        "edition" => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unsupported directive `# edition={v}` (edition is the toolchain default in v0.1): {line}"),
        )
        .with_suggestion(
            "remove the directive — edition is the toolchain default in v0.1".to_string(),
        )
        .into()),
        "symbol" => {
            // `# symbol=...` without name — invalid; canonical is `# symbol N=P`.
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid symbol directive (want `# symbol Name=path`): {line}"),
            )
            .into())
        }
        _ => {
            for known in KNOWN_KV_KEYS {
                if edit_distance_one(k, known) {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        format!(
                            "unknown directive `#{k}={v}` (did you mean `#{known}=...`?): {line}"
                        ),
                    )
                    .with_suggestion(format!("use `#{known}=...`"))
                    .into());
                }
            }
            // T113: unknown `k=v` directives fail fast (SPEC 2.6).
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("unknown directive `#{k}={v}`: {line}"),
            )
            .with_suggestion("check SPEC 9.3 for the supported directive list".to_string())
            .into())
        }
    }
}

fn validate_metavar_name(n: &str) -> Result<()> {
    if n.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            "empty metavariable/capture name".to_string(),
        )
        .into());
    }
    let mut chars = n.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid name `{n}` (must start with letter or _)"),
        )
        .into());
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid name `{n}` (only alphanumeric + _ allowed)"),
        )
        .into());
    }
    Ok(())
}

/// Early validation: reject `` r#`type` `` (split raw-ident quoting).
/// Valid: `` `r#type` `` or bare `r#type`. Invalid: `r#` followed by backtick.
fn validate_no_broken_raw_quote_pattern(text: &str) -> Result<()> {
    if text.contains("r#`") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!(
                "invalid raw-identifier quoting `r#` + backtick in pattern `{text}` (use `` `r#...` ``)"
            ),
        )
        .into());
    }
    // Unbalanced backticks.
    let count = text.chars().filter(|&c| c == '`').count();
    if count % 2 != 0 {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unbalanced backtick quoting in pattern `{text}`"),
        )
        .into());
    }
    Ok(())
}

fn validate_no_broken_raw_quote_directive(line: &str) -> Result<()> {
    if line.contains("r#`") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid raw-identifier quoting in directive `{line}`"),
        )
        .into());
    }
    Ok(())
}

/// Cross-hunk validation that needs the whole patch (SPEC 20.4, 24.x).
fn validate_patch(patch: &Patch) -> Result<()> {
    use std::collections::{HashMap, HashSet};
    // Collect binds; check duplicates; check bind+allow=many (SPEC 20.4).
    let mut bind_to_hunk: HashMap<String, (String, String)> = HashMap::new();
    let mut use_refs: Vec<(String, String, String)> = Vec::new(); // (capture, file, rule)
    for f in &patch.files {
        for h in &f.hunks {
            if let Hunk::Rsmpl(rh) = h {
                for b in rh.binds() {
                    if bind_to_hunk.contains_key(b) {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            format!("duplicate `# bind={b}` (scalar captures must be unique)"),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .into());
                    }
                    if rh.allow() == Allow::Many {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            "`# bind=<Name>` cannot be combined with `# allow=many` in v0.1 (scalar captures only)",
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .into());
                    }
                    bind_to_hunk.insert(b.to_string(), (f.target_path.clone(), rh.rule_name()));
                }
                for u in rh.uses() {
                    use_refs.push((u.to_string(), f.target_path.clone(), rh.rule_name()));
                }
                // Hunk with neither - nor + nor context? Already rejected empty.
                // use without bind resolved at engine stage (forward refs allowed),
                // but unknown refs are reported there with full-patch view.
                let _ = &use_refs;
            }
        }
    }
    // Cycle check placeholder: single bind->use edges cannot cycle by
    // construction in v0.1 (uses don't define new values), so any cycle
    // would require bind depending on use of itself. Detect self-dependency:
    // hunk with both bind=X and use=X is suspicious but allowed only if
    // independent. We reject direct self-use to keep v0.1 simple.
    for f in &patch.files {
        for h in &f.hunks {
            if let Hunk::Rsmpl(rh) = h {
                let binds: HashSet<&str> = rh.binds().into_iter().collect();
                for u in rh.uses() {
                    if binds.contains(u) {
                        return Err(StructuredError::new(
                            ErrorCode::CaptureConflict,
                            format!("hunk both binds and uses `{u}` (self-cycle)"),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .into());
                    }
                }
                // T20: `# repeat` folds nested matches into a rewrite hunk.
                // It is meaningless on captures (verbatim moves) and on
                // insertions/deletions (which already cover nested matches).
                if rh.repeats() {
                    if !rh.binds().is_empty() || !rh.uses().is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            "`# repeat` cannot be combined with `# bind=`/`# use=` (captures move verbatim)".to_string(),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .with_suggestion("remove `# repeat` from the cut/paste hunk".to_string())
                        .into());
                    }
                    if rh.minus_text().trim().is_empty() || rh.plus_text().trim().is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            "`# repeat` requires both `-` and `+` patterns (insertions/deletions already cover nested matches)".to_string(),
                        )
                        .with_rule(rh.rule_name())
                        .with_file(&f.target_path)
                        .with_suggestion("remove `# repeat`, or write a rewrite hunk".to_string())
                        .into());
                    }
                }
            }
        }
    }
    Ok(())
}

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
