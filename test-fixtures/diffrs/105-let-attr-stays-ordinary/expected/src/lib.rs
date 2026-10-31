fn run() {
    #[allow(clippy::x)]
    let a = compute_one();
    #[allow(clippy::x)]
    let b = compute_two();
    #[allow(clippy::x)]
    let c = compute_three();
    #[allow(clippy::x)]
    let d = compute_four();
    let _ = (a, b, c, d);
}
