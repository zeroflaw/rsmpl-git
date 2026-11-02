trait Labeled {}
struct Inner;
type New = Inner;
impl Labeled for New {
}
fn f(x: New) -> New {
    x
}
