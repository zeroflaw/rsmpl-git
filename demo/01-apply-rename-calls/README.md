# Demo 01: rename `foo`→`bar` at 12 call sites

One 5-line semantic patch (`patch.rsmpl`) rewrites 12 call sites in
`repo/src/lib.rs`. The demo previews the lowering with `--check`,
applies it, proves all 12 sites changed, and measures the patch at
83 bytes against the 409-byte native equivalent.

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

Output of `./run.sh` (abbreviated nowhere; the `history:` id varies
per run):

```text
$ cat patch.rsmpl
diff --git src/lib.rs
@@ -1,23 @@ rule=rename_calls
# allow=many
- foo(E)
+ bar(E)
$ gitrs apply --file patch.rsmpl --check
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -2 +2 @@
-    foo(1) + foo(2)
+    bar(1) + bar(2)
@@ -6 +6 @@
-    foo(3) + foo(4)
+    bar(3) + bar(4)
@@ -10 +10 @@
-    foo(5) + foo(6)
+    bar(5) + bar(6)
@@ -14 +14 @@
-    foo(7) + foo(8)
+    bar(7) + bar(8)
@@ -18 +18 @@
-    foo(9) + foo(10)
+    bar(9) + bar(10)
@@ -22 +22 @@
-    foo(11) + foo(12)
+    bar(11) + bar(12)
check: lowered patch applies cleanly; semantics verified
$ gitrs apply --file patch.rsmpl --output lowered.patch
history: ./.history/<id varies>.rsmpl
applied 1 file(s): src/lib.rs; semantics verified
$ grep -o 'bar' src/lib.rs | wc -l
12
$ wc -c patch.rsmpl lowered.patch
 83 patch.rsmpl
409 lowered.patch
492 total
```
