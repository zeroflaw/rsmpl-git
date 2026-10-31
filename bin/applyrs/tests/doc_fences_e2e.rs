//! WI-20: linted documents — every RSmPL fence in SPEC.md, AGENTS.md,
//! and `skills/gitrs/SKILL.md` is machine-checked, so the documents an
//! agent learns the language from cannot teach a spelling the engine
//! rejects.
//!
//! Extraction is by shape, never by tag alone: a fence is a candidate
//! when its info string carries the `rsmpl` token OR its first
//! non-empty line starts with `diff --git` or `@@` (the heuristic is
//! stated here, per IMPROVE WI-20). Pairing is sequential ```` ``` ````
//! scanning; a stray bare fence line poisons every later pair, so the
//! total-count floor below guards the pairing itself.
//!
//! Each unmarked candidate must `parse_patch`, survive `canonicalize`
//! as a fixed point, and `--check` clean-or-honest against an empty
//! repo: success, `RSMPL_MATCH_CONFLICT`, `RSMPL_RANGE_CONFLICT`, or
//! `GIT_HUNK_CONFLICT` are well-formed (empty-dir artifacts per the
//! SPEC D-triaging note); `PARSE`/`PATTERN`/`SYMBOL`/`CAPTURE`/`IO`
//! conflicts are malformed. A fence that is deliberately not a
//! standalone patch (error illustration, grammar placeholder, header
//! fragment) carries `<!-- rsmpl:invalid CODE -->` on the nearest
//! non-empty line above its opener, and must fail with exactly CODE.

mod common;
use common::*;

/// One extracted fence: info string, body, 1-based opener line.
struct Fence {
    info: String,
    body: String,
    line: usize,
}

fn extract_fences(text: &str) -> Vec<Fence> {
    let mut out = Vec::new();
    let mut cur: Option<(String, Vec<String>, usize)> = None;
    for (i, line) in text.lines().enumerate() {
        if let Some((info, mut body, start)) = cur.take() {
            if line.starts_with("```") {
                body.push(String::new());
                let mut joined = body.join("\n");
                if !joined.ends_with('\n') {
                    joined.push('\n');
                }
                out.push(Fence {
                    info,
                    body: joined,
                    line: start,
                });
            } else {
                let mut body = body;
                body.push(line.to_string());
                cur = Some((info, body, start));
            }
        } else if line.starts_with("```") {
            cur = Some((
                line.trim_start_matches('`').trim().to_string(),
                Vec::new(),
                i + 1,
            ));
        }
    }
    out
}

fn is_candidate(f: &Fence) -> bool {
    if f.info.split_whitespace().any(|t| t == "rsmpl") {
        return true;
    }
    match f.body.lines().find(|l| !l.trim().is_empty()) {
        Some(first) => first.starts_with("diff --git") || first.starts_with("@@"),
        None => false,
    }
}

fn complete_patch(f: &Fence) -> String {
    if f.body.lines().any(|l| l.starts_with("diff --git")) {
        f.body.clone()
    } else {
        format!("diff --git src/lib.rs\n{}", f.body)
    }
}

/// Nearest non-empty line above the opener, when it is an invalid marker.
fn invalid_marker(lines: &[&str], opener_line: usize) -> Option<String> {
    // `opener_line` is 1-based; lines above are `..opener_line - 1`.
    for l in lines[..opener_line.saturating_sub(1)].iter().rev() {
        if l.trim().is_empty() {
            continue;
        }
        let t = l.trim();
        if let Some(rest) = t
            .strip_prefix("<!-- rsmpl:invalid")
            .and_then(|r| r.strip_suffix("-->"))
        {
            return Some(rest.trim().to_string());
        }
        return None;
    }
    None
}

fn first_code(output: &std::process::Output) -> Option<String> {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with("RSMPL_") || t.starts_with("GIT_") {
            return t.split_whitespace().next().map(str::to_string);
        }
    }
    None
}

fn assert_diet(rel: &str, f: &Fence) {
    let mut has_scope_file = false;
    let mut has_allow_many = false;
    for line in f.body.lines() {
        let t = line.trim();
        if !t.starts_with('#') {
            continue;
        }
        assert!(
            !t.starts_with("# lang="),
            "{rel} fence @{}: diet omits `# lang=rust` (SPEC 7.1), got: {t}",
            f.line
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
                "{rel} fence @{}: diet infers metavars, no declaration (SPEC 15.8), got: {t}",
                f.line
            );
        }
    }
    assert!(
        !(has_scope_file && has_allow_many),
        "{rel} fence @{}: diet fuses `# allow=many` + `# scope=file` into `# sweep` (SPEC 9.3)",
        f.line
    );
}

#[test]
fn doc_fences_lint_clean_or_marked() {
    let mut total = 0;
    let mut marked = 0;
    for rel in ["SPEC.md", "AGENTS.md", "skills/gitrs/SKILL.md"] {
        let root = workspace_root();
        let text = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"));
        let raw: Vec<&str> = text.lines().collect();
        let fences = extract_fences(&text);
        for f in fences.iter().filter(|f| is_candidate(f)) {
            total += 1;
            let patch = complete_patch(f);
            if let Some(code) = invalid_marker(&raw, f.line) {
                marked += 1;
                // Marked fences must fail with exactly the named code,
                // either at parse/canonicalize time or on `--check`.
                let parse_ok = rsmpl_patch::parse_patch(&patch).is_ok();
                let canon_ok = rsmpl_patch::canonicalize(&patch).is_ok();
                let dir = init_repo();
                let out = run_stdin(dir.path(), &patch, &["--check"]);
                let got = if out.status.success() {
                    "APPLIED".to_string()
                } else {
                    first_code(&out).unwrap_or_else(|| "NO_CODE".to_string())
                };
                assert_eq!(
                    got, code,
                    "{rel} fence @{}: marked invalid ({code}) but got {got} \
                     (parse_ok={parse_ok}, canon_ok={canon_ok})",
                    f.line
                );
                continue;
            }
            if f.info.split_whitespace().any(|t| t == "rsmpl") {
                assert_diet(rel, f);
            }
            rsmpl_patch::parse_patch(&patch)
                .unwrap_or_else(|e| panic!("{rel} fence @{} must parse: {e}\n{patch}", f.line));
            let once = rsmpl_patch::canonicalize(&patch)
                .unwrap_or_else(|e| panic!("{rel} fence @{} must canonicalize: {e}", f.line));
            let twice = rsmpl_patch::canonicalize(&once).unwrap_or_else(|e| {
                panic!("{rel} fence @{} canonical form must re-parse: {e}", f.line)
            });
            assert_eq!(
                once, twice,
                "{rel} fence @{}: canonicalize must be a fixed point",
                f.line
            );
            let dir = init_repo();
            let out = run_stdin(dir.path(), &patch, &["--check"]);
            if out.status.success() {
                continue;
            }
            let code = first_code(&out).unwrap_or_else(|| "NO_CODE".to_string());
            assert!(
                matches!(
                    code.as_str(),
                    "RSMPL_MATCH_CONFLICT" | "RSMPL_RANGE_CONFLICT" | "GIT_HUNK_CONFLICT"
                ),
                "{rel} fence @{}: well-formed fences fail only with \
                 MATCH/RANGE/GIT_HUNK on an empty repo, got: {code}\n{}",
                f.line,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    // Pairing guard: one stray bare ``` line collapses every later pair
    // (the cascade this lint was built to catch), so the total must not
    // silently shrink; every marker must earn its keep.
    assert!(
        total >= 100,
        "expected at least 100 doc fences, got {total}"
    );
    assert!(
        marked >= 7,
        "expected at least 7 invalid markers, got {marked}"
    );
}
