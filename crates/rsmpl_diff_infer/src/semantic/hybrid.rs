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

/// One native hunk with verbatim text (WI-4f-next hunk-level claiming).
pub(crate) struct NativeHunk {
    pub(crate) text: String,
    pub(crate) minuses: Vec<String>,
    pub(crate) pluses: Vec<String>,
    pub(crate) sites: Vec<usize>,
    pub(crate) balanced: bool,
}

/// Split a native-only section view (`prelude + native hunks verbatim`,
/// as produced by `split_semantic_native`) into per-hunk entries with
/// verbatim text. Sites are computed per single-hunk slice, so they stay
/// original sites by construction. Unbalanced hunks are returned with
/// `balanced == false` and empty `sites` (callers decide: leftover vs
/// veto).
pub(crate) fn split_native_hunks(native_sec: &str) -> Vec<NativeHunk> {
    let mut out: Vec<NativeHunk> = Vec::new();
    let mut header: Option<String> = None;
    let mut body: Vec<String> = Vec::new();
    let mut minuses: Vec<String> = Vec::new();
    let mut pluses: Vec<String> = Vec::new();
    let flush = |header: &mut Option<String>,
                 body: &mut Vec<String>,
                 minuses: &mut Vec<String>,
                 pluses: &mut Vec<String>,
                 out: &mut Vec<NativeHunk>| {
        if let Some(h) = header.take() {
            let mut text = String::new();
            text.push_str(&h);
            text.push('\n');
            for l in body.iter() {
                text.push_str(l);
                text.push('\n');
            }
            let balanced = !minuses.is_empty() && minuses.len() == pluses.len();
            let sites = if balanced {
                crate::totality::changed_old_lines(&text)
            } else {
                Vec::new()
            };
            out.push(NativeHunk {
                text,
                minuses: std::mem::take(minuses),
                pluses: std::mem::take(pluses),
                sites,
                balanced,
            });
            body.clear();
        }
    };
    for line in native_sec.lines().skip(1) {
        if line.starts_with("@@") {
            flush(&mut header, &mut body, &mut minuses, &mut pluses, &mut out);
            header = Some(line.to_string());
            continue;
        }
        if header.is_none() {
            continue;
        }
        body.push(line.to_string());
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(line[1..].to_string());
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(line[1..].to_string());
        }
    }
    flush(&mut header, &mut body, &mut minuses, &mut pluses, &mut out);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_native_hunks_partitions_verbatim() {
        let native = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -10,1 +10,1 @@\n-let x = 1;\n+let x = 2;\n";
        let hunks = split_native_hunks(native);
        assert_eq!(hunks.len(), 2);
        assert!(hunks[0].balanced);
        assert_eq!(hunks[0].sites, vec![1]);
        assert!(hunks[0].text.contains("-struct Old {"));
        assert!(hunks[1].text.contains("-let x = 1;"));
        // Unbalanced hunk: leftover, no sites.
        let mixed = "diff --git a/f.rs b/f.rs\n--- a/f.rs\n+++ b/f.rs\n@@ -1,1 +1,1 @@\n-struct Old {\n+struct New {\n@@ -5,2 +5,1 @@\n-a\n-b\n+c\n";
        let hunks2 = split_native_hunks(mixed);
        assert_eq!(hunks2.len(), 2);
        assert!(hunks2[0].balanced);
        assert!(!hunks2[1].balanced);
        assert!(hunks2[1].sites.is_empty());
    }
}
