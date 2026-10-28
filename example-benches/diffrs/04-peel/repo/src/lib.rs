use std::path::Path;
fn run(dir: &Path, args: &[&str]) -> String {
    String::new()
}
fn run_stdin(dir: &Path, patch: &str, extra: &[&str]) -> String {
    String::new()
}
fn m(dir: &Path, base: &Path, patch: &str) {
    let a = run(dir, &["--patch", patch]);
    let b = run(base, &["--patch", ""]);
    let c = run(dir, &["--patch", rename_patch()]);
    let keep_a = "stable";
    let d = run(dir, &["--patch", patch, "--check"]);
    let e = run(base, &["--patch", rename_patch(), "--check"]);
    let keep_b = "stable";
    let f = run(dir, &["--patch", patch, "--format", "json"]);
}
fn rename_patch() -> String {
    String::new()
}
