struct Rec {
    text: String,
    keep: i32,
}
fn make_text(s: &str) -> String {
    s.to_string()
}
fn make_text_new(s: &str) -> String {
    s.to_string()
}
fn gap() {}
fn m(sub: &str) -> i32 {
    let a = Rec {
        text: make_text(sub),
        keep: 1,
    };
    gap();
    let b = Rec {
        text: make_text(sub),
        keep: 1,
    };
    a.keep + b.keep
}
