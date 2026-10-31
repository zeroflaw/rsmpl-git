fn start() {
    log::info("old-msg");
}

fn stop(code: i32) {
    eprintln_code(code, "old-msg");
}

fn pause() {
    let m = "old-msg";
    send(m);
}

fn resume() -> &'static str {
    return "old-msg";
}
