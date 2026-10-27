fn foo(x: i32) -> i32 {
    let y = x + 1;
    let z = y * 2;
    let w = z - 3;
    w
}

async fn foo(x: i32) -> i32 {
    let y = x + 1;
    let z = y * 2;
    let w = z - 3;
    w
}

pub fn foo(x: i32) -> i32 {
    let y = x + 1;
    let z = y * 2;
    let w = z - 3;
    w
}

pub(crate) async fn foo<T>(x: T) -> T where T: Clone {
    let y = x.clone();
    let z = y.clone();
    let w = z.clone();
    w
}
