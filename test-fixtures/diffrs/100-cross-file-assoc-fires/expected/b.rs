struct S;
impl Tr for S {
    type Output = u8;
}
fn f() -> S::Output {
    S::Output::default()
}
