trait T {
    fn old_name(&self) -> i32;
}

trait U<T: Clone> {
    fn old_name(&self) -> i32;
}

trait V {
    fn old_name(&self) -> i32 {
        3
    }
}

impl Svc {
    fn old_name(&self) -> i32 {
        0
    }
}

fn old_name() -> i32 {
    1
}
