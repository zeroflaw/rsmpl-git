//! Capture store for code moves (SPEC 20).
//!
//! Scalar captures hold exactly one item's source text, while a
//! `# bind=<Name>` + `# allow=many` cut hunk holds an ordered
//! collection (WI-6). Binds are only supported on item-capture cut
//! hunks and uses only on pure insertion hunks. Forward references are
//! resolved by the two-pass planner in `plan.rs` (SPEC 20.3, 23.2).

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};

use crate::edits::TextEdit;
use crate::lines::{FileState, LineIndex};
use crate::matcher_fn;
use crate::{RuleReport, RuleSemantics};

/// Patch-local capture map: name -> captured item text(s).
///
/// Scalar captures hold exactly one item's source text. WI-6 collection
/// captures (`# bind=` + `# allow=many`) hold every matched item in
/// source order (SPEC 20.3).
#[derive(Debug, Default)]
pub struct CaptureStore {
    map: HashMap<String, Captured>,
}

#[derive(Debug)]
struct Captured {
    texts: Vec<String>,
    collection: bool,
}

impl CaptureStore {
    pub fn define(
        &mut self,
        name: &str,
        text: String,
        rule: &str,
        file: &str,
    ) -> Result<(), StructuredError> {
        if self.map.contains_key(name) {
            let mut known: Vec<String> = self.map.keys().cloned().collect();
            known.sort();
            return Err(StructuredError::new(
                ErrorCode::CaptureConflict,
                format!("duplicate `# bind={name}` (capture names must be unique)"),
            )
            .with_rule(rule)
            .with_file(file)
            .with_repair(StructuredError::fix_name_repair(name, &known)));
        }
        self.map.insert(
            name.to_string(),
            Captured {
                texts: vec![text],
                collection: false,
            },
        );
        Ok(())
    }

    /// WI-6: define an ordered collection (possibly empty via `# optional`).
    pub fn define_collection(
        &mut self,
        name: &str,
        texts: Vec<String>,
        rule: &str,
        file: &str,
    ) -> Result<(), StructuredError> {
        if self.map.contains_key(name) {
            let mut known: Vec<String> = self.map.keys().cloned().collect();
            known.sort();
            return Err(StructuredError::new(
                ErrorCode::CaptureConflict,
                format!("duplicate `# bind={name}` (capture names must be unique)"),
            )
            .with_rule(rule)
            .with_file(file)
            .with_repair(StructuredError::fix_name_repair(name, &known)));
        }
        self.map.insert(
            name.to_string(),
            Captured {
                texts,
                collection: true,
            },
        );
        Ok(())
    }

    pub fn get_all(
        &self,
        name: &str,
        rule: &str,
        file: &str,
    ) -> Result<(&[String], bool), StructuredError> {
        self.map
            .get(name)
            .map(|c| (c.texts.as_slice(), c.collection))
            .ok_or_else(|| {
                let mut known: Vec<&str> = self.map.keys().map(|s| s.as_str()).collect();
                known.sort();
                let owned: Vec<String> = known.iter().map(|s| s.to_string()).collect();
                StructuredError::new(
                    ErrorCode::CaptureConflict,
                    format!("capture `{name}` is not bound by any hunk"),
                )
                .with_rule(rule)
                .with_file(file)
                .with_suggestion(if known.is_empty() {
                    "add a hunk with `# bind=<Name>`".to_string()
                } else {
                    format!("available captures: {}", known.join(", "))
                })
                .with_repair(StructuredError::fix_name_repair(name, &owned))
            })
    }
}
/// Paste captured item text at a zero-length anchor (SPEC 10.5, 20.2).
/// Single pastes end with exactly one `\n`; multi-pastes are
/// blank-joined after pass 2 (SPEC 20.4) and materialize in document order
/// (SPEC 23.5: first hunk topmost).
/// WI-8a: what a `# use=` paste inserts — the bare capture name
/// (verbatim move) or a single-line header template (a single-line
/// `fn`/`struct`/`enum`/`union`/`type`/`const`/`static`/`mod`/`trait` header with `...`) that
/// the captured body/fields are spliced under (evolution move).
pub(crate) struct PasteTarget<'a> {
    pub(crate) name: &'a str,
    pub(crate) template: Option<&'a str>,
    pub(crate) anchor: u32,
}
pub(crate) fn paste_capture(
    captures: &CaptureStore,
    target: PasteTarget<'_>,
    index: &LineIndex,
    rule: &str,
    file: &str,
    edits: &mut Vec<TextEdit>,
) -> Result<(), StructuredError> {
    let (texts, collection) = captures.get_all(target.name, rule, file)?;
    if collection {
        // WI-6: one header template cannot evolve many items; paste the
        // bare collection verbatim or fail loud (SPEC 20.3).
        if target.template.is_some_and(|t| t.trim() != target.name) {
            return Err(StructuredError::new(
                ErrorCode::CaptureConflict,
                "collection captures cannot use evolution pastes (one header template cannot evolve many items)",
            )
            .with_rule(rule)
            .with_file(file)
            .with_repair(StructuredError::fix_pattern_repair(
                "use",
                "paste with a bare `+ Name` (`# use=<Name>`) or move items singly",
            )));
        }
        if texts.is_empty() {
            return Ok(());
        }
        let mut ins = texts
            .iter()
            .map(|t| t.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n\n");
        ins.push('\n');
        let pos = if target.anchor == 0 {
            0
        } else {
            index
                .line_start(target.anchor + 1)
                .unwrap_or_else(|| index.eof())
        };
        edits.push(TextEdit::new(pos, pos, ins));
        return Ok(());
    }
    let text = match target.template {
        Some(t) if t.trim() != target.name => splice_evolved_header(&texts[0], t, rule)?,
        _ => texts[0].clone(),
    };
    let pos = if target.anchor == 0 {
        0
    } else {
        index
            .line_start(target.anchor + 1)
            .unwrap_or_else(|| index.eof())
    };
    let mut ins = text.trim_end().to_string();
    if !ins.ends_with('\n') {
        ins.push('\n');
    }
    edits.push(TextEdit::new(pos, pos, ins));
    Ok(())
}

/// WI-6: evaluate an `# optional` collection cut with zero matches
/// (pass 1). Binds the empty collection and stages the zero-match rule
/// report; the paste then splices nothing.
pub(crate) fn eval_empty_collection_cut(
    rule: &str,
    file: &str,
    bind: &str,
    captures: &mut CaptureStore,
) -> Result<RuleReport, StructuredError> {
    captures.define_collection(bind, Vec::new(), rule, file)?;
    Ok(RuleReport {
        rule: rule.to_string(),
        file: file.to_string(),
        matches: 0,
        spans: Vec::new(),
        span_labels: Vec::new(),
        sites: Vec::new(),
        follow_skipped: None,
        follow_skipped_details: Vec::new(),
        orphans: Vec::new(),
        semantics: RuleSemantics::ReparseOnly,
    })
}

/// WI-6: evaluate a collection cut hunk (pass 1): capture + delete every
/// in-scope item in source order, define the collection, and stage the
/// per-file edits. Returns the cut's rule report.
pub(crate) fn eval_collection_cut(
    state: &FileState,
    path: &str,
    rule: &str,
    bind: &str,
    targets: &[(u32, u32)],
    captures: &mut CaptureStore,
    file_edits: &mut Vec<TextEdit>,
) -> Result<RuleReport, StructuredError> {
    let mut spans = Vec::with_capacity(targets.len());
    let mut texts = Vec::with_capacity(targets.len());
    for &(sl, el) in targets {
        let cap_start = state
            .index
            .line_start(sl)
            .ok_or_else(|| super::plan::range_err(rule, path, sl))?;
        let cap_end = state
            .index
            .line_start(el + 1)
            .unwrap_or_else(|| state.index.eof());
        texts.push(
            state
                .original
                .get(cap_start..cap_end)
                .unwrap_or("")
                .to_string(),
        );
        let cut = matcher_fn::delete_lines(&state.original, &state.index, sl, el, rule)
            .map_err(|e| e.with_file(path))?;
        file_edits.push(cut);
        spans.push((sl, el));
    }
    captures.define_collection(bind, texts, rule, path)?;
    Ok(RuleReport {
        rule: rule.to_string(),
        file: path.to_string(),
        matches: spans.len(),
        spans,
        span_labels: Vec::new(),
        sites: Vec::new(),
        follow_skipped: None,
        follow_skipped_details: Vec::new(),
        orphans: Vec::new(),
        semantics: RuleSemantics::ReparseOnly,
    })
}

/// WI-8a + WI-8 slices 1-8 + WI-8 slice 9: splice a captured item under an
/// evolution-template header (`pub fn New(...) { ... }`,
/// `pub fn New(x: i64) -> i64 { ... }` (slice 9 concrete signature),
/// `pub struct New { ... }`, `pub enum New { ... }`,
/// `pub union New { ... }`, `pub type New = ...;`,
/// `pub const New = ...;`, `pub static [mut ]New = ...;`,
/// `pub mod New { ... }`, or `pub trait New { ... }`). The
/// template's prefix and new ident replace the capture's header line;
/// every other line stays byte-verbatim. Returns the spliced item text.
fn splice_evolved_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    // WI-8 slice 9: the concrete-signature `fn` template carries the new
    // signature verbatim — checked before the abstract `(...)` form (a
    // concrete template never contains `(...)`, so the two shapes are
    // disjoint, but the concrete splice replaces the whole header line
    // while the abstract splice keeps the captured suffix).
    if rsmpl_pattern::parse_fn_full_header_template(template.trim()).is_some() {
        return splice_fn_full_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_trait_evolution_template(template.trim()).is_some() {
        return splice_trait_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_mod_evolution_template(template.trim()).is_some() {
        return splice_mod_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_static_evolution_template(template.trim()).is_some() {
        return splice_static_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_const_evolution_template(template.trim()).is_some() {
        return splice_const_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_type_evolution_template(template.trim()).is_some() {
        return splice_type_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_union_evolution_template(template.trim()).is_some() {
        return splice_union_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_enum_evolution_template(template.trim()).is_some() {
        return splice_enum_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_struct_evolution_template(template.trim()).is_some() {
        return splice_struct_header(captured, template, rule);
    }
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `fn` header with `(...)` + `...` or a concrete signature + `{{ ... }}` (got `{}`), a single-line `struct` header with `...`, a single-line `enum` header with `...`, a single-line `union` header with `...`, a single-line `type` header with `= ...;`, a single-line `const` header with `= ...;`, a single-line `static` header with `= ...;`, a single-line `mod` header with `...`, or a single-line `trait` header with `...`",
                template.trim()
            ))
        })?;
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Fn(f)] => f.sig.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `fn` item for a `fn` template (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)"
                    .to_string(),
            ));
        }
    };
    // First `fn <Old>` header line: word-bounded match so a body
    // occurrence of the name never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        // Doc/comment lines may textually contain `fn <name>`
        // (e.g. `/// calls fn helper`) — only code lines evolve.
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if fn_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx =
        target.ok_or_else(|| fail(format!("captured text has no `fn {old_name}` header line")))?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    // The `fn <Old>` keyword itself (not a `fn ` inside a string or
    // attribute on the same line): first `fn ` whose remainder is
    // exactly the captured ident plus a non-ident boundary.
    let mut fn_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("fn ") {
        let after = &search[pos + 3..];
        let boundary = after[old_name.len()..]
            .chars()
            .next()
            .map(|c| c != '_' && !c.is_ascii_alphanumeric())
            .unwrap_or(true);
        if after.starts_with(old_name.as_str()) && boundary {
            fn_at = Some(base + pos);
            break;
        }
        search = &search[pos + 3..];
        base += pos + 3;
    }
    let fn_at =
        fn_at.ok_or_else(|| fail(format!("captured text has no `fn {old_name}` header line")))?;
    let after_fn = &rest[fn_at + 3..];
    let suffix = after_fn[old_name.len()..].to_string();
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("fn {new_name}")
    } else {
        format!("{pre} fn {new_name}")
    };
    lines[idx] = format!("{ws}{head}{suffix}");
    Ok(lines.join("\n"))
}

/// WI-8 slice 9: splice a captured `fn` item under a concrete-signature
/// evolution-template header (`pub fn New(x: i64) -> i64 { ... }`). The
/// whole captured header line is replaced with the template's header
/// part (everything before the `{ ... }` marker) — the new signature
/// rides verbatim from the plus side, never from the capture. Attr,
/// doc, and body lines ride byte-verbatim. Single-line items and
/// multi-line headers fail loud: replacing one line of a multi-line
/// header (or the only line of a single-line item) would silently
/// corrupt the signature or drop the body.
fn splice_fn_full_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Fn(f)] => f.sig.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `fn` item for a concrete-signature `fn` template"
                    .to_string(),
            ));
        }
    };
    // First `fn <Old>` header line: word-bounded match so a body
    // occurrence of the name never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        // Doc/comment lines may textually contain `fn <name>`
        // (e.g. `/// calls fn helper`) — only code lines evolve.
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if fn_line_ident(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx =
        target.ok_or_else(|| fail(format!("captured text has no `fn {old_name}` header line")))?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    // The captured header must open and close on this one line (`... {`
    // with nothing after the brace): a multi-line header would leave
    // stale signature lines behind, and a single-line item has no body
    // lines to ride.
    if !lines[idx].trim_end().ends_with('{') {
        return Err(fail(format!(
            "concrete-signature evolution paste needs a single-line-header `fn {old_name}` capture (multi-line headers stay ordinary)"
        )));
    }
    if idx + 1 >= lines.len() {
        return Err(fail(format!(
            "concrete-signature evolution paste needs a multi-line `fn {old_name}` capture (single-line items stay ordinary)"
        )));
    }
    // The template header part (everything before the first `{` — the
    // signature never contains braces) replaces the whole line,
    // preserving the capture's indentation.
    let tmpl = template.trim();
    let brace = tmpl.find('{').ok_or_else(|| {
        fail(format!(
            "evolution paste template must be a single-line `fn` header with a concrete signature + `{{ ... }}` (got `{tmpl}`)"
        ))
    })?;
    let header_part = tmpl[..brace].trim_end();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, _) = line.split_at(ws_len);
    lines[idx] = format!("{ws}{header_part} {{");
    Ok(lines.join("\n"))
}

/// WI-8 slice 1: splice a captured `struct` item under an
/// evolution-template header (`pub struct New { ... }`). The
/// template's visibility prefix and new ident replace the capture's
/// `struct <Old>` header span; generics, where-clauses, and fields
/// (named/unnamed/unit) ride byte-verbatim as the suffix after the
/// old ident.
fn splice_struct_header(
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
fn splice_enum_header(
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
fn splice_union_header(
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
fn splice_type_header(
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
fn splice_static_header(
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
fn static_line_ident(line: &str) -> Option<String> {
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
fn splice_trait_header(
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
fn trait_line_ident(line: &str) -> Option<String> {
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
fn splice_mod_header(
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
fn mod_line_ident(line: &str) -> Option<String> {
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
fn splice_const_header(
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
fn const_line_ident(line: &str) -> Option<String> {
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
fn type_line_ident(line: &str) -> Option<String> {
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
fn union_line_ident(line: &str) -> Option<String> {
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
fn enum_line_ident(line: &str) -> Option<String> {
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
fn struct_line_ident(line: &str) -> Option<String> {
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
/// The ident of a `fn <Ident>` header line (`None` when the line holds
/// no word-bounded `fn` + ident pair).
fn fn_line_ident(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..].starts_with(b"fn")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 2).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 2;
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
