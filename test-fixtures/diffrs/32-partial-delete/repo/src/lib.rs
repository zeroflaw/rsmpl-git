use std::path::PathBuf;

fn binary() -> PathBuf {
    let exe = env!("CARGO_BIN_EXE_tool");
    PathBuf::from(exe)
}

fn init_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    dir
}

fn keep() {
    work()
}
