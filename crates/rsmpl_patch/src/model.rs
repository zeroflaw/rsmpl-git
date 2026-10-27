//! Patch document model (SPEC sections 5, 6, 8, 9).

/// Top-level patch document: zero or more file sections.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    pub version: Option<String>,
    pub files: Vec<FileSection>,
}

/// A `diff --git a/x b/y` section with its hunks.
#[derive(Debug, Clone)]
pub struct FileSection {
    pub old_path: String,
    pub new_path: String,
    /// Target file path (b-side stripped of `b/` prefix, `/dev/null` aware).
    pub target_path: String,
    pub is_new_file: bool,
    pub is_deleted_file: bool,
    pub metadata: Vec<String>,
    pub hunks: Vec<Hunk>,
}

impl FileSection {
    pub fn target(&self) -> &str {
        &self.target_path
    }

    /// True for a file rename section: both halves are real paths
    /// (neither `/dev/null`) and the stripped paths differ (T377).
    /// diffrs emits these with rename detection on; applyrs lowers
    /// them back to `rename from/to` sections.
    pub fn is_rename(&self) -> bool {
        if is_dev_null(&self.old_path) || is_dev_null(&self.new_path) {
            return false;
        }
        stripped_path(&self.old_path) != stripped_path(&self.new_path)
    }
}

/// Strip the `a/`/`b/` diff prefix (mirrors the parser's
/// `strip_ab_prefix` for the two-path form).
pub fn stripped_path(p: &str) -> String {
    if let Some(s) = p.strip_prefix("a/") {
        return s.to_string();
    }
    if let Some(s) = p.strip_prefix("b/") {
        return s.to_string();
    }
    p.to_string()
}

fn is_dev_null(p: &str) -> bool {
    p == "/dev/null" || p == "dev/null"
}

/// Unified-diff hunk header ranges + trailing tags.
#[derive(Debug, Clone)]
pub struct HunkHeader {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// True when the `+new,len` side was elided (`@@ -old,len @@`).
    /// RSmPL-only (SPEC 8): the new side defaults to the old side.
    pub new_elided: bool,
    pub rule: Option<String>,
    pub raw_tail: String,
}

impl HunkHeader {
    /// Old-side range as 1-based inclusive line span in original coordinates.
    /// Returns `None` for zero-length insertion anchors.
    pub fn old_range_inclusive(&self) -> Option<(u32, u32)> {
        if self.old_len == 0 {
            None
        } else {
            Some((self.old_start, self.old_start + self.old_len - 1))
        }
    }

    /// Insertion anchor line for zero-length old ranges (SPEC 10.5).
    pub fn insertion_anchor(&self) -> Option<u32> {
        if self.old_len == 0 {
            Some(self.old_start)
        } else {
            None
        }
    }
}

/// Match cardinality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Allow {
    #[default]
    One,
    Many,
}

/// Optional-match flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Optional(pub bool);

/// Scope modes (SPEC 17.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    #[default]
    Contained,
    Anchor,
    Overlap,
    Exact,
    File,
}

/// Attribute policies (SPEC 14.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttrsPolicy {
    #[default]
    Preserve,
    Absent,
    Explicit,
    Strip,
}

/// Parse mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParseMode {
    #[default]
    Ast,
    Tokens,
}

/// Macro matching modes (SPEC 21).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MacroMode {
    #[default]
    Invocation,
    TokenTree,
    Expanded,
}

/// Output formatting (SPEC 22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatMode {
    #[default]
    Preserve,
    Rustfmt,
    Canonical,
}

/// Metavariable kinds (SPEC 15.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MetavarKind {
    Identifier,
    Path,
    Symbol,
    Expr,
    Stmt,
    Item,
    Type,
    Pat,
    Lifetime,
    GenericParams,
    WhereClause,
    Visibility,
    Qualifier,
    QualifierList,
    Attributes,
    Block,
    Field,
    Macro,
    TokenTree,
}

impl MetavarKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "identifier" => Some(Self::Identifier),
            "path" => Some(Self::Path),
            "symbol" => Some(Self::Symbol),
            "expr" => Some(Self::Expr),
            "stmt" => Some(Self::Stmt),
            "item" => Some(Self::Item),
            "type" => Some(Self::Type),
            "pat" => Some(Self::Pat),
            "lifetime" => Some(Self::Lifetime),
            "generic_params" => Some(Self::GenericParams),
            "where_clause" => Some(Self::WhereClause),
            "visibility" => Some(Self::Visibility),
            "qualifier" => Some(Self::Qualifier),
            "qualifier_list" => Some(Self::QualifierList),
            "attributes" => Some(Self::Attributes),
            "block" => Some(Self::Block),
            "field" => Some(Self::Field),
            "macro" => Some(Self::Macro),
            "token_tree" => Some(Self::TokenTree),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::Path => "path",
            Self::Symbol => "symbol",
            Self::Expr => "expr",
            Self::Stmt => "stmt",
            Self::Item => "item",
            Self::Type => "type",
            Self::Pat => "pat",
            Self::Lifetime => "lifetime",
            Self::GenericParams => "generic_params",
            Self::WhereClause => "where_clause",
            Self::Visibility => "visibility",
            Self::Qualifier => "qualifier",
            Self::QualifierList => "qualifier_list",
            Self::Attributes => "attributes",
            Self::Block => "block",
            Self::Field => "field",
            Self::Macro => "macro",
            Self::TokenTree => "token_tree",
        }
    }
}

/// A single directive line (SPEC 9).
/// T113: unknown directives are rejected at parse time (SPEC 2.6) — there
/// is no `Unknown` catch-all, no `Edition`, and no `FormatScope`: the
/// parser is the single source of supported-directive truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    LangRust,
    Allow(Allow),
    Optional(bool),
    Repeat,
    Scope(Scope),
    Bind(String),
    Use(String),
    Attrs(AttrsPolicy),
    Mode(ParseMode),
    MacroMode(MacroMode),
    Format(FormatMode),
    ReturnsNone(bool),
    Root(String),
    FallbackSyntax(bool),
    SigiledMetavars(bool),
    Symbol {
        name: String,
        path: String,
    },
    MetaVar {
        kind: MetavarKind,
        names: Vec<String>,
    },
}

impl Directive {
    /// Whether this directive declares `# lang=rust` (SPEC 7.1).
    pub fn is_lang_rust(&self) -> bool {
        matches!(self, Self::LangRust)
    }
}

/// A body line inside a hunk (prefix + pattern/source text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyLine {
    /// One of `' '`, `'+'`, `'-'`, `'\\'`.
    pub prefix: char,
    pub text: String,
}

impl BodyLine {
    pub fn is_context(&self) -> bool {
        self.prefix == ' '
    }
    pub fn is_removal(&self) -> bool {
        self.prefix == '-'
    }
    pub fn is_addition(&self) -> bool {
        self.prefix == '+'
    }
}

/// Ordinary Git hunk (no directive block).
#[derive(Debug, Clone)]
pub struct OrdinaryHunk {
    pub header: HunkHeader,
    pub lines: Vec<BodyLine>,
}

/// RSmPL semantic hunk (directive block + pattern body).
#[derive(Debug, Clone)]
pub struct RsmplHunk {
    pub header: HunkHeader,
    pub directives: Vec<Directive>,
    /// Raw directive lines (for diagnostics / lowering fidelity).
    pub raw_directives: Vec<String>,
    pub lines: Vec<BodyLine>,
}

impl RsmplHunk {
    pub fn has_lang_rust(&self) -> bool {
        self.directives.iter().any(|d| d.is_lang_rust())
    }

    pub fn allow(&self) -> Allow {
        self.directives
            .iter()
            .rev()
            .find_map(|d| match d {
                Directive::Allow(a) => Some(*a),
                _ => None,
            })
            .unwrap_or(Allow::One)
    }

    pub fn is_optional(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, Directive::Optional(true)))
    }

    pub fn repeats(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, Directive::Repeat))
    }

    pub fn scope(&self) -> Scope {
        self.directives
            .iter()
            .rev()
            .find_map(|d| match d {
                Directive::Scope(s) => Some(*s),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn attrs_policy(&self) -> AttrsPolicy {
        self.directives
            .iter()
            .rev()
            .find_map(|d| match d {
                Directive::Attrs(a) => Some(*a),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn macro_mode(&self) -> MacroMode {
        self.directives
            .iter()
            .rev()
            .find_map(|d| match d {
                Directive::MacroMode(m) => Some(*m),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn parse_mode(&self) -> ParseMode {
        self.directives
            .iter()
            .rev()
            .find_map(|d| match d {
                Directive::Mode(m) => Some(*m),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn format_mode(&self) -> FormatMode {
        self.directives
            .iter()
            .rev()
            .find_map(|d| match d {
                Directive::Format(f) => Some(*f),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn returns_none(&self) -> bool {
        self.directives
            .iter()
            .any(|d| matches!(d, Directive::ReturnsNone(true)))
    }

    pub fn binds(&self) -> Vec<&str> {
        self.directives
            .iter()
            .filter_map(|d| match d {
                Directive::Bind(n) => Some(n.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn uses(&self) -> Vec<&str> {
        self.directives
            .iter()
            .filter_map(|d| match d {
                Directive::Use(n) => Some(n.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn rule_name(&self) -> String {
        self.header
            .rule
            .clone()
            .unwrap_or_else(|| "<unnamed>".to_string())
    }

    /// Removed-pattern text (without `-` prefix), joined with newlines.
    pub fn minus_text(&self) -> String {
        self.lines
            .iter()
            .filter(|l| l.is_removal())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Added-pattern text (without `+` prefix), joined with newlines.
    pub fn plus_text(&self) -> String {
        self.lines
            .iter()
            .filter(|l| l.is_addition())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Context-pattern text (without ` ` prefix), joined with newlines.
    pub fn context_text(&self) -> String {
        self.lines
            .iter()
            .filter(|l| l.is_context())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Either hunk kind.
#[derive(Debug, Clone)]
pub enum Hunk {
    Ordinary(OrdinaryHunk),
    Rsmpl(RsmplHunk),
}

impl Hunk {
    pub fn header(&self) -> &HunkHeader {
        match self {
            Self::Ordinary(h) => &h.header,
            Self::Rsmpl(h) => &h.header,
        }
    }

    pub fn is_rsmpl(&self) -> bool {
        matches!(self, Self::Rsmpl(_))
    }

    pub fn rule_name(&self) -> Option<&str> {
        self.header().rule.as_deref()
    }
}
