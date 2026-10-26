pub fn helper(x: i32) -> i64 {
    x + 1
}

async fn compute<T>(values: Vec<i32>) -> i32 where T: Clone {
    values.iter().sum()
}
