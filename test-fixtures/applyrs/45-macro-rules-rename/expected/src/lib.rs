macro_rules! new_m {
    ($x:expr) => { $x + 1 };
}
fn f() -> i32 {
    new_m!(41)
}
