use std::collections::HashMap;

pub struct Catalog {
    entries: HashMap<String, u32>,
}

impl Catalog {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}
