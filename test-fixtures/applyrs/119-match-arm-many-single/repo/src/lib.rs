fn a(o: Option<i32>) {
    let v = match o { Some(v) => 1, _ => 2 };
    let _ = v;
}

fn b(o: Option<i32>) {
    let w = match o { Some(w) => 3, _ => 2 };
    let _ = w;
}
