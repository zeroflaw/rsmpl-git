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
/// (verbatim move) or a header template (a single-line
/// `fn`/`struct`/`enum`/`union`/`type`/`const`/`static`/`mod`/`trait` header with `...`,
/// or the multi-line concrete-signature `fn` header with `...`, or the
/// WI-19 slice 3 joint `fn <Old> as <New>` header with a concrete
/// signature and `...`) that
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
        // WI-18: a collection paste is either the bare name (verbatim,
        // WI-6) or an evolution template carrying an `N`-hole
        // (`N`/`new_N`/`N_new`/`pre_N_post`); each item replays through
        // the template with `N` bound per item. Concrete hole-less
        // templates still fail loud (would duplicate one name).
        if let Some(t) = target.template {
            if t.trim() != target.name {
                return crate::capture_evolve::paste_evolving_collection(
                    texts, t, target, index, rule, file, edits,
                );
            }
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
        follow_skipped_details: Vec::new(),
        orphans: Vec::new(),
        semantics: RuleSemantics::ReparseOnly,
        symbol: None,
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
            .ok_or_else(|| super::plan_dispatch::range_err(rule, path, sl))?;
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
        follow_skipped_details: Vec::new(),
        orphans: Vec::new(),
        semantics: RuleSemantics::ReparseOnly,
        symbol: None,
    })
}

/// WI-8a + WI-8 slices 1-8 + WI-8 slice 9 + WI-6 slice 3b: splice a
/// captured item under an evolution-template header (`pub fn New(...) { ... }`,
/// `pub fn New(x: i64) -> i64 { ... }` (slice 9 concrete signature),
/// `pub fn Old as New(...) { ... }` (slice 3b body-evolving: header **and**
/// self-reference body rename), `pub fn Old as New(x: i64) -> i64 { ... }`
/// (WI-19 slice 3 signature+body joint: concrete header span **and**
/// self-reference body rename), `pub struct New { ... }`,
/// `pub enum New { ... }`,
/// `pub union New { ... }`, `pub type New = ...;`,
/// `pub const New = ...;`, `pub static [mut ]New = ...;`,
/// `pub mod New { ... }`, or `pub trait New { ... }`), or a WI-19 slice 4
/// container body-evolving header (`pub struct Old as New { ... }`,
/// `pub enum Old as New { ... }`, `pub union Old as New { ... }`). The
/// template's prefix and new ident replace the capture's header span
/// (one line for every form except the multi-line concrete-signature
/// `fn` template, whose span replaces the span);
/// every other line stays byte-verbatim — except the slice-3b `as` form
/// and the WI-19 slice 4 container `as` forms,
/// which additionally rename the captured body's self-references.
/// Returns the spliced item text.
pub(crate) fn splice_evolved_header(
    captured: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| StructuredError::new(ErrorCode::PatternConflict, msg).with_rule(rule);
    // WI-19 slice 3: the signature+body joint form is checked before the
    // `as` and concrete arms (its spelling contains `as` plus a concrete
    // signature, which each single-purpose parser declines by ownership).
    if let Some((new_prefix, old_t, new_t)) =
        rsmpl_pattern::parse_fn_sig_body_template(template.trim())
    {
        return splice_fn_sig_body_evolution(captured, &new_prefix, &old_t, &new_t, template, rule);
    }
    // WI-6 slice 3b: the `as` body-evolving form is checked before the
    // abstract `(...)` form (its tail also contains `(...)`+`...`, but the
    // abstract parser declines the `as` spelling by ownership).
    if let Some((new_prefix, old_t, new_t)) =
        rsmpl_pattern::parse_fn_body_evolution_template(template.trim())
    {
        return splice_fn_body_evolution(captured, &new_prefix, &old_t, &new_t, rule);
    }
    // WI-19 slice 4: the container (`struct`/`enum`/`union`) `as`
    // body-evolving forms are checked before their abstract forms (the
    // abstract parsers decline the `as` spelling by ownership; the
    // splices live in the `capture_container_body` sibling, D5).
    if let Some((new_prefix, old_t, new_t)) =
        rsmpl_pattern::parse_struct_body_evolution_template(template.trim())
    {
        return crate::capture_container_body::splice_struct_body_evolution(
            captured,
            &new_prefix,
            &old_t,
            &new_t,
            rule,
        );
    }
    if let Some((new_prefix, old_t, new_t)) =
        rsmpl_pattern::parse_enum_body_evolution_template(template.trim())
    {
        return crate::capture_container_body::splice_enum_body_evolution(
            captured,
            &new_prefix,
            &old_t,
            &new_t,
            rule,
        );
    }
    if let Some((new_prefix, old_t, new_t)) =
        rsmpl_pattern::parse_union_body_evolution_template(template.trim())
    {
        return crate::capture_container_body::splice_union_body_evolution(
            captured,
            &new_prefix,
            &old_t,
            &new_t,
            rule,
        );
    }
    // WI-19 residue: the inherent-`impl` `as` body-evolving form is
    // checked before the abstract shapes (no abstract `impl` evolution
    // shape exists, so no ownership conflict).
    if let Some((new_prefix, old_t, new_t)) =
        rsmpl_pattern::parse_impl_body_evolution_template(template.trim())
    {
        return crate::capture_impl_body::splice_impl_body_evolution(
            captured,
            &new_prefix,
            &old_t,
            &new_t,
            rule,
        );
    }
    // WI-8 slice 9: the concrete-signature `fn` template carries the new
    // signature verbatim — checked before the abstract `(...)` form (a
    // concrete template never contains `(...)`, so the two shapes are
    // disjoint, but the concrete splice replaces the whole header line
    // while the abstract splice keeps the captured suffix).
    if rsmpl_pattern::parse_fn_full_header_template(template.trim()).is_some() {
        return splice_fn_full_header(captured, template, rule);
    }
    // WI-19 slice 4 (D5): kind-specific header splices live in the
    // `capture_container_body` sibling; the dispatch below only routes.
    if rsmpl_pattern::parse_trait_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_trait_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_mod_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_mod_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_static_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_static_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_const_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_const_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_type_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_type_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_union_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_union_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_enum_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_enum_header(captured, template, rule);
    }
    if rsmpl_pattern::parse_struct_evolution_template(template.trim()).is_some() {
        return crate::capture_container_body::splice_struct_header(captured, template, rule);
    }
    let (new_prefix, new_name) =
        rsmpl_pattern::parse_evolution_template(template.trim()).ok_or_else(|| {
            fail(format!(
                "evolution paste template must be a single-line `fn` header with `(...)` + `...`, `fn <Old> as <New>(...) {{ ... }}` for a self-reference body rename, a concrete signature + `{{ ... }}`, or `fn <Old> as <New>(concrete signature) {{ ... }}` for a signature+body joint (single- or multi-line, got `{}`), a single-line `struct` header with `...` (or `struct <Old> as <New> {{ ... }}` for a self-reference body rename), a single-line `enum` header with `...` (or `enum <Old> as <New> {{ ... }}`), a single-line `union` header with `...` (or `union <Old> as <New> {{ ... }}`), `impl <Old> as <New> {{ ... }}` for an inherent-`impl` self-ty rename, a single-line `type` header with `= ...;`, a single-line `const` header with `= ...;`, a single-line `static` header with `= ...;`, a single-line `mod` header with `...`, or a single-line `trait` header with `...`",
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

/// WI-6 slice 3b: splice a captured `fn` item under a body-evolving
/// template header (`pub fn Old as New(...) { ... }`). The header splices
/// abstract-style (new prefix + new name, captured params/return/where
/// suffix verbatim) and every body line renames `old` → `new` through
/// [`rsmpl_pattern::rename_ident_in_code`] (strings, chars, and comments
/// ride byte-verbatim). The template `old` must equal the captured fn
/// ident — a mismatch fails loud, never guesses. Whole-text (not
/// per-line) renaming keeps multi-line block comments sound.
fn splice_fn_body_evolution(
    captured: &str,
    new_prefix: &str,
    old_t: &str,
    new_t: &str,
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
                "evolution paste needs a captured `fn` item for a `fn <Old> as <New>` template"
                    .to_string(),
            ));
        }
    };
    if old_name != old_t {
        return Err(fail(format!(
            "body-evolving paste names `{old_t}` but the capture holds `fn {old_name}`"
        )));
    }
    // WI-2: binding-aware boundary — a shadowing local, field/method
    // name, macro name, or attribute interior named like the old ident
    // vetoes the authored paste (rename by hand or split the move).
    // Lifetimes/strings/comments ride verbatim with no veto.
    if let Some(veto) = rsmpl_pattern::body_evolution_veto(captured, &old_name) {
        return Err(fail(format!(
            "body-evolving paste vetoes: `{old_name}` has a {} occurrence at captured line {} (rename it by hand or split the move)",
            veto.kind, veto.line
        )));
    }
    // First `fn <Old>` header line (same word-bounded search as the
    // abstract splice; doc/comment lines never evolve).
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
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
    // WI-19 slice-4 review: the header suffix renames in place (same
    // scanner as the body), so a same-line self-reference
    // (`fn fact() -> i32 { fact() }`) rides along like the container
    // splices. The WI-2 veto above already declines shadowing bindings.
    let renamed_suffix = rsmpl_pattern::rename_ident_in_code(&suffix, &old_name, new_t);
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("fn {new_t}")
    } else {
        format!("{pre} fn {new_t}")
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

/// WI-19 slice 3: splice a captured `fn` item under a signature+body
/// joint template (`pub fn Old as New(x: i64) -> i64 { ... }`, single-
/// or multi-line like the concrete-signature form). The captured header
/// *span* is replaced with the template's header span (the new name and
/// signature ride verbatim from the template) and the captured body
/// tail is renamed `old` → `new` through
/// [`rsmpl_pattern::rename_ident_in_code`]. The WI-2 binding boundary
/// applies: a shadowing local, field/method name, macro name, or
/// attribute interior vetoes loud instead of renaming. Single-line
/// items fail loud (replacing the only line would drop the body).
#[allow(clippy::too_many_arguments)]
fn splice_fn_sig_body_evolution(
    captured: &str,
    _new_prefix: &str,
    old_t: &str,
    new_t: &str,
    template: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| {
        StructuredError::new(ErrorCode::PatternConflict, msg)
            .with_rule(rule)
            .with_repair(StructuredError::fix_pattern_repair(
                "joint-paste",
                "write the joint template `+ <prefix>fn <Old> as <New>(concrete signature) { ... }` (exactly one signature slot, WI-2 binding law), or split the move",
            ))
    };
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name = match file.items.as_slice() {
        [syn::Item::Fn(f)] => f.sig.ident.to_string(),
        _ => {
            return Err(fail(
                "evolution paste needs a captured `fn` item for a signature+body joint template"
                    .to_string(),
            ));
        }
    };
    if old_name != old_t {
        return Err(fail(format!(
            "signature+body joint paste names `{old_t}` but the capture holds `fn {old_name}`"
        )));
    }
    if let Some(veto) = rsmpl_pattern::body_evolution_veto(captured, &old_name) {
        return Err(fail(format!(
            "signature+body joint paste vetoes: `{old_name}` has a {} occurrence at captured line {} (rename it by hand or split the move)",
            veto.kind, veto.line
        )));
    }
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
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
    let mut cap_end = None;
    for (off, line) in lines[idx..].iter().enumerate() {
        if line.trim_end().ends_with('{') {
            cap_end = Some(idx + off);
            break;
        }
    }
    let Some(cap_end) = cap_end else {
        return Err(fail(format!(
            "signature+body joint paste needs a multi-line `fn {old_name}` capture (single-line items stay ordinary)"
        )));
    };
    if cap_end + 1 >= lines.len() {
        return Err(fail(format!(
            "signature+body joint paste needs a multi-line `fn {old_name}` capture (single-line items stay ordinary)"
        )));
    }
    let tmpl = template.trim();
    let tmpl_lines: Vec<&str> = tmpl.lines().collect();
    let mut tmpl_end = None;
    for (off, line) in tmpl_lines.iter().enumerate() {
        if line.contains('{') && line.contains("...") {
            tmpl_end = Some(off);
            break;
        }
    }
    let tmpl_end = tmpl_end.ok_or_else(|| {
        fail(format!(
            "evolution paste template must be a joint `fn <Old> as <New>` header with a concrete signature + `{{ ... }}` (got `{tmpl}`)"
        ))
    })?;
    if tmpl_end + 1 != tmpl_lines.len() {
        return Err(fail(
            "evolution paste template must end at the `{ ... }` line (got trailing lines after it)"
                .to_string(),
        ));
    }
    let brace = tmpl.find('{').ok_or_else(|| {
        fail(format!(
            "evolution paste template must be a joint `fn <Old> as <New>` header with a concrete signature + `{{ ... }}` (got `{tmpl}`)"
        ))
    })?;
    let header_part = tmpl[..brace].trim_end();
    let mut span: Vec<String> = header_part.lines().map(str::to_string).collect();
    if let Some(last) = span.last_mut() {
        *last = format!("{} {{", last.trim_end());
    }
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, _) = line.split_at(ws_len);
    if span.is_empty() {
        return Err(fail(
            "signature+body joint paste needs a header span (got an empty template header)"
                .to_string(),
        ));
    }
    // The template spells `fn <old> as <new>`; the spliced header keeps
    // only the new name (`fn <new>`), like every other evolution paste.
    let raw_first = span[0].trim_start().to_string();
    let fn_at = raw_first.find("fn ").ok_or_else(|| {
        fail(format!(
            "signature+body joint paste template holds no `fn {old_t}` header line"
        ))
    })?;
    let after_fn = raw_first[fn_at + 3..].trim_start();
    let after_old = after_fn.strip_prefix(old_t).ok_or_else(|| {
        fail(format!(
            "signature+body joint paste template holds no `fn {old_t}` header line"
        ))
    })?;
    let after_ws = after_old.trim_start();
    let after_as = after_ws.strip_prefix("as").ok_or_else(|| {
        fail(format!(
            "signature+body joint paste template holds no `fn {old_t} as {new_t}` header line"
        ))
    })?;
    if after_as
        .chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        return Err(fail(format!(
            "signature+body joint paste template holds no `fn {old_t} as {new_t}` header line"
        )));
    }
    let after_new = after_as.trim_start();
    let new_tail = after_new.strip_prefix(new_t).ok_or_else(|| {
        fail(format!(
            "signature+body joint paste template holds no `fn {old_t} as {new_t}` header line"
        ))
    })?;
    span[0] = format!("{ws}{}fn {new_t}{new_tail}", &raw_first[..fn_at]);
    lines.splice(idx..=cap_end, span);
    // Body rename over the post-span tail (one scanner pass, so
    // multi-line block comments stay sound); the replaced header span
    // is excluded. `cap_end` indexed the pre-splice lines; recompute
    // the tail start from the new span length.
    let tail_start = idx + tmpl_end + 1;
    if tail_start < lines.len() {
        let body = lines[tail_start..].join("\n");
        let renamed = rsmpl_pattern::rename_ident_in_code(&body, &old_name, new_t);
        let mut new_lines: Vec<String> = renamed.split('\n').map(str::to_string).collect();
        lines.truncate(tail_start);
        lines.append(&mut new_lines);
    }
    Ok(lines.join("\n"))
}

/// WI-8 slice 9 (+ WI-19 slice 2 multi-line): splice a captured `fn`
/// item under a concrete-signature evolution-template header
/// (`pub fn New(x: i64) -> i64 { ... }`, or the multi-line plus
/// header verbatim with only the `{ ... }` marker closing it). The
/// captured header *span* (first `fn <Old>` code line through the
/// first `{`-ended line) is replaced with the template's header span
/// — the new signature rides verbatim from the plus side, never from
/// the capture. Attr, doc, and body lines ride byte-verbatim.
/// Single-line items fail loud: replacing the only line would drop
/// the body.
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
    // The captured header spans from the `fn <Old>` line through the
    // first line whose trimmed end opens the block (`... {`): a
    // multi-line header's trailing signature lines are part of the
    // span, so no stale line survives the splice. A single-line item
    // (no body lines after the span) stays loud, as before.
    let mut cap_end = None;
    for (off, line) in lines[idx..].iter().enumerate() {
        if line.trim_end().ends_with('{') {
            cap_end = Some(idx + off);
            break;
        }
    }
    // A single-line item (`fn old(x: i32) { x }`) has no `{`-opened
    // header line and no body lines to ride — it stays loud, as
    // before. (A parsed `fn` item always opens its block somewhere,
    // so a multi-line capture always finds its span above.)
    let Some(cap_end) = cap_end else {
        return Err(fail(format!(
            "concrete-signature evolution paste needs a multi-line `fn {old_name}` capture (single-line items stay ordinary)"
        )));
    };
    if cap_end + 1 >= lines.len() {
        return Err(fail(format!(
            "concrete-signature evolution paste needs a multi-line `fn {old_name}` capture (single-line items stay ordinary)"
        )));
    }
    // The template header span (first line through the `{ ... }`
    // line) replaces the captured span. The signature never contains
    // braces, so the first `{` — which opens the `{ ... }` marker —
    // ends the header part; everything after it is the marker.
    let tmpl = template.trim();
    let tmpl_lines: Vec<&str> = tmpl.lines().collect();
    let mut tmpl_end = None;
    for (off, line) in tmpl_lines.iter().enumerate() {
        if line.contains('{') && line.contains("...") {
            tmpl_end = Some(off);
            break;
        }
    }
    let tmpl_end = tmpl_end.ok_or_else(|| {
        fail(format!(
            "evolution paste template must be an `fn` header with a concrete signature + `{{ ... }}` (got `{tmpl}`)"
        ))
    })?;
    if tmpl_end + 1 != tmpl_lines.len() {
        return Err(fail(
            "evolution paste template must end at the `{ ... }` line (got trailing lines after it)"
                .to_string(),
        ));
    }
    let brace = tmpl.find('{').ok_or_else(|| {
        fail(format!(
            "evolution paste template must be an `fn` header with a concrete signature + `{{ ... }}` (got `{tmpl}`)"
        ))
    })?;
    let header_part = tmpl[..brace].trim_end();
    // Rebuild the replacement span: the header part's own lines, with
    // the first line re-indented to the capture's indent and the last
    // line closed as `{`.
    let mut span: Vec<String> = header_part.lines().map(str::to_string).collect();
    if let Some(last) = span.last_mut() {
        *last = format!("{} {{", last.trim_end());
    }
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, _) = line.split_at(ws_len);
    if let Some(first) = span.first_mut() {
        *first = format!("{ws}{}", first.trim_start());
    }
    lines.splice(idx..=cap_end, span);
    Ok(lines.join("\n"))
}

// WI-19 slice 4 (D5 split): kind-specific header splices live in
// capture_container_body.rs; behavior unchanged.

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

#[cfg(test)]
mod tests {
    use super::*;

    const FACT: &str = "fn fact(n: u64) -> u64 {\n    if n <= 1 {\n        return 1;\n    }\n    n * fact(n - 1)\n}";

    #[test]
    fn body_evolution_splice_renames_header_and_self_ref() {
        let out = splice_evolved_header(FACT, "fn fact as factorial(...) { ... }", "r")
            .expect("as splice must fire");
        assert!(out.contains("fn factorial(n: u64) -> u64 {"), "got:\n{out}");
        assert!(out.contains("n * factorial(n - 1)"), "got:\n{out}");
        assert!(
            !out.contains("n * fact(n - 1)"),
            "old call gone, got:\n{out}"
        );
    }

    #[test]
    fn body_evolution_splice_keeps_strings_and_comments() {
        let cap =
            "fn fact(n: u64) -> u64 {\n    let s = \"fact\";\n    // fact\n    n * fact(n - 1)\n}";
        let out = splice_evolved_header(cap, "fn fact as factorial(...) { ... }", "r")
            .expect("as splice must fire");
        assert!(out.contains("\"fact\""), "string kept, got:\n{out}");
        assert!(out.contains("// fact"), "comment kept, got:\n{out}");
        assert!(
            out.contains("n * factorial(n - 1)"),
            "call renamed, got:\n{out}"
        );
    }

    #[test]
    fn body_evolution_splice_renames_same_line_self_ref() {
        // WI-19 slice-4 review gap: the pairing proves whole-text
        // agreement, so a same-line self-reference must ride along.
        // Before the header-suffix rename this emitted
        // `fn factorial() -> i32 { fact() }` (stale self-ref).
        let cap = "fn fact() -> i32 { fact() }";
        let out = splice_evolved_header(cap, "fn fact as factorial(...) { ... }", "r")
            .expect("single-line as splice must fire");
        assert_eq!(out, "fn factorial() -> i32 { factorial() }", "got:\n{out}");
    }

    #[test]
    fn body_evolution_splice_single_line_keeps_string() {
        // Same-line scanner discipline, not regex: literals ride
        // verbatim while the code ident renames.
        let cap = "fn fact() -> i32 { let s = \"fact\"; fact() }";
        let out = splice_evolved_header(cap, "fn fact as factorial(...) { ... }", "r")
            .expect("single-line as splice must fire");
        assert!(out.contains("\"fact\""), "string kept, got:\n{out}");
        assert!(out.contains("factorial()"), "call renamed, got:\n{out}");
    }

    #[test]
    fn body_evolution_old_mismatch_fails_loud() {
        let err = splice_evolved_header(FACT, "fn other as factorial(...) { ... }", "r")
            .expect_err("old mismatch must fail");
        assert!(err.to_string().contains("other"), "got: {err}");
    }

    #[test]
    fn abstract_splice_stays_verbatim_on_self_ref() {
        // The old spelling never renames: a stale self-ref rides verbatim.
        let out = splice_evolved_header(FACT, "fn factorial(...) { ... }", "r")
            .expect("abstract splice must fire");
        assert!(out.contains("fn factorial(n: u64) -> u64 {"), "got:\n{out}");
        assert!(
            out.contains("n * fact(n - 1)"),
            "stale call kept, got:\n{out}"
        );
    }

    #[test]
    fn body_evolution_splice_vetoes_binding_risks() {
        // WI-2: shadowed locals, fields, and macros veto the authored
        // paste with the occurrence line named.
        for (cap, kind) in [
            (
                "fn fact() -> i32 {\n    let fact = 1;\n    fact + fact(2)\n}",
                "shadowed-local",
            ),
            ("fn fact(s: S) -> i32 {\n    s.fact + fact(1)\n}", "field"),
            (
                "fn fact(n: i32) -> i32 {\n    fact!(n) + fact(n)\n}",
                "macro",
            ),
        ] {
            let err = splice_evolved_header(cap, "fn fact as factorial(...) { ... }", "r")
                .expect_err("binding risk must veto");
            let msg = err.to_string();
            assert!(msg.contains(kind), "kind {kind} named, got: {msg}");
            assert!(msg.contains("captured line 2"), "line named, got: {msg}");
        }
    }

    #[test]
    fn body_evolution_splice_keeps_lifetimes() {
        // WI-2: lifetimes ride verbatim while the self-call renames.
        let cap = "fn fact<'fact>(x: &'fact str) -> &'fact str {\n    fact(x)\n}";
        let out = splice_evolved_header(cap, "fn fact as factorial(...) { ... }", "r")
            .expect("lifetime splice must fire");
        assert!(out.contains("'fact"), "lifetime kept, got:\n{out}");
        assert!(
            !out.contains("'factorial"),
            "lifetime not renamed, got:\n{out}"
        );
        assert!(out.contains("factorial(x)"), "call renamed, got:\n{out}");
    }

    #[test]
    fn joint_splice_replaces_span_and_renames_body() {
        // WI-19 slice 3: the joint template replaces the whole header
        // span with the new signature and renames the body self-ref.
        let cap = "fn fact(n: u32) -> u64 {\n    n as u64 * fact(n - 1)\n}";
        let out =
            splice_evolved_header(cap, "pub fn fact as factorial(n: u64) -> u64 { ... }", "r")
                .expect("joint splice must fire");
        assert!(
            out.contains("pub fn factorial(n: u64) -> u64 {"),
            "new header spliced, got:\n{out}"
        );
        assert!(
            out.contains("n as u64 * factorial(n - 1)"),
            "call renamed, got:\n{out}"
        );
        assert!(!out.contains("fact("), "old call gone, got:\n{out}");
    }

    #[test]
    fn joint_splice_multiline_header() {
        // A multi-line joint template replaces the whole captured span.
        let cap = "fn fact(\n    n: u32,\n) -> u64 {\n    n as u64 * fact(n - 1)\n}";
        let out = splice_evolved_header(
            cap,
            "pub fn fact as factorial(\n    n: u64,\n) -> u64 { ... }",
            "r",
        )
        .expect("multi-line joint splice must fire");
        assert!(
            out.contains("pub fn factorial(\n    n: u64,\n) -> u64 {"),
            "multi-line header spliced, got:\n{out}"
        );
        assert!(
            out.contains("factorial(n - 1)"),
            "call renamed, got:\n{out}"
        );
    }

    #[test]
    fn joint_splice_vetoes_binding_risks() {
        // WI-2 boundary shared with the `as` form: a shadowed local
        // vetoes the authored joint paste with the line named.
        let cap = "fn fact(n: u32) -> u64 {\n    let fact = 1;\n    fact + fact(n - 1)\n}";
        let err = splice_evolved_header(cap, "fn fact as factorial(n: u64) -> u64 { ... }", "r")
            .expect_err("binding risk must veto");
        let msg = err.to_string();
        assert!(msg.contains("shadowed-local"), "kind named, got: {msg}");
        assert!(msg.contains("captured line 2"), "line named, got: {msg}");
    }
}
