fn clamp(a: i32, b: i32, c: i32, d: i32) -> i32 {
    let x = if a > 0 { 1 } else { 2 };
    let y = if b > 0 { 1 } else { 2 };
    let z = if c > 0 { 1 } else { 2 };
    let w = if d > 0 { 1 } else { 2 };
    x + y + z + w
}
