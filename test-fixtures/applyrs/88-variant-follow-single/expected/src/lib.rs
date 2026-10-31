enum E {
    New(u32),
    Keep,
}
fn f(e: E) -> u32 {
    match e {
        E::New(v) => v,
        E::Keep => 0,
    }
}
