fn a(o: Option<i32>) {
    let v = match o { Some(v) => 9, _ => 2 };
    let _ = v;
}
