use E::*;

enum E {
    A,
}

fn f(e: E) -> u32 {
    match e {
        A => 1,
    }
}
