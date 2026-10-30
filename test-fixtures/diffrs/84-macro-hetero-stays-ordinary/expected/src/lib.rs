const FIRST_MODE: &str = "first";
const SECOND_MODE: &str = "second";

fn handle_only(request_context: &str) {
    check!(request_context, SECOND_MODE);
}
