const FIRST_MODE: &str = "first";
const SECOND_MODE: &str = "second";

fn handle_alpha(request_context: &str) {
    check!(request_context, FIRST_MODE);
}

fn handle_beta(request_context: &str) {
    check!(request_context, FIRST_MODE);
}
