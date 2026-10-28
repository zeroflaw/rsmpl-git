use std::path::Path;
fn run(dir: &Path, args: &[&str]) -> String {
    String::new()
}
fn run_stdin(dir: &Path, patch: &str, extra: &[&str]) -> String {
    String::new()
}
fn helper() -> String {
    String::new()
}
fn m(dir: &Path, base: &Path, patch: &str) {
    let a = run_stdin(dir, patch, &[]);
    let b = run_stdin(base, "", &[]);
    let c = run_stdin(dir, rename_patch(), &[]);
    let keep_a = "stable";
    let d = run_stdin(dir, patch, &["--check"]);
    let e = run_stdin(base, rename_patch(), &["--check"]);
    let keep_b = "stable";
    let f = run_stdin(dir, patch, &["--format", "json"]);
}
fn rename_patch() -> String {
    String::new()
}
