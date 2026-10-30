//! Scalar capture store for code moves (SPEC 20).
//!
//! v0.1: captures hold exactly one AST object's source text. `# bind=<Name>`
//! is only supported on item-capture cut hunks; `# use=<Name>` only on pure
//! insertion hunks. Forward references are resolved by the two-pass planner
//! in `plan.rs` (SPEC 20.3, 23.2).

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};

use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// Patch-local capture map: name -> captured source text.
#[derive(Debug, Default)]
pub struct CaptureStore {
    map: HashMap<String, String>,
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
            return Err(StructuredError::new(
                ErrorCode::CaptureConflict,
                format!("duplicate `# bind={name}` (scalar captures must be unique)"),
            )
            .with_rule(rule)
            .with_file(file));
        }
        self.map.insert(name.to_string(), text);
        Ok(())
    }

    pub fn get(&self, name: &str, rule: &str, file: &str) -> Result<&str, StructuredError> {
        self.map.get(name).map(|s| s.as_str()).ok_or_else(|| {
            let mut known: Vec<&str> = self.map.keys().map(|s| s.as_str()).collect();
            known.sort();
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
        })
    }
}
/// Paste captured item text at a zero-length anchor (SPEC 10.5, 20.2).
/// Single pastes end with exactly one `\n`; multi-pastes are
/// blank-joined after pass 2 (SPEC 20.4) and materialize in document order
/// (SPEC 23.5: first hunk topmost).
/// WI-8a: what a `# use=` paste inserts — the bare capture name
/// (verbatim move) or a single-line `fn` header template (a single-line
/// `fn` header with `(...)` + `...`) that the captured body is spliced
/// under (evolution move).
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
    let text = captures.get(target.name, rule, file)?;
    let text = match target.template {
        Some(t) if t.trim() != target.name => splice_evolved_header(text, t, rule)?,
        _ => text.to_string(),
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

/// WI-8a: splice a captured `fn` item under an evolution-template
/// header (`pub fn New(...) { ... }`). The template's prefix before
/// `fn` (vis + qualifiers) and new ident replace the capture's header
/// line; every other line (multi-line params, body) stays byte-verbatim.
fn splice_evolved_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `fn` header with `(...)` + `...` (got `{}`)",
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
                "evolution paste needs a captured `fn` item in v0.1 (only fn headers evolve)"
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
