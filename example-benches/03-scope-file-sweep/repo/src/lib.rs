fn init() -> i32 {
    legacy(1)
}

fn load(path: &str) -> i32 {
    let base = legacy(2);
    base + legacy(3)
}

fn save(value: i32) -> i32 {
    legacy(value) + legacy(4)
}

fn reset() -> i32 {
    legacy(0)
}

fn ping() -> i32 {
    legacy(10) + legacy(20) + legacy(30)
}

fn version() -> i32 {
    1
}
