struct S {
    new: u32,
    keep: bool,
}
fn f(s: S) -> u32 {
    let lit = S { new: 1, keep: true };
    s.new + lit.new
}
