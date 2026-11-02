# Demo 01 (manual): rename `foo`→`bar` at 12 call sites

Same demo as `run.sh`, step by step. One 5-line semantic patch rewrites
12 call sites; you preview it, apply it, then prove all 12 landed.

## 0. Prerequisites

- `git` and `cargo` on `PATH` (Rust via [rustup](https://rustup.rs)).
- From the repo root, build once:

```sh
cargo build --release -p gitrs -p applyrs
export PATH="$PWD/target/release:$PATH"
```

## 1. Set up a throwaway work copy

```sh
tmp=$(mktemp -d)
cp -r demo/01-apply-rename-calls/repo/. "$tmp/"
cp demo/01-apply-rename-calls/patch.rsmpl "$tmp/"
cd "$tmp" && git init -q .
```

## 2. Look at the semantic patch

```sh
cat patch.rsmpl
```

```text
diff --git src/lib.rs
@@ -1,23 @@ rule=rename_calls
# allow=many
- foo(E)
+ bar(E)
```

Five lines. The agent never names the 12 call sites — the engine finds them.

## 3. Dry-run: lower + verify, apply nothing

```sh
gitrs apply --file patch.rsmpl --check
```

```text
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
```

## 4. Apply, keeping the lowered patch for comparison

```sh
gitrs apply --file patch.rsmpl --output lowered.patch
```

```text
history: ./.history/<id>.rsmpl
applied 1 file(s): src/lib.rs; semantics verified
```

(The `history:` id varies per run; the patch is archived for audit.)

## 5. Prove all 12 sites changed, then measure

```sh
grep -o 'bar' src/lib.rs | wc -l
wc -c patch.rsmpl lowered.patch
```

```text
12
 83 patch.rsmpl
409 lowered.patch
492 total
```

83 bytes of agent output did the work of a 409-byte native patch —
80% smaller, identical end state.

## 6. Clean up

```sh
cd / && rm -rf "$tmp"
```
