use std::path::PathBuf;
use std::process::Command;

fn run(_out: std::process::Child) {}

fn one() {
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_applyrs")));
    run(out);
}

fn two() {
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_applyrs")));
    run(out);
}

fn three() {
    let out = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_applyrs")));
    run(out);
}
