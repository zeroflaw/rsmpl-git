struct S {
    a: u32,
    b: bool,
}

fn f() -> S {
    let x = S { a: 1, b: true };
    x
}

impl Default for S {
    fn default() -> S {
        S { a: 0, ..Default::default() }
    }
}
