//! WI-16: the one T361 loop, shared by the CLI and the tests.
//!
//! `verify` scratch-applies a raised RSmPL patch to an in-memory copy of
//! the pre-image and byte-compares it against the true post-tree derived
//! by applying the native diff to the same pre-image. Lowering
//! validation alone (`applyrs --check`: parse + lower + `git apply
//! --check`) cannot see sweep over-match — the lowered diff applies
//! cleanly while rewriting sites the native diff never touched — so a
//! verify command that reused it would be true by construction (D2).
//!
//! Boundary: this proves changed-files closure (the T361 class:
//! over-match *within* touched files). Off-diff same-name uses living
//! in untouched files stay the engine's apply-time veto backstop — the
//! same context infer's own totality probes run in (they too see only
//! the changed-files pre-image map).

use std::collections::{HashMap, HashSet};

/// First byte divergence between the true post-tree and the raised-post
/// tree. `want` is the true-post line, `got` the raised-post line (`""`
/// when that side is shorter); both capped at 240 chars so one long
/// generated line cannot flood the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstDivergence {
    pub file: String,
    pub line: usize,
    pub want: String,
    pub got: String,
}

/// The T361 verdict. `files_compared` counts changed text files both
/// sides derived (binary-marker sections excluded, underivable files
/// skipped — a missing base blob cannot prove anything, and the CLI
/// already warns per path for the staged/rev case via WI-10a).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    pub verified: bool,
    pub files_compared: usize,
    pub first_divergence: Option<FirstDivergence>,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("native patch text is not a parseable diff: {0}")]
    NativeParse(String),
    #[error("raised patch does not apply to the pre-image: {0}")]
    Raise(String),
}

const LINE_CAP: usize = 240;

fn cap_line(s: &str) -> String {
    if s.len() <= LINE_CAP {
        return s.to_string();
    }
    // Char-boundary safe truncation with an ellipsis marker.
    let mut end = LINE_CAP;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Scratch-apply `rsmpl` to a copy of `pre` and byte-compare against the
/// native-applied true post-tree. `pre` maps repo-relative paths to
/// pre-image text for every changed text file (absent ≡ empty covers
/// new files and deletions on both sides).
pub fn verify(
    pre: &HashMap<String, String>,
    raw_native: &str,
    rsmpl: &str,
) -> Result<VerifyReport, VerifyError> {
    let native = apply_native(pre, raw_native)?;
    if native.files.is_empty() {
        // Nothing text-bearing to prove (binary-only or empty change):
        // vacuous truth without parsing the raise (a binary-only change
        // raises to the empty patch, which `parse_patch` rejects as an
        // authoring typo — that rejection is the `applyrs` input side,
        // not a verdict on this raise).
        return Ok(VerifyReport {
            verified: true,
            files_compared: 0,
            first_divergence: None,
        });
    }
    let patch = rsmpl_patch::parse_patch(rsmpl)
        .map_err(|e| VerifyError::Raise(describe_parse_error(&e)))?;
    let mut raised = pre.clone();
    // New files have no pre entry; seed them so renames (`new -> old`)
    // and the union below stay total without depending on engine
    // missing-target leniency.
    for section in &patch.files {
        let rel = section.target_path.clone();
        raised.entry(rel).or_default();
    }
    rsmpl_engine::apply_patch_to_tree(&mut raised, &patch)
        .map_err(|e| VerifyError::Raise(e.to_string()))?;
    let mut files: Vec<&String> = native.files.iter().collect();
    files.sort();
    let mut compared = 0usize;
    for path in files {
        let want = native.post.get(path).map(String::as_str).unwrap_or("");
        let got = raised.get(path).map(String::as_str).unwrap_or("");
        if want == got {
            compared += 1;
            continue;
        }
        return Ok(VerifyReport {
            verified: false,
            files_compared: compared + 1,
            first_divergence: Some(first_line_divergence(path, want, got)),
        });
    }
    Ok(VerifyReport {
        verified: true,
        files_compared: compared,
        first_divergence: None,
    })
}

fn first_line_divergence(file: &str, want: &str, got: &str) -> FirstDivergence {
    let want_lines = split_lines(want);
    let got_lines = split_lines(got);
    let n = want_lines.len().max(got_lines.len());
    for (i, (w, g)) in want_lines
        .iter()
        .chain(std::iter::repeat(&""))
        .zip(got_lines.iter().chain(std::iter::repeat(&"")))
        .take(n)
        .enumerate()
    {
        if w != g {
            return FirstDivergence {
                file: file.to_string(),
                line: i + 1,
                want: cap_line(w),
                got: cap_line(g),
            };
        }
    }
    // Only reachable when the texts differ solely in the trailing
    // newline (line vectors compare equal): name the terminator
    // position explicitly instead of fabricating a line-1 claim.
    FirstDivergence {
        file: file.to_string(),
        line: n.max(1),
        want: if want.ends_with('\n') {
            "<newline>".to_string()
        } else {
            "<no newline>".to_string()
        },
        got: if got.ends_with('\n') {
            "<newline>".to_string()
        } else {
            "<no newline>".to_string()
        },
    }
}

/// Split text into lines without normalizing `\r` (CRLF rides the body
/// lines; T385): trailing-`\n` state is irrelevant here since both sides
/// split identically and unequal trailing state already implies an
/// unequal final element (`""` vs content).
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

struct NativePost {
    /// Repo-relative paths whose post text is derived (text files the
    /// native raw touches, minus binary-marker sections).
    files: HashSet<String>,
    post: HashMap<String, String>,
}

/// Apply a native unified diff (`git diff -U0` shape, two-path headers,
/// `index` lines, hunk titles, plus the synthesized new-file sections)
/// to `pre`. Returns the true post-tree for every touched text file.
/// Files with no pre entry that are not new-file sections are skipped
/// (missing base blob — the staged/rev CLI path already warns per path
/// via WI-10a), hence uncounted in `files_compared`.
fn apply_native(pre: &HashMap<String, String>, raw: &str) -> Result<NativePost, VerifyError> {
    let mut out = NativePost {
        files: HashSet::new(),
        post: HashMap::new(),
    };
    if raw.trim().is_empty() {
        return Ok(out);
    }
    for section in split_sections(raw) {
        apply_section(pre, section, &mut out)?;
    }
    Ok(out)
}

/// Split raw diff text on line-start `diff --git ` (byte-level, so the
/// split itself never depends on decoding).
fn split_sections(raw: &str) -> Vec<&str> {
    let mut starts = vec![0usize];
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // `i + 1` stays on the byte after `\n`: a char boundary, so the
        // `starts_with` below cannot panic; slicing `[i+1..]` is always
        // in bounds (`i < len`).
        if bytes[i] == b'\n' && raw[i + 1..].starts_with("diff --git ") {
            starts.push(i + 1);
        }
        i += 1;
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, s)| {
            let e = if k + 1 < starts.len() {
                starts[k + 1]
            } else {
                raw.len()
            };
            &raw[*s..e]
        })
        .collect()
}

struct SectionMeta {
    old_path: String,
    new_path: String,
    created: bool,
    deleted: bool,
    renamed: bool,
    /// Hunk bodies: header line + `-`/`+`/` `/marker lines.
    hunks: Vec<Vec<String>>,
}

fn section_error(section: &str, what: &str) -> VerifyError {
    VerifyError::NativeParse(format!(
        "{what} in section: {}",
        cap_line(section.lines().next().unwrap_or(""))
    ))
}

fn parse_section_meta(section: &str) -> Result<SectionMeta, VerifyError> {
    let err = |what: &str| section_error(section, what);
    let header = section
        .lines()
        .next()
        .ok_or_else(|| err("missing `diff --git` header"))?;
    let rest = header
        .strip_prefix("diff --git ")
        .ok_or_else(|| err("missing `diff --git` header"))?;
    // Binary-marker sections carry no text to compare; both sides agree
    // by construction (infer strips them with a warning).
    if section.lines().any(|l| l.starts_with("Binary files ")) {
        return Ok(SectionMeta {
            old_path: String::new(),
            new_path: String::new(),
            created: false,
            deleted: false,
            renamed: false,
            hunks: Vec::new(),
        });
    }
    let (mut old_path, mut new_path) = match rsmpl_patch::split_git_paths(rest) {
        Some(parts) if parts.len() >= 2 => {
            let n = parts.len();
            (strip_ab(&parts[n - 2]), strip_ab(&parts[n - 1]))
        }
        _ => {
            let p = rest.trim().to_string();
            (p.clone(), p)
        }
    };
    let mut created = false;
    let mut deleted = false;
    let mut rename_from: Option<String> = None;
    let mut rename_to: Option<String> = None;
    let mut hunks: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut active = false;
    for line in section.lines().skip(1) {
        if line.starts_with("@@ ") {
            if active {
                hunks.push(std::mem::take(&mut cur));
            }
            active = true;
            cur.push(line.to_string());
        } else if active {
            cur.push(line.to_string());
        } else if let Some(from) = line.strip_prefix("rename from ") {
            rename_from = Some(rsmpl_patch::decode_git_path_token(from.trim()));
        } else if let Some(to) = line.strip_prefix("rename to ") {
            rename_to = Some(rsmpl_patch::decode_git_path_token(to.trim()));
        } else if let Some(p) = line.strip_prefix("--- ") {
            let p = p.trim();
            if p == "/dev/null" {
                created = true;
            } else {
                old_path = strip_ab(&rsmpl_patch::decode_git_path_token(p));
            }
        } else if let Some(p) = line.strip_prefix("+++ ") {
            let p = p.trim();
            if p == "/dev/null" {
                deleted = true;
            } else {
                new_path = strip_ab(&rsmpl_patch::decode_git_path_token(p));
            }
        } else if line.starts_with("new file mode") {
            created = true;
        } else if line.starts_with("deleted file mode") {
            deleted = true;
        }
    }
    if active {
        hunks.push(cur);
    }
    let renamed = rename_from.is_some() && rename_to.is_some();
    if let (Some(f), Some(t)) = (rename_from, rename_to) {
        old_path = f;
        new_path = t;
    }
    Ok(SectionMeta {
        old_path,
        new_path,
        created,
        deleted,
        renamed,
        hunks,
    })
}

fn apply_section(
    pre: &HashMap<String, String>,
    section: &str,
    out: &mut NativePost,
) -> Result<(), VerifyError> {
    let meta = parse_section_meta(section)?;
    if meta.old_path.is_empty() {
        // Binary-marker section (parsed above): nothing to prove.
        return Ok(());
    }
    if meta.deleted {
        out.files.insert(meta.new_path.clone());
        out.post.remove(&meta.new_path);
        return Ok(());
    }
    let base = if meta.created {
        String::new()
    } else if let Some(text) = pre.get(&meta.old_path) {
        text.clone()
    } else {
        // Missing base blob and not a create: underivable, skipped
        // (uncounted; the CLI already warns per path).
        return Ok(());
    };
    let mut file = LineFile::new(&base);
    if meta.created {
        // `+` body lines are newline-terminated unless the
        // `\ No newline` marker says otherwise; the empty base's
        // `trailing_newline: false` would otherwise drop the final `\n`.
        file.trailing_newline = true;
    }
    let mut delta: i64 = 0;
    for hunk in &meta.hunks {
        delta = apply_hunk(section, &mut file, hunk, delta)?;
    }
    out.files.insert(meta.new_path.clone());
    out.post.insert(meta.new_path.clone(), file.join());
    if meta.renamed && meta.old_path != meta.new_path {
        out.files.insert(meta.old_path.clone());
        out.post.remove(&meta.old_path);
    }
    Ok(())
}

/// Apply one hunk body (header + content lines) at original coordinates
/// plus the running `delta` from previous hunks; returns the new delta.
fn apply_hunk(
    section: &str,
    file: &mut LineFile,
    hunk: &[String],
    delta: i64,
) -> Result<i64, VerifyError> {
    let err = |what: String| section_error(section, &what);
    let (old_start, old_len, new_len) =
        parse_hunk_header(&hunk[0]).ok_or_else(|| err("bad hunk header".to_string()))?;
    let start_i = if old_start == 0 {
        // New-file first hunk (`@@ -0,0 ...`): insert at the top.
        0i64
    } else {
        old_start as i64 - 1 + delta
    };
    if start_i < 0 {
        return Err(err(format!(
            "hunk at -{old_start} underflows the pre-image"
        )));
    }
    let start = start_i as usize;
    let mut idx = start;
    let mut old_consumed = 0usize;
    // Owned: `plus_lines` mixes pre-image borrows with hunk-text
    // borrows, and the file outlives both the hunk slice and (via the
    // returned post map) this frame — clone once per changed line.
    let mut plus_lines: Vec<String> = Vec::new();
    let mut new_trailing = file.trailing_newline;
    for (k, line) in hunk.iter().enumerate().skip(1) {
        if let Some(content) = line.strip_prefix('-') {
            let cur = file
                .lines
                .get(idx)
                .map(String::as_str)
                .unwrap_or("<missing>");
            if cur != content {
                return Err(err(format!(
                    "`-` line does not match pre-image at line {}",
                    idx + 1
                )));
            }
            idx += 1;
            old_consumed += 1;
        } else if let Some(content) = line.strip_prefix('+') {
            plus_lines.push(content.to_string());
        } else if let Some(content) = line.strip_prefix(' ') {
            let cur = file
                .lines
                .get(idx)
                .map(String::as_str)
                .unwrap_or("<missing>");
            if cur != content {
                return Err(err(format!(
                    "context line does not match pre-image at line {}",
                    idx + 1
                )));
            }
            idx += 1;
            old_consumed += 1;
            plus_lines.push(content.to_string());
        } else if *line == "\\ No newline at end of file" {
            // The marker annotates the preceding content line: a `+`
            // (or context) predecessor means the post lacks the trailing
            // newline; a `-` predecessor describes the pre side only.
            let prev_is_plus = k > 1
                && (hunk[k - 1].starts_with('+') && !hunk[k - 1].starts_with("+++")
                    || hunk[k - 1].starts_with(' '));
            if prev_is_plus {
                new_trailing = false;
            }
        } else {
            return Err(err("unexpected hunk body line".to_string()));
        }
    }
    if old_consumed != old_len {
        return Err(err(format!(
            "hunk consumes {old_consumed} old lines, header says {old_len}"
        )));
    }
    if plus_lines.len() != new_len {
        return Err(err(format!(
            "hunk emits {} new lines, header says {new_len}",
            plus_lines.len()
        )));
    }
    if start + old_consumed > file.lines.len() {
        return Err(err("hunk range exceeds pre-image".to_string()));
    }
    file.lines.splice(start..start + old_consumed, plus_lines);
    file.trailing_newline = new_trailing;
    Ok(delta + new_len as i64 - old_len as i64)
}

fn strip_ab(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")) {
        // A genuine repo-relative name starting with `a/` keeps its
        // prefix only when the strip would leave nothing.
        if rest.is_empty() {
            return p.to_string();
        }
        return rest.to_string();
    }
    p.to_string()
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize)> {
    // `@@ -a[,b] +c[,d] @@[ title]`; lengths default to 1 when omitted.
    let inner = line.strip_prefix("@@ ")?;
    let end = inner.find(" @@")?;
    let mut parts = inner[..end].split(' ');
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let parse_side = |s: &str| -> Option<(usize, usize)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (old_start, old_len) = parse_side(old)?;
    let (_, new_len) = parse_side(new)?;
    Some((old_start, old_len, new_len))
}

struct LineFile {
    lines: Vec<String>,
    trailing_newline: bool,
}

impl LineFile {
    fn new(text: &str) -> Self {
        if text.is_empty() {
            return Self {
                lines: Vec::new(),
                trailing_newline: false,
            };
        }
        let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let trailing = text.ends_with('\n');
        if trailing {
            lines.pop();
        }
        Self {
            lines,
            trailing_newline: trailing,
        }
    }

    fn join(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut s = self.lines.join("\n");
        if self.trailing_newline {
            s.push('\n');
        }
        s
    }
}

fn describe_parse_error(e: &rsmpl_patch::ParseError) -> String {
    match e {
        rsmpl_patch::ParseError::Structured(s) => s.to_string(),
        rsmpl_patch::ParseError::Message(m) => m.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pre(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    const RENAME_RSMPL: &str =
        "diff --git src/lib.rs\n@@ -1,1 @@ rule=r1\n# sweep\n- fn foo\n+ fn bar\n";

    #[test]
    fn clean_rename_verifies_with_site_count() {
        let pre = pre(&[("src/lib.rs", "fn foo() {}\nfn other() {}\n")]);
        let raw = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-fn foo() {}\n+fn bar() {}\n";
        let report = verify(&pre, raw, RENAME_RSMPL).unwrap();
        assert!(report.verified, "got: {report:?}");
        assert_eq!(report.files_compared, 1);
        assert_eq!(report.first_divergence, None);
    }

    #[test]
    fn over_match_sweep_fails_where_lowering_validation_passes() {
        // T361 contrast pin: two identical call sites, native changes
        // one; the raised sweep rewrites both. Parse succeeds and the
        // engine reports a match (lowering validation — what
        // `applyrs --check` computes — stays green), but the T361 loop
        // fails naming file + line.
        let pre = pre(&[("src/lib.rs", "fn m() {\nfoo(1);\nfoo(1);\n}\n")]);
        let raw = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2 +2 @@\n-foo(1);\n+bar(1);\n";
        let rsmpl = "diff --git src/lib.rs\n@@ -2,2 @@ rule=r1\n# allow=many\n- foo(E)\n+ bar(E)\n";
        // Lowering validation accepts: parses and matches.
        let patch = rsmpl_patch::parse_patch(rsmpl).expect("must parse");
        let mut raised = pre.clone();
        let engine_report =
            rsmpl_engine::apply_patch_to_tree(&mut raised, &patch).expect("must apply");
        assert!(
            engine_report.rules.iter().any(|r| r.rule == "r1"),
            "must match r1"
        );
        // The T361 loop fails, naming the second (untouched) line.
        let report = verify(&pre, raw, rsmpl).unwrap();
        assert!(!report.verified, "over-match must not verify");
        let div = report.first_divergence.expect("must name divergence");
        assert_eq!(div.file, "src/lib.rs");
        assert_eq!(div.line, 3, "got: {div:?}");
        assert_eq!(div.want, "foo(1);");
        assert_eq!(div.got, "bar(1);");
    }

    #[test]
    fn new_file_create_verifies() {
        let pre = pre(&[]);
        let raw = "diff --git a/src/new.rs b/src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,1 @@\n+fn n() {}\n";
        let rsmpl = "diff --git src/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/new.rs\n@@ -0,0 +1,1 @@\n+fn n() {}\n";
        let report = verify(&pre, raw, rsmpl).unwrap();
        assert!(report.verified, "got: {report:?}");
        assert_eq!(report.files_compared, 1);
    }

    #[test]
    fn deleted_file_verifies() {
        let pre = pre(&[("src/old.rs", "fn o() {}\n")]);
        let raw = "diff --git a/src/old.rs b/src/old.rs\ndeleted file mode 100644\n--- a/src/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-fn o() {}\n";
        let rsmpl = "diff --git src/old.rs\ndeleted file mode 100644\n--- a/src/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-fn o() {}\n";
        let report = verify(&pre, raw, rsmpl).unwrap();
        assert!(report.verified, "got: {report:?}");
        assert_eq!(report.files_compared, 1);
    }

    #[test]
    fn multi_hunk_offset_applies_at_original_coordinates() {
        let pre = pre(&[("f.txt", "a\nb\nc\nd\ne\nf\n")]);
        let raw = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+A\n@@ -6 +6 @@\n-f\n+F\n";
        let rsmpl = "diff --git f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+A\n@@ -6 +6 @@\n-f\n+F\n";
        let report = verify(&pre, raw, rsmpl).unwrap();
        assert!(report.verified, "got: {report:?}");
        // A raised patch missing the second hunk diverges at line 6.
        let partial = "diff --git f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+A\n";
        let report = verify(&pre, raw, partial).unwrap();
        assert!(!report.verified);
        assert_eq!(report.first_divergence.as_ref().unwrap().line, 6);
    }

    #[test]
    fn missing_trailing_newline_round_trips() {
        let pre = pre(&[("f.txt", "a\nb")]);
        let raw = "diff --git a/f.txt b/f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -2 +2 @@\n-b\n+B\n\\ No newline at end of file\n";
        let rsmpl = "diff --git f.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -2 +2 @@\n-b\n+B\n\\ No newline at end of file\n";
        let report = verify(&pre, raw, rsmpl).unwrap();
        assert!(report.verified, "got: {report:?}");
    }

    #[test]
    fn stale_base_is_a_raise_error_not_a_bare_false() {
        // The raised patch cites line 1, but the pre-image is empty:
        // engine apply fails, surfacing as `VerifyError::Raise`
        // (exit-2 error envelope) rather than a bare
        // `verified: false` with no divergence.
        let pre = pre(&[]);
        let raw = "diff --git a/src/lib.rs b/src/lib.rs\nnew file mode 100644\n--- /dev/null\n+++ b/src/lib.rs\n@@ -0,0 +1,1 @@\n+fn n() {}\n";
        let err = verify(&pre, raw, RENAME_RSMPL).expect_err("must not apply");
        assert!(matches!(err, VerifyError::Raise(_)), "got: {err:?}");
    }

    #[test]
    fn garbage_native_text_is_a_parse_error() {
        let pre = pre(&[]);
        let err = verify(&pre, "this is not a diff\n", RENAME_RSMPL).expect_err("must fail");
        assert!(matches!(err, VerifyError::NativeParse(_)), "got: {err:?}");
    }

    #[test]
    fn binary_only_change_is_vacuously_true() {
        let pre = pre(&[]);
        let raw =
            "diff --git a/blob.bin b/blob.bin\nBinary files a/blob.bin and b/blob.bin differ\n";
        let report = verify(&pre, raw, "").unwrap();
        assert!(report.verified);
        assert_eq!(report.files_compared, 0);
    }
}
