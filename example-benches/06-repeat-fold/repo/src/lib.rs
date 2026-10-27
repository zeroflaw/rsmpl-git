fn a(x: i32) -> i32 {
    let v = Box::new(Box::new(Box::new(x)));
    v
}

fn b(y: i32) -> i32 {
    let w = Box::new(Box::new(y));
    w
}

fn c(z: i32) -> i32 {
    z + 1
}
