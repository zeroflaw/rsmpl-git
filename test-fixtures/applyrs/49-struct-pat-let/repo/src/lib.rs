struct S {
    f: i32,
    g: i32,
}
fn f(e: S) -> i32 {
    let S { f: a, g: b } = e;
    a + b
}
