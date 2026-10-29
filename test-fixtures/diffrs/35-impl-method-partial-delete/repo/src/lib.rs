use std::collections::HashMap;

pub struct Catalog {
    entries: HashMap<String, u32>,
}

impl Catalog {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Warn (not error) on lowercase names.
    pub fn lowercase_warnings(&self) -> Vec<String> {
        self.entries
            .keys()
            .filter(|n| n.starts_with(|c: char| c.is_ascii_lowercase()))
            .map(|n| format!("entry `{n}` is lowercase"))
            .collect()
    }
}
