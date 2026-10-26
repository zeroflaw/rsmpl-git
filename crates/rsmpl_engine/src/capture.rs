//! Scalar capture store for code moves (SPEC 20).
//!
//! v0.1: captures hold exactly one AST object's source text. `# bind=<Name>`
//! is only supported on item-capture cut hunks; `# use=<Name>` only on pure
//! insertion hunks. Forward references are resolved by the two-pass planner
//! in `plan.rs` (SPEC 20.3, 23.2).

use std::collections::HashMap;

use rsmpl_patch::{ErrorCode, StructuredError};

/// Patch-local capture map: name -> captured source text.
#[derive(Debug, Default)]
pub struct CaptureStore {
    map: HashMap<String, String>,
}

impl CaptureStore {
    pub fn define(
        &mut self,
        name: &str,
        text: String,
        rule: &str,
        file: &str,
    ) -> Result<(), StructuredError> {
        if self.map.contains_key(name) {
            return Err(StructuredError::new(
                ErrorCode::CaptureConflict,
                format!("duplicate `# bind={name}` (scalar captures must be unique)"),
            )
            .with_rule(rule)
            .with_file(file));
        }
        self.map.insert(name.to_string(), text);
        Ok(())
    }

    pub fn get(&self, name: &str, rule: &str, file: &str) -> Result<&str, StructuredError> {
        self.map.get(name).map(|s| s.as_str()).ok_or_else(|| {
            let mut known: Vec<&str> = self.map.keys().map(|s| s.as_str()).collect();
            known.sort();
            StructuredError::new(
                ErrorCode::CaptureConflict,
                format!("capture `{name}` is not bound by any hunk"),
            )
            .with_rule(rule)
            .with_file(file)
            .with_suggestion(if known.is_empty() {
                "add a hunk with `# bind=<Name>`".to_string()
            } else {
                format!("available captures: {}", known.join(", "))
            })
        })
    }
}
