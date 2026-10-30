//! WI-3c: executable written contract — every `diff rsmpl` fence in
//! SPEC §§28–29, AGENTS.md, and SKILL.md parses, is diet-canonical,
//! and the curated matrix applies for real.
//!
//! Extraction is by fence tag (`rsmpl` in the info string), never
//! heuristics. Hunk-only fences without a `diff --git` line are
//! completed with `diff --git src/lib.rs` (SPEC §29 intro).

mod common;
use common::*;

fn read_root(rel: &str) -> String {
    let root = workspace_root();
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// Fences whose info string carries the `rsmpl` token.
fn extract_rsmpl_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut cur: Option<Vec<String>> = None;
    for line in text.lines() {
        if let Some(mut c) = cur.take() {
            if line.starts_with("```") {
                c.push(String::new());
                let mut body = c.join("\n");
                if !body.ends_with('\n') {
                    body.push('\n');
                }
                blocks.push(body);
            } else {
                c.push(line.to_string());
                cur = Some(c);
            }
        } else if line.starts_with("```") {
            let info = line.trim_start_matches('`').trim();
            if info.split_whitespace().any(|t| t == "rsmpl") {
                cur = Some(Vec::new());
            }
        }
    }
    blocks
}

fn complete_patch(block: &str) -> String {
    if block.lines().any(|l| l.starts_with("diff --git")) {
        block.to_string()
    } else {
        format!("diff --git src/lib.rs\n{block}")
    }
}

fn assert_diet(rel: &str, i: usize, block: &str) {
    let mut has_scope_file = false;
    let mut has_allow_many = false;
    for line in block.lines() {
        let t = line.trim();
        if !t.starts_with('#') {
            continue;
        }
        assert!(
            !t.starts_with("# lang="),
            "{rel} fence {i}: diet omits `# lang=rust` (SPEC 7.1), got: {t}"
        );
        if t == "# scope=file" {
            has_scope_file = true;
        }
        if t == "# allow=many" {
            has_allow_many = true;
        }
        for prefix in ["# expr ", "# item ", "# type ", "# pat ", "# stmt "] {
            assert!(
                !t.starts_with(prefix),
                "{rel} fence {i}: diet infers metavars, no declaration (SPEC 15.8), got: {t}"
            );
        }
    }
    assert!(
        !(has_scope_file && has_allow_many),
        "{rel} fence {i}: diet fuses `# allow=many` + `# scope=file` into `# sweep` (SPEC 9.3)"
    );
}

fn find_block_by_rule(spec: &str, rule: &str) -> String {
    // The curated matrix executes the complete §29 patches: prefer the
    // fence carrying its own `diff --git` header (§28 repeats some
    // shapes as hunk-only fragments).
    let needle = format!("rule={rule}");
    extract_rsmpl_blocks(spec)
        .into_iter()
        .find(|b| b.contains(&needle) && b.contains("diff --git"))
        .unwrap_or_else(|| panic!("no complete rsmpl fence with {needle} in SPEC.md"))
}

#[test]
fn tagged_fences_parse_and_are_diet() {
    let mut total = 0;
    for rel in ["SPEC.md", "AGENTS.md", "skills/gitrs/SKILL.md"] {
        let text = read_root(rel);
        let blocks = extract_rsmpl_blocks(&text);
        assert!(!blocks.is_empty(), "{rel}: no rsmpl-tagged fences");
        for (i, b) in blocks.iter().enumerate() {
            assert_diet(rel, i, b);
            let patch = complete_patch(b);
            rsmpl_patch::parse_patch(&patch)
                .unwrap_or_else(|e| panic!("{rel} fence {i} must parse: {e}\n{patch}"));
            total += 1;
        }
    }
    assert!(
        total >= 15,
        "expected at least 15 tagged fences, got {total}"
    );
}

fn filler_comments(from: usize, to: usize) -> Vec<String> {
    (from..=to).map(|n| format!("// filler {n}")).collect()
}

#[test]
fn spec_29_1_rename_plain_applies() {
    let spec = read_root("SPEC.md");
    let patch = find_block_by_rule(&spec, "rename_plain_foo");
    // Range @@ -10,7 @@: `fn foo` item at line 10 of 16.
    let mut pre = filler_comments(1, 9);
    pre.push("fn foo() {}".to_string());
    pre.extend(filler_comments(11, 16));
    let pre = pre.join("\n") + "\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", &pre);
    let out = run_stdin(dir.path(), &patch, &[]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("fn bar() {}"), "got:\n{post}");
    assert!(!post.contains("fn foo"), "got:\n{post}");
}

#[test]
fn spec_29_3_call_sweep_applies() {
    let spec = read_root("SPEC.md");
    let patch = find_block_by_rule(&spec, "rename_calls");
    // Range @@ -40,8 @@: three `foo(N);` sites at lines 40-42.
    let mut pre = vec!["fn main() {".to_string(), "    let _a = 0;".to_string()];
    for n in 3..=39 {
        pre.push(format!("    let _f{n} = {n};"));
    }
    pre.push("    foo(1);".to_string());
    pre.push("    foo(2);".to_string());
    pre.push("    foo(3);".to_string());
    for n in 43..=47 {
        pre.push(format!("    let _g{n} = {n};"));
    }
    pre.push("}".to_string());
    assert_eq!(pre.len(), 48);
    let pre = pre.join("\n") + "\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", &pre);
    let out = run_stdin(dir.path(), &patch, &[]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("    bar(1);"), "got:\n{post}");
    assert!(post.contains("    bar(2);"), "got:\n{post}");
    assert!(post.contains("    bar(3);"), "got:\n{post}");
    assert!(!post.contains("foo("), "got:\n{post}");
}

#[test]
fn spec_29_4_method_to_function_applies() {
    let spec = read_root("SPEC.md");
    let patch = find_block_by_rule(&spec, "method_to_free_function");
    // Range @@ -75,20 @@: one method site at line 75.
    let mut pre = vec!["fn main() {".to_string(), "    let x = 0;".to_string()];
    for n in 3..=74 {
        pre.push(format!("    let _f{n} = {n};"));
    }
    pre.push("    x.old_method(1);".to_string());
    for n in 76..=94 {
        pre.push(format!("    let _g{n} = {n};"));
    }
    pre.push("}".to_string());
    assert_eq!(pre.len(), 95);
    let pre = pre.join("\n") + "\n";
    let dir = init_repo();
    write(dir.path(), "src/service.rs", &pre);
    let out = run_stdin(dir.path(), &patch, &[]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let post = read(dir.path(), "src/service.rs");
    assert!(post.contains("    new_function(x, 1);"), "got:\n{post}");
    assert!(!post.contains("old_method"), "got:\n{post}");
}

#[test]
fn spec_29_5_symbol_rename_applies() {
    let spec = read_root("SPEC.md");
    let patch = find_block_by_rule(&spec, "rename_symbol_calls");
    // Range @@ -80,20 @@: one qualified site at line 80.
    let mut pre = vec!["fn main() {".to_string()];
    for n in 2..=79 {
        pre.push(format!("    let _f{n} = {n};"));
    }
    pre.push("    crate::old_api(1);".to_string());
    for n in 81..=99 {
        pre.push(format!("    let _g{n} = {n};"));
    }
    pre.push("}".to_string());
    assert_eq!(pre.len(), 100);
    let pre = pre.join("\n") + "\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", &pre);
    let out = run_stdin(dir.path(), &patch, &[]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let post = read(dir.path(), "src/lib.rs");
    assert!(post.contains("    crate::new_api(1);"), "got:\n{post}");
    assert!(!post.contains("old_api"), "got:\n{post}");
}

#[test]
fn spec_29_6_move_applies() {
    let spec = read_root("SPEC.md");
    let patch = find_block_by_rule(&spec, "cut_helper");
    assert!(
        patch.contains("rule=paste_helper"),
        "move patch must carry both hunks"
    );
    // Cut range @@ -20,12 @@: `fn Helper` item at line 20 of 31.
    let mut old = filler_comments(1, 19);
    old.push("fn Helper() {}".to_string());
    old.extend(filler_comments(21, 31));
    let old = old.join("\n") + "\n";
    let new = "// new file\n// line 2\n// line 3\n// line 4\n// line 5\n";
    let dir = init_repo();
    write(dir.path(), "src/old.rs", &old);
    write(dir.path(), "src/new.rs", new);
    let out = run_stdin(dir.path(), &patch, &[]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let old_post = read(dir.path(), "src/old.rs");
    let new_post = read(dir.path(), "src/new.rs");
    assert!(!old_post.contains("Helper"), "cut failed, got:\n{old_post}");
    assert!(
        new_post.contains("fn Helper() {}"),
        "paste failed, got:\n{new_post}"
    );
}

#[test]
fn spec_29_10_optional_zero_match_succeeds_unchanged() {
    let spec = read_root("SPEC.md");
    let patch = find_block_by_rule(&spec, "optional_old_api_cleanup");
    // Range @@ -100,30 @@ on a 130-line file with no `old_api` calls:
    // zero matches must succeed unchanged.
    let mut pre = vec!["fn main() {".to_string()];
    for n in 2..=129 {
        pre.push(format!("    let _f{n} = {n};"));
    }
    pre.push("}".to_string());
    assert_eq!(pre.len(), 130);
    let pre = pre.join("\n") + "\n";
    let dir = init_repo();
    write(dir.path(), "src/lib.rs", &pre);
    let out = run_stdin(dir.path(), &patch, &["--check"]);
    assert!(
        out.status.success(),
        "optional zero-match --check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = run_stdin(dir.path(), &patch, &[]);
    assert!(
        out.status.success(),
        "optional zero-match apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read(dir.path(), "src/lib.rs"), pre);
}
