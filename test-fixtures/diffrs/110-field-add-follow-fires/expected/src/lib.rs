struct S {
    a: u32,
    b: bool,
}

fn f() -> S {
    S { a: 1, b: true }
}

fn g() -> S {
    S { a: 2, b: false }
}
