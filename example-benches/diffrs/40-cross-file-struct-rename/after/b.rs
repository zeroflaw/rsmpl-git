struct Pair {
    first: New,
    second: New,
}
impl New {
    fn make() -> New {
        New { x: 1, y: 2 }
    }
}
fn get(o: New) -> New {
    o
}
