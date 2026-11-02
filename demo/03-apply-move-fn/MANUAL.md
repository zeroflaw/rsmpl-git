# Demo 03 (manual): move a 44-line fn across files

Same demo as `run.sh`, step by step. A 9-line cut+paste patch moves
`helper` from `src/old.rs` to `src/new.rs` — the 44-line body is named
once via `# bind` / `# use`, never restated.

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
cp -r demo/03-apply-move-fn/repo/. "$tmp/"
cp demo/03-apply-move-fn/patch.rsmpl "$tmp/"
cd "$tmp" && git init -q .
```

## 2. Look at the semantic patch

```sh
cat patch.rsmpl
```

```text
diff --git src/old.rs
@@ -1,44 @@ rule=cut_helper
# bind=Helper
- I

diff --git src/new.rs
@@ -3,0 @@ rule=paste_helper
# use=Helper
+ Helper
```

Two hunks, one shared binding: cut the item out of `old.rs`, paste it
into `new.rs`. No line of the body appears here.

## 3. Dry-run: lower + verify, apply nothing

```sh
gitrs apply --file patch.rsmpl --check
```

Output is the full lowered diff — all 44 `-` lines in `src/old.rs`
followed by all 44 `+` lines in `src/new.rs` — ending with:

```text
check: lowered patch applies cleanly; semantics verified
```

## 4. Apply, keeping the lowered patch for comparison

```sh
gitrs apply --file patch.rsmpl --output lowered.patch
```

```text
history: ./.history/<id>.rsmpl
applied 2 file(s): src/old.rs, src/new.rs; semantics verified
```

(The `history:` id varies per run; the patch is archived for audit.)

## 5. Prove the fn moved, then measure

```sh
grep -rn 'fn helper' src/
wc -c patch.rsmpl lowered.patch
```

```text
src/new.rs:4:fn helper(seed: i32) -> i32 {
 142 patch.rsmpl
1576 lowered.patch
1718 total
```

`helper` now lives only in `src/new.rs`. 142 bytes of agent output did
the work of a 1,576-byte native patch — 91% smaller, byte-identical
end state.

## 6. Clean up

```sh
cd / && rm -rf "$tmp"
```
