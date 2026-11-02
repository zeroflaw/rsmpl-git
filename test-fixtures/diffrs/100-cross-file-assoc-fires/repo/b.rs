struct S;
impl Tr for S {
    type Item = u8;
}
fn f() -> S::Item {
    S::Item::default()
}
