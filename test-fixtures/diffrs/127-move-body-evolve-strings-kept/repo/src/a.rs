fn keep() {}
fn fact(n: u64) -> u64 {
    // fact: recursive helper
    let label = "fact";
    if n <= 1 {
        return 1;
    }
    n * fact(n - 1)
}
