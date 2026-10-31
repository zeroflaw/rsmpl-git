struct Pair {
    first: Old,
    second: Old,
}
impl Old {
    fn make() -> Old {
        Old { x: 1, y: 2 }
    }
}
fn get(o: Old) -> Old {
    o
}
