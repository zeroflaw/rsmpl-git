fn keep() {}
fn foo(x: i32) -> i32 {
    let v0 = 0;
    let v1 = 1;
    let v2 = 2;
    let v3 = 3;
    let y = x + v0 + v1 + v2 + v3;
    y * 2
}
fn caller() { let r = foo(1); }
