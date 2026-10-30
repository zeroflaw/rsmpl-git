pub fn a() {
    let r = load(ctx, &["h", m1, t1]);
    ping(1);
}
pub fn b() {
    let r = load(ctx, &["h", m2, t1]);
}
pub fn c() {
    let r = load(ctx, &["h", m3, t1]);
}
pub fn d() {
    let r = load(ctx, &["h", m4, t1]);
}
pub fn e() {
    let r = load(ctx, &["h", m5, t1]);
}
pub fn filler() {
    keep();
}
pub fn gone() {
    drop();
}
