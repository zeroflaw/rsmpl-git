//! WI-2d: absolute `diffrs --format json` hunks-budget pin.
//!
//! SPEC D.4 budget: bytes beyond `patch` ≤ `150 + 190·H + 10·T + 75·D`
//! (`H` = hunks, `T` = total sites, `D` = total `declined` entries;
//! WI-15 prices each declined entry at 75 B — fixed detector/reason
//! vocabulary, the longest family names covered). Multi-file fixture
//! so the pin applies on a realistic report, not a toy single hunk.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_diffrs"))
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&st.stderr)
    );
}

fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn commit_all(dir: &Path) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "commit.gpgsign=false",
            "-c",
            "user.email=test@test",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "base",
        ],
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    if let Some(parent) = Path::new(rel).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(dir.join(parent)).unwrap();
        }
    }
    std::fs::write(dir.join(rel), text).unwrap()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .arg("--repo")
        .arg(dir)
        .output()
        .unwrap()
}

#[test]
fn json_hunks_within_absolute_budget() {
    let dir = init_repo();
    for i in 0..6 {
        let mut text = String::from("fn foo() {\n");
        for j in 0..10 {
            text.push_str(&format!("    old_call({j});\n"));
        }
        text.push_str("}\n");
        write(dir.path(), &format!("src/f{i:02}.rs"), &text);
    }
    commit_all(dir.path());
    for i in 0..6 {
        let rel = format!("src/f{i:02}.rs");
        let path = dir.path().join(&rel);
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("old_call", "new_call");
        std::fs::write(&path, text).unwrap();
    }
    let out = run(dir.path(), &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    let patch = v["patch"].as_str().unwrap();
    assert!(!patch.trim().is_empty(), "got: {v}");
    let hunks = v["hunks"].as_array().unwrap();
    assert!(!hunks.is_empty(), "got: {v}");
    let h = hunks.len();
    let t: usize = hunks
        .iter()
        .map(|e| e["sites"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    // WI-15: `declined` arrays ride the `75·D` term (zero here — the
    // fixture fully compresses, so ordinary entries never appear and
    // the term is inert on this pin).
    let d: usize = hunks
        .iter()
        .map(|e| {
            e.get("declined")
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        })
        .sum();
    let overhead = stdout.len() - patch.len();
    let budget = 150 + 190 * h + 10 * t + 75 * d;
    assert!(
        overhead <= budget,
        "overhead {overhead} B exceeds budget {budget} B (H={h} T={t} D={d}): {v}"
    );
    for e in hunks {
        assert!(e.get("rule").is_some(), "got: {v}");
        assert!(e.get("file").is_some(), "got: {v}");
        assert!(e.get("detector").is_some(), "got: {v}");
        assert!(e.get("totality").is_some(), "got: {v}");
        assert!(e.get("proven").is_some(), "got: {v}");
        assert!(e.get("sites_exact").is_some(), "got: {v}");
        assert!(e.get("saved_bytes").is_some(), "got: {v}");
    }
}

#[test]
fn wi15_ordinary_section_names_declined_detectors() {
    // WI-15 acceptance: a comment-only diff (no detector claims comment
    // lines) reports one entry per wholly-ordinary section with
    // `detector: "ordinary"` and the chain-level `declined` list —
    // never `"hunks": []`. Semantic entries never carry `declined`.
    let dir = init_repo();
    write(
        dir.path(),
        "src/f.rs",
        "fn m() {\n    // hello\n    let x = 1;\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/f.rs",
        "fn m() {\n    // hello world\n    let x = 1;\n}\n",
    );
    let out = run(dir.path(), &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let hunks = v["hunks"].as_array().unwrap();
    assert_eq!(hunks.len(), 1, "got: {v}");
    let e = &hunks[0];
    assert_eq!(e["detector"], serde_json::json!("ordinary"), "got: {v}");
    assert_eq!(e["rule"], serde_json::json!(""), "got: {v}");
    assert_eq!(e["saved_bytes"], serde_json::json!(0), "got: {v}");
    let declined = e["declined"].as_array().unwrap();
    assert!(!declined.is_empty(), "got: {v}");
    // Pair-chain families tried in chain order; every reason is
    // chain-level (`no-match` shape decline here). A 1-pair whole
    // section routes through the singles arms + per-hunk fallback, so
    // the list names those families (whole-section multi-pair drivers
    // never run on it — the routing, not a gap).
    let names: Vec<&str> = declined
        .iter()
        .map(|d| d["detector"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"item_rename"), "got: {names:?}");
    assert!(names.contains(&"call_sweep"), "got: {names:?}");
    assert!(names.contains(&"delete"), "got: {names:?}");
    assert!(names.contains(&"subhunk"), "got: {names:?}");
    for d in declined {
        assert_eq!(d["reason"], serde_json::json!("no-match"), "got: {d}");
    }
    // WI-15 review (pre-pass silence): whole-patch pre-pass declines are
    // patch-global and never attributed per-section — the pre-pass-only
    // detectors never appear in a section's `declined` list. The shared
    // names (`member_rename`, `assoc_rename`, `trait_method_rename`) also
    // fire as per-section chain detectors, so only the five pre-pass-only
    // names are pinned absent here. There is deliberately no patch-level
    // `declined` key.
    for prepass in [
        "move",
        "item_intent",
        "item_joint_impl",
        "fn_joint",
        "mod_rename",
    ] {
        assert!(
            !names.contains(&prepass),
            "pre-pass {prepass} leaked into section declined: {names:?}"
        );
    }
    assert!(v.get("declined").is_none(), "got: {v}");
    // The ordinary entry still carries the covering site + totality.
    assert_eq!(e["sites"], serde_json::json!([2]), "got: {v}");
    assert_eq!(e["totality"], serde_json::json!("pre-image"), "got: {v}");
    // WI-15 budget: the ordinary entry prices under the `75·D` term.
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let patch = v["patch"].as_str().unwrap();
    let overhead = stdout.len() - patch.len();
    let d = declined.len();
    let budget = 150 + 190 + 10 + 75 * d;
    assert!(
        overhead <= budget,
        "overhead {overhead} B exceeds budget {budget} B (D={d}): {v}"
    );
}

#[test]
fn wi15_partial_section_gains_remainder_ordinary_entry() {
    // WI-15 partial: a section holding one firing hunk (const rename)
    // plus one ordinary hunk (comment edit) emits two entries — one
    // semantic plus one remainder `detector: "ordinary"` whose `sites`
    // name only the remainder line and whose `declined` list is the
    // section's chain-level evidence. Semantic entries never carry
    // `declined`; per-run losing-decline detail stays with the
    // subhunk-inner follow-up (chain-level here, by design).
    let dir = init_repo();
    write(
        dir.path(),
        "src/f.rs",
        "fn header() {}\nconst OLD_CONST_VALUE_WITH_LONG_NAME_AAA: usize = 1001;\nfn m() {\n    // hello\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/f.rs",
        "fn header() {}\nconst NEW_CONST_VALUE_WITH_LONG_NAME_AAA: usize = 1001;\nfn m() {\n    // hello world\n}\n",
    );
    let out = run(dir.path(), &["--format", "json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&out.stdout)).expect("stdout is JSON");
    let hunks = v["hunks"].as_array().unwrap();
    assert_eq!(hunks.len(), 2, "got: {v}");
    let sem = hunks
        .iter()
        .find(|e| e["detector"] != serde_json::json!("ordinary"))
        .expect("one semantic entry");
    let ord = hunks
        .iter()
        .find(|e| e["detector"] == serde_json::json!("ordinary"))
        .expect("one remainder ordinary entry");
    assert_eq!(
        sem["detector"],
        serde_json::json!("item_rename"),
        "got: {v}"
    );
    assert!(sem.get("declined").is_none(), "got: {v}");
    assert_eq!(sem["sites"], serde_json::json!([2]), "got: {v}");
    assert_eq!(ord["rule"], serde_json::json!(""), "got: {v}");
    assert_eq!(ord["saved_bytes"], serde_json::json!(0), "got: {v}");
    assert_eq!(ord["sites"], serde_json::json!([4]), "got: {v}");
    assert_eq!(ord["totality"], serde_json::json!("pre-image"), "got: {v}");
    let declined = ord["declined"].as_array().unwrap();
    assert!(!declined.is_empty(), "got: {v}");
    let names: Vec<&str> = declined
        .iter()
        .map(|d| d["detector"].as_str().unwrap())
        .collect();
    for prepass in [
        "move",
        "item_intent",
        "item_joint_impl",
        "fn_joint",
        "mod_rename",
    ] {
        assert!(
            !names.contains(&prepass),
            "pre-pass {prepass} leaked into remainder declined: {names:?}"
        );
    }
    // WI-15 budget over both entries.
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let patch = v["patch"].as_str().unwrap();
    let overhead = stdout.len() - patch.len();
    let h = hunks.len();
    let t: usize = hunks
        .iter()
        .map(|e| e["sites"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    let d: usize = hunks
        .iter()
        .map(|e| {
            e.get("declined")
                .and_then(|a| a.as_array())
                .map(|a| a.len())
                .unwrap_or(0)
        })
        .sum();
    let budget = 150 + 190 * h + 10 * t + 75 * d;
    assert!(
        overhead <= budget,
        "overhead {overhead} B exceeds budget {budget} B (H={h} T={t} D={d}): {v}"
    );
}
