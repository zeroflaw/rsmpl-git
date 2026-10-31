struct S { f: i32 }
fn a(s: S) {
    let S { f: v } = s;
    let _ = v;
}
