# Demo 02: compress a diff into one semantic patch

Twelve `foo(n)`→`bar(n)` call-site edits are pending in the worktree
(`repo/` is the pre-state, `after/` the post-state). `gitrs diff`
compresses them into a single 5-line semantic patch, measured at
75 bytes against the 416-byte `git diff -U0` of the same change.

## Run it

Two ways, same steps:

- **Automatic** — from this directory:
  ```sh
  ./run.sh            # throwaway work copy, deleted on exit
  ./run.sh --keep     # keep the work copy for inspection (path is printed)
  ```
  The script builds the release binaries on first run, so all you need
  is `git` and `cargo` on `PATH` (Rust via <https://rustup.rs>).
- **Manual** — follow [`MANUAL.md`](MANUAL.md): the same commands as
  copy-pasteable steps with the expected output at each stage.

## Sample trace

Output of `./run.sh`, in full:

```text
$ gitrs diff --repo .
diff --git src/lib.rs
@@ -2,12 @@ rule=r1
# allow=many
- foo(E0)
+ bar(E0)
$ gitrs diff --repo . > sem.patch
$ git diff --unified=0 --no-color | grep -v '^index ' > native.patch
$ wc -c native.patch sem.patch
416 native.patch
 75 sem.patch
491 total
```
