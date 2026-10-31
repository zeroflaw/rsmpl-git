enum E {
    A,
}

fn f(e: E) -> u32 {
    match e {
        E::A => 1,
    }
}

fn g(e: E) -> u32 {
    match e {
        E::A => 10,
    }
}
