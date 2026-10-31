fn a(o: Option<i32>, p: Option<i32>) {
    let Some(v) = o;
    let Some(w) = p;
    let _ = (v, w);
}
