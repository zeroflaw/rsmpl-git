//! N=1 item rename: one-ident-delta `try_item_rename` plus the `split_item` header splitter.
//! Split off verbatim under the WI-19 1900-line warning gate (D5): pure move,
//! no behavior change. Tests stay inline in the parent module per WI-22.
use super::{contains_word, ident_len, item_emission_header, unique_rule};

/// One-ident-delta item rename: exactly one `-`/`+` line pair, same
/// kind/prefix/suffix, idents differ. WI-19c: leading whitespace is
/// tolerated for non-`fn` kinds (mod-nested / block-level items — the
/// engine matches them via `syn`, the emission keeps the bare
/// spelling); `fn` stays column-0 (indented `fn` lines are methods,
/// owned by the slot sweep).
pub(crate) fn try_item_rename(
    old_line: &str,
    new_line: &str,
    path: &str,
    file_old: Option<&str>,
    sites: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let old = old_line.trim_end();
    let new = new_line.trim_end();
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    // WI-19c: indented `fn` lines are methods, never item renames.
    if old_kind == "fn"
        && (old.len() != old.trim_start().len() || new.len() != new.trim_start().len())
    {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let mut hunk = item_emission_header(path, &rule, file_old, sites);
    if old_kind == "fn" {
        // WI-1e diet terse: `- fn Old` / `+ fn New` (params/return/
        // generics/where unconstrained, SPEC 14.1.1). Plain top-level
        // fns only; vis-prefixed or evolved signatures stay ordinary
        // (slot sweep covers the multi-site case).
        if !old_prefix.is_empty() {
            return None;
        }
        let suffix = old_suffix.trim_start();
        if !suffix.starts_with('(') || !old.trim_end().ends_with('{') {
            return None;
        }
        if suffix.contains("->") || contains_word(suffix, "where") {
            return None;
        }
        hunk.push_str(&format!("- fn {old_ident}\n+ fn {new_ident}\n"));
    } else {
        hunk.push_str(&format!(
            "- {old_prefix}{old_kind} {old_ident}\n+ {new_prefix}{new_kind} {new_ident}\n"
        ));
    }
    Some(hunk)
}

/// Split `fn foo() {`-style header lines into
/// (vis prefix, kind, ident, suffix). Prefix is `""` or `"pub … "`
/// (with trailing space); suffix is everything after the ident.
pub(crate) fn split_item(line: &str) -> Option<(String, String, String, String)> {
    const KINDS: &[&str] = &[
        "fn",
        "struct",
        "enum",
        "trait",
        "type",
        "mod",
        "union",
        "const",
        "static",
        "macro_rules!",
    ];
    let trimmed = line.trim_start();
    // WI-19c: leading whitespace is tolerated for nested-item kinds
    // (`struct`/`enum`/`union`/`mod` — `mod`-nested and block-level
    // items the engine matches via `syn`; emission keeps the bare
    // spelling). All other kinds stay column-0: indented `fn` lines are
    // methods (slot sweep owns them), indented `type`/`const`/`static`
    // lines inside trait/impl blocks are assoc definitions (assoc
    // def-only owns them), and indented `macro_rules!` lines stay with
    // the macro detector.
    let indented = trimmed.len() != line.len();
    let (prefix, rest) = if let Some(r) = trimmed.strip_prefix("pub") {
        // `pub`, `pub(…)`, each followed by whitespace.
        if r.is_empty() || r.starts_with([' ', '\t']) {
            ("pub ".to_string(), r.trim_start().to_string())
        } else if r.starts_with('(') {
            let end = r.find(')')?;
            let after = &r[end + 1..];
            if after.is_empty() || !after.starts_with([' ', '\t']) {
                return None;
            }
            (
                format!("pub{} ", &r[..=end]),
                after.trim_start().to_string(),
            )
        } else {
            return None;
        }
    } else {
        (String::new(), trimmed.to_string())
    };
    let kind_end = rest.find([' ', '\t'])?;
    let kind = &rest[..kind_end];
    if !KINDS.contains(&kind) {
        return None;
    }
    // WI-19c allowlist: only nested-item kinds tolerate indent.
    if indented && !matches!(kind, "struct" | "enum" | "union" | "mod") {
        return None;
    }
    let after_kind = rest[kind_end..].trim_start();
    let ident_len = ident_len(after_kind)?;
    let ident = &after_kind[..ident_len];
    let suffix = after_kind[ident_len..].to_string();
    if ident.is_empty() || suffix.is_empty() {
        return None;
    }
    Some((prefix, kind.to_string(), ident.to_string(), suffix))
}
