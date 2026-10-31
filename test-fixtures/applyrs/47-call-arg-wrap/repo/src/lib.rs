fn use_it(x: i32) -> i32 {
    x
}
fn f(a: i32, b: i32) -> i32 {
    use_it(a) + use_it(b)
}
