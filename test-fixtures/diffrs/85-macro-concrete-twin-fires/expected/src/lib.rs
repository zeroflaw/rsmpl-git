const FIRST_DELTA: &str = "first";
const SECOND_DELTA: &str = "second";
const THIRD_DELTA: &str = "third";

fn handle_alpha(x: &str) {
    check!(x, SECOND_DELTA);
}

fn handle_beta(x: &str) {
    check!(x, SECOND_DELTA);
}

fn handle_gamma(x: &str) {
    check!(x, THIRD_DELTA);
}
