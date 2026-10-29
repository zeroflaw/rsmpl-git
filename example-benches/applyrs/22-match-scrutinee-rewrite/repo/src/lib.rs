fn f(a: Option<i32>, b: Option<i32>, c: Option<i32>, d: Option<i32>) -> i32 {
    let x = match a { Some(x) => 1, _ => 2 };
    let y = match b { Some(x) => 1, _ => 2 };
    let z = match c { Some(x) => 1, _ => 2 };
    let w = match d { Some(x) => 1, _ => 2 };
    x + y + z + w
}
