fn keep() {}
fn fact(
    n: u32,
) -> u64 {
    n as u64 * fact(n - 1)
}
