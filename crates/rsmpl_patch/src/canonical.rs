//! WI-1a (IMPROVE.md): canonical RSmPL form (SPEC D.5).
//!
//! `canonicalize` parses a patch and re-emits it in canonical form:
//! files sorted by target path, semantic rules renumbered `r1…` in
//! canonical order, directives in fixed order (`# sweep` fused when
//! effective), RSmPL headers with the new side elided (SPEC 10.4), and
//! `index …` metadata dropped (T374).
//! The re-emission is semantics-preserving: effective directive values,
//! body lines, file order aside, and all original ranges are kept; only
//! advisory text (titles, index lines, rule names, directive order,
//! file order) is normalized. Hunks keep document order within a file:
//! insertion anchors at a range edge are order-sensitive (a paste at
//! `(1,0)` plus an edit of `[1,1]` apply in document order per SPEC
//! 20.2), so range-sorting hunks would change application results
//! (probed via the 03-move-fn shape). With `# scope=file` the old
//! range is a dummy (SPEC 17.3), so sweep hunks normalize to
//! `@@ -1,1 @@` — except zero-length insertion anchors, which ignore
//! scope (SPEC 10.5) and keep their anchor verbatim.

use crate::model::*;
use crate::parser::{collapse_git_header, parse_patch, ParseError};

/// Re-emit `patch_text` in canonical form (SPEC D.5).
///
/// Fails exactly when the input fails to parse (including the T59 empty
/// patch); the output always re-parses and re-canonicalizes identically
/// (fixed point, pinned by test).
pub fn canonicalize(patch_text: &str) -> Result<String, ParseError> {
    let mut patch = parse_patch(patch_text)?;
    patch
        .files
        .sort_by(|a, b| a.target_path.cmp(&b.target_path));
    let mut out = String::new();
    if let Some(v) = &patch.version {
        out.push_str(&format!("#{v}\n"));
    }
    let mut rule_seq = 0u32;
    for sec in &mut patch.files {
        // Hunks keep document order: insertion anchors at a range edge
        // apply in document order (SPEC 20.2), so range-sorting is not
        // application-safe. Files sort freely: cross-file bind/use
        // resolution is order-independent (pastes may precede cuts).
        out.push_str(&emit_file_header(sec));
        out.push('\n');
        for line in &sec.metadata {
            if line.starts_with("index ") {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        for hunk in &sec.hunks {
            match hunk {
                Hunk::Ordinary(o) => emit_ordinary(&mut out, o),
                Hunk::Rsmpl(r) => {
                    rule_seq += 1;
                    emit_rsmpl(&mut out, r, rule_seq);
                }
            }
        }
    }
    Ok(out)
}

/// `diff --git` header line: single-path when both halves agree,
/// else the two-path form collapsed to single-path when SPEC 6 allows
/// it (round-trip-guarded inside `collapse_git_header`); renames,
/// creates, deletes, and quoted headers keep two paths.
fn emit_file_header(sec: &FileSection) -> String {
    if sec.old_path == sec.new_path {
        return format!("diff --git {}", sec.old_path);
    }
    collapse_git_header(&format!("diff --git {} {}", sec.old_path, sec.new_path))
}

fn emit_ordinary(out: &mut String, o: &OrdinaryHunk) {
    let h = &o.header;
    out.push_str(&format!(
        "@@ -{},{} +{},{} @@",
        h.old_start, h.old_len, h.new_start, h.new_len
    ));
    if let Some(rule) = &h.rule {
        out.push_str(&format!(" rule={rule}"));
    }
    // Git `@@ … @@ <title>` trailers are advisory; `diffrs` already
    // drops them and `git apply` ignores them.
    out.push('\n');
    for line in &o.lines {
        out.push(line.prefix);
        out.push_str(&line.text);
        out.push('\n');
    }
}

fn emit_rsmpl(out: &mut String, r: &RsmplHunk, rule_seq: u32) {
    let h = &r.header;
    // With `# scope=file` the old range is a dummy (SPEC 17.3) — pin it
    // to the canonical dummy, unless this is a zero-length insertion
    // anchor, which ignores scope (SPEC 10.5).
    let (start, len) = if r.scope() == Scope::File && h.old_len != 0 {
        (1, 1)
    } else {
        (h.old_start, h.old_len)
    };
    // The RSmPL new side is ignored by the lowering (SPEC 10.4).
    out.push_str(&format!("@@ -{start},{len} @@ rule=r{rule_seq}\n"));
    for d in canonical_directives(r) {
        out.push_str(&d);
        out.push('\n');
    }
    for line in &r.lines {
        out.push(line.prefix);
        out.push_str(&line.text);
        out.push('\n');
    }
}

/// Directives in fixed canonical order (SPEC D.5), derived from
/// effective values so positional `# sweep` expansion (which a later
/// explicit directive may override) re-emits identically.
fn canonical_directives(r: &RsmplHunk) -> Vec<String> {
    let mut dirs = Vec::new();
    let sweep = r.allow() == Allow::Many && r.scope() == Scope::File;
    if sweep {
        dirs.push("# sweep".to_string());
    } else {
        if r.allow() == Allow::Many {
            dirs.push("# allow=many".to_string());
        }
        match r.scope() {
            Scope::Contained => {}
            Scope::Anchor => dirs.push("# scope=anchor".to_string()),
            Scope::Overlap => dirs.push("# scope=overlap".to_string()),
            Scope::Exact => dirs.push("# scope=exact".to_string()),
            Scope::File => dirs.push("# scope=file".to_string()),
        }
    }
    if r.is_optional() {
        dirs.push("# optional".to_string());
    }
    if r.repeats() {
        dirs.push("# repeat".to_string());
    }
    if r.has_lang_rust() {
        dirs.push("# lang=rust".to_string());
    }
    for d in &r.directives {
        if let Directive::Root(v) = d {
            dirs.push(format!("# root={v}"));
        }
    }
    for d in &r.directives {
        if let Directive::Bind(n) = d {
            dirs.push(format!("# bind={n}"));
        }
    }
    for d in &r.directives {
        if let Directive::Use(n) = d {
            dirs.push(format!("# use={n}"));
        }
    }
    for d in &r.directives {
        if let Directive::Symbol { name, path } = d {
            dirs.push(format!("# symbol {name}={path}"));
        }
    }
    // Metavariable declarations merged per kind (the parser drops any
    // `= constraint`, so plain `# kind N1, N2` re-emits faithfully);
    // kinds sorted for determinism, names deduped first-seen.
    let mut kinds: Vec<(MetavarKind, Vec<String>)> = Vec::new();
    for d in &r.directives {
        if let Directive::MetaVar { kind, names } = d {
            match kinds.iter_mut().find(|(k, _)| k == kind) {
                Some((_, have)) => {
                    for n in names {
                        if !have.contains(n) {
                            have.push(n.clone());
                        }
                    }
                }
                None => kinds.push((*kind, names.clone())),
            }
        }
    }
    kinds.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    for (kind, names) in &kinds {
        dirs.push(format!("# {} {}", kind.as_str(), names.join(", ")));
    }
    for d in &r.directives {
        match d {
            Directive::Attrs(a) if *a != AttrsPolicy::Preserve => {
                let v = match a {
                    AttrsPolicy::Absent => "absent",
                    AttrsPolicy::Explicit => "explicit",
                    AttrsPolicy::Strip => "strip",
                    AttrsPolicy::Preserve => unreachable!(),
                };
                dirs.push(format!("# attrs={v}"));
            }
            Directive::Mode(m) if *m != ParseMode::Ast => {
                dirs.push("# mode=tokens".to_string());
            }
            Directive::MacroMode(m) if *m != MacroMode::Invocation => {
                let v = match m {
                    MacroMode::TokenTree => "token_tree",
                    MacroMode::Expanded => "expanded",
                    MacroMode::Invocation => unreachable!(),
                };
                dirs.push(format!("# macro={v}"));
            }
            Directive::Format(f) if *f != FormatMode::Preserve => {
                let v = match f {
                    FormatMode::Rustfmt => "rustfmt",
                    FormatMode::Canonical => "canonical",
                    FormatMode::Preserve => unreachable!(),
                };
                dirs.push(format!("# format={v}"));
            }
            Directive::ReturnsNone(true) => dirs.push("# returns=none".to_string()),
            Directive::FallbackSyntax(true) => dirs.push("# fallback=syntax".to_string()),
            Directive::FollowUses => dirs.push("# follow=uses".to_string()),
            Directive::SigiledMetavars(true) => dirs.push("# metavars=sigiled".to_string()),
            _ => {}
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::canonicalize;
    use crate::parser::parse_patch;

    fn canon(text: &str) -> String {
        canonicalize(text).expect("canonicalize must succeed")
    }

    #[test]
    fn sweep_fuses_and_dummy_range_pins() {
        // Expanded allow+scope with a stray range normalizes to diet.
        let text = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -5,3 @@ rule=zzz\n# allow=many\n# scope=file\n# expr E\n- foo(E)\n+ bar(E)\n";
        let got = canon(text);
        assert!(
            got.contains("@@ -1,1 @@ rule=r1\n# sweep\n# expr E\n- foo(E)\n+ bar(E)\n"),
            "got:\n{got}"
        );
    }

    #[test]
    fn directive_order_is_fixed() {
        // Shuffled input directives emit in canonical order.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=z\n# expr E\n# lang=rust\n# sweep\n# repeat\n- foo(E)\n+ bar(E)\n";
        let got = canon(text);
        let sweep = got.find("# sweep").unwrap();
        let repeat = got.find("# repeat").unwrap();
        let lang = got.find("# lang=rust").unwrap();
        let expr = got.find("# expr E").unwrap();
        assert!(
            sweep < repeat && repeat < lang && lang < expr,
            "got:\n{got}"
        );
    }

    #[test]
    fn rules_renumber_in_file_order_then_document_order() {
        // Files sort by path; hunks keep document order; rules follow.
        let text = "diff --git src/z.rs\n@@ -20,1 @@ rule=second\n# sweep\n- a\n+ b\ndiff --git src/a.rs\n@@ -3,1 @@ rule=first\n# sweep\n- c\n+ d\n";
        let got = canon(text);
        let a = got.find("diff --git src/a.rs").unwrap();
        let z = got.find("diff --git src/z.rs").unwrap();
        assert!(a < z, "got:\n{got}");
        let r1 = got.find("rule=r1").unwrap();
        let r2 = got.find("rule=r2").unwrap();
        assert!(a < r1 && r1 < z && z < r2, "got:\n{got}");
    }

    #[test]
    fn anchor_plus_range_edge_keeps_document_order() {
        // 03-move-fn shape: an ordinary edit of [1,1] authored before a
        // paste at anchor (1,0) must stay in that order — application
        // results differ when they swap (SPEC 20.2 document order).
        let text = "diff --git src/new.rs\n@@ -1,1 +1,2 @@\n use crate::common;\n+use crate::old_dependency;\n\n@@ -1,0 +1,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n";
        let got = canon(text);
        let ordinary = got.find("use crate::old_dependency").unwrap();
        let paste = got.find("rule=r1").unwrap();
        assert!(ordinary < paste, "got:\n{got}");
        assert!(got.contains("@@ -1,1 +1,2 @@\n"), "got:\n{got}");
        assert!(got.contains("@@ -1,0 @@ rule=r1\n"), "got:\n{got}");
    }

    #[test]
    fn contained_many_keeps_real_range() {
        // No scope=file: the real range is load-bearing, keep it.
        let text = "diff --git src/lib.rs\n@@ -5,3 @@ rule=z\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
        let got = canon(text);
        assert!(
            got.contains("@@ -5,3 @@ rule=r1\n# allow=many\n"),
            "got:\n{got}"
        );
    }

    #[test]
    fn version_marker_survives() {
        let text = "# gitrs=0.1\ndiff --git src/lib.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        let got = canon(text);
        assert!(got.starts_with("#gitrs=0.1\n"), "got:\n{got}");
        let twice = canon(&got);
        assert_eq!(got, twice);
    }

    #[test]
    fn insertion_anchor_survives_scope_file() {
        // Zero-length anchors ignore scope (SPEC 10.5): keep verbatim.
        let text = "diff --git src/lib.rs\n@@ -7,0 +8,1 @@ rule=z\n# scope=file\n# use=H\n+ H\n";
        let got = canon(text);
        assert!(
            got.contains("@@ -7,0 @@ rule=r1\n# scope=file\n"),
            "got:\n{got}"
        );
    }

    #[test]
    fn index_lines_drop_and_headers_collapse() {
        let text = "diff --git a/src/lib.rs b/src/lib.rs\nindex abc..def 100644\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ title\n-old\n+new\n";
        let got = canon(text);
        assert!(!got.contains("index "), "got:\n{got}");
        assert!(!got.contains("title"), "got:\n{got}");
        assert!(got.contains("diff --git src/lib.rs\n"), "got:\n{got}");
        assert!(got.contains("@@ -1,1 +1,1 @@\n-old\n+new\n"), "got:\n{got}");
    }

    #[test]
    fn renames_keep_two_paths() {
        let text = "diff --git a/old.rs b/new.rs\nsimilarity index 90%\nrename from old.rs\nrename to new.rs\n";
        // No hunks: still a parseable section; canonical keeps halves.
        let got = canon(text);
        assert!(
            got.contains("diff --git a/old.rs b/new.rs\n"),
            "got:\n{got}"
        );
        assert!(got.contains("rename from old.rs\n"), "got:\n{got}");
    }

    #[test]
    fn wi1b_ranged_sweep_is_fixed_point() {
        // WI-1b emission: tight range + `# allow=many`, inferred
        // metavars, no `# scope=file` — canonicalizes to itself.
        let text =
            "diff --git src/lib.rs\n@@ -2,3 @@ rule=r9\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let got = canon(text);
        assert!(
            got.contains("@@ -2,3 @@ rule=r1\n# allow=many\n- foo(E0)\n+ bar(E0)\n"),
            "got:\n{got}"
        );
        assert_eq!(got, canon(&got));
    }

    #[test]
    fn follow_uses_emits_in_fixed_order() {
        // WI-6a: `# follow=uses` survives canonicalization after fallback.
        // Tail directives keep input relative order (existing
        // canonical semantics); the pin is survival + fixed point.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=z\n# fallback=syntax\n# follow=uses\n- struct Old\n+ struct New\n";
        let got = canon(text);
        assert!(
            got.contains("# fallback=syntax\n# follow=uses\n- struct Old\n+ struct New\n"),
            "got:\n{got}"
        );
        assert_eq!(got, canon(&got));
    }

    #[test]
    fn idempotent_and_reparseable() {
        let text = "diff --git a/src/b.rs b/src/b.rs\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -9,2 @@ rule=b\n# expr E\n# sweep\n- foo(E)\n+ bar(E)\ndiff --git a/src/a.rs b/src/a.rs\nindex 111..222 100644\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        let once = canon(text);
        parse_patch(&once).expect("canonical output must re-parse");
        let twice = canon(&once);
        assert_eq!(once, twice, "canonical form must be a fixed point");
    }
}
