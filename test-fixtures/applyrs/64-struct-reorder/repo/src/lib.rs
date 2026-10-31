struct S { f: i32, g: i32 }
fn a(s: S) {
    let S { g: 1, f: v } = s;
    let _ = v;
}
