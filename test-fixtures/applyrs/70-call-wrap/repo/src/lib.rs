fn use_it(x: i32) -> i32 { x }
fn a(x: i32) -> i32 {
    use_it(x)
}
fn b(y: i32) -> i32 {
    use_it(y)
}
