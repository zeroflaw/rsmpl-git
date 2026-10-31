fn ready(flag: i32) -> bool {
    true
}
fn ready_done(flag: i32) -> bool {
    true
}
fn m(flag: i32) {
    !ready(flag);
}
