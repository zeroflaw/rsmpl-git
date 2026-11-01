fn bar(x: i32) -> i32 {
    let y = x + 1;
    y * 2
}
fn caller() {
    let r = bar(1);
}
