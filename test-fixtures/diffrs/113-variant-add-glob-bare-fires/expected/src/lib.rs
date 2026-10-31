use E::*;

enum E {
    A,
    C,
}

fn f(e: E) -> u32 {
    match e {
        A => 1,
        C => 3,
    }
}
