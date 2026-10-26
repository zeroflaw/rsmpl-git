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

    /// Machine-readable form for `--format json` failure envelopes (SPEC D.2):
    /// `{"code","rule","file","range","reason","suggestion"}`.
    /// Absent optionals serialize as `null` so agents see a stable key set.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code.as_str(),
            "rule": self.rule,
            "file": self.file,
            "range": self.range,
            "reason": self.message,
            "suggestion": self.suggestion,
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
    }
}
