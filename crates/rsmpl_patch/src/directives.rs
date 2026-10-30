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
    "returns", "root", "fallback", "metavars", "symbol", "follow",
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
        // WI-6a: `# follow=uses` (SPEC 9.3/14.8) — same-file use-site
        // rewriting for fn/item renames. Any other value is rejected.
        "follow" => match v {
            "uses" => Ok(Directive::FollowUses),
            _ => Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!("invalid follow value `{v}` (only `uses` is supported in v0.1): {line}"),
            )
            .with_suggestion("write `# follow=uses`".to_string())
            .into()),
        },
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
