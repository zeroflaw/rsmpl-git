//! T160: mixed `...` in fn param/body slots fails fast with
//! `RSMPL_PATTERN_CONFLICT` instead of silently corrupting or ignoring.
//!
//! Split into its own file to keep every test file under the 2000-LOC
//! repo limit (`no_rust_file_exceeds_2000_lines`).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"))
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let st = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(st.success());
    fs::create_dir_all(dir.path().join("src")).unwrap();
    dir
}

fn write(dir: &Path, rel: &str, text: &str) {
    fs::write(dir.join(rel), text).unwrap();
}

fn read(dir: &Path, rel: &str) -> String {
    fs::read_to_string(dir.join(rel)).unwrap()
}

fn run_stdin(dir: &Path, patch: &str, extra: &[&str]) -> std::process::Output {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(binary())
        .args(extra)
        .arg("--repo")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(patch.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn mixed_param_ellipsis_fails_fast_text() {
    // T160 P1: `- fn foo(..., c: T)` + `+ fn foo(c: T)` used to emit a
    // literal `fn foo(c: T) {}` with exit 0 (unbound metavar leak). Now
    // the parser rejects the mixed `...` shape before matching.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(a: i32, b: i32, c: i32) {}\n",
    );
    let before = read(dir.path(), "src/lib.rs");
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# lang=rust\n# type T\n- fn foo(..., c: T) {\n+ fn foo(c: T) {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(stderr.contains("must stand alone"), "got: {stderr}");
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "worktree untouched");
}

#[test]
fn mixed_body_ellipsis_fails_fast_json() {
    // T160 P4: a plus body of `{ ..., foo(); }` used to silently
    // preserve the body (exit 0) without verifying `foo();`. Now the
    // parser rejects it; the stdout-only JSON envelope carries the code.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn foo(x: i32) -> i32 {\n    x + 1\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=t\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ..., foo();\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json"]);
    assert!(!out.status.success(), "must fail");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
    assert!(stdout.contains("exactly `...`"), "got: {stdout}");
}

#[test]
fn ellipsis_in_generics_where_return_fails_fast() {
    // T161: `...` inside `<...>`, `where ...`, or `-> ...` used to fail
    // safe with a bare `no match found` (never equal to a real slice).
    // Now the parser fails fast with a repair needle; worktree untouched.
    let cases = [
        (
            "fn foo<T>(x: T) -> i32 {\n    1\n}\n",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# scope=file\n- fn foo<..., T>(...) {\n+ fn bar<..., T>(...) {\n      ...\n  }\n",
            "generic parameter",
        ),
        (
            "fn foo<T>(x: T) where T: Clone {\n    x\n}\n",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# scope=file\n- fn foo<T>(...) where ..., T: Clone {\n+ fn bar<T>(...) where ..., T: Clone {\n      ...\n  }\n",
            "where clause",
        ),
        (
            "fn bar(x: i32) -> i32 {\n    x\n}\n",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# scope=file\n- fn bar(...) -> ... {\n+ fn baz(...) -> ... {\n      ...\n  }\n",
            "return type",
        ),
    ];
    for (src, patch, needle) in cases {
        let dir = init_repo();
        write(dir.path(), "src/lib.rs", src);
        let before = read(dir.path(), "src/lib.rs");
        let out = run_stdin(dir.path(), patch, &["--check"]);
        assert!(!out.status.success(), "must fail for {needle}");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
        assert!(stderr.contains(needle), "got: {stderr}");
        assert_eq!(read(dir.path(), "src/lib.rs"), before, "untouched");
        // JSON envelope carries the same needle stdout-only.
        let out = run_stdin(dir.path(), patch, &["--format", "json"]);
        assert!(!out.status.success(), "json must fail for {needle}");
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(stdout.contains("RSMPL_PATTERN_CONFLICT"), "got: {stdout}");
        assert!(stdout.contains(needle), "got: {stdout}");
    }
}

#[test]
fn unterminated_signature_fails_fast_with_needle() {
    // T174: a dropped `{` lets the return/where accumulator swallow the
    // body lines, tripping the T161 `...`-content gate with a needle that
    // blames the slot content. Name the missing brace instead.
    let cases = [
        (
            "fn rtest(x: i32) -> i32 {\n    x\n}\n",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# scope=file\n# lang=rust\n- fn rtest(...) -> i32\n+ fn rtest2(...) -> i32\n      ...\n  }\n",
        ),
        (
            "fn wtest<T>(x: T) where T: Clone {\n    x\n}\n",
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# scope=file\n- fn wtest<T>(...)\n- where T: Clone\n+ fn wtest2<T>(...)\n+ where T: Clone\n      ...\n  }\n",
        ),
    ];
    for (src, patch) in cases {
        let dir = init_repo();
        write(dir.path(), "src/lib.rs", src);
        let before = read(dir.path(), "src/lib.rs");
        let out = run_stdin(dir.path(), patch, &["--check"]);
        assert!(!out.status.success(), "must fail");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
        assert!(stderr.contains("missing its opening"), "got: {stderr}");
        assert_eq!(read(dir.path(), "src/lib.rs"), before, "untouched");
    }
}

#[test]
fn brace_on_where_line_applies() {
    // T174 PA control: the `{` may sit on the trailing `where` line —
    // only the unterminated shape fails.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "fn wtest<T>(x: T) where T: Clone {\n    x\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,1 @@ rule=t\n# scope=file\n# lang=rust\n- fn wtest<T>(...)\n- where T: Clone {\n+ fn wtest2<T>(...)\n+ where T: Clone {\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("fn wtest2"), "got: {stdout}");
}

#[test]
fn impl_pub_method_evolution_applies() {
    // T175: explicit-`pub` impl-context evolution used to fail with
    // `no match found` (the presence-gate synth dropped the method's
    // visibility). `--check` previews the evolved signature, then a real
    // apply lands byte-exact with the body preserved.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\n\nimpl S {\n    pub fn get(&self, x: i32) -> i32 {\n        x\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,4 @@ rule=t\n# lang=rust\n  impl S {\n      ...\n-     pub fn get(&self, x: i32) -> i32 {\n+     pub fn get(&self, x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("pub fn get(&self, x: i64) -> i64"),
        "got: {stdout}"
    );
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "struct S;\n\nimpl S {\n    pub fn get(&self, x: i64) -> i64 {\n        x\n    }\n}\n"
    );
}

#[test]
fn impl_pub_method_evolution_json_check() {
    // T175 JSON pin: `--check` reports `"check":true` with the rule match.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\n\nimpl S {\n    pub fn make(x: i32) -> i32 {\n        x\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,4 @@ rule=t\n# lang=rust\n  impl S {\n      ...\n-     pub fn make(x: i32) -> i32 {\n+     pub fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("\"check\":true"), "got: {stdout}");
}

#[test]
fn trait_declaration_rename_applies() {
    // T182: `;` trait-method declarations rename through the trait half of
    // method-context matching (previously `RSMPL_MATCH_CONFLICT`).
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "trait T {\n    fn foo();\n}\n");
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=t\n# lang=rust\n  trait T {\n      ...\n-     fn foo();\n+     fn bar();\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "trait T {\n    fn bar();\n}\n"
    );
}

#[test]
fn unsafe_trait_rename_applies() {
    // T185: `unsafe trait` headers route like plain `trait` ones (the
    // fn-block scan skips block headers despite the `unsafe ` prefix).
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "unsafe trait T {\n    fn foo();\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=t\n# lang=rust\n  unsafe trait T {\n      ...\n-     fn foo();\n+     fn bar();\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "unsafe trait T {\n    fn bar();\n}\n"
    );
}

#[test]
fn unsafe_impl_rename_applies() {
    // T185: same block-header skip on the impl half.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "trait U { fn m(x: i32) -> i32; }\nstruct S;\nunsafe impl U for S {\n    fn m(x: i32) -> i32 { x }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,3 @@ rule=t\n# lang=rust\n  unsafe impl U for S {\n      ...\n-     fn m(...) {\n+     fn n(...) {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        read(dir.path(), "src/lib.rs").contains("fn n(x: i32) -> i32 { x }"),
        "got: {}",
        read(dir.path(), "src/lib.rs")
    );
}

#[test]
fn trait_plus_pub_fails_fast() {
    // T182: trait methods never carry visibility — a `+` side writing
    // `pub` fails with the repair needle, not a reparse error.
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", "trait T {\n    fn foo();\n}\n");
    let before = read(dir.path(), "src/lib.rs");
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=t\n# lang=rust\n  trait T {\n      ...\n-     fn foo();\n+     pub fn bar();\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--check"]);
    assert!(!out.status.success(), "must fail");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("RSMPL_PATTERN_CONFLICT"), "got: {stderr}");
    assert!(
        stderr.contains("cannot declare visibility"),
        "got: {stderr}"
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), before, "worktree untouched");
}

#[test]
fn impl_const_fn_return_evolution_applies() {
    // T190: qualified methods share the impl-context slot planner --
    // `const fn` return evolution applies byte-exact.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\n\nimpl S {\n    const fn make(x: i32) -> i32 {\n        x + 1\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,5 @@ rule=t\n# lang=rust\n  impl S {\n      ...\n-     const fn make(x: i32) -> i32 {\n+     const fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "struct S;\n\nimpl S {\n    const fn make(x: i64) -> i64 {\n        x + 1\n    }\n}\n"
    );
}

#[test]
fn impl_async_fn_evolution_json_report() {
    // T190: JSON report pin -- `async fn` evolution carries matches/spans.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\n\nimpl S {\n    async fn make(x: i32) -> i32 {\n        x + 1\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,5 @@ rule=t\n# lang=rust\n  impl S {\n      ...\n-     async fn make(x: i32) -> i32 {\n+     async fn make(x: i64) -> i64 {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "must check clean: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("\"matches\":1"), "got: {stdout}");
    assert!(stdout.contains("\"spans\":[[4,4],[4,4]]"), "got: {stdout}");
    assert!(stdout.contains("\"check\":true"), "got: {stdout}");
}

#[test]
fn impl_returns_none_constrains_and_applies() {
    // T209: `# returns=none` in method context — the engine already
    // enforced it, only the compile gate rejected the spelling.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\n\nimpl S {\n    pub fn make(x: i32) {\n        let _ = x;\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,4 @@ rule=t\n# lang=rust\n# returns=none\n  impl S {\n      ...\n-     pub fn make(...) {\n+     pub fn build(...) {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "struct S;\n\nimpl S {\n    pub fn build(x: i32) {\n        let _ = x;\n    }\n}\n"
    );
}

#[test]
fn impl_returns_none_rejects_typed_target() {
    // T209 negative control: the target carries `-> i32`, so the
    // minus-side constraint fails safe instead of rewriting.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\n\nimpl S {\n    pub fn make(x: i32) -> i32 {\n        x\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -3,4 @@ rule=t\n# lang=rust\n# returns=none\n  impl S {\n      ...\n-     pub fn make(...) {\n+     pub fn build(...) {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        !out.status.success(),
        "must fail safe: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("RSMPL_MATCH_CONFLICT"), "got: {stdout}");
}

#[test]
fn trait_returns_none_decl_applies() {
    // T209 trait half: `;`-declaration with no return type renames.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "trait T {\n    fn make(x: i32);\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -1,3 @@ rule=t\n# lang=rust\n# returns=none\n  trait T {\n      ...\n-     fn make(...);\n+     fn build(...);\n      ...\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "trait T {\n    fn build(x: i32);\n}\n"
    );
}

#[test]
fn impl_lifetime_receiver_rename_applies() {
    // T221: `&'a self` used to break the impl-context synth (blind
    // `str::replace("self", …)` turned it into `& 'a _self_placeholder
    // : u8`, unparsable as a reference pattern) with the bare
    // `cannot analyze method signature` needle.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\nimpl S {\n    fn get<'a>(&'a self, x: &'a i32) -> &'a i32 {\n        x\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -2,4 @@ rule=t\n# lang=rust\n  impl S {\n      ...\n-     fn get<'a>(&'a self, x: &'a i32) -> &'a i32 {\n+     fn fetch<'a>(&'a self, x: &'a i32) -> &'a i32 {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &[]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(dir.path(), "src/lib.rs"),
        "struct S;\nimpl S {\n    fn fetch<'a>(&'a self, x: &'a i32) -> &'a i32 {\n        x\n    }\n}\n"
    );
}

#[test]
fn impl_typed_self_receiver_rename_json() {
    // T221: `self: Box<Self>` used to break the same synth (`self`
    // gained a bogus `: u8` after the written type). JSON pin: rename
    // applies with the typed receiver preserved verbatim.
    let dir = init_repo();
    write(
        dir.path(),
        "src/lib.rs",
        "struct S;\nimpl S {\n    fn boxed(self: Box<Self>) -> i32 {\n        1\n    }\n}\n",
    );
    let patch = "diff --git src/lib.rs\n@@ -2,4 @@ rule=t\n# lang=rust\n  impl S {\n      ...\n-     fn boxed(self: Box<Self>) -> i32 {\n+     fn unboxed(self: Box<Self>) -> i32 {\n          ...\n      }\n  }\n";
    let out = run_stdin(dir.path(), patch, &["--format", "json", "--check"]);
    assert!(
        out.status.success(),
        "must apply: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("\"check\":true"), "got: {stdout}");
}
