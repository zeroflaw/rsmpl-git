enum E {
    A,
    C,
}

fn f(e: E) -> u32 {
    match e {
        E::A => 1,
        E::C => 3,
    }
}

fn g(e: E) -> u32 {
    match e {
        E::A => 10,
        E::C => 30,
    }
}
