fn m() {
    let out = run(dir.path(), &["--patch", patch]);
    gap();
    let out = run(dir.path(), &["--patch", patch]);
}
