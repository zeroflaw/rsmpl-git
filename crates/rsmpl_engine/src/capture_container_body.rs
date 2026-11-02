//! WI-19 slice 4: container (`struct`/`enum`/`union`) body-evolving
//! `as` splices (D5 sibling of `capture.rs`).
//!
//! A `+ <vis>struct Old as New { ... }` (likewise `enum`/`union`) paste
//! splices the captured header to the new name (abstract-style: verbatim
//! prefix + new ident, captured suffix verbatim) and renames `Old` →
//! `New` through [`rsmpl_pattern::rename_ident_in_code`] in the captured
//! suffix and body. The WI-2 binding boundary
//! ([`rsmpl_pattern::item_body_evolution_veto`]) applies: a field or
//! variant ident, a macro name, or an attribute interior vetoes loud
//! instead of renaming. Single- and multi-line captures share one path
//! (the header suffix is renamed in place, so single-line items with a
//! same-line self-reference round-trip).

use rsmpl_patch::{ErrorCode, StructuredError};

/// The ident of a `<keyword> <Ident>` header line (`None` when the line
/// holds no word-bounded keyword + ident pair). Thin dispatch over the
/// moved kind-specific `*_line_ident` helpers below.
fn keyword_line_ident(line: &str, keyword: &str) -> Option<String> {
    match keyword {
        "struct" => struct_line_ident(line),
        "enum" => enum_line_ident(line),
        "union" => union_line_ident(line),
        _ => None,
    }
}

/// Shared container `as` splice: `captured` must hold a single item of
/// kind `keyword` (`struct`/`enum`/`union`) named `old_t`; the template
/// prefix rides verbatim and the new ident is `new_t`.
fn splice_container_body_evolution(
    captured: &str,
    new_prefix: &str,
    old_t: &str,
    new_t: &str,
    keyword: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| {
        StructuredError::new(ErrorCode::PatternConflict, msg)
            .with_rule(rule)
            .with_repair(StructuredError::fix_pattern_repair(
                "container-as-paste",
                &format!(
                    "write `+ <vis>{keyword} <Old> as <New> {{ ... }}` (exactly the container rename, WI-2 binding law), or split the move"
                ),
            ))
    };
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name: String = match file.items.as_slice() {
        [syn::Item::Struct(s)] if keyword == "struct" => s.ident.to_string(),
        [syn::Item::Enum(e)] if keyword == "enum" => e.ident.to_string(),
        [syn::Item::Union(u)] if keyword == "union" => u.ident.to_string(),
        _ => {
            return Err(fail(format!(
                "evolution paste needs a captured `{keyword}` item for a `{keyword} <Old> as <New>` template"
            )));
        }
    };
    if old_name != old_t {
        return Err(fail(format!(
            "body-evolving paste names `{old_t}` but the capture holds `{keyword} {old_name}`"
        )));
    }
    // WI-19 slice 4 / WI-2: binding-aware boundary — a field or variant
    // ident, a macro name, or an attribute interior named like the old
    // ident vetoes the authored paste (rename it by hand or split the
    // move). Lifetimes/strings/comments ride verbatim with no veto.
    if let Some(veto) = rsmpl_pattern::item_body_evolution_veto(captured, &old_name) {
        return Err(fail(format!(
            "body-evolving paste vetoes: `{old_name}` has a {} occurrence at captured line {} (rename it by hand or split the move)",
            veto.kind, veto.line
        )));
    }
    // First `<keyword> <Old>` header line: word-bounded match so a body
    // occurrence of the name never wins over the header. Doc/comment
    // lines never evolve.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if keyword_line_ident(line, keyword).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `{keyword} {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    // Word-bounded `<keyword> <old>` within the code line (never a
    // keyword inside another ident); the remainder after the old ident
    // is the suffix (generics/where/body-on-same-line), renamed in
    // place so single-line items round-trip.
    let mut kw_at = None;
    let mut search = rest;
    let mut base = 0;
    let kw = keyword;
    while let Some(pos) = search.find(kw) {
        let before = search.as_bytes().get(pos.wrapping_sub(1));
        let before_ok =
            pos == 0 || before.is_some_and(|c| !c.is_ascii_alphanumeric() && *c != b'_');
        let after = &search[pos + kw.len()..];
        let after_ok = after.starts_with(|c: char| c.is_whitespace());
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) {
            let tail = &after.trim_start()[old_name.len()..];
            let boundary = tail
                .chars()
                .next()
                .map(|c| c != '_' && !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            if boundary {
                kw_at = Some(base + pos);
                break;
            }
        }
        search = &search[pos + kw.len()..];
        base += pos + kw.len();
    }
    let kw_at = kw_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `{keyword} {old_name}` header line"
        ))
    })?;
    let after_kw = &rest[kw_at + kw.len()..];
    let trimmed = after_kw.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let renamed_suffix = rsmpl_pattern::rename_ident_in_code(&suffix, &old_name, new_t);
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("{keyword} {new_t}")
    } else {
        format!("{pre} {keyword} {new_t}")
    };
    lines[idx] = format!("{ws}{head}{renamed_suffix}");
    // Body rename over the whole post-header text (one scanner pass, so
    // multi-line block comments stay sound); the header line above is
    // already spliced and excluded.
    if idx + 1 < lines.len() {
        let body = lines[idx + 1..].join("\n");
        let renamed = rsmpl_pattern::rename_ident_in_code(&body, &old_name, new_t);
        let mut new_lines: Vec<String> = renamed.split('\n').map(str::to_string).collect();
        lines.truncate(idx + 1);
        lines.append(&mut new_lines);
    }
    Ok(lines.join("\n"))
}

/// WI-19 slice 4: splice a captured `struct` item under a
/// body-evolving template header (`pub struct Old as New { ... }`).
pub(crate) fn splice_struct_body_evolution(
    captured: &str,
    new_prefix: &str,
    old_t: &str,
    new_t: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    splice_container_body_evolution(captured, new_prefix, old_t, new_t, "struct", rule)
}

/// WI-19 slice 4: splice a captured `enum` item under a
/// body-evolving template header (`pub enum Old as New { ... }`).
pub(crate) fn splice_enum_body_evolution(
    captured: &str,
    new_prefix: &str,
    old_t: &str,
    new_t: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    splice_container_body_evolution(captured, new_prefix, old_t, new_t, "enum", rule)
}

/// WI-19 slice 4: splice a captured `union` item under a
/// body-evolving template header (`pub union Old as New { ... }`).
pub(crate) fn splice_union_body_evolution(
    captured: &str,
    new_prefix: &str,
    old_t: &str,
    new_t: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    splice_container_body_evolution(captured, new_prefix, old_t, new_t, "union", rule)
}

// WI-19 slice 4 (D5 split): kind-specific header splices moved here
// verbatim out of capture.rs (which sat at 2026/2000). Only visibility
// widened; behavior unchanged.

/// WI-8 slice 1: splice a captured `struct` item under an
/// evolution-template header (`pub struct New { ... }`). The
/// template's visibility prefix and new ident replace the capture's
/// `struct <Old>` header span; generics, where-clauses, and fields
/// (named/unnamed/unit) ride byte-verbatim as the suffix after the
/// old ident.
pub(crate) fn splice_struct_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_struct_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `struct` header with `...` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-struct-splice-template",
                "write `+ <vis>struct <New> { ... }` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Struct(s)] => s.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `struct` item for a `struct` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `struct <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if struct_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `struct {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut struct_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("struct") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 6..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            struct_at = Some(base + pos);
            break;
        }
        search = &search[pos + 6..];
        base += pos + 6;
    }
    let struct_at = struct_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `struct {old_name}` header line"
        ))
    })?;
    let after_struct = &rest[struct_at + 6..];
    let trimmed = after_struct.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("struct {new_name}")
    } else {
        format!("{pre} struct {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// WI-8 slice 2: splice a captured `enum` item under an
/// evolution-template header (`pub enum New { ... }`). The
/// template's visibility prefix and new ident replace the capture's
/// `enum <Old>` header span; generics, where-clauses, and variants
/// ride byte-verbatim as the suffix after the old ident.
pub(crate) fn splice_enum_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_enum_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `enum` header with `...` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-enum-splice-template",
                "write `+ <vis>enum <New> { ... }` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Enum(e)] => e.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `enum` item for an `enum` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `enum <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if enum_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `enum {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut enum_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("enum") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 4..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            enum_at = Some(base + pos);
            break;
        }
        search = &search[pos + 4..];
        base += pos + 4;
    }
    let enum_at = enum_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `enum {old_name}` header line"
        ))
    })?;
    let after_enum = &rest[enum_at + 4..];
    let trimmed = after_enum.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("enum {new_name}")
    } else {
        format!("{pre} enum {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// WI-8 slice 3: splice a captured `union` item under an
/// evolution-template header (`pub union New { ... }`). The
/// template's visibility prefix and new ident replace the capture's
/// `union <Old>` header span; generics, where-clauses, and fields
/// ride byte-verbatim as the suffix after the old ident.
pub(crate) fn splice_union_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_union_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `union` header with `...` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-union-splice-template",
                "write `+ <vis>union <New> { ... }` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Union(u)] => u.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `union` item for a `union` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `union <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if union_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `union {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut union_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("union") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 5..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            union_at = Some(base + pos);
            break;
        }
        search = &search[pos + 5..];
        base += pos + 5;
    }
    let union_at = union_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `union {old_name}` header line"
        ))
    })?;
    let after_union = &rest[union_at + 5..];
    let trimmed = after_union.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("union {new_name}")
    } else {
        format!("{pre} union {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// WI-8 slice 4: splice a captured `type` alias under an
/// evolution-template header (`pub type New = ...;`). The
/// template's visibility prefix and new ident replace the capture's
/// `type <Old>` header span; generics and `= ty;` ride
/// byte-verbatim as the suffix after the old ident.
pub(crate) fn splice_type_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_type_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `type` header with `= ...;` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-type-splice-template",
                "write `+ <vis>type <New> = ...;` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Type(t)] => t.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `type` item for a `type` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `type <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if type_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `type {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut type_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("type") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 4..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            type_at = Some(base + pos);
            break;
        }
        search = &search[pos + 4..];
        base += pos + 4;
    }
    let type_at = type_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `type {old_name}` header line"
        ))
    })?;
    let after_type = &rest[type_at + 4..];
    let trimmed = after_type.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("type {new_name}")
    } else {
        format!("{pre} type {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// WI-8 slice 6: splice a captured `static` item under an
/// evolution-template header (`pub static [mut ]New = ...;`). The
/// template's visibility prefix, `mut`, and new ident replace the
/// capture's `static [mut ]<Old>` header span; leading indent and the
/// full captured suffix (`: ty = expr;`) stay verbatim.
pub(crate) fn splice_static_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name, new_mut) =
        rsmpl_pattern::parse_static_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `static` header with `= ...;` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-static-splice-template",
                "write `+ <vis>static [mut ]<New> = ...;` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Static(s)] => s.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `static` item for a `static` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `static [mut ]<Old>` header line: word-bounded match so a
    // body occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if static_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `static {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut static_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("static") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 6..];
        let after_ok = after.starts_with(char::is_whitespace);
        // Skip an optional `mut` between the keyword and the ident,
        // then require the captured ident with a non-ident boundary.
        let tail = after.trim_start();
        let tail = tail
            .strip_prefix("mut")
            .filter(|r| r.starts_with(char::is_whitespace))
            .map(str::trim_start)
            .unwrap_or(tail);
        let starts = tail.starts_with(old_name.as_str());
        let boundary = starts
            && tail[old_name.len()..]
                .chars()
                .next()
                .map(|c| c != '_' && !c.is_ascii_alphanumeric())
                .unwrap_or(true);
        if before_ok && after_ok && boundary {
            static_at = Some(base + pos);
            break;
        }
        search = &search[pos + 6..];
        base += pos + 6;
    }
    let static_at = static_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `static {old_name}` header line"
        ))
    })?;
    let after_static = &rest[static_at + 6..];
    let trimmed = after_static.trim_start();
    // The suffix starts after the optional `mut`: the template's `mut`
    // rides verbatim, so a captured `mut` never leaks into the output
    // when the template drops it (or vice versa).
    let after_mut = trimmed
        .strip_prefix("mut")
        .filter(|r| r.starts_with(char::is_whitespace))
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let suffix = after_mut[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let mut_word = if new_mut { "mut " } else { "" };
    let head = if pre.is_empty() {
        format!("static {mut_word}{new_name}")
    } else {
        format!("{pre} static {mut_word}{new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// The ident of a `static [mut ]<Ident>` header line (`None` when the
/// line holds no word-bounded `static` + optional `mut` + ident).
pub(crate) fn static_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"static")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 6;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            // Optional word-bounded `mut` qualifier.
            let mut k = j;
            while bytes.get(k).is_some_and(|c| is_ident(*c)) {
                k += 1;
            }
            if line.get(j..k) == Some("mut")
                && bytes.get(k).is_some_and(|c| c.is_ascii_whitespace())
            {
                j = k;
                while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                    j += 1;
                }
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// WI-8 slice 8: splice a captured `trait` item under an
/// evolution-template header (`pub trait New { ... }`,
/// `pub unsafe trait New { ... }`). The template's visibility prefix
/// (+ optional `unsafe`) and new ident replace the capture's
/// `trait <Old>` header span; leading indent and the full captured
/// suffix (generics/supertraits/`{...items}`) stay verbatim.
pub(crate) fn splice_trait_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) = rsmpl_pattern::parse_trait_evolution_template(template.trim())
        .ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `trait` header with `...` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-trait-splice-template",
                "write `+ <vis>trait <New> { ... }` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Trait(t)] => t.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `trait` item for a `trait` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `trait <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if trait_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `trait {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut trait_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("trait") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 5..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            trait_at = Some(base + pos);
            break;
        }
        search = &search[pos + 5..];
        base += pos + 5;
    }
    let trait_at = trait_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `trait {old_name}` header line"
        ))
    })?;
    let after_trait = &rest[trait_at + 5..];
    let trimmed = after_trait.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("trait {new_name}")
    } else {
        format!("{pre} trait {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// The ident of a `trait <Ident>` header line (`None` when the line
/// holds no word-bounded `trait` + ident pair).
pub(crate) fn trait_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"trait")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 5;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}
/// WI-8 slice 7: splice a captured `mod` item under an
/// evolution-template header (`pub mod New { ... }`,
/// `pub unsafe mod New { ... }`). The template's visibility prefix
/// (+ optional `unsafe`) and new ident replace the capture's
/// `mod <Old>` header span; leading indent and the full captured
/// suffix (`;` or `{}` — declaration or empty inline) stay verbatim.
pub(crate) fn splice_mod_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) = rsmpl_pattern::parse_mod_evolution_template(template.trim())
        .ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `mod` header with `...` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-mod-splice-template",
                "write `+ <vis>mod <New> { ... }` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Mod(m)] => m.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `mod` item for a `mod` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `mod <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if mod_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx =
        target.ok_or_else(|| fail(format!("captured text has no `mod {old_name}` header line")))?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut mod_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("mod") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 3..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            mod_at = Some(base + pos);
            break;
        }
        search = &search[pos + 3..];
        base += pos + 3;
    }
    let mod_at =
        mod_at.ok_or_else(|| fail(format!("captured text has no `mod {old_name}` header line")))?;
    let after_mod = &rest[mod_at + 3..];
    let trimmed = after_mod.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("mod {new_name}")
    } else {
        format!("{pre} mod {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// The ident of a `mod <Ident>` header line (`None` when the line
/// holds no word-bounded `mod` + ident pair).
pub(crate) fn mod_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 3 <= bytes.len() {
        if bytes[i..].starts_with(b"mod")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 3).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 3;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}
/// WI-8 slice 5: splice a captured `const` item under an
/// evolution-template header (`pub const New = ...;`). The
/// template's visibility prefix and new ident replace the capture's
/// `const <Old>` header span; generics and `: ty = expr;` ride
/// byte-verbatim as the suffix after the old ident.
pub(crate) fn splice_const_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_const_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `const` header with `= ...;` (got `{}`)",
                template.trim()
            ))
            .with_repair(StructuredError::fix_pattern_repair(
                "evolution-const-splice-template",
                "write `+ <vis>const <New> = ...;` as the paste template (splice side), or paste the bare capture name for a verbatim move",
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Const(c)] => c.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `const` item for a `const` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `const <Old>` header line: word-bounded match so a body
    // occurrence (e.g. a doc comment) never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if const_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `const {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    let mut const_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("const") {
        let before_ok = pos == 0
            || !search.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && search.as_bytes()[pos - 1] != b'_';
        let after = &search[pos + 5..];
        let after_ok = after.starts_with(char::is_whitespace);
        let boundary = after.trim_start()[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) && boundary {
            const_at = Some(base + pos);
            break;
        }
        search = &search[pos + 5..];
        base += pos + 5;
    }
    let const_at = const_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `const {old_name}` header line"
        ))
    })?;
    let after_const = &rest[const_at + 5..];
    let trimmed = after_const.trim_start();
    let suffix = trimmed[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("const {new_name}")
    } else {
        format!("{pre} const {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// The ident of a `const <Ident>` header line (`None` when the line
/// holds no word-bounded `const` + ident pair).
pub(crate) fn const_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"const")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 5;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// The ident of a `type <Ident>` header line (`None` when the line
/// holds no word-bounded `type` + ident pair).
pub(crate) fn type_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"type")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 4;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// The ident of a `union <Ident>` header line (`None` when the line
/// holds no word-bounded `union` + ident pair).
pub(crate) fn union_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"union")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 5;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// The ident of an `enum <Ident>` header line (`None` when the line
/// holds no word-bounded `enum` + ident pair).
pub(crate) fn enum_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"enum")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 4;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// The ident of a `struct <Ident>` header line (`None` when the line
/// holds no word-bounded `struct` + ident pair).
pub(crate) fn struct_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"struct")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 6;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::capture::splice_evolved_header;

    const STRUCT: &str = "struct Old {\n    next: Option<Box<Old>>,\n}";

    #[test]
    fn struct_as_splice_renames_header_and_field_type() {
        let out = splice_evolved_header(STRUCT, "struct Old as New { ... }", "r")
            .expect("struct as splice must fire");
        assert!(out.contains("struct New {"), "got:\n{out}");
        assert!(out.contains("Option<Box<New>>"), "got:\n{out}");
        assert!(!out.contains("Old"), "old name gone, got:\n{out}");
    }

    #[test]
    fn struct_as_splice_single_line_round_trips() {
        // The header suffix renames in place, so a same-line
        // self-reference rides along.
        let cap = "struct Old(Option<Box<Old>>);";
        let out = splice_evolved_header(cap, "struct Old as New { ... }", "r")
            .expect("single-line struct as splice must fire");
        assert!(out.contains("struct New(Option<Box<New>>);"), "got:\n{out}");
    }

    #[test]
    fn struct_as_splice_vetoes_field_name() {
        let cap = "struct Old {\n    Old: u32,\n}";
        let err = splice_evolved_header(cap, "struct Old as New { ... }", "r")
            .expect_err("field-name capture must veto");
        assert!(err.to_string().contains("field"), "got: {err}");
    }

    #[test]
    fn enum_as_splice_renames_payload() {
        let cap = "pub enum Old {\n    A(Box<Old>),\n    B,\n}";
        let out = splice_evolved_header(cap, "pub enum Old as New { ... }", "r")
            .expect("enum as splice must fire");
        assert!(out.contains("pub enum New {"), "got:\n{out}");
        assert!(out.contains("A(Box<New>)"), "got:\n{out}");
    }

    #[test]
    fn union_as_splice_renames_field_type() {
        let cap = "union Old {\n    x: Old,\n    y: u32,\n}";
        let out = splice_evolved_header(cap, "union Old as New { ... }", "r")
            .expect("union as splice must fire");
        assert!(out.contains("union New {"), "got:\n{out}");
        assert!(out.contains("x: New"), "got:\n{out}");
    }

    #[test]
    fn container_as_splice_rejects_kind_mismatch() {
        let err = splice_evolved_header(STRUCT, "enum Old as New { ... }", "r")
            .expect_err("kind mismatch must fail");
        assert!(err.to_string().contains("enum"), "got: {err}");
    }

    #[test]
    fn container_as_splice_rejects_old_mismatch() {
        let err = splice_evolved_header(STRUCT, "struct Other as New { ... }", "r")
            .expect_err("old mismatch must fail");
        assert!(err.to_string().contains("Other"), "got: {err}");
    }

    fn splice_repair_code(err: &rsmpl_patch::StructuredError) -> String {
        err.repair
            .as_ref()
            .expect("WI-18 slice 2: must carry repair")
            .get("code")
            .and_then(|v| v.as_str())
            .expect("code str")
            .to_string()
    }

    fn splice_repair_write_instead(err: &rsmpl_patch::StructuredError) -> String {
        err.repair
            .as_ref()
            .expect("WI-18 slice 2: must carry repair")
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str")
            .to_string()
    }

    #[test]
    fn wi18_slice2_struct_splice_template_repair_is_specific() {
        let err = super::splice_struct_header("struct Old { x: u32, }", "struct Bad", "r")
            .expect_err("bad struct template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-struct-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>struct <New>"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_enum_splice_template_repair_is_specific() {
        let err = super::splice_enum_header("enum Old { A, B, }", "enum Bad", "r")
            .expect_err("bad enum template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-enum-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>enum <New>"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_union_splice_template_repair_is_specific() {
        let err = super::splice_union_header("union Old { x: u32, }", "union Bad", "r")
            .expect_err("bad union template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-union-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>union <New>"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_type_splice_template_repair_is_specific() {
        let err = super::splice_type_header("type Old = u32;", "type Bad", "r")
            .expect_err("bad type template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-type-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>type <New> = ...;"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_static_splice_template_repair_is_specific() {
        let err = super::splice_static_header("static Old: u32 = 1;", "static Bad", "r")
            .expect_err("bad static template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-static-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>static [mut ]<New> = ...;"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_trait_splice_template_repair_is_specific() {
        let err = super::splice_trait_header("trait Old { fn f(&self); }", "trait Bad", "r")
            .expect_err("bad trait template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-trait-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>trait <New>"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_mod_splice_template_repair_is_specific() {
        let err = super::splice_mod_header("mod Old { }", "mod Bad", "r")
            .expect_err("bad mod template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-mod-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>mod <New>"),
            "got: {err:?}"
        );
    }

    #[test]
    fn wi18_slice2_const_splice_template_repair_is_specific() {
        let err = super::splice_const_header("const Old: u32 = 1;", "const Bad", "r")
            .expect_err("bad const template must fail");
        assert_eq!(splice_repair_code(&err), "evolution-const-splice-template");
        assert!(
            splice_repair_write_instead(&err).contains("+ <vis>const <New> = ...;"),
            "got: {err:?}"
        );
    }
}
