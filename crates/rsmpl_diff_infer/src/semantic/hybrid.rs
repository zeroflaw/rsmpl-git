//! WI-4e hunk-level pre-pass inputs (D5 split from `semantic/mod.rs`).
//!
//! Later intent pre-passes skip `rule=` *hunks*, not whole sections, so
//! leftover native pairs in hybrid sections (e.g. move leftovers) can
//! still form intents. Pure-semantic sections stay skipped. Helpers are
//! pure text surgery: hunk texts keep exact `@@` headers, so
//! `changed_old_lines` sites on the native view are original sites by
//! construction.

/// Prelude (everything before the first `@@`) of a section.
pub(crate) fn section_prelude(sec: &str) -> String {
    let mut prelude = String::new();
    for line in sec.lines() {
        if line.starts_with("@@") {
            break;
        }
        prelude.push_str(line);
        prelude.push('\n');
    }
    prelude
}

/// Split a section into pre-existing semantic hunks (`rule=` `@@`
/// headers) and a native-only view (`prelude + native hunks verbatim`).
/// Returns `(prelude, semantic_hunks, native_view)`; `native_view` is
/// `None` when the section holds no native hunks (pure-semantic skip).
pub(crate) fn split_semantic_native(sec: &str) -> (String, Vec<String>, Option<String>) {
    let prelude = section_prelude(sec);
    let mut semantic: Vec<String> = Vec::new();
    let mut native = prelude.clone();
    let mut current: Option<(bool, String)> = None;
    for line in sec.lines() {
        if line.starts_with("@@") {
            if let Some((is_sem, text)) = current.take() {
                if is_sem {
                    semantic.push(text);
                } else {
                    native.push_str(&text);
                }
            }
            let is_sem = line.contains("rule=");
            let mut text = String::new();
            text.push_str(line);
            text.push('\n');
            current = Some((is_sem, text));
            continue;
        }
        if let Some((_, text)) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    if let Some((is_sem, text)) = current.take() {
        if is_sem {
            semantic.push(text);
        } else {
            native.push_str(&text);
        }
    }
    // No native hunk beyond the prelude: pure-semantic section.
    let has_native = native.len() > prelude.len();
    let native_view = has_native.then_some(native);
    (prelude, semantic, native_view)
}

/// Hunk texts of an emission (everything from each `@@` on), without
/// the `diff --git` header, for splicing after preserved pre-existing
/// semantic hunks in hybrid sections.
pub(crate) fn emission_hunks(emission: &str) -> Vec<String> {
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in emission.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        if let Some(h) = current.as_mut() {
            h.push_str(line);
            h.push('\n');
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    hunks
}
