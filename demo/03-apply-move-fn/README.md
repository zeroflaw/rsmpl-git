# Demo 03: move a 44-line fn across files

A 9-line cut+paste patch moves `helper` from `src/old.rs` to
`src/new.rs`. The 44-line body is named once via `# bind` / `# use`
and never restated; the demo previews the lowering with `--check`,
applies it, proves the fn moved, and measures the patch at 142 bytes
against the 1,576-byte native equivalent.

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

Output of `./run.sh`. The `--check` diff (44 `-` lines plus 44 `+`
lines) is elided in the middle; the `history:` id varies per run:

```text
$ cat patch.rsmpl
diff --git src/old.rs
@@ -1,44 @@ rule=cut_helper
# bind=Helper
- I

diff --git src/new.rs
@@ -3,0 @@ rule=paste_helper
# use=Helper
+ Helper
$ gitrs apply --file patch.rsmpl --check
diff --git a/src/old.rs b/src/old.rs
--- a/src/old.rs
+++ b/src/old.rs
@@ -1,45 +0,0 @@
-fn helper(seed: i32) -> i32 {
-    let mut acc = seed;
-    acc += 1;
[... remaining cut lines, then the paste into src/new.rs ...]
+    acc
+}
check: lowered patch applies cleanly; semantics verified
$ gitrs apply --file patch.rsmpl --output lowered.patch
history: ./.history/<id varies>.rsmpl
applied 2 file(s): src/old.rs, src/new.rs; semantics verified
$ grep -rn 'fn helper' src/
src/new.rs:4:fn helper(seed: i32) -> i32 {
$ wc -c patch.rsmpl lowered.patch
 142 patch.rsmpl
1576 lowered.patch
1718 total
```
