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
}
