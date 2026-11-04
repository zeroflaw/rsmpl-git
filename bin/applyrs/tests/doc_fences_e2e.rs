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
    // WI-18 pins the exact counts (SPEC 361 total / 133 candidates /
    // 7 marked + AGENTS 6/3/0 + SKILL 4/3/0 = 139 candidates, 7 marked):
    // the SPEC diet moves no fence, so any drift names the edit that
    // added or dropped one.
    assert!(
        total == 139,
        "expected exactly 139 candidate doc fences (WI-18 pin), got {total}"
    );
    assert!(
        marked == 7,
        "expected exactly 7 invalid markers (WI-18 pin), got {marked}"
    );
}

/// WI-24 slice 1: directive-inventory coverage gate — the parser is the
/// single source of supported-directive truth (`KNOWN_KV_KEYS` /
/// `BARE_FLAGS` / `MetavarKind::parse`), SPEC 9.3/15.1 must cover every
/// entry. Mirrors `python3 tools/gen_directives.py --check` (same
/// layering as `d5_exclusions_sync`); both layers are mutation-checked.
#[test]
fn directive_inventory_sync() {
    let root = workspace_root();
    let directives = std::fs::read_to_string(root.join("crates/rsmpl_patch/src/directives.rs"))
        .expect("directives.rs must be readable");
    let model = std::fs::read_to_string(root.join("crates/rsmpl_patch/src/model.rs"))
        .expect("model.rs must be readable");
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md must be readable");

    fn str_list(src: &str, name: &str) -> Vec<String> {
        let i = src.find(name).unwrap_or_else(|| panic!("{name} not found"));
        let after = &src[i..];
        let l = after.find('[').expect("list open");
        let r = after.find("];").expect("list close");
        let mut out = Vec::new();
        let mut rest = &after[l..r];
        while let Some(a) = rest.find('"') {
            let b = rest[a + 1..].find('"').expect("closing quote");
            out.push(rest[a + 1..a + 1 + b].to_string());
            rest = &rest[a + 1 + b + 1..];
        }
        assert!(!out.is_empty(), "{name} parsed empty");
        out
    }

    let keys = str_list(&directives, "KNOWN_KV_KEYS");
    let bare = str_list(&directives, "BARE_FLAGS");
    let all = str_list(&directives, "const ALL");
    // `MetavarKind::parse` arms: `"kind" => Some(Self::...)`.
    let mut kinds = Vec::new();
    let mut rest = model.as_str();
    while let Some(a) = rest.find("=> Some(Self::") {
        let head = &rest[..a];
        let q = head.rfind('"').expect("kind quote");
        let open = head[..q].rfind('"').expect("kind open quote");
        kinds.push(head[open + 1..q].to_string());
        rest = &rest[a + 1..];
    }
    assert!(!kinds.is_empty(), "MetavarKind arms parsed empty");

    fn section<'a>(spec: &'a str, start: &str, stops: &[&str]) -> &'a str {
        let i = spec.find(start).expect("SPEC section missing");
        let tail = &spec[i + start.len()..];
        let mut end = tail.len();
        for s in stops {
            if let Some(j) = tail.find(s) {
                if j < end {
                    end = j;
                }
            }
        }
        &tail[..end]
    }

    let s93 = section(&spec, "### 9.3 Core directives", &["### 9.4"]);
    let s151 = section(
        &spec,
        "### 15.1 Supported metavariable kinds",
        &["### 15.2"],
    );
    for k in &keys {
        assert!(
            s93.contains(&format!("# {k}")),
            "SPEC 9.3 must cover directive key `{k}`: run `python3 tools/gen_directives.py --check`"
        );
    }
    for b in &bare {
        assert!(
            s93.contains(&format!("# {b}")),
            "SPEC 9.3 must cover bare flag `{b}`: run `python3 tools/gen_directives.py --check`"
        );
    }
    for k in &kinds {
        assert!(
            s151.contains(&format!("`{k}`")),
            "SPEC 15.1 must name metavar kind `{k}`: run `python3 tools/gen_directives.py --check`"
        );
    }
    for d in ["# edition", "# format_scope"] {
        assert!(
            spec.contains(d),
            "SPEC must document rejected directive `{d}`: run `python3 tools/gen_directives.py --check`"
        );
    }
    let mut union: Vec<String> = {
        let mut set = std::collections::BTreeSet::new();
        for k in keys.iter().chain(bare.iter()) {
            set.insert(k.clone());
        }
        set.into_iter().collect()
    };
    let mut all_sorted = all.clone();
    union.sort();
    all_sorted.sort();
    assert_eq!(
        union, all_sorted,
        "suggest_bare ALL must equal KEYS ∪ BARE: run `python3 tools/gen_directives.py --check`"
    );
}

/// WI-24 slice 2: error-code inventory coverage gate — the engine is the
/// single source of error-code truth (`ErrorCode::as_str`), SPEC 25 plus
/// the AGENTS.md / SKILL.md "Errors: code → fix" tables must cover every
/// code. Mirrors `python3 tools/gen_errors.py --check` (same layering as
/// `directive_inventory_sync`); both layers are mutation-checked.
#[test]
fn error_inventory_sync() {
    let root = workspace_root();
    let errors = std::fs::read_to_string(root.join("crates/rsmpl_patch/src/errors.rs"))
        .expect("errors.rs must be readable");
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md must be readable");
    let agents =
        std::fs::read_to_string(root.join("AGENTS.md")).expect("AGENTS.md must be readable");
    let skill = std::fs::read_to_string(root.join("skills/gitrs/SKILL.md"))
        .expect("SKILL.md must be readable");

    // `ErrorCode::as_str` arms: `Self::X => "CODE_CONFLICT"`.
    let mut codes = Vec::new();
    let mut rest = errors.as_str();
    while let Some(a) = rest.find("=> \"") {
        let tail = &rest[a + 4..];
        let end = tail.find('"').expect("code closing quote");
        let code = tail[..end].to_string();
        if code.ends_with("CONFLICT") {
            codes.push(code);
        }
        rest = &tail[end + 1..];
    }
    assert!(!codes.is_empty(), "ErrorCode arms parsed empty");
    let mut seen = std::collections::BTreeSet::new();
    for c in &codes {
        assert!(
            seen.insert(c.clone()),
            "duplicate error code `{c}`: run `python3 tools/gen_errors.py --check`"
        );
    }
    for c in &codes {
        assert!(
            spec.contains(c),
            "SPEC 25 must cover error code `{c}`: run `python3 tools/gen_errors.py --check`"
        );
        assert!(
            agents.contains(c),
            "AGENTS.md error table must cover code `{c}`: run `python3 tools/gen_errors.py --check`"
        );
        assert!(
            skill.contains(c),
            "SKILL.md error table must cover code `{c}`: run `python3 tools/gen_errors.py --check`"
        );
    }
}

/// WI-9: generated guard table sync — `guard_for` in `guard.rs` plus
/// `PRECISION_GUARD` are the single source of the SPEC D.4 guard table
/// and the SPEC-head contract-guard line. Mirrors
/// `python3 tools/gen_guard.py --check` (same layering as
/// `d5_exclusions_sync`); both layers are mutation-checked.
#[test]
fn generated_guard_sync() {
    let root = workspace_root();
    let guard = std::fs::read_to_string(root.join("crates/rsmpl_diff_infer/src/semantic/guard.rs"))
        .expect("guard.rs must be readable");
    let evo = std::fs::read_to_string(
        root.join("crates/rsmpl_diff_infer/src/semantic/definition_evolution.rs"),
    )
    .expect("definition_evolution.rs must be readable");
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md must be readable");

    // Precision arms: quoted names between `match detector {` and the
    // `_ =>` default (every other detector rides Strict by construction).
    let m = guard.find("fn guard_for").expect("guard_for missing");
    let after = &guard[m..];
    let end = after.find("_ =>").expect("guard_for default arm missing");
    let mut precision = Vec::new();
    let mut rest = &after[..end];
    while let Some(a) = rest.find('"') {
        let b = rest[a + 1..].find('"').expect("closing quote");
        precision.push(rest[a + 1..a + 1 + b].to_string());
        rest = &rest[a + 1 + b + 1..];
    }
    assert!(!precision.is_empty(), "Precision set parsed empty");
    let mut seen = std::collections::BTreeSet::new();
    for d in &precision {
        assert!(seen.insert(d.clone()), "duplicate Precision detector `{d}`");
    }
    let i = evo
        .find("const PRECISION_GUARD")
        .expect("PRECISION_GUARD missing");
    let tail = &evo[i..];
    let eq = tail.find('=').expect("const value missing");
    let semi = tail[eq..].find(';').expect("const terminator missing");
    let n: usize = tail[eq + 1..eq + semi]
        .trim()
        .parse()
        .expect("PRECISION_GUARD must parse");

    const START: &str = "<!-- generated: guard start";
    const END: &str = "<!-- generated: guard end -->";
    const CSTART: &str = "<!-- generated: contract-guard start";
    const CEND: &str = "<!-- generated: contract-guard end -->";
    let mut expected = format!(
        "{START} -- do not hand-edit; run `python3 tools/gen_guard.py --write` -->\n\
         Guard posture (code is the source: `guard_for` in `guard.rs`, \
         `PRECISION_GUARD` = {n}): {len} Precision detectors; \
         every other detector rides Strict.\n\
         | Detector | Guard |\n\
         |---|---|\n",
        len = precision.len()
    );
    for d in &precision {
        expected.push_str(&format!("| `{d}` | Precision |\n"));
    }
    expected.push_str(&format!("{END}\n"));
    let expected_c = format!(
        "{CSTART} -- do not hand-edit; run `python3 tools/gen_guard.py --write` -->\n\
         Guards: {len} Precision, rest Strict \
         (`PRECISION_GUARD` = {n}; SPEC D.4, generated block).\n\
         {CEND}\n",
        len = precision.len()
    );
    let cur = |text: &str, s: &str, e: &str| {
        let a = text
            .find(s)
            .unwrap_or_else(|| panic!("marker missing: {s}"));
        let b = text
            .find(e)
            .unwrap_or_else(|| panic!("marker missing: {e}"));
        format!("{}\n", &text[a..b + e.len()])
    };
    assert_eq!(
        cur(&spec, START, END),
        expected,
        "SPEC D.5 guard table drifts from `guard_for`: run `python3 tools/gen_guard.py --write`"
    );
    assert_eq!(
        cur(&spec, CSTART, CEND),
        expected_c,
        "SPEC head contract-guard drifts: run `python3 tools/gen_guard.py --write`"
    );
}

/// WI-9: generated inventory blocks sync — the directive/error extractors
/// own the `generated: directives` / `generated: errors` blocks in
/// SPEC.md, AGENTS.md, SKILL.md and the SPEC-head contract one-liners.
/// Mirrors `python3 tools/gen_directives.py --check` and
/// `python3 tools/gen_errors.py --check` for the block layer (the
/// coverage layer stays in `directive_inventory_sync` /
/// `error_inventory_sync`); both layers are mutation-checked.
#[test]
fn generated_inventory_blocks_sync() {
    let root = workspace_root();
    let directives = std::fs::read_to_string(root.join("crates/rsmpl_patch/src/directives.rs"))
        .expect("directives.rs must be readable");
    let model = std::fs::read_to_string(root.join("crates/rsmpl_patch/src/model.rs"))
        .expect("model.rs must be readable");
    let errors = std::fs::read_to_string(root.join("crates/rsmpl_patch/src/errors.rs"))
        .expect("errors.rs must be readable");
    let spec = std::fs::read_to_string(root.join("SPEC.md")).expect("SPEC.md must be readable");
    let agents =
        std::fs::read_to_string(root.join("AGENTS.md")).expect("AGENTS.md must be readable");
    let skill = std::fs::read_to_string(root.join("skills/gitrs/SKILL.md"))
        .expect("SKILL.md must be readable");

    fn str_list(src: &str, name: &str) -> Vec<String> {
        let i = src.find(name).unwrap_or_else(|| panic!("{name} not found"));
        let after = &src[i..];
        let l = after.find('[').expect("list open");
        let r = after.find("];").expect("list close");
        let mut out = Vec::new();
        let mut rest = &after[l..r];
        while let Some(a) = rest.find('"') {
            let b = rest[a + 1..].find('"').expect("closing quote");
            out.push(rest[a + 1..a + 1 + b].to_string());
            rest = &rest[a + 1 + b + 1..];
        }
        out
    }
    let keys = str_list(&directives, "KNOWN_KV_KEYS");
    let bare = str_list(&directives, "BARE_FLAGS");
    // `MetavarKind::parse` arms: `"kind" => Some(Self::...)`.
    let mut kinds = Vec::new();
    let mut rest = model.as_str();
    while let Some(a) = rest.find("=> Some(Self::") {
        let head = &rest[..a];
        let q = head.rfind('"').expect("kind quote");
        let open = head[..q].rfind('"').expect("kind open quote");
        kinds.push(head[open + 1..q].to_string());
        rest = &rest[a + 1..];
    }
    // `ErrorCode::as_str` arms: `Self::X => "CODE_CONFLICT"`.
    let mut codes = Vec::new();
    let mut tail = errors.as_str();
    while let Some(a) = tail.find("=> \"") {
        let after = &tail[a + 4..];
        let end = after.find('"').expect("code closing quote");
        let code = after[..end].to_string();
        if code.ends_with("CONFLICT") {
            codes.push(code);
        }
        tail = &after[end + 1..];
    }
    assert!(!kinds.is_empty() && !codes.is_empty(), "parse walked empty");

    const DSTART: &str = "<!-- generated: directives start";
    const DEND: &str = "<!-- generated: directives end -->";
    const DCSTART: &str = "<!-- generated: contract-directives start";
    const DCEND: &str = "<!-- generated: contract-directives end -->";
    const ESTART: &str = "<!-- generated: errors start";
    const EEND: &str = "<!-- generated: errors end -->";
    const ECSTART: &str = "<!-- generated: contract-errors start";
    const ECEND: &str = "<!-- generated: contract-errors end -->";
    let expected_d = format!(
        "{DSTART} -- do not hand-edit; run `python3 tools/gen_directives.py --write` -->\n\
         Directive inventory (code is the source: `directives.rs` + `model.rs`): \
         {nk} keys, {nb} bare flags, {nn} kinds.\n\
         Keys: {keys}.\n\
         Bare: {bare}.\n\
         Kinds: {kinds}.\n\
         {DEND}\n",
        nk = keys.len(),
        nb = bare.len(),
        nn = kinds.len(),
        keys = keys.join(", "),
        bare = bare.join(", "),
        kinds = kinds.join(", "),
    );
    let expected_dc = format!(
        "{DCSTART} -- do not hand-edit; run `python3 tools/gen_directives.py --write` -->\n\
         Directives: {nk} keys, {nb} bare flags, {nn} kinds (SPEC 9.3; generated block).\n\
         {DCEND}\n",
        nk = keys.len(),
        nb = bare.len(),
        nn = kinds.len(),
    );
    let expected_e = format!(
        "{ESTART} -- do not hand-edit; run `python3 tools/gen_errors.py --write` -->\n\
         Error-code inventory (code is the source: `errors.rs`): {n} codes.\n\
         Codes: {codes}.\n\
         {EEND}\n",
        n = codes.len(),
        codes = codes.join(", "),
    );
    let expected_ec = format!(
        "{ECSTART} -- do not hand-edit; run `python3 tools/gen_errors.py --write` -->\n\
         Errors: {n} codes (SPEC 25; generated block).\n\
         {ECEND}\n",
        n = codes.len(),
    );
    let cur = |text: &str, s: &str, e: &str| {
        let a = text
            .find(s)
            .unwrap_or_else(|| panic!("marker missing: {s}"));
        let b = text
            .find(e)
            .unwrap_or_else(|| panic!("marker missing: {e}"));
        format!("{}\n", &text[a..b + e.len()])
    };
    for (text, name) in [
        (&spec, "SPEC.md"),
        (&agents, "AGENTS.md"),
        (&skill, "SKILL.md"),
    ] {
        assert_eq!(
            cur(text, DSTART, DEND),
            expected_d,
            "{name} directives block drifts: run `python3 tools/gen_directives.py --write`"
        );
        assert_eq!(
            cur(text, ESTART, EEND),
            expected_e,
            "{name} errors block drifts: run `python3 tools/gen_errors.py --write`"
        );
    }
    assert_eq!(
        cur(&spec, DCSTART, DCEND),
        expected_dc,
        "SPEC head contract-directives drifts: run `python3 tools/gen_directives.py --write`"
    );
    assert_eq!(
        cur(&spec, ECSTART, ECEND),
        expected_ec,
        "SPEC head contract-errors drifts: run `python3 tools/gen_errors.py --write`"
    );
}

/// WI-10: fixture numbers are unambiguous — numeric prefixes unique
/// within each corpus (`100`/`101` each named two diffrs dirs before
/// the renumber to `144`/`145`).
#[test]
fn fixture_numbers_unique() {
    let root = workspace_root();
    for corpus in ["test-fixtures/diffrs", "test-fixtures/applyrs"] {
        let mut seen = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(root.join(corpus)).expect("corpus must list") {
            let name = entry
                .expect("entry must read")
                .file_name()
                .to_string_lossy()
                .to_string();
            let num = name
                .split('-')
                .next()
                .unwrap_or(&name)
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("[{corpus}] fixture {name:?} has no numeric prefix"));
            if let Some(prev) = seen.insert(num, name.clone()) {
                panic!(
                    "[{corpus}] duplicate fixture number {num}: {prev:?} vs {name:?} — \
                     renumber the newer dir (highest free number) and update its harness branch"
                );
            }
        }
        assert!(!seen.is_empty(), "[{corpus}] no fixtures found");
    }
}

/// WI-10: learnability — every catalogue shape has its AGENTS.md row,
/// its SKILL.md row/example, and a fixture/bench/test pin. Each entry
/// is `(shape, agents needle, skill needle, proof needle)`; the proof
/// needle matches a fixture/bench dir name or `.rs` test/crate source
/// (authored-only spellings such as the `as` pastes pin via e2e/unit
/// tests, never via a fixture an agent cannot re-apply).
#[test]
fn learnability_three_surface_sync() {
    let root = workspace_root();
    let agents =
        std::fs::read_to_string(root.join("AGENTS.md")).expect("AGENTS.md must be readable");
    let skill = std::fs::read_to_string(root.join("skills/gitrs/SKILL.md"))
        .expect("SKILL.md must be readable");

    // Proof corpus: fixture/bench dir names plus every `.rs` source
    // under `bin/` and `crates/` (tests pin authored-only spellings).
    let mut proof = String::new();
    for dir in [
        "test-fixtures/diffrs",
        "test-fixtures/applyrs",
        "example-benches/diffrs",
        "example-benches/applyrs",
    ] {
        for entry in std::fs::read_dir(root.join(dir)).expect("proof dir must list") {
            proof.push_str(
                &entry
                    .expect("entry must read")
                    .file_name()
                    .to_string_lossy(),
            );
            proof.push('\n');
        }
    }
    fn collect_rs(dir: &std::path::Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).expect("src dir must list") {
            let entry = entry.expect("entry must read");
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if name == "target" {
                    continue;
                }
                collect_rs(&path, out);
            } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
                out.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
                out.push('\n');
            }
        }
    }
    for tree in ["bin", "crates"] {
        collect_rs(&root.join(tree), &mut proof);
    }

    // (shape, AGENTS.md needle, SKILL.md needle, proof needle).
    // Needles are single-line substrings of hand-owned prose — a row
    // deletion fails naming the shape (mutation-checked).
    const SHAPES: &[(&str, &str, &str, &str)] = &[
        ("fn-rename", "Fn rename", "Fn rename", "01-rename-fn"),
        (
            "call-sweep",
            "Call rewrite",
            "rule=rename_calls",
            "01-call-sweep",
        ),
        (
            "method-m2f",
            "Method-call rewrite",
            "Method-call rewrite",
            "method-to-function",
        ),
        (
            "method-m2m",
            "composite receivers",
            "bind the receiver",
            "method-to-method",
        ),
        (
            "use-prefix",
            "Use-path prefix",
            "Use-path prefix",
            "use-prefix",
        ),
        ("use-regroup", "regroup", "regroup", "regroup"),
        (
            "item-rename",
            "Other items rename",
            "Item rename:",
            "14-item-rename",
        ),
        (
            "member-rename",
            "Rename + member evolution",
            "Rename+member evolution",
            "field-rename-fires",
        ),
        (
            "member-add-del",
            "field-addition",
            "field-add/del",
            "field-add",
        ),
        ("assoc-rename", "assoc claims", "assoc", "assoc-rename"),
        ("trait-method", "trait T", "trait T", "trait-method"),
        (
            "mod-rename",
            "`mod` claims",
            "`mod` claims",
            "42-mod-rename",
        ),
        (
            "definition-evolution",
            "templates its value",
            "Definition RHS evolution",
            "106-definition-alias-fires",
        ),
        (
            "sig-evolution",
            "Slots written",
            "Explicit slots evolve",
            "73-sig-evolution-fires",
        ),
        ("let-shape", "Beyond calls", "Beyond calls", "20-let-sweep"),
        ("type-shape", "Box<[T]>", "Box<[T]>", "type-sweep"),
        (
            "general-expr",
            "expressions (`- E + 1`)",
            "General-expression",
            "general-expr",
        ),
        ("sequence", "2–4-line", "2–4-line", "sequence"),
        ("macro-shape", "Macros —", "Macros:", "macro-rename"),
        (
            "macro-tokentree",
            "binds one hole",
            "binds one hole",
            "macro-sweep",
        ),
        (
            "symbol-shape",
            "Symbol rewrite —",
            "Symbol rewrite:",
            "symbol-single",
        ),
        (
            "symbol-fallback",
            "# fallback=syntax",
            "# fallback=syntax",
            "08-symbol-fallback",
        ),
        ("diet-sweep", "# sweep", "# sweep", "call-sweep"),
        ("diet-repeat", "# repeat", "# repeat", "repeat-fold"),
        (
            "diet-optional",
            "# optional",
            "# optional",
            "05-optional-noop",
        ),
        (
            "move-basic",
            "Move — cut",
            "Move: cut",
            "03-deleted-file-move",
        ),
        (
            "move-fn-as",
            "`as` paste",
            "`as` paste",
            "122-move-rename-fires",
        ),
        (
            "move-joint-as",
            "Joint paste",
            "adds one sig-slot evolution",
            "fn old as new(x",
        ),
        (
            "move-container-as",
            "Container `as` paste",
            "renames container",
            "struct Old as New",
        ),
        (
            "move-trait-as",
            "Trait `as` paste",
            "renames trait",
            "trait Old as New",
        ),
        (
            "move-joint-trait-as",
            "Joint trait paste",
            "adds one method rename",
            "m_old as m_new",
        ),
        (
            "move-impl-as",
            "Inherent-`impl` `as` paste",
            "renames `impl` block",
            "impl Old as New",
        ),
        (
            "move-attr-e1",
            "Move+attr (E1)",
            "move+attr (E1)",
            "hunk-interior",
        ),
        (
            "move-trait-e2",
            "trait-move (E2)",
            "trait-move (E2)",
            "m_old as m_new",
        ),
        (
            "rename-body-joint",
            "Rename+body joint",
            "rename+body joints",
            "119-fn-joint-body-fires",
        ),
        (
            "joint-impl",
            "Joint `impl Trait for Type`",
            "follow-rewritten, never written",
            "joint-impl",
        ),
        (
            "collection-move",
            "binds an ordered collection",
            "collection moves",
            "collection-move",
        ),
        (
            "binder-rename",
            "capture-risk",
            "capture-risk",
            "let_binder",
        ),
        (
            "array-repeat-raw",
            "array-repeat",
            "array-repeat",
            "repeat-hole",
        ),
        ("qself-call", "qself", "qself", "qself-call"),
    ];
    for (shape, a_needle, s_needle, p_needle) in SHAPES {
        assert!(
            agents.contains(a_needle),
            "shape `{shape}` lost its AGENTS.md row (needle {a_needle:?}): restore the row"
        );
        assert!(
            skill.contains(s_needle),
            "shape `{shape}` lost its SKILL.md row (needle {s_needle:?}): restore the row"
        );
        assert!(
            proof.contains(p_needle),
            "shape `{shape}` lost its fixture/bench/test pin (needle {p_needle:?}): \
             add a fixture, bench, or e2e pin naming it"
        );
    }
}
