struct A;
struct B<T>(T);
struct C;
struct D;

trait T {
    fn old_name(&self) -> i32;
}

impl A {
    fn old_name(&self) -> i32 {
        1
    }
}

impl<T: Clone> B<T> {
    fn old_name(&self) -> i32 {
        2
    }
}

impl C {
    fn old_name(&self) -> i32 {
        3
    }
}

impl crate::T for D {
    fn old_name(&self) -> i32 {
        4
    }
}

fn old_name() -> i32 {
    0
}
