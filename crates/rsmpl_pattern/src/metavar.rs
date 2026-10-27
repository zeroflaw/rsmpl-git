//! Metavariable table (SPEC 15).
//!
//! Bare mode (default): declared `E` shadows literal syntax in compatible
//! positions. Sigiled mode (`# metavars=sigiled`): only `$E` is a metavariable.

use rsmpl_patch::{Directive, ErrorCode, MetavarKind, StructuredError};
use std::collections::{HashMap, HashSet};

/// Declared metavariables + symbol bindings + sigiled flag.
#[derive(Debug, Clone, Default)]
pub struct MetavarTable {
    /// name -> kind (from `# <kind> Name` directives, comma form expanded).
    pub kinds: HashMap<String, MetavarKind>,
    /// symbol name -> fully-qualified path.
    pub symbols: HashMap<String, String>,
    pub sigiled: bool,
    /// All names in declaration order (for repeated-occurrence checks).
    pub declared: Vec<(MetavarKind, String)>,
}

impl MetavarTable {
    pub fn from_directives(directives: &[Directive]) -> Result<Self, StructuredError> {
        let mut t = Self::default();
        for d in directives {
            match d {
                Directive::MetaVar { kind, names } => {
                    for n in names {
                        if let Some(prev) = t.kinds.get(n) {
                            if *prev != *kind {
                                return Err(StructuredError::new(
                                    ErrorCode::PatternConflict,
                                    format!(
                                        "metavariable `{n}` declared twice with different kinds ({} vs {})",
                                        prev.as_str(),
                                        kind.as_str()
                                    ),
                                ));
                            }
                        }
                        t.kinds.insert(n.clone(), *kind);
                        t.declared.push((*kind, n.clone()));
                    }
                }
                Directive::Symbol { name, path } => {
                    if let Some(prev) = t.kinds.get(name) {
                        if *prev != MetavarKind::Symbol {
                            return Err(StructuredError::new(
                                ErrorCode::PatternConflict,
                                format!(
                                    "metavariable `{name}` declared twice with different kinds ({} vs symbol)",
                                    prev.as_str(),
                                ),
                            ));
                        }
                    }
                    t.kinds.insert(name.clone(), MetavarKind::Symbol);
                    t.declared.push((MetavarKind::Symbol, name.clone()));
                    t.symbols.insert(name.clone(), path.clone());
                }
                Directive::SigiledMetavars(s) => t.sigiled = *s,
                _ => {}
            }
        }
        Ok(t)
    }

    pub fn is_meta(&self, name: &str) -> bool {
        self.kinds.contains_key(name)
    }

    pub fn kind_of(&self, name: &str) -> Option<MetavarKind> {
        self.kinds.get(name).copied()
    }

    /// Names appearing more than once in declaration are fine (same kind);
    /// this returns names bound in a pattern string (bare `$`-aware scan).
    pub fn bound_in_text(&self, text: &str) -> HashSet<String> {
        let mut out = HashSet::new();
        for tok in tokenize_idents(text) {
            let name = if self.sigiled {
                tok.strip_prefix('$').unwrap_or(tok)
            } else {
                tok.trim_start_matches('$')
            };
            // In sigiled mode only `$Name` counts; bare `Name` is literal.
            if self.sigiled && !tok.starts_with('$') {
                continue;
            }
            if self.kinds.contains_key(name) {
                out.insert(name.to_string());
            }
        }
        out
    }
}

/// Classification of a pattern token for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    /// Declared metavariable use.
    Meta(MetavarKind, String),
    /// `...` wildcard.
    Ellipsis,
    /// Backtick-quoted literal or undeclared identifier.
    Literal(String),
}

/// Scan identifier-like tokens (alphanumeric+_+$ prefix, lifetimes kept).
fn tokenize_idents(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c == '$' || c.is_ascii_alphabetic() || c == '_' {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(&text[i..j]);
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> Vec<Directive> {
        vec![
            Directive::MetaVar {
                kind: MetavarKind::Expr,
                names: vec!["E".to_string()],
            },
            Directive::Symbol {
                name: "Old".to_string(),
                path: "crate::old".to_string(),
            },
        ]
    }

    #[test]
    fn table_lookup() {
        let t = MetavarTable::from_directives(&dirs()).unwrap();
        assert!(t.is_meta("E"));
        assert!(t.is_meta("Old"));
        assert!(!t.is_meta("foo"));
    }

    #[test]
    fn sigiled_scan() {
        let mut t = MetavarTable::from_directives(&dirs()).unwrap();
        t.sigiled = true;
        assert!(t.bound_in_text("old($E)").contains("E"));
        assert!(!t.bound_in_text("old(E)").contains("E"));
    }
}
