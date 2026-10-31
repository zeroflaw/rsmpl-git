fn start() {
    log::info("new-msg");
}

fn stop(code: i32) {
    eprintln_code(code, "new-msg");
}

fn pause() {
    let m = "new-msg";
    send(m);
}

fn resume() -> &'static str {
    return "new-msg";
}
