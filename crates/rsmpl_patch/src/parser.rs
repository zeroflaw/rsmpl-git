//! Line-oriented parser for RSmPL-Git patch files.
//!
//! Grammar (SPEC section 30):
//! `patch ::= file_section*`, file sections start with `diff --git`,
//! hunks start with `@@ -a,b +c,d @@ tail`. A hunk is RSmPL iff the
//! first line after the header is an unprefixed `#` directive (SPEC
//! 7.1); `# lang=rust` is accepted but optional (Rust is the default).

use crate::errors::{ErrorCode, StructuredError};
use crate::model::*;
use std::collections::HashMap;

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
    // Normalize CRLF to LF; keep trailing-newline info per line separately.
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();

    // Handle optional top-level version marker `# rsmpl-git=0.1` (SPEC 33.1).
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
            let (hunk, next) = parse_hunk(&lines, idx)?;
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
            if let Some(sec) = current.as_mut() {
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
             (see SPEC 6 minimal recipe); check for an empty --patch/--file/stdin",
        )
        .into());
    }

    validate_patch(&patch)?;

    Ok(patch)
}

fn is_version_marker(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("# rsmpl-git=") || t.starts_with("#rsmpl-git=")
}

fn parse_diff_git_header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    // Paths may be quoted; split into two (old, new) honoring quotes.
    let parts = split_paths(rest)?;
    if parts.len() != 2 {
        return None;
    }
    Some((parts[0].clone(), parts[1].clone()))
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

fn parse_hunk(lines: &[&str], at: usize) -> Result<(Hunk, usize)> {
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
        let (body, next) = parse_body_lines(lines, idx, false)?;
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
            let next_is_body = idx + 1 < lines.len() && is_body_start(lines[idx + 1]);
            let prev_was_body = !out.is_empty();
            if prev_was_body && next_is_body {
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
            // Whitespace-only line: same lookahead rule.
            let next_is_body = idx + 1 < lines.len() && is_body_start(lines[idx + 1]);
            if !out.is_empty() && next_is_body {
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
    // `@@ -old[,len] +new[,len] @@ tail`
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
    let new_s = parts
        .next()
        .ok_or_else(|| err(format!("invalid hunk header ranges: {line}")))?;
    let (old_start, old_len) = parse_range(old_s, '-')?;
    let (new_start, new_len) = parse_range(new_s, '+')?;
    let (rule, id, depends, extra) = parse_header_tags(&tail);
    Ok(HunkHeader {
        old_start,
        old_len,
        new_start,
        new_len,
        rule,
        id,
        depends,
        extra_tags: extra,
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

fn parse_header_tags(
    tail: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    HashMap<String, String>,
) {
    let mut rule = None;
    let mut id = None;
    let mut depends = None;
    let mut extra = HashMap::new();
    for tok in tail.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            // Strip trailing commas (e.g. `rule=x,`).
            let v = v.trim_end_matches(',').to_string();
            match k {
                "rule" => rule = Some(v),
                "id" => id = Some(v),
                "depends" => depends = Some(v),
                _ => {
                    extra.insert(k.to_string(), v);
                }
            }
        }
    }
    (rule, id, depends, extra)
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
            Ok(Directive::Unknown {
                key: single.to_string(),
                value: None,
            })
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
            // Unknown multi-token directive: keep as Unknown for forward compat.
            Ok(Directive::Unknown {
                key: tokens[0].to_string(),
                value: Some(tokens[1..].join(" ")),
            })
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

/// Known `key=value` directive keys (SPEC 9.3). Unknown keys are kept as
/// `Directive::Unknown` for forward compatibility, except near-miss typos
/// (edit distance 1 to a known key) which are rejected to catch `# alow=...`.
const KNOWN_KV_KEYS: &[&str] = &[
    "lang",
    "allow",
    "optional",
    "scope",
    "bind",
    "use",
    "attrs",
    "mode",
    "macro",
    "format",
    "format_scope",
    "returns",
    "root",
    "fallback",
    "metavars",
    "edition",
    "symbol",
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
        "scope" => match v {
            "contained" => Ok(Directive::Scope(Scope::Contained)),
            "anchor" => Ok(Directive::Scope(Scope::Anchor)),
            "overlap" => Ok(Directive::Scope(Scope::Overlap)),
            "exact" => Ok(Directive::Scope(Scope::Exact)),
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
        "format_scope" => match v {
            "node" => Ok(Directive::FormatScope(FormatScope::Node)),
            "range" => Ok(Directive::FormatScope(FormatScope::Range)),
            "file" => Ok(Directive::FormatScope(FormatScope::File)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid format_scope `{v}`: {line}"),
            )
            .into()),
        },
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
        "edition" => Ok(Directive::Edition(v.to_string())),
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
            Ok(Directive::Unknown {
                key: k.to_string(),
                value: Some(v.to_string()),
            })
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
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 +1,5 @@\n mod old;\n+mod new;\n\n fn main() {}\n";
        let p = parse_patch(text).unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].target_path, "src/lib.rs");
        assert_eq!(p.files[0].hunks.len(), 1);
        assert!(!p.files[0].hunks[0].is_rsmpl());
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
        let text = "# rsmpl-git=0.1\ndiff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-foo\n+bar\n";
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
    fn empty_line_between_body_lines_is_context() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,5 +1,5 @@\n a\n\n b\n";
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
        for text in ["", "  \n\n", "# rsmpl-git=0.1\n"] {
            let e = parse_patch(text).unwrap_err();
            let msg = format!("{e}");
            assert!(msg.contains("RSMPL_PARSE_CONFLICT"), "got: {msg}");
            assert!(msg.contains("diff --git"), "got: {msg}");
        }
    }
}
