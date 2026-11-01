fn keep2() {}
fn factorial(n: u64) -> u64 {
    // fact: recursive helper
    let label = "fact";
    if n <= 1 {
        return 1;
    }
    n * factorial(n - 1)
}
