struct S;
impl Tr for S {
    fn run(&self) {
        let _ = self;
    }
}
fn f(s: S) {
    s.run();
}
