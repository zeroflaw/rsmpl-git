fn keep2() {}
pub fn factorial(
    n: u64,
) -> u64 {
    n as u64 * factorial(n - 1)
}
