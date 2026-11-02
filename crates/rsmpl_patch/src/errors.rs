//! Structured error codes (SPEC section 25).

use thiserror::Error;

/// Machine-readable error codes mirroring SPEC section 25.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    RangeConflict,
    ParseConflict,
    PatternConflict,
    MatchConflict,
    SymbolConflict,
    CaptureConflict,
    GitHunkConflict,
    /// I/O failure reading the patch, a repo file, or writing output (SPEC 25.8).
    IoConflict,
    /// Post-lowering cargo check failure under --verify build (SPEC 25.10).
    BuildConflict,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RangeConflict => "RSMPL_RANGE_CONFLICT",
            Self::ParseConflict => "RSMPL_PARSE_CONFLICT",
            Self::PatternConflict => "RSMPL_PATTERN_CONFLICT",
            Self::MatchConflict => "RSMPL_MATCH_CONFLICT",
            Self::SymbolConflict => "RSMPL_SYMBOL_CONFLICT",
            Self::CaptureConflict => "RSMPL_CAPTURE_CONFLICT",
            Self::GitHunkConflict => "GIT_HUNK_CONFLICT",
            Self::IoConflict => "RSMPL_IO_CONFLICT",
            Self::BuildConflict => "RSMPL_BUILD_CONFLICT",
        }
    }
}

/// A structured diagnostic with optional rule/file/range context.
#[derive(Debug, Error, Clone)]
#[error("{code}: {message}")]
pub struct StructuredError {
    pub code: ErrorCode,
    pub rule: Option<String>,
    pub file: Option<String>,
    pub range: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
    pub repair: Option<serde_json::Value>,
}

impl StructuredError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            rule: None,
            file: None,
            range: None,
            message: message.into(),
            suggestion: None,
            repair: None,
        }
    }

    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_range(mut self, range: impl Into<String>) -> Self {
        self.range = Some(range.into());
        self
    }

    pub fn with_suggestion(mut self, s: impl Into<String>) -> Self {
        self.suggestion = Some(s.into());
        self
    }

    /// Machine-readable repair payload (SPEC D.2): additive `repair` object
    /// for errors that carry self-correction data; `None` renders as null.
    pub fn with_repair(mut self, repair: serde_json::Value) -> Self {
        self.repair = Some(repair);
        self
    }

    /// WI-2: build-failure `read_file` repair payload (SPEC 25.10, D.2).
    ///
    /// `range` is the `L..L` single-line range of the first diagnostic
    /// (or the first orphan fallback). The agent reads that range and
    /// fixes the breakage, then retries.
    pub fn read_file_repair(file: &str, range: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "read_file",
            "file": file,
            "range": range,
        })
    }

    /// WI-3b: C-table `fix_pattern` repair payload.
    ///
    /// `code` is the SPEC 18.6 compile-gate code (`C1`, `C3`…`C18`);
    /// `write_instead` is the exact SPEC 18.6 diagnostic text telling the
    /// agent what to write instead. Prose `reason`/`suggestion` stay
    /// unchanged — this only adds the machine payload.
    pub fn fix_pattern_repair(code: &str, write_instead: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "fix_pattern",
            "code": code,
            "write_instead": write_instead,
        })
    }

    /// WI-3b: symbol/capture `add_directive` repair payload.
    ///
    /// Mirrors the existing prose suggestion verbatim (e.g.
    /// `# fallback=syntax`); the agent applies the directive and retries
    /// without parsing English.
    pub fn add_directive_repair(directive: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "add_directive",
            "directive": directive,
        })
    }

    /// WI-3b: capture `fix_name` repair payload.
    ///
    /// `name` is the offending capture name; `available` lists the bound
    /// captures the agent can use instead (possibly empty, meaning the
    /// agent must add a `# bind=` hunk first).
    pub fn fix_name_repair(name: &str, available: &[String]) -> serde_json::Value {
        serde_json::json!({
            "action": "fix_name",
            "name": name,
            "available": available,
        })
    }

    /// WI-3: defensive-backstop `fix_pattern` payload (SPEC 25.9).
    ///
    /// Span guards and edit-bounds checks fire only on engine
    /// inconsistency (inputs are immutable during matching); if one
    /// ever fires, the agent still gets the patch-side exit first and
    /// the honesty caveat second. One constructor so the wording
    /// cannot drift per site.
    pub fn internal_repair() -> serde_json::Value {
        Self::fix_pattern_repair(
            "internal",
            "restrict the hunk to a smaller range and retry; if the failure persists it is an engine bug, not a patch bug",
        )
    }

    /// WI-3: range-past-EOF `fix_range` repair payload (SPEC D.2).
    ///
    /// `write_instead` is the largest in-file `@@ -start,len @@` range
    /// starting at the cited start (or the last line when the cited
    /// start itself is past EOF); the agent pastes it verbatim.
    pub fn fix_range_repair(file: &str, file_lines: u32, write_instead: &str) -> serde_json::Value {
        serde_json::json!({
            "action": "fix_range",
            "file": file,
            "file_lines": file_lines,
            "write_instead": write_instead,
        })
    }

    /// WI-3: `fix_range` with the clamped header computed from the cited
    /// range and the file length (single clamping rule shared by the
    /// range validator and the ordinary-hunk applier — no second copy).
    pub fn fix_range_for(file: &str, start: u32, len: u32, file_lines: u32) -> serde_json::Value {
        let write_instead = if len == 0 {
            format!("@@ -{},0 @@", file_lines + 1)
        } else if file_lines == 0 {
            "@@ -1,0 @@".to_string()
        } else if start == 0 || start > file_lines {
            format!("@@ -{file_lines},1 @@")
        } else {
            format!("@@ -{start},{} @@", file_lines - start + 1)
        };
        Self::fix_range_repair(file, file_lines, &write_instead)
    }

    /// WI-3: unknown-directive `fix_directive` repair payload (SPEC D.2).
    ///
    /// `found` is the offending spelling, `did_you_mean` the closest
    /// known spelling within edit distance 2 (`None` serializes as
    /// `null` — honest when nothing is close), and `write_instead` the
    /// full corrected directive line the agent pastes verbatim. Never
    /// echoes the English reason.
    pub fn fix_directive_repair(
        found: &str,
        did_you_mean: Option<&str>,
        write_instead: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "action": "fix_directive",
            "found": found,
            "did_you_mean": did_you_mean,
            "write_instead": write_instead,
        })
    }

    /// Machine-readable form for `--format json` failure envelopes (SPEC D.2):
    /// `{"code","rule","file","range","reason","suggestion","repair"}`.
    /// Absent optionals serialize as `null` so agents see a stable key set.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code.as_str(),
            "rule": self.rule,
            "file": self.file,
            "range": self.range,
            "reason": self.message,
            "suggestion": self.suggestion,
            "repair": self.repair,
        })
    }

    /// Render in the SPEC section 25 text-block style.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(self.code.as_str());
        out.push('\n');
        if let Some(r) = &self.rule {
            out.push_str(&format!("rule: {r}\n"));
        }
        if let Some(f) = &self.file {
            out.push_str(&format!("file: {f}\n"));
        }
        if let Some(r) = &self.range {
            out.push_str(&format!("old_range: {r}\n"));
        }
        out.push_str(&format!("reason: {}\n", self.message));
        if let Some(s) = &self.suggestion {
            out.push_str(&format!("suggestion: {s}\n"));
        }
        if let Some(repair) = &self.repair {
            out.push_str(&format!("repair: {repair}\n"));
        }
        out
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_code_renders() {
        assert_eq!(ErrorCode::IoConflict.as_str(), "RSMPL_IO_CONFLICT");
    }

    #[test]
    fn build_conflict_code_and_repair_pin() {
        assert_eq!(ErrorCode::BuildConflict.as_str(), "RSMPL_BUILD_CONFLICT");
        let r = StructuredError::read_file_repair("src/lib.rs", "3..3");
        assert_eq!(r["action"], serde_json::json!("read_file"));
        assert_eq!(r["file"], serde_json::json!("src/lib.rs"));
        assert_eq!(r["range"], serde_json::json!("3..3"));
    }

    #[test]
    fn to_json_has_stable_keys_with_nulls() {
        let e = StructuredError::new(ErrorCode::MatchConflict, "no match found");
        let v = e.to_json();
        assert_eq!(v["code"], serde_json::json!("RSMPL_MATCH_CONFLICT"));
        assert_eq!(v["reason"], serde_json::json!("no match found"));
        assert!(v["rule"].is_null(), "got: {v}");
        assert!(v["file"].is_null(), "got: {v}");
        assert!(v["range"].is_null(), "got: {v}");
        assert!(v["suggestion"].is_null(), "got: {v}");
        assert!(v["repair"].is_null(), "got: {v}");
    }

    #[test]
    fn to_json_carries_context() {
        let e = StructuredError::new(ErrorCode::RangeConflict, "old range exceeds file")
            .with_rule("r")
            .with_file("src/lib.rs")
            .with_range("40..47")
            .with_suggestion("read the file to list valid ranges");
        let v = e.to_json();
        assert_eq!(v["code"], serde_json::json!("RSMPL_RANGE_CONFLICT"));
        assert_eq!(v["rule"], serde_json::json!("r"));
        assert_eq!(v["file"], serde_json::json!("src/lib.rs"));
        assert_eq!(v["range"], serde_json::json!("40..47"));
        assert_eq!(
            v["suggestion"],
            serde_json::json!("read the file to list valid ranges")
        );
        assert!(v["repair"].is_null(), "got: {v}");
    }

    #[test]
    fn repair_constructors_pin_shapes() {
        // WI-3b: exact payload shapes agents match on.
        let fp = StructuredError::fix_pattern_repair("C3", "write a `let` statement pattern");
        assert_eq!(fp["action"], serde_json::json!("fix_pattern"));
        assert_eq!(fp["code"], serde_json::json!("C3"));
        assert_eq!(
            fp["write_instead"],
            serde_json::json!("write a `let` statement pattern")
        );
        let ad = StructuredError::add_directive_repair("# fallback=syntax");
        assert_eq!(ad["action"], serde_json::json!("add_directive"));
        assert_eq!(ad["directive"], serde_json::json!("# fallback=syntax"));
        let fn_ =
            StructuredError::fix_name_repair("Helper", &["Bound".to_string(), "Other".to_string()]);
        assert_eq!(fn_["action"], serde_json::json!("fix_name"));
        assert_eq!(fn_["name"], serde_json::json!("Helper"));
        assert_eq!(fn_["available"], serde_json::json!(["Bound", "Other"]));
    }

    #[test]
    fn wi3_repair_shapes_pin() {
        let fr = StructuredError::fix_range_repair("src/lib.rs", 10, "@@ -10,1 @@");
        assert_eq!(fr["action"], serde_json::json!("fix_range"));
        assert_eq!(fr["file"], serde_json::json!("src/lib.rs"));
        assert_eq!(fr["file_lines"], serde_json::json!(10));
        assert_eq!(fr["write_instead"], serde_json::json!("@@ -10,1 @@"));
        let fd = StructuredError::fix_directive_repair(
            "# alow=many",
            Some("# allow=many"),
            "# allow=many",
        );
        assert_eq!(fd["action"], serde_json::json!("fix_directive"));
        assert_eq!(fd["found"], serde_json::json!("# alow=many"));
        assert_eq!(fd["did_you_mean"], serde_json::json!("# allow=many"));
        assert_eq!(fd["write_instead"], serde_json::json!("# allow=many"));
        let far = StructuredError::fix_directive_repair("# zzzzzz=1", None, "check SPEC 9.3");
        assert!(far["did_you_mean"].is_null(), "got: {far}");
    }

    #[test]
    fn with_repair_round_trips_through_json_and_render() {
        let repair = serde_json::json!({
            "action": "fix_header",
            "expected_old": 3,
            "expected_new": 3,
        });
        let e =
            StructuredError::new(ErrorCode::ParseConflict, "counts").with_repair(repair.clone());
        let v = e.to_json();
        assert_eq!(v["repair"], repair, "got: {v}");
        let text = e.render();
        assert!(text.contains("repair:"), "got: {text}");
        assert!(text.contains("fix_header"), "got: {text}");
        assert!(
            !StructuredError::new(ErrorCode::ParseConflict, "counts")
                .render()
                .contains("repair:"),
            "bare render must stay silent"
        );
    }

    /// WI-18: source-scan repair gate — no new prose-only failure ships.
    ///
    /// Every `StructuredError::new` site in production code
    /// (`crates/*/src`, `bin/*/src`, before `#[cfg(test)]`) must carry a
    /// machine repair or join a named class. `IoConflict` bare sites are
    /// the environmental allow-list (SPEC 25.9/D.2 permit `null`: no
    /// patch spelling fixes them). `PatternConflict`/`SymbolConflict`/
    /// `CaptureConflict` bare sites are the engine-tail class
    /// (`rsmpl_engine::validate::fill_generic_repair`, applied at the
    /// `apply_inner` boundary, keys one concrete template per class and
    /// never echoes prose, so they are never null at runtime); the tail
    /// count is pinned by `PATTERN_TAIL_CEILING` so the class never
    /// grows without an explicit site-level repair. WI-18 follow-up
    /// slice 1 moved the 9-site evolution-paste template family to
    /// site-specific `evolution-*-template` repairs (ceiling 96 -> 87).
    /// WI-18 follow-up slice 2 moved the 8-site container-splice
    /// template family to site-specific `evolution-*-splice-template`
    /// repairs (ceiling 87 -> 79). Any other bare site
    /// (`Match`/`Range`/`Parse`/`GitHunk`/`Build` or an unclassified
    /// code) fails naming file + line + code, because the filler does
    /// not cover those codes and a bare site there is a prose-only
    /// failure the agent cannot act on.
    ///
    /// The companion runtime enumeration lives in
    /// `bin/applyrs/tests/wi3_repair_complete_e2e.rs` (one failing
    /// patch per patch-input class); this test is the source gate.
    const PATTERN_TAIL_CEILING: usize = 79;

    #[test]
    fn wi18_source_scan_every_site_carries_repair_or_allow_list() {
        let root = workspace_root();
        let mut with_repair_per_code: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut io_allow_list = 0usize;
        let mut tail_bare: Vec<String> = Vec::new();
        let mut hard_bare: Vec<String> = Vec::new();
        for dir in ["crates", "bin"] {
            let base = root.join(dir);
            let mut stack = vec![base];
            while let Some(top) = stack.pop() {
                let entries = std::fs::read_dir(&top)
                    .unwrap_or_else(|e| panic!("WI-18 scan cannot read {}: {e}", top.display()));
                for entry in entries {
                    let path = entry.expect("dir entry").path();
                    if path.is_dir() {
                        // Production sources only: `src` trees (integration
                        // tests under `tests/` assert runtime behavior and
                        // construct bare errors on purpose).
                        let under_src = top.components().any(|c| c.as_os_str() == "src");
                        if path.file_name().is_some_and(|n| n == "src")
                            || top.ends_with(dir)
                            || under_src
                        {
                            stack.push(path);
                        }
                        continue;
                    }
                    if path.extension().is_none_or(|e| e != "rs") {
                        continue;
                    }
                    scan_file(
                        &path,
                        &root,
                        &mut with_repair_per_code,
                        &mut io_allow_list,
                        &mut tail_bare,
                        &mut hard_bare,
                    );
                }
            }
        }
        assert!(
            !with_repair_per_code.is_empty(),
            "WI-18 scan found no with_repair sites (walk broken?)"
        );
        for code in [
            "PatternConflict",
            "SymbolConflict",
            "CaptureConflict",
            "MatchConflict",
            "RangeConflict",
            "ParseConflict",
            "GitHunkConflict",
            "BuildConflict",
        ] {
            assert!(
                with_repair_per_code.get(code).copied().unwrap_or(0) > 0,
                "WI-18 scan: no with_repair site for {code} (gate vacuous?)"
            );
        }
        assert!(
            io_allow_list > 0,
            "WI-18 scan: IO allow-list empty (walk broken or allow-list stale?)"
        );
        assert!(
            hard_bare.is_empty(),
            "WI-18: bare non-tail sites without with_repair (add with_repair at the site, never a central patch-up):\n{}",
            hard_bare.join("\n")
        );
        assert!(
            tail_bare.len() <= PATTERN_TAIL_CEILING,
            "WI-18: engine-tail bare count {} exceeds ceiling {PATTERN_TAIL_CEILING} (new prose-only sites must carry with_repair):\n{}",
            tail_bare.len(),
            tail_bare.join("\n")
        );
    }

    fn workspace_root() -> std::path::PathBuf {
        // `CARGO_MANIFEST_DIR` is `crates/rsmpl_patch`; the workspace root
        // holds both `crates/` and `bin/`.
        let mut dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        loop {
            if dir.join("crates").is_dir() && dir.join("bin").is_dir() {
                return dir;
            }
            if !dir.pop() {
                panic!("WI-18 scan cannot locate the workspace root");
            }
        }
    }

    fn scan_file(
        path: &std::path::Path,
        root: &std::path::Path,
        with_repair_per_code: &mut std::collections::HashMap<String, usize>,
        io_allow_list: &mut usize,
        tail_bare: &mut Vec<String>,
        hard_bare: &mut Vec<String>,
    ) {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("WI-18 scan cannot read {}: {e}", path.display()));
        // Production part only: unit tests construct bare errors on
        // purpose (shape pins, null-render pins).
        let prod = text.split("#[cfg(test)]").next().unwrap_or(&text);
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string();
        let mut rest = prod;
        // Byte offset of `rest` within `prod` for line numbers.
        let mut base: usize = 0;
        while let Some(idx) = rest.find("StructuredError::new(") {
            let abs = base + idx;
            let line = prod[..abs].lines().count().max(1);
            let after = &prod[abs..];
            let code = code_token(after);
            // The chained `.with_repair` always follows within the same
            // expression; bound the window at the next construction so
            // a neighboring site's repair never credits this one.
            let window = &after[..after.len().min(1800)];
            let bound = match window[1..].find("StructuredError::new(") {
                Some(n) => &window[..n + 1],
                None => window,
            };
            let has_repair = bound.contains(".with_repair");
            let short = short_code(&code);
            if has_repair {
                *with_repair_per_code.entry(short.clone()).or_insert(0) += 1;
            } else if short == "IoConflict" {
                *io_allow_list += 1;
            } else if short == "PatternConflict"
                || short == "SymbolConflict"
                || short == "CaptureConflict"
            {
                tail_bare.push(format!("{rel}:{line} ({short})"));
            } else {
                hard_bare.push(format!("{rel}:{line} ({short})"));
            }
            rest = &after["StructuredError::new(".len()..];
            base = abs + "StructuredError::new(".len();
        }
    }

    fn code_token(after: &str) -> String {
        // `after` starts at `StructuredError::new(`; the first argument
        // is the code (`ErrorCode::X`, `rsmpl_patch::ErrorCode::X`,
        // `code`, or a literal for helpers — never a string).
        let open = after.find('(').unwrap_or(0);
        let tail = after[open + 1..].trim_start();
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
            .unwrap_or(tail.len());
        tail[..end].to_string()
    }

    fn short_code(code: &str) -> String {
        code.rsplit("::").next().unwrap_or(code).to_string()
    }
}
