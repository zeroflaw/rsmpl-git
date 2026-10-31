fn sum(values: Box<[i32]>) -> i32 {
    values.iter().sum()
}

fn count(items: &[u8]) -> usize {
    items.len()
}
