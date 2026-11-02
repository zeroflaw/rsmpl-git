pub(crate) struct New {
    x: i32,
}
fn f(s: New) -> i32 {
    s.x
}
fn g(o: New) -> New {
    o
}
