macro_rules! old_m {
    ($x:expr) => { $x + 1 };
}
fn f() -> i32 {
    old_m!(41)
}
