struct S;
impl Tr for S {
    fn walk(&self) {
        let _ = self;
    }
}
fn f(s: S) {
    s.walk();
}
