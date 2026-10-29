//! Old helpers, superseded by the new engine.

fn old_a() -> u32 {
    let base = 1;
    base + 1
}

fn old_b(x: u32) -> u32 {
    let doubled = x * 2;
    doubled + 1
}

/// Scales its input by three.
fn old_c(y: u32) -> u32 {
    let tripled = y * 3;
    tripled
}
