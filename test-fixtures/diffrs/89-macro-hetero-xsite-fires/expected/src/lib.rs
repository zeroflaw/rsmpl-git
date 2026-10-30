const FIRST_MODE: &str = "first";
const SECOND_MODE: &str = "second";

fn handle_alpha(alpha_context: &str) {
    check!(alpha_context, SECOND_MODE);
}

fn handle_beta(beta_context: &str) {
    check!(beta_context, SECOND_MODE);
}
