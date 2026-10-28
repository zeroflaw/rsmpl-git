struct Out {
    rsmpl: String,
}

const TEXT_SECTION: &str = "section";
const TEXT_STRIPPED: &str = "stripped";

fn check_alpha(out: &Out) {
    assert_eq!(out.rsmpl, TEXT_STRIPPED);
}

fn check_beta(out: &Out) {
    assert_eq!(out.rsmpl, TEXT_STRIPPED);
}

#[test]
fn extra_probe() {
    assert!(true);
}
