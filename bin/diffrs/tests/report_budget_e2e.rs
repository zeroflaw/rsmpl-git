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
    // WI-17 slice 1: the trial log is complete — every chain-registry
    // detector appears exactly once across attempted (`no-match` here)
    // and `not-tried` (whole-section multi-pair drivers never run on a
    // 1-pair section — the routing, recorded as absence, not silence).
    // Every reason parses under the closed taxonomy.
    let names: Vec<&str> = declined
        .iter()
        .map(|d| d["detector"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"call_sweep"), "got: {names:?}");
    assert!(names.contains(&"delete"), "got: {names:?}");
    assert!(names.contains(&"subhunk"), "got: {names:?}");
    for d in declined {
        let reason = d["reason"].as_str().unwrap();
        let closed = reason == "no-match"
            || reason == "not-tried"
            || reason.starts_with("guard:")
            || reason.starts_with("totality:")
            || reason.starts_with("veto:")
            || reason.starts_with("precondition:")
            || reason == "byte-guard"
            || reason == "precision-guard"
            || reason == "crlf-content"
            || reason == "stranded-joint"
            || reason == "cross-kind-member"
            || reason == "unbalanced-hunks";
        assert!(closed, "open reason: {d}");
    }
    // Chain-registry detectors ride `not-tried` when the 1-pair routing
    // never attempts them (multi-pair drivers); attempted families ride
    // their chain reason. `item_rename` is a registry detector (present
    // exactly once, either posture).
    let item_count = declined
        .iter()
        .filter(|d| d["detector"] == serde_json::json!("item_rename"))
        .count();
    assert_eq!(item_count, 1, "got: {names:?}");
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

#[test]
fn wi15_partial_subhunk_mix_reports_context_exact_remainder() {
    // WI-15 P3 residue: a balanced single-hunk section whose method-vis
    // pair raises via the subhunk partition (context-carrying
    // `sig_evolution` body) plus an ordinary literal remainder reports
    // two entries — semantic sites `[2]` plus one remainder ordinary
    // entry with sites `[3]` only (range-span intersection would
    // over-claim `[2, 3]` and stay silent). Inner classification probes
    // stay on a throwaway by design; `declined` here is the section's
    // chain-level evidence.
    let dir = init_repo();
    write(
        dir.path(),
        "src/f.rs",
        "impl Svc {\n    fn alpha(x: i32) {\n        1\n    }\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/f.rs",
        "impl Svc {\n    pub fn alpha(x: i32) {\n        2\n    }\n}\n",
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
        serde_json::json!("sig_evolution"),
        "got: {v}"
    );
    assert!(sem.get("declined").is_none(), "got: {v}");
    assert_eq!(sem["sites"], serde_json::json!([2]), "got: {v}");
    assert_eq!(ord["rule"], serde_json::json!(""), "got: {v}");
    assert_eq!(ord["sites"], serde_json::json!([3]), "got: {v}");
    assert_eq!(ord["totality"], serde_json::json!("pre-image"), "got: {v}");
    let declined = ord["declined"].as_array().unwrap();
    assert!(!declined.is_empty(), "got: {v}");
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

#[test]
fn wi17_text_parity_matches_json_trial() {
    // WI-17 slice 1 text parity: the same comment-only input reports
    // the same ordinary-section trial in text mode (stderr
    // `declined:` lines, stdout patch untouched) and in JSON mode
    // (`declined` arrays). The two sets must be equal as
    // (file, detector, reason) triples.
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
    let json_out = run(dir.path(), &["--format", "json"]);
    assert!(
        json_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&json_out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&json_out.stdout)).expect("stdout is JSON");
    let mut want = std::collections::BTreeSet::new();
    for e in v["hunks"].as_array().unwrap() {
        if e["detector"] != serde_json::json!("ordinary") {
            continue;
        }
        let file = e["file"].as_str().unwrap().to_string();
        for d in e["declined"].as_array().unwrap() {
            want.insert(format!(
                "{} {} {}",
                file,
                d["detector"].as_str().unwrap(),
                d["reason"].as_str().unwrap()
            ));
        }
    }
    assert!(!want.is_empty(), "got: {v}");
    let text_out = run(dir.path(), &[]);
    assert!(
        text_out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&text_out.stderr)
    );
    let patch_text = String::from_utf8_lossy(&text_out.stdout).to_string();
    let patch_json = v["patch"].as_str().unwrap();
    assert_eq!(patch_text, patch_json, "text stdout must equal JSON patch");
    let mut got = std::collections::BTreeSet::new();
    for line in String::from_utf8_lossy(&text_out.stderr).lines() {
        if let Some(rest) = line.strip_prefix("declined: ") {
            got.insert(rest.to_string());
        }
    }
    assert_eq!(got, want, "text trial must equal JSON trial");
}

#[test]
fn wi17_guard_veto_carries_verdict_numbers() {
    // WI-17 slice 1 guard numbers: every `guard:` reason on the
    // pair-chain path carries the exact verdict inputs (`sem`, `ord`,
    // `allow`), so the overflow (`sem > ord + allow`) is computable
    // from the report alone. NOTE (WI-17 slice 2 review): the fixture
    // below fires (allowance admits it), so the `guard:` loop is empty
    // by design — the non-vacuous veto pin is `per_hunk.rs`
    // `wi17_slice2_item_rename_veto_carries_verdict_numbers`.
    // This pin owns the taxonomy shape over
    // whatever the fixture below raises (fire or veto): no bare veto
    // without numbers on the pair-chain path, and every reason closed.
    let dir = init_repo();
    write(
        dir.path(),
        "src/f.rs",
        "fn m() {\n    old_a_variable_with_a_long_name();\n    old_a_variable_with_a_long_name();\n}\n",
    );
    commit_all(dir.path());
    write(
        dir.path(),
        "src/f.rs",
        "fn m() {\n    new_a_variable_with_a_long_name();\n    new_a_variable_with_a_long_name();\n}\n",
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
    for e in hunks {
        for d in e
            .get("declined")
            .and_then(|a| a.as_array())
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            let reason = d["reason"].as_str().unwrap();
            let closed = reason == "no-match"
                || reason == "not-tried"
                || reason.starts_with("guard:")
                || reason.starts_with("totality:")
                || reason.starts_with("veto:")
                || reason.starts_with("precondition:")
                || reason == "byte-guard"
                || reason == "precision-guard"
                || reason == "crlf-content"
                || reason == "stranded-joint"
                || reason == "cross-kind-member"
                || reason == "unbalanced-hunks";
            assert!(closed, "open reason: {d}");
            if reason.starts_with("guard:") {
                let rest = reason.strip_prefix("guard:").unwrap();
                let mut parts = rest.split(':');
                let kind = parts.next().unwrap();
                assert!(kind == "byte" || kind == "precision", "got: {reason}");
                let (mut sem, mut ord, mut allow) = (None, None, None);
                for p in parts {
                    if let Some(x) = p.strip_prefix("sem=") {
                        sem = x.parse::<usize>().ok();
                    } else if let Some(x) = p.strip_prefix("ord=") {
                        ord = x.parse::<usize>().ok();
                    } else if let Some(x) = p.strip_prefix("allow=") {
                        allow = x.parse::<usize>().ok();
                    } else {
                        panic!("bad guard slot: {reason}");
                    }
                }
                let (s, o, a) = (sem.unwrap(), ord.unwrap(), allow.unwrap());
                assert!(s > o + a, "guard veto must name its overflow: {reason}");
            }
        }
    }
}
#[test]
fn wi17_slice2_no_bare_guard_vetoes() {
    // WI-17 slice 2 surface invariant: no bare `byte-guard` /
    // `precision-guard` reason is emitted on any input. Guard vetoes
    // are rare by design (the WI-7 allowance admits proved singles),
    // so this battery pins the negative over shapes that record
    // declines on every unified path (whole-single, pair-chain,
    // partition, partial, per-hunk); the non-vacuous veto-shape pin
    // lives inline in `per_hunk.rs` (forced `hunk_len=0` veto).
    for (before, after) in [
        (
            "fn m() {\n    // hello\n    let x = 1;\n}\n",
            "fn m() {\n    // hello world\n    let x = 1;\n}\n",
        ),
        (
            "fn m() {\n    old_a_variable_with_a_long_name();\n    old_a_variable_with_a_long_name();\n}\n",
            "fn m() {\n    new_a_variable_with_a_long_name();\n    new_a_variable_with_a_long_name();\n}\n",
        ),
        (
            "fn m() {\n    a();\n}\n",
            "fn m() {\n    a_very_long_function_name_that_goes_on_and_on_and_on();\n}\n",
        ),
    ] {
        let dir = init_repo();
        write(dir.path(), "src/f.rs", before);
        commit_all(dir.path());
        write(dir.path(), "src/f.rs", after);
        let out = run(dir.path(), &["--format", "json"]);
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(
            !stdout.contains("\"byte-guard\""),
            "bare byte-guard emitted: {stdout}"
        );
        assert!(
            !stdout.contains("\"precision-guard\""),
            "bare precision-guard emitted: {stdout}"
        );
    }
}
