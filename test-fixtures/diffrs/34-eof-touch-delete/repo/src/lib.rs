fn keep() {
    work()
}

fn stale_helper() -> usize {
    let x = compute();
    let y = transform(x);
    y + 1
}
