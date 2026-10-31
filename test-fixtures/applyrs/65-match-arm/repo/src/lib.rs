fn a(o: Option<i32>) {
    let v = match o { Some(v) => 1, _ => 2 };
    let _ = v;
}
