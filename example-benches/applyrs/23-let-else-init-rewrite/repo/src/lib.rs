fn f(a: Option<i32>, b: Option<i32>, c: Option<i32>, d: Option<i32>) -> i32 {
    let x = a else { return 0 };
    let y = b else { return 0 };
    let z = c else { return 0 };
    let w = d else { return 0 };
    x + y + z + w
}
