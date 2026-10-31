trait Tr {
    type Item;
    fn make() -> Self::Item;
}

struct S;

impl Tr for S {
    type Item = u8;
    fn make() -> Self::Item {
        Self::Item::default()
    }
}

struct T;

impl Tr for T {
    type Item = u16;
    fn make() -> S::Item {
        S::Item::default()
    }
}

fn first() -> Tr::Item {
    S::Item::default()
}

fn second(s: S::Item) -> Tr::Item {
    s
}
