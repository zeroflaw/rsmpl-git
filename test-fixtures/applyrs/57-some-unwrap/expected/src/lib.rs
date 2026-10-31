fn a(o: Option<i32>, p: Option<i32>) {
    let Some(v) = o.unwrap_or(0);
    let Some(w) = p.unwrap_or(0);
    let _ = (v, w);
}
