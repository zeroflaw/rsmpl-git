//! Directive parsing for RSmPL-Git patch files (SPEC section 9).
//!
//! Split from `parser.rs` (WI-6a): hunk/file-section parsing stays in
//! `parser.rs`; `#`-directive parsing lives here. Pure move, no behavior
//! change — except the WI-6a addition of `# follow=uses` (SPEC 9.3/14.8).
//! `parser.rs` re-exports `parse_directives` so existing import paths keep
//! working.

use super::parser::Result;
use crate::errors::{ErrorCode, StructuredError};
use crate::model::*;
/// Parse `# key=value` / `# flag` / `# kind Name` directives (SPEC 9).
pub fn parse_directives(raw: &[String]) -> Result<Vec<Directive>> {
    let mut out = Vec::new();
    for line in raw {
        // WI-1a: `# sweep` (SPEC 9.3) expands in place; later explicits win.
        if crate::sweep::is_sweep_line(line) {
            out.extend(crate::sweep::expansion());
            continue;
        }
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
            .with_repair(StructuredError::fix_directive_repair(
                line,
                None,
                "start the directive line with `#` (e.g. `# allow=many`)",
            ))
        })?
        .trim();
    if body.is_empty() {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("empty directive: {line}"),
        )
        .with_repair(StructuredError::fix_directive_repair(
            line,
            None,
            "remove the empty `#` line or write a directive (e.g. `# allow=many`)",
        ))
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
        .with_repair(StructuredError::fix_directive_repair(
            line,
            None,
            "remove the empty `#` line or write a directive (e.g. `# allow=many`)",
        ))
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
                .with_repair(StructuredError::fix_pattern_repair(
                    "name",
                    format!("add the metavariable name (e.g. `# {single} E`)").as_str(),
                ))
                .into());
            }
            // T113: unknown bare directives fail fast (SPEC 2.6) — no
            // forward-compat silent ignore. WI-3: machine repair.
            let corrected = suggest_bare(single);
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("unknown directive `#{single}`: {line}"),
            )
            .with_suggestion("check SPEC 9.3 for the supported directive list".to_string())
            .with_repair(directive_repair(line, corrected))
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
                        .with_repair(StructuredError::fix_directive_repair(
                            line,
                            None,
                            "write `# symbol Name=path` (replace Name/path with the capture name and crate path)",
                        ))
                        .into());
                    }
                    let (n, p) = rest[0].split_once('=').ok_or_else(|| {
                        StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("invalid symbol directive (want `# symbol Name=path`): {line}"),
                        )
                        .with_repair(StructuredError::fix_directive_repair(
                            line,
                            None,
                            "write `# symbol Name=path` (replace Name/path with the capture name and crate path)",
                        ))
                    })?;
                    validate_metavar_name(n)?;
                    if p.is_empty() {
                        return Err(StructuredError::new(
                            ErrorCode::PatternConflict,
                            format!("symbol path must not be empty: {line}"),
                        )
                        .with_repair(StructuredError::fix_directive_repair(
                            line,
                            None,
                            "write `# symbol Name=path` (replace Name/path with the capture name and crate path)",
                        ))
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
                        .with_repair(StructuredError::fix_pattern_repair(
                            "name",
                            "rename the metavariable to a single identifier (e.g. `E`)",
                        ))
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
                    .with_repair(StructuredError::fix_pattern_repair(
                        "name",
                        format!("add the metavariable name (e.g. `# {kind} E`)").as_str(),
                    ))
                    .into());
                }
                return Ok(Directive::MetaVar { kind: mk, names });
            }
            // T113: unknown multi-token directives fail fast (SPEC 2.6).
            // WI-3: machine repair.
            let corrected = suggest_bare(tokens[0]);
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("unknown directive `#{}`: {line}", tokens[0]),
            )
            .with_suggestion("check SPEC 9.3 for the supported directive list".to_string())
            .with_repair(directive_repair(line, corrected))
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
/// with `RSMPL_PATTERN_CONFLICT` (T113 fail-fast, SPEC 2.6); near-misses
/// within edit distance 2 get the did-you-mean suggestion, every other
/// unknown key gets the SPEC 9.3 pointer. WI-3: this table is the single
/// spelling list — engine/pattern reuse `suggest_directive_key` /
/// `suggest_value` instead of a second table.
const KNOWN_KV_KEYS: &[&str] = &[
    "lang", "allow", "optional", "scope", "bind", "use", "attrs", "mode", "macro", "format",
    "returns", "root", "fallback", "metavars", "symbol", "follow",
];

/// Levenshtein edit distance over chars (directive keys/values are tiny;
/// quadratic DP is fine and has no dependency).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Closest candidate within edit distance 2 (`None` when nothing is
/// close — the WI-3 honesty gate). Ties break toward the earlier table
/// entry so suggestions are deterministic.
pub fn closest_within<'a>(needle: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&'a str, usize)> = None;
    for c in candidates {
        let d = edit_distance(needle, c);
        match best {
            Some((_, bd)) if d >= bd => {}
            _ => best = Some((c, d)),
        }
    }
    match best {
        Some((c, d)) if d <= 2 => Some(c),
        _ => None,
    }
}

/// Closest candidate with the same first character and within edit
/// distance 2 (`None` otherwise).
///
/// The first-character gate keeps far misses honest (`# foo=bar`
/// suggests nothing — `foo` is distance 2 from `root` but shares no
/// prefix intent, and a wrong `did_you_mean` costs the agent a retry
/// round). Typo'd words overwhelmingly keep their first letter, so the
/// `# alow=many` → `# allow=many` class still fires. Values stay
/// ungated (`closest_within`): their closed sets are tiny and the far
/// miss lists every legal option.
fn closest_gated<'a>(needle: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let first = needle.chars().next()?;
    let mut best: Option<(&'a str, usize)> = None;
    for c in candidates {
        if !c.starts_with(first) {
            continue;
        }
        let d = edit_distance(needle, c);
        match best {
            Some((_, bd)) if d >= bd => {}
            _ => best = Some((c, d)),
        }
    }
    match best {
        Some((c, d)) if d <= 2 => Some(c),
        _ => None,
    }
}

/// Closest known `key=value` directive key: same first character and
/// within edit distance 2 (`None` otherwise).
pub fn suggest_directive_key(k: &str) -> Option<&'static str> {
    // The table entries are `&'static str`; re-match the gated hit
    // against the same table to recover the static lifetime.
    let hit = closest_gated(k, KNOWN_KV_KEYS)?;
    KNOWN_KV_KEYS.iter().find(|c| **c == hit).copied()
}

/// Bare (valueless) directive flags (SPEC 9.3).
const BARE_FLAGS: &[&str] = &["optional", "repeat", "sweep"];

/// Closest bare-word directive spelling (bare flag or `k=v` key),
/// rendered as the corrected line: flags replay verbatim, `lang`
/// replays its only spelling, other keys name the value slot.
fn suggest_bare(word: &str) -> Option<String> {
    // Gated over flags + keys; the display mapping is the only second
    // list, and it renders rather than re-spells.
    const ALL: &[&str] = &[
        "optional", "repeat", "sweep", "lang", "allow", "scope", "bind", "use", "attrs", "mode",
        "macro", "format", "returns", "root", "fallback", "metavars", "symbol", "follow",
    ];
    debug_assert!(ALL.len() == BARE_FLAGS.len() + KNOWN_KV_KEYS.len() - 1);
    match closest_gated(word, ALL) {
        Some("lang") => Some("# lang=rust".to_string()),
        Some(c) if BARE_FLAGS.contains(&c) => Some(format!("#{c}")),
        Some(c) => Some(format!("#{c}=... (see SPEC 9.3 for the value)")),
        None => None,
    }
}

/// Closest allowed value within distance 2 (for closed value sets).
pub fn suggest_value<'a>(v: &str, allowed: &[&'a str]) -> Option<&'a str> {
    closest_within(v, allowed)
}

/// WI-3 `fix_directive` repair for one offending directive line.
///
/// `corrected` is the full corrected `# ...` line when a close spelling
/// exists; when `None`, `write_instead` names the removal (the only
/// machine-applicable exit for a far miss).
fn directive_repair(found_line: &str, corrected: Option<String>) -> serde_json::Value {
    match corrected {
        Some(line) => {
            StructuredError::fix_directive_repair(found_line, Some(line.as_str()), line.as_str())
        }
        None => StructuredError::fix_directive_repair(
            found_line,
            None,
            format!("remove `{found_line}` (see SPEC 9.3 for the supported directive list)")
                .as_str(),
        ),
    }
}

/// Corrected full line for an unknown key carrying the authored value
/// (canonical `# key=value` spacing — the diet form agents write).
fn corrected_kv_line(best_key: &str, v: &str) -> String {
    format!("# {best_key}={v}")
}

/// WI-3 repair for an invalid value of a known key with a closed set
/// (canonical `# key=value` spacing throughout).
///
/// A near-miss (within distance 2) repairs to the full corrected line;
/// a far miss repairs to a removal naming every legal spelling, so the
/// agent never has to guess the value domain.
fn bad_value_repair(key: &str, v: &str, allowed: &[&str]) -> serde_json::Value {
    let found = format!("# {key}={v}");
    match suggest_value(v, allowed) {
        Some(best) => {
            let corrected = format!("# {key}={best}");
            StructuredError::fix_directive_repair(
                &found,
                Some(corrected.as_str()),
                corrected.as_str(),
            )
        }
        None => {
            let options = allowed
                .iter()
                .map(|a| format!("# {key}={a}"))
                .collect::<Vec<_>>()
                .join(", ");
            let write_instead = format!("remove `{found}` or use one of: {options}");
            StructuredError::fix_directive_repair(&found, None, write_instead.as_str())
        }
    }
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
                .with_repair(bad_value_repair("lang", v, &["rust"]))
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
            .with_repair(bad_value_repair("allow", v, &["one", "many"]))
            .into()),
        },
        "optional" => match v {
            "true" => Ok(Directive::Optional(true)),
            "false" => Ok(Directive::Optional(false)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid optional value `{v}`: {line}"),
            )
            .with_repair(bad_value_repair("optional", v, &["true", "false"]))
            .into()),
        },
        "repeat" => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid repeat value `{v}` (write bare `# repeat`): {line}"),
        )
        .with_suggestion("write bare `# repeat` with no value".to_string())
        .with_repair(StructuredError::fix_directive_repair(
            line,
            Some("# repeat"),
            "# repeat",
        ))
        .into()),
        "sweep" => crate::sweep::reject_value(v, line),
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
            .with_repair(bad_value_repair(
                "scope",
                v,
                &["contained", "anchor", "overlap", "exact", "file"],
            ))
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
            .with_repair(bad_value_repair(
                "attrs",
                v,
                &["preserve", "absent", "explicit", "strip"],
            ))
            .into()),
        },
        "mode" => match v {
            "ast" => Ok(Directive::Mode(ParseMode::Ast)),
            "tokens" => Ok(Directive::Mode(ParseMode::Tokens)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid mode `{v}`: {line}"),
            )
            .with_repair(bad_value_repair("mode", v, &["ast", "tokens"]))
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
                .with_repair(bad_value_repair(
                    "macro",
                    v,
                    &["invocation", "token_tree", "expanded"],
                ))
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
            .with_repair(bad_value_repair(
                "format",
                v,
                &["preserve", "rustfmt", "canonical"],
            ))
            .into()),
        },
        "format_scope" => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unsupported directive `# format_scope={v}` (only `# format=preserve` is supported in v0.1): {line}"),
        )
        .with_suggestion(
            "remove the directive — only `# format=preserve` is supported".to_string(),
        )
        .with_repair(StructuredError::fix_directive_repair(
            line,
            None,
            format!("remove `{line}` — only `# format=preserve` is supported").as_str(),
        ))
        .into()),
        "returns" => match v {
            "none" => Ok(Directive::ReturnsNone(true)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid returns `{v}` (only `none` supported): {line}"),
            )
            .with_repair(bad_value_repair("returns", v, &["none"]))
            .into()),
        },
        "root" => Ok(Directive::Root(v.to_string())),
        "fallback" => match v {
            "syntax" => Ok(Directive::FallbackSyntax(true)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid fallback `{v}`: {line}"),
            )
            .with_repair(bad_value_repair("fallback", v, &["syntax"]))
            .into()),
        },
        "metavars" => match v {
            "sigiled" => Ok(Directive::SigiledMetavars(true)),
            "bare" => Ok(Directive::SigiledMetavars(false)),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid metavars `{v}`: {line}"),
            )
            .with_repair(bad_value_repair("metavars", v, &["sigiled", "bare"]))
            .into()),
        },
        "edition" => Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("unsupported directive `# edition={v}` (edition is the toolchain default in v0.1): {line}"),
        )
        .with_suggestion(
            "remove the directive — edition is the toolchain default in v0.1".to_string(),
        )
        .with_repair(StructuredError::fix_directive_repair(
            line,
            None,
            format!("remove `{line}` — edition is the toolchain default in v0.1").as_str(),
        ))
        .into()),
        "symbol" => {
            // `# symbol=...` without name — invalid; canonical is `# symbol N=P`.
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid symbol directive (want `# symbol Name=path`): {line}"),
            )
            .with_repair(StructuredError::fix_directive_repair(
                line,
                None,
                "write `# symbol Name=path` (replace Name/path with the capture name and crate path)",
            ))
            .into())
        }
        // WI-6a: `# follow=uses` (SPEC 9.3/14.8) — same-file use-site
        // rewriting for fn/item renames. Any other value is rejected.
        "follow" => match v {
            "uses" => Ok(Directive::FollowUses),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid follow value `{v}` (only `uses` is supported in v0.1): {line}"),
            )
            .with_suggestion("write `# follow=uses`".to_string())
            .with_repair(bad_value_repair("follow", v, &["uses"]))
            .into()),
        },
        _ => {
            // WI-3: machine repair over the shared table (distance ≤ 2).
            let corrected = suggest_directive_key(k).map(|best| corrected_kv_line(best, v));
            let suggestion = match suggest_directive_key(k) {
                Some(best) => format!("use `#{best}=...`"),
                None => "check SPEC 9.3 for the supported directive list".to_string(),
            };
            let message = match suggest_directive_key(k) {
                Some(best) => format!(
                    "unknown directive `#{k}={v}` (did you mean `#{best}=...`?): {line}"
                ),
                None => format!("unknown directive `#{k}={v}`: {line}"),
            };
            Err(StructuredError::new(ErrorCode::PatternConflict, message)
                .with_suggestion(suggestion)
                .with_repair(directive_repair(line, corrected))
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
        .with_repair(StructuredError::fix_pattern_repair(
            "name",
            "name the metavariable with a single letter (e.g. `# expr E`)",
        ))
        .into());
    }
    let mut chars = n.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid name `{n}` (must start with letter or _)"),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "name",
            "rename the metavariable to start with a letter or `_` (e.g. `E`)",
        ))
        .into());
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("invalid name `{n}` (only alphanumeric + _ allowed)"),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "name",
            "rename the metavariable to alphanumeric + `_` only (e.g. `E`)",
        ))
        .into());
    }
    // WI-5b-follow: `ELLIPSIS*` is reserved for `...` markers — a declared
    // `ELLIPSIS_FOO` would placeholder-substitute to `__RSMPL_ELLIPSIS_FOO`
    // (and `ELLIPSIS_0` byte-identical to marker 0), misreading as an
    // ellipsis. Backticked `` `ELLIPSIS_FOO` `` stays a literal (quoting
    // bypasses this validation).
    if n == "ELLIPSIS" || n.starts_with("ELLIPSIS_") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("reserved name `{n}` (`ELLIPSIS*` names the `...` marker; rename the metavariable, e.g. `E`)"),
        )
        .with_suggestion("rename the metavariable (e.g. `E`); quote the literal with backticks (e.g. `` `ELLIPSIS_FOO` ``)".to_string())
        .with_repair(StructuredError::fix_pattern_repair(
            "name",
            "rename the metavariable (e.g. `E`); quote the literal with backticks (e.g. `` `ELLIPSIS_FOO` ``)",
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_uses_parses() {
        let d = parse_directives(&["# follow=uses".to_string()]).unwrap();
        assert!(d.iter().any(|x| matches!(x, Directive::FollowUses)));
    }

    #[test]
    fn follow_other_value_rejected() {
        let err = parse_directives(&["# follow=all".to_string()]).unwrap_err();
        match err {
            crate::parser::ParseError::Structured(s) => {
                assert_eq!(s.code, ErrorCode::PatternConflict);
                assert!(s.message.contains("only `uses`"), "got: {}", s.message);
            }
            crate::parser::ParseError::Message(m) => panic!("unexpected message: {m}"),
        }
    }

    #[test]
    fn ellipsis_names_reserved() {
        // WI-5b-follow: `ELLIPSIS*` would placeholder-substitute into the
        // internal marker namespace (`__RSMPL_ELLIPSIS_FOO`, or byte-identical
        // to marker 0 for `ELLIPSIS_0`).
        for bad in [
            "# expr ELLIPSIS",
            "# expr ELLIPSIS_FOO",
            "# expr ELLIPSIS_0",
        ] {
            let err = parse_directives(&[bad.to_string()]).unwrap_err();
            match err {
                crate::parser::ParseError::Structured(s) => {
                    assert!(s.message.contains("reserved"), "got: {}", s.message);
                }
                crate::parser::ParseError::Message(m) => panic!("unexpected message: {m}"),
            }
        }
        assert!(parse_directives(&["# expr E".to_string()]).is_ok());
    }
}
