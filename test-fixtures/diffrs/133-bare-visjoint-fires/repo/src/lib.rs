pub struct Old {
    x: i32,
}
fn f(s: Old) -> i32 {
    s.x
}
fn g(o: Old) -> Old {
    o
}
