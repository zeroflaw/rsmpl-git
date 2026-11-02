mod a;
use a::Tr;

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

fn first() -> Tr::Output {
    S::Output::default()
}

fn second(s: S::Output) -> Tr::Output {
    s
}
