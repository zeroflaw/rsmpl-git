fn a() {
    let f: Box<dyn Fn(u8) -> u8> = build();
    f
}
fn b() {
    let f: Box<dyn Fn(u16) -> u8> = build();
    f
}
fn c() {
    let f: Box<dyn Fn(u32) -> u8> = build();
    f
}
fn d() {
    let f: Box<dyn Fn(bool) -> u8> = build();
    f
}
