//! Pure inference for the inverse mapping (`diffrs`).
//!
//! Turns a native unified diff into RSmPL-Git. v0 is ordinary-hunk
//! passthrough (no semantic compression — T341 plugs rename and
//! call-sweep detection behind the same signature): binary file
//! sections are stripped (a patch is UTF-8; `applyrs` cannot read
//! binary worktree files) and the remainder is validated by parsing
//! it as RSmPL-Git, so every `diffrs` output is applyrs-consumable
//! by construction.

#[derive(Debug, thiserror::Error)]
pub enum InferError {
    #[error("native diff is not valid RSmPL-Git: {0}")]
    Unparseable(String),
}

/// The v0 inference result: the RSmPL patch plus which binary paths
/// were stripped (the binary reports them as `warning:` lines).
pub struct InferredPatch {
    pub rsmpl: String,
    pub stripped_binary: Vec<String>,
}

pub fn infer_rsmpl(raw: &str) -> Result<InferredPatch, InferError> {
    // No changes is the legitimate empty shape here (the binary, not
    // infer, decides how to report it); an empty patch is only an
    // authoring typo on the `applyrs` side (T59).
    if raw.trim().is_empty() {
        return Ok(InferredPatch {
            rsmpl: String::new(),
            stripped_binary: Vec::new(),
        });
    }
    let (kept, stripped) = strip_binary_sections(raw);
    let candidate = kept.join("");
    rsmpl_patch::parse_patch(&candidate)
        .map_err(|e| InferError::Unparseable(describe_parse_error(&e)))?;
    Ok(InferredPatch {
        rsmpl: candidate,
        stripped_binary: stripped,
    })
}

fn describe_parse_error(e: &rsmpl_patch::ParseError) -> String {
    match e {
        rsmpl_patch::ParseError::Structured(s) => s.render(),
        rsmpl_patch::ParseError::Message(m) => m.clone(),
    }
}

/// Split `raw` into `diff --git` sections; drop sections holding a
/// `Binary files … differ` line. Returns (kept sections, stripped
/// `b/`-side paths).
fn strip_binary_sections(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut stripped = Vec::new();
    // `raw` always ends sections with `\n` (git output); a missing
    // trailing section is still flushed below.
    let mut current = String::new();
    let mut current_path: Option<String> = None;
    let mut current_binary = false;
    let mut seen_any = false;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if seen_any {
                flush(
                    &mut kept,
                    &mut stripped,
                    &mut current,
                    &current_path,
                    current_binary,
                );
            }
            seen_any = true;
            current_binary = false;
            current_path = b_side_path(rest);
        } else if line.starts_with("Binary files ") {
            current_binary = true;
        }
        if seen_any {
            current.push_str(line);
            current.push('\n');
        }
    }
    if seen_any {
        flush(
            &mut kept,
            &mut stripped,
            &mut current,
            &current_path,
            current_binary,
        );
    }
    (kept, stripped)
}

fn flush(
    kept: &mut Vec<String>,
    stripped: &mut Vec<String>,
    current: &mut String,
    path: &Option<String>,
    binary: bool,
) {
    if binary {
        if let Some(p) = path {
            stripped.push(p.clone());
        }
    } else {
        kept.push(std::mem::take(current));
        return;
    }
    current.clear();
}

fn b_side_path(rest: &str) -> Option<String> {
    let mut parts = rest.splitn(2, ' ');
    parts.nth(1).map(|b| {
        b.strip_prefix("b/")
            .unwrap_or(b)
            .trim_matches('"')
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_SECTION: &str = "diff --git a/f.rs b/f.rs\nindex 1..2 100644\n--- a/f.rs\n+++ b/f.rs\n@@ -1 +1 @@\n-a\n+b\n";
    const BINARY_SECTION: &str = "diff --git a/img.bin b/img.bin\nindex 1..2 100644\nBinary files a/img.bin and b/img.bin differ\n";

    #[test]
    fn text_only_passes_through_validated() {
        let out = infer_rsmpl(TEXT_SECTION).unwrap();
        assert_eq!(out.rsmpl, TEXT_SECTION);
        assert!(out.stripped_binary.is_empty());
    }

    #[test]
    fn binary_section_is_stripped_and_named() {
        let raw = format!("{TEXT_SECTION}{BINARY_SECTION}");
        let out = infer_rsmpl(&raw).unwrap();
        assert_eq!(out.rsmpl, TEXT_SECTION);
        assert_eq!(out.stripped_binary, vec!["img.bin"]);
    }

    #[test]
    fn empty_diff_is_an_empty_patch() {
        // Empty input is an authoring typo for `applyrs` but the
        // legitimate no-changes shape here; the binary (not infer)
        // decides how to report it.
        let out = infer_rsmpl("").unwrap();
        assert!(out.rsmpl.is_empty());
        assert!(out.stripped_binary.is_empty());
    }
}
