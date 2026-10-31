struct Out {
    rsmpl: String,
}

const TEXT_SECTION: &str = "section";
const TEXT_STRIPPED: &str = "stripped";

fn check_alpha(out: &Out) {
    assert_eq!(out.rsmpl, TEXT_STRIPPED);
}

fn check_beta(other: &Out) {
    assert_eq!(other.rsmpl, TEXT_STRIPPED);
}

fn check_gamma(third: &Out) {
    assert_eq!(third.rsmpl, TEXT_STRIPPED);
}

fn check_delta(fourth: &Out) {
    assert_eq!(fourth.rsmpl, TEXT_STRIPPED);
}

fn unchanged_helper() {}

#[test]
fn extra_probe() {
    assert!(true);
}
