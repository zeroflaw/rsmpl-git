struct S { f: i32, g: i32 }
fn a(s: S) {
    let S { f: 1, g: v } = s;
    let _ = v;
}
