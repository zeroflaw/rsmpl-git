fn ready(flag: i32) -> bool {
    true
}
fn ready_done(flag: i32) -> bool {
    true
}
fn gap() {}
fn m(flag: i32) {
    !ready_done(flag);
    gap();
    !ready_done(flag);
}
