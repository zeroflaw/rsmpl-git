# Demo 02 (manual): compress a diff into one semantic patch

Same demo as `run.sh`, step by step. Twelve `foo(n)`→`bar(n)` call-site
edits are pending in the worktree; `gitrs diff` compresses them into a
single 5-line semantic patch, which you compare against `git diff -U0`.

## 0. Prerequisites

- `git` and `cargo` on `PATH` (Rust via [rustup](https://rustup.rs)).
- From the repo root, build once:

```sh
cargo build --release -p gitrs -p diffrs
export PATH="$PWD/target/release:$PATH"
```

## 1. Set up a throwaway repo with pending changes

`repo/` is the pre-state, `after/` is the post-state. Commit the base,
then overlay the post-state so the change is pending in the worktree:

```sh
ROOT="$PWD"   # run from the repo root
tmp=$(mktemp -d)
cp -r demo/02-diff-call-sweep/repo/. "$tmp/"
cd "$tmp" && git init -q .
printf '*.patch\n' > .gitignore
git add -A
git -c commit.gpgsign=false -c user.email=demo@local -c user.name=demo commit -qm base
cp -r "$ROOT/demo/02-diff-call-sweep/after/." .
```

(The `.gitignore` keeps the `*.patch` measurement files below invisible
to later diffs — diffrs reads untracked files too.)

## 2. View the change as a semantic patch

```sh
gitrs diff --repo .
```

```text
diff --git src/lib.rs
@@ -2,12 @@ rule=r1
# allow=many
- foo(E0)
+ bar(E0)
```

## 3. Capture both spellings and measure

diffrs first: `sem.patch` must not exist yet when it runs (see step 1
note). Plain `git diff` ignores untracked files, so capturing
`native.patch` second is safe either way:

```sh
gitrs diff --repo . > sem.patch
git diff --unified=0 --no-color | grep -v '^index ' > native.patch
wc -c native.patch sem.patch
```

```text
416 native.patch
 75 sem.patch
491 total
```

The 416-byte native diff and the 75-byte semantic patch describe the
same 12-site change — 82% smaller.

## 4. Clean up

```sh
cd / && rm -rf "$tmp"
```
