trait T {
    fn new_name(&self) -> i32;
}

trait U<T: Clone> {
    fn new_name(&self) -> i32;
}

trait V {
    fn new_name(&self) -> i32 {
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
