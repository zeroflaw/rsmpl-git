/// Adds one.
#[inline]
fn foo(x: i32) -> i32 {
    x + 1
}

fn main() {
    let y = foo(41);
    println!("{y}");
}
