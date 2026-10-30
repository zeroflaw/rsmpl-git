struct Out {
    rsmpl: String,
}

const TEXT_SECTION: &str = "section";
const TEXT_STRIPPED: &str = "stripped";

fn check_alpha(out: &Out) {
    assert_eq!(out.rsmpl, TEXT_SECTION);
}

fn check_beta(out: &Out) {
    assert_eq!(out.rsmpl, TEXT_SECTION);
}
