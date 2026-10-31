enum E {
    Old(u32),
    Keep,
}
fn f(e: E) -> u32 {
    let a = E::Old(1);
    match e {
        E::Old(v) => v,
        E::Keep => 0,
    }
}
