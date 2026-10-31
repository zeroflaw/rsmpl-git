//! `gitrs`: agent entry point dispatching to the subcommand binaries.
//!
//! ```sh
//! gitrs apply --file change.rsmpl --check   # invoke applyrs
//! gitrs diff --repo .                       # invoke diffrs
//! ```
//!
//! The dispatcher never parses subcommand flags itself: `apply` re-execs
//! the sibling `applyrs` binary with the passthrough args (and `diff`
//! the sibling `diffrs`), propagating its exit code. Dispatcher-level
//! failures (unknown subcommand, missing sibling) exit 2 with the same
//! stdout-only JSON envelope family as `applyrs` when `--format json`
//! is present (SPEC D.2).

use std::path::PathBuf;

/// Resolve the sibling binary `name` next to the running dispatcher.
///
/// `$GITRS_<NAME>_BIN` (uppercased, e.g. `GITRS_APPLYRS_BIN`) overrides
/// for tests; otherwise the sibling is looked up beside `current_exe`.
fn sibling(name: &str) -> PathBuf {
    let env_key = format!("GITRS_{}_BIN", name.to_uppercase());
    if let Some(p) = std::env::var_os(&env_key) {
        let p = PathBuf::from(p);
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&file)))
        .unwrap_or_else(|| PathBuf::from(&file))
}

fn wants_json(args: &[String]) -> bool {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--format" {
            if it.next().is_some_and(|v| v == "json") {
                return true;
            }
        } else if let Some(v) = a.strip_prefix("--format=") {
            if v == "json" {
                return true;
            }
        }
    }
    false
}

fn fail(reason: &str, args: &[String]) -> ! {
    if wants_json(args) {
        let payload = serde_json::json!({
            "version": 3,
            "error": {
                "code": "RSMPL_IO_CONFLICT",
                "rule": null,
                "file": null,
                "range": null,
                "reason": reason,
                "suggestion": "run `gitrs --help` for usage",
                "repair": null,
            }
        });
        println!("{payload}");
    } else {
        eprintln!("error: {reason}");
        eprintln!("run `gitrs --help` for usage");
    }
    std::process::exit(2);
}

fn help_text() -> &'static str {
    "gitrs: agent-centric Rust refactoring via semantic patches\n\
     \n\
     Usage: gitrs <apply|diff> ...\n\
     \n\
     Subcommands:\n\
     \x20 apply   Apply an RSmPL-Git semantic patch (invokes applyrs)\n\
     \x20 diff    Compress git changes into a semantic patch (invokes diffrs)\n\
     \n\
     Run `gitrs apply --help` or `gitrs diff --help` for subcommand flags.\n"
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1).map(String::as_str).unwrap_or("");
    match sub {
        "apply" => invoke("applyrs", &args[2..], &args),
        "diff" => invoke("diffrs", &args[2..], &args),
        "-h" | "--help" | "help" => {
            print!("{}", help_text());
        }
        "-V" | "--version" | "version" => {
            println!("gitrs {}", env!("CARGO_PKG_VERSION"));
        }
        "" => fail("no subcommand given (want `apply` or `diff`)", &args),
        other => fail(
            &format!("unknown subcommand `{other}` (want `apply` or `diff`)"),
            &args,
        ),
    }
}

/// Re-exec the sibling `binary` with `rest`, propagating its exit code.
///
/// WI-10a handshake: unless the sibling path came from a `GITRS_*_BIN`
/// test-double override (which skips the check), the sibling's
/// `--version` must equal this dispatcher's `CARGO_PKG_VERSION` before
/// re-exec. A skew fails loud with `RSMPL_IO_CONFLICT` naming both
/// versions instead of silently running mixed semantics.
fn invoke(binary: &str, rest: &[String], args: &[String]) -> ! {
    let bin = sibling(binary);
    let overridden = {
        let env_key = format!("GITRS_{}_BIN", binary.to_uppercase());
        std::env::var_os(&env_key).is_some_and(|p| !p.as_os_str().is_empty())
    };
    if !overridden {
        match sibling_version(&bin) {
            Some(v) if v == env!("CARGO_PKG_VERSION") => {}
            Some(v) => fail(
                &format!(
                    "sibling version skew: gitrs {} but {} {v} (rebuild the workspace so all binaries match)",
                    env!("CARGO_PKG_VERSION"),
                    bin.display()
                ),
                args,
            ),
            None => fail(
                &format!(
                    "cannot verify {} version (ran `{}` with `--version`)",
                    bin.display(),
                    bin.display()
                ),
                args,
            ),
        }
    }
    match std::process::Command::new(&bin).args(rest).status() {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => fail(&format!("cannot run {}: {e}", bin.display()), args),
    }
}

/// Last whitespace token of `<sibling> --version` output
/// (`applyrs 0.1.0` → `0.1.0`), or `None` when the sibling cannot be
/// queried. Pure helper so the token parse is unit-pinned.
fn parse_sibling_version(output: &str) -> Option<String> {
    output.split_whitespace().last().map(str::to_string)
}

fn sibling_version(bin: &std::path::Path) -> Option<String> {
    let out = std::process::Command::new(bin)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_sibling_version(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::parse_sibling_version;

    #[test]
    fn version_token_is_last_word() {
        assert_eq!(
            parse_sibling_version("applyrs 0.1.0\n"),
            Some("0.1.0".to_string())
        );
        assert_eq!(parse_sibling_version(""), None);
    }
}
