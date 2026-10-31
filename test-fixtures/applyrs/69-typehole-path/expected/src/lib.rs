struct S { f: i32 }
fn a(s: S) {
    let S { f: v } = s + 1;
    let _ = v;
}
