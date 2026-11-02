trait Printable {}
struct Inner;
type Old = Inner;
impl Printable for Old {
}
fn f(x: Old) -> Old {
    x
}
