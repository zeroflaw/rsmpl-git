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
