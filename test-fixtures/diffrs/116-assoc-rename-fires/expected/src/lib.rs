trait Tr {
    type Output;
    fn make() -> Self::Output;
}

struct S;

impl Tr for S {
    type Output = u8;
    fn make() -> Self::Output {
        Self::Output::default()
    }
}

struct T;

impl Tr for T {
    type Output = u16;
    fn make() -> S::Output {
        S::Output::default()
    }
}
