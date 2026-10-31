struct S {
    a: u32,
}

fn f() -> S {
    let x = S { a: 1 };
    x
}

impl Default for S {
    fn default() -> S {
        S { a: 0, ..Default::default() }
    }
}
