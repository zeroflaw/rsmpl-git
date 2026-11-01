//! WI-2 `--verify build` failure classification (SPEC 25.10).
//!
//! Distinguishes a post-lowering `cargo check` failure (new
//! `RSMPL_BUILD_CONFLICT` with a `read_file` repair) from an
//! environmental failure (missing `cargo` / missing `Cargo.toml`,
//! which keeps `RSMPL_IO_CONFLICT` with null repair). The diagnostic
//! parser keys on cargo's stable ` --> file:line[:col]` lines and
//! degrades to the first orphan fallback, never panicking.

use rsmpl_patch::StructuredError;

/// Post-`cargo check` outcome: success, a real build breakage (with the
/// full stderr for location parsing), or an environmental failure that
/// no patch spelling can fix.
pub enum VerifyBuildOutcome {
    BuildFailed { short: String, stderr: String },
    Environmental(String),
}

/// Run `cargo check` in `dir`: `Ok` on success, otherwise classified.
pub fn run_verify_build_in(dir: &std::path::Path) -> Result<(), VerifyBuildOutcome> {
    let out = std::process::Command::new("cargo")
        .arg("check")
        .arg("--quiet")
        .current_dir(dir)
        .output()
        .map_err(|e| VerifyBuildOutcome::Environmental(format!("cannot run `cargo check`: {e}")))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if is_missing_manifest(&stderr) {
        return Err(VerifyBuildOutcome::Environmental(stderr.trim().to_string()));
    }
    let short = stderr
        .lines()
        .find(|l| l.contains("error"))
        .map(str::trim)
        .unwrap_or("cargo check failed");
    let short: String = short.chars().take(300).collect();
    Err(VerifyBuildOutcome::BuildFailed { short, stderr })
}

/// True when cargo reports no manifest: environmental (SPEC 25.10), not
/// a patch breakage. Keys on the stable `could not find ... Cargo.toml`
/// wording in either quoting style.
fn is_missing_manifest(stderr: &str) -> bool {
    stderr.contains("Cargo.toml") && stderr.contains("could not find")
}

/// First ` --> path:line[:col]` location in cargo human output
/// (`--quiet` keeps this format). Returns the repo-relative path and
/// the 1-based line, or `None` when no location parses.
pub fn parse_build_location(stderr: &str) -> Option<(String, u32)> {
    for line in stderr.lines() {
        let Some(idx) = line.find("-->") else {
            continue;
        };
        let rest = line[idx + 3..].trim();
        if rest.is_empty() {
            continue;
        }
        let mut parts = rest.split(':');
        let file = parts.next()?.trim();
        let line_no: u32 = parts.next()?.trim().parse().ok()?;
        if file.is_empty() || line_no == 0 {
            continue;
        }
        return Some((file.to_string(), line_no));
    }
    None
}

/// WI-2 repair precedence (SPEC 25.10, D.2): the first cargo diagnostic
/// wins; otherwise the first orphan's `file`/`line`; otherwise `None`
/// (double-miss, joins the WI-3 allow-list by name).
pub fn build_repair_for(
    stderr: &str,
    orphans: &[rsmpl_engine::OrphanReport],
) -> Option<serde_json::Value> {
    if let Some((file, line)) = parse_build_location(stderr) {
        return Some(StructuredError::read_file_repair(
            &file,
            &format!("{line}..{line}"),
        ));
    }
    let first = orphans.first()?;
    Some(StructuredError::read_file_repair(
        &first.file,
        &format!("{}..{}", first.line, first.line),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn orphan(file: &str, line: u32) -> rsmpl_engine::OrphanReport {
        rsmpl_engine::OrphanReport {
            file: file.to_string(),
            line,
            text: "foo()".to_string(),
            reason: "off-target".to_string(),
        }
    }

    #[test]
    fn parses_arrow_location_with_col() {
        let stderr =
            "error[E0425]: cannot find function `undefined_fn`\n --> src/lib.rs:3:5\n  |\n";
        assert_eq!(
            parse_build_location(stderr),
            Some(("src/lib.rs".to_string(), 3))
        );
    }

    #[test]
    fn parses_arrow_location_without_col() {
        let stderr = "error: failed to parse manifest\n --> Cargo.toml:2\n";
        assert_eq!(
            parse_build_location(stderr),
            Some(("Cargo.toml".to_string(), 2))
        );
    }

    #[test]
    fn no_location_yields_none() {
        assert_eq!(parse_build_location("cargo check failed\n"), None);
        assert_eq!(parse_build_location(""), None);
    }

    #[test]
    fn first_error_location_wins() {
        let stderr = "error[E1]\n --> src/a.rs:1:1\nerror[E2]\n --> src/b.rs:9:2\n";
        assert_eq!(
            parse_build_location(stderr),
            Some(("src/a.rs".to_string(), 1))
        );
    }

    #[test]
    fn diagnostic_beats_orphan_fallback() {
        let stderr = "error[E0425]\n --> src/lib.rs:3:5\n";
        let orphans = vec![orphan("src/other.rs", 7)];
        let r = build_repair_for(stderr, &orphans).expect("repair");
        assert_eq!(r["file"], serde_json::json!("src/lib.rs"));
        assert_eq!(r["range"], serde_json::json!("3..3"));
    }

    #[test]
    fn orphan_fallback_when_no_location() {
        let orphans = vec![orphan("src/lib.rs", 5)];
        let r = build_repair_for("cargo check failed", &orphans).expect("repair");
        assert_eq!(r["file"], serde_json::json!("src/lib.rs"));
        assert_eq!(r["range"], serde_json::json!("5..5"));
    }

    #[test]
    fn double_miss_stays_null() {
        assert!(build_repair_for("cargo check failed", &[]).is_none());
    }

    #[test]
    fn missing_manifest_detected() {
        assert!(is_missing_manifest(
            "error: could not find `Cargo.toml` in `/tmp/x` or any parent directory"
        ));
        assert!(!is_missing_manifest("error[E0425]: cannot find function"));
    }
}
