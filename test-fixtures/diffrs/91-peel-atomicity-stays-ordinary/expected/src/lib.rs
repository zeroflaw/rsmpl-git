pub fn a() {
    let r = fetch(ctx, m1, &[t1]);
    ping(2);
}
pub fn b() {
    let r = fetch(ctx, m2, &[t1]);
}
pub fn c() {
    let r = fetch(ctx, m3, &[t1]);
}
pub fn d() {
    let r = fetch(ctx, m4, &[t1]);
}
pub fn e() {
    let r = fetch(ctx, m5, &[t1]);
}
pub fn filler() {
    keep();
}
pub fn gone() {
}
