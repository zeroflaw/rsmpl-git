//! T41 anti-rot: `AGENTS.md` stays short and its citations stay true.
//!
//! Every `SPEC <token>` anchor cited by the cheat-sheet must resolve in
//! `SPEC.md`, and every ` --<flag>` it cites must exist in `rsmpl-git
//! --help`. The three recipe skeletons must appear verbatim so the sheet
//! alone suffices for rename + call-rewrite + move patches.

use std::path::PathBuf;
use std::process::Command;

mod common;
use common::*;

fn read_root(rel: &str) -> String {
    let root = workspace_root();
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

#[test]
fn agents_md_citations_resolve() {
    let agents = read_root("AGENTS.md");
    let spec = read_root("SPEC.md");
    let readme = read_root("README.md");

    let lines = agents.lines().count();
    assert!(lines <= 180, "AGENTS.md budget is 180 lines, got {lines}");

    // SPEC section anchors cited by the cheat-sheet: (agents-side citation,
    // spec-side resolution). Short section numbers use the distinctive
    // `SPEC N` citation so a SPEC renumbering breaks this test loudly
    // instead of matching a stray substring.
    for (cite, anchor) in [
        ("9.3", "## 9."),
        ("13.4", "## 13."),
        ("14.1", "## 14."),
        ("14.8", "## 14."),
        ("SPEC 15", "## 15."),
        ("SPEC 16", "## 16."),
        ("17.7", "## 17."),
        ("18.6", "### 18.6"),
        ("SPEC 19", "## 19."),
        ("SPEC 25", "## 25."),
        ("26.1", "## 26."),
        ("D.1", "### D.1"),
        ("D.2", "### D.2"),
        ("D.2.1", "D.2.1"),
        ("D.2.2", "D.2.2"),
        ("D.3", "### D.3"),
    ] {
        assert!(agents.contains(cite), "AGENTS.md should cite {cite}");
        assert!(
            spec.contains(anchor),
            "SPEC.md has no anchor {anchor} cited by AGENTS.md ({cite})"
        );
    }

    // CLI flags cited by the cheat-sheet must exist in --help.
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_rsmpl-git"));
    let out = Command::new(&binary).arg("--help").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8_lossy(&out.stdout).to_string();
    for flag in [
        "--file",
        "--repo",
        "--check",
        "--format",
        "--diff-max-lines",
        "--output",
    ] {
        assert!(agents.contains(flag), "AGENTS.md should cite {flag}");
        assert!(
            help.contains(flag),
            "--help has no {flag} cited by AGENTS.md"
        );
    }

    // The four recipe skeletons, verbatim.
    for skeleton in ["fn foo(...)", "foo(E)", "old_method", "bind="] {
        assert!(
            agents.contains(skeleton),
            "AGENTS.md missing recipe skeleton {skeleton:?}"
        );
    }

    // Entry pointers both ways.
    assert!(
        readme.contains("AGENTS.md"),
        "README.md should point to AGENTS.md"
    );
    assert!(
        spec.contains("AGENTS.md"),
        "SPEC.md Appendix D should point to AGENTS.md"
    );

    // T117 anti-rot: the patch-first skill exists and teaches the token
    // loop (scope=file over file reads, budget flags) plus the repair path.
    let skill = read_root("skills/rsmpl-git/SKILL.md");
    assert!(
        skill.lines().count() <= 150,
        "skill budget is 150 lines, got {}",
        skill.lines().count()
    );
    for needle in [
        "scope=file",
        "RSMPL_MATCH_CONFLICT",
        "--diff-max-lines 0",
        "TASKS.md",
    ] {
        assert!(skill.contains(needle), "skill missing needle {needle:?}");
    }
    // T146 anti-rot: SKILL.md's SPEC citations must resolve too (the
    // `SPEC D.3.3` pointer rotted silently after T122 deleted the D.3
    // subsections). Same (cite, anchor) shape as the AGENTS.md table.
    for (cite, anchor) in [
        ("17.3", "### 17.3"),
        ("10.4", "### 10.4"),
        ("SPEC 10", "## 10."),
        ("7.1", "### 7.1"),
        ("SPEC 9.3", "### 9.3"),
        ("SPEC 14", "## 14."),
        ("14.1", "### 14.1"),
        ("14.7", "### 14.7"),
        ("17.8", "### 17.8"),
        ("18.5", "### 18.5"),
        ("14.8", "### 14.8"),
        ("14.9", "### 14.9"),
        ("SPEC 20", "## 20."),
        ("SPEC 21", "## 21."),
        ("D.2.1", "D.2.1"),
        ("D.2.2", "D.2.2"),
        ("D.2.4", "D.2.4"),
        ("D.1", "### D.1"),
        ("SPEC 25", "## 25."),
        ("SPEC D.3", "### D.3"),
    ] {
        assert!(skill.contains(cite), "SKILL.md should cite {cite}");
        assert!(
            spec.contains(anchor),
            "SPEC.md has no anchor {anchor} cited by SKILL.md ({cite})"
        );
    }
    // Deleted sections must stay uncited (T122 removed the D.3 subsections
    // with the inspect surface; a bare `contains` pin is blind to `D.3.3`
    // because it contains `D.3` as a substring).
    for stale in ["D.3.1", "D.3.2", "D.3.3"] {
        assert!(
            !skill.contains(stale),
            "SKILL.md cites removed section {stale}"
        );
    }
    assert!(
        agents.contains("skills/rsmpl-git/SKILL.md"),
        "AGENTS.md should point to the skill"
    );
    assert!(
        readme.contains("skills/rsmpl-git/SKILL.md"),
        "README.md should point to the skill"
    );
    assert!(
        spec.contains("skills/rsmpl-git/SKILL.md"),
        "SPEC.md Appendix D should point to the skill"
    );

    // T121 anti-rot: the `.opencode/skills/` discovery symlink must resolve
    // to the canonical skill (OpenCode only searches `.opencode/`,
    // `.claude/`, `.agents/` project-local paths — `skills/` alone is
    // invisible to the `skill` tool). Content equality fails loudly if the
    // link is deleted or the canonical moves.
    let via_link = read_root(".opencode/skills/rsmpl-git/SKILL.md");
    assert_eq!(
        via_link, skill,
        ".opencode/skills/rsmpl-git/SKILL.md should match skills/rsmpl-git/SKILL.md"
    );
}
