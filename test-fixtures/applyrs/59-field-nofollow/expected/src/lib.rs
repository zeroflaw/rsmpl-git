struct S {
    new: u32,
    keep: bool,
}
fn f(s: S) -> u32 {
    let lit = S { old: 1, keep: true };
    s.old + lit.old
}
