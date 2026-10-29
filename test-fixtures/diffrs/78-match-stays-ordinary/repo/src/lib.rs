fn a(flag: bool) -> i32 {
    let v = match flag { true => 1, _ => 2 };
    v
}

fn b(flag: bool) -> i32 {
    let v = match flag { true => 1, _ => 2 };
    v
}

fn untouched(flag: bool) -> i32 {
    let v = match flag { true => 1, _ => 2 };
    v
}
