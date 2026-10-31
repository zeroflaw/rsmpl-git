macro_rules! new_m {
    ($x:expr) => { $x + 1 };
}
fn f() -> i32 {
    old_m!(41)
}
