#[must_use]
fn bar(x: i32) {
    x
}
fn m() {
    bar(1);
    bar(2);
}
