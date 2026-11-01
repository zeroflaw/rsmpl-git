//! WI-1a (IMPROVE.md): canonical RSmPL form (SPEC D.5).
//!
//! `canonicalize` parses a patch and re-emits it in canonical form:
//! files sorted by target path (WI-7b: shared `rename_Old_to_New` intents
//! order definition-first), semantic rules renumbered `r1…` except
//! shared `rename_*` names occurring in ≥2 hunks (preserved verbatim),
//! directives in fixed order (`# sweep` fused when
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

use std::collections::HashMap;

use crate::model::*;
use crate::parser::{collapse_git_header, parse_patch, ParseError};

/// D.5 emission range rule: a multi-site hunk whose sites fit in this
/// many original lines cites the tight range; wider spreads use the
/// `# sweep` dummy.
pub const TIGHT_RANGE_MAX_LINES: u32 = 64;

/// Re-emit `patch_text` in canonical form (SPEC D.5).
///
/// Fails exactly when the input fails to parse (including the T59 empty
/// patch); the output always re-parses and re-canonicalizes identically
/// (fixed point, pinned by test).
pub fn canonicalize(patch_text: &str) -> Result<String, ParseError> {
    let mut patch = parse_patch(patch_text)?;
    order_files(&mut patch);
    Ok(emit_canonical(&mut patch))
}

/// Tree-aware canonical form (SPEC D.5, WI-1a + WI-8): like [`canonicalize`],
/// but every hunk whose key `(target file, rule name)` has an entry in
/// `spans` is re-emitted under the D.5 range rule — multi-site `allow ==
/// many` hunks take the tight `@@ -min,len @@` with `# allow=many` when
/// the union of matched site spans fits in [`TIGHT_RANGE_MAX_LINES`]
/// lines, else the `# sweep` dummy `@@ -1,1 @@`; single-site hunks of
/// either cardinality collapse to the diet directive-free tight actual
/// range. This unifies the spellings of one rewrite (authored sweep vs
/// inferred ranged vs inferred single vs stale-cited) for comparison.
///
/// `spans` values are original-coordinate inclusive `[start, end]` line
/// spans, one per matched site (as reported by the engine). Hunks left
/// untouched: zero-length insertion anchors (SPEC 10.5), `allow == one`
/// hunks with ≠1 spans, ordinary hunks, non-contained/file scopes (`anchor`/`overlap`/
/// `exact` ranges are load-bearing for matching), and keys with no
/// spans (zero-match `# optional`, unapplied trees).
pub fn canonicalize_with_spans(
    patch_text: &str,
    spans: &HashMap<(String, String), Vec<(u32, u32)>>,
) -> Result<String, ParseError> {
    let patch = parse_patch(patch_text)?;
    Ok(canonicalize_with_spans_parsed(patch, spans))
}

/// [`canonicalize_with_spans`] over an already-parsed patch (WI-1b: the
/// tree-aware comparator spell-normalizes before ranging, so it threads
/// the mutated patch through instead of re-parsing the original text).
/// Infallible: the input already parsed.
pub fn canonicalize_with_spans_parsed(
    mut patch: Patch,
    spans: &HashMap<(String, String), Vec<(u32, u32)>>,
) -> String {
    normalize_ranges_with_spans(&mut patch, spans);
    normalize_file_markers(&mut patch);
    order_files(&mut patch);
    emit_canonical(&mut patch)
}

/// WI-6 slice 2: the tree-aware form compares rewrites, not citations.
/// `---`/`+++` file markers are git-apply conveniences the engine never
/// reads (whole-section inferrer emissions never carry them; per-hunk
/// and delete-sweep emissions preserve them from the native prelude),
/// so an authored marker-free sweep and an inferred ranged hunk over
/// the same sites share one form only once markers normalize away.
/// Mode/rename/index metadata stays (load-bearing elsewhere). Pure
/// [`canonicalize`] is untouched (spelling-preserving fixed point).
fn normalize_file_markers(patch: &mut Patch) {
    for sec in &mut patch.files {
        sec.metadata
            .retain(|l| !l.starts_with("--- ") && !l.starts_with("+++ "));
    }
}

/// Apply the D.5 range rule to hunks with known spans (SPEC D.5, WI-8):
/// multi-site `allow == many` hunks as before (tight `@@ -min,len @@`
/// with `# allow=many` when the union fits in [`TIGHT_RANGE_MAX_LINES`]
/// lines, else the `# sweep` dummy), plus single-site hunks of either
/// cardinality (an authored sweep single and an inferred precision
/// single over the same site share one diet form: directive-free tight
/// actual range; a stale cited single re-ranges to its proven span).
/// `allow == one` hunks with ≠1 spans never occur on success (the apply
/// would fail, leaving no spans) and stay untouched, so error shapes
/// never widen — except `# follow=uses` hunks (WI-9): a follow hunk
/// legitimately proves def + follow spans under the default `allow ==
/// one` (the authored 46/89 fixtures carry no `# allow` line), so
/// multi-site follow hunks normalize like `allow == many` (tight
/// actual + `# allow=many`, comparison-only; the engine already proved
/// the spans).
fn normalize_ranges_with_spans(
    patch: &mut Patch,
    spans: &HashMap<(String, String), Vec<(u32, u32)>>,
) {
    for sec in &mut patch.files {
        for hunk in &mut sec.hunks {
            let Hunk::Rsmpl(r) = hunk else { continue };
            if r.header.old_len == 0 {
                continue;
            }
            if !matches!(r.scope(), Scope::Contained | Scope::File) {
                continue;
            }
            let key = (sec.target_path.clone(), r.rule_name());
            let Some(sites) = spans.get(&key) else {
                continue;
            };
            if sites.is_empty() {
                continue;
            }
            // WI-8: single-site hunks (either cardinality) collapse to
            // the diet single form; multi-site `allow == one` is an
            // error shape (apply fails → no spans in practice) and is
            // left byte-identical — unless the hunk carries
            // `# follow=uses` (WI-9), whose def + follow spans are
            // legitimate under the default `allow == one`.
            let single = sites.len() == 1;
            if r.allow() != Allow::Many && !single && !r.follows_uses() {
                continue;
            }
            let min_s = sites.iter().map(|(s, _)| *s).min().unwrap_or(1);
            let max_e = sites.iter().map(|(_, e)| *e).max().unwrap_or(1);
            if max_e < min_s {
                continue;
            }
            let len = max_e - min_s + 1;
            if len <= TIGHT_RANGE_MAX_LINES {
                r.header.old_start = min_s;
                r.header.old_len = len;
                if single {
                    // Diet single: directive-free tight (drop any
                    // `allow` + file-scope lines; effective allow falls
                    // back to `one`, scope to `contained`).
                    r.directives.retain(|d| {
                        !matches!(d, Directive::Allow(_) | Directive::Scope(Scope::File))
                    });
                } else {
                    // Tight multi: `# allow=many` alone (drop file
                    // scope; WI-9 follow hunks gain the line —
                    // comparison-only, spans already proved).
                    r.directives
                        .retain(|d| !matches!(d, Directive::Scope(Scope::File)));
                    if r.allow() != Allow::Many {
                        r.directives.push(Directive::Allow(Allow::Many));
                    }
                }
            } else {
                r.header.old_start = 1;
                r.header.old_len = 1;
                if r.scope() != Scope::File {
                    r.directives.push(Directive::Scope(Scope::File));
                }
                if r.allow() != Allow::Many {
                    r.directives.push(Directive::Allow(Allow::Many));
                }
            }
        }
    }
}

/// WI-7b shared intents: a `rename_*` rule occurring in ≥2 hunks
/// keeps its name verbatim instead of renumbering to `rN`.
fn shared_rule_names(patch: &Patch) -> std::collections::HashSet<String> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for sec in &patch.files {
        for hunk in &sec.hunks {
            if let Hunk::Rsmpl(r) = hunk {
                if let Some(rule) = r.header.rule.as_deref() {
                    if rule.starts_with("rename_") {
                        *counts.entry(rule.to_string()).or_default() += 1;
                    }
                }
            }
        }
    }
    counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(r, _)| r)
        .collect()
}

fn order_files(patch: &mut Patch) {
    patch
        .files
        .sort_by(|a, b| a.target_path.cmp(&b.target_path));
    // WI-7b: shared descriptive intents (`rename_Old_to_New` in ≥2 hunks)
    // keep their names verbatim; everything else renumbers `r1…`.
    let shared = shared_rule_names(patch);
    // WI-7b definition-first file order for shared groups: path-sort,
    // then per shared label (sorted) stable-sort the containing file
    // subsequence definitions-first (item-header minus → definition).
    let mut labels: Vec<&String> = shared.iter().collect();
    labels.sort();
    for label in labels {
        let mut involved: Vec<usize> = Vec::new();
        for (idx, sec) in patch.files.iter().enumerate() {
            if file_contains_rule(sec, label) {
                involved.push(idx);
            }
        }
        if involved.len() < 2 {
            continue;
        }
        let mut sorted = involved.clone();
        sorted.sort_by_key(|&i| {
            let def = patch.files[i]
                .hunks
                .iter()
                .filter_map(|h| match h {
                    Hunk::Rsmpl(r) if r.header.rule.as_deref() == Some(label.as_str()) => {
                        Some(is_definition_hunk(r))
                    }
                    _ => None,
                })
                .any(|d| d);
            ((!def) as u8, patch.files[i].target_path.clone())
        });
        if sorted == involved {
            continue;
        }
        let snapshot = patch.files.clone();
        for (slot, src) in involved.iter().zip(sorted.iter()) {
            patch.files[*slot] = snapshot[*src].clone();
        }
    }
}

fn emit_canonical(patch: &mut Patch) -> String {
    let shared = shared_rule_names(patch);
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
                    if let Some(rule) = r.header.rule.as_deref() {
                        if shared.contains(rule) {
                            emit_rsmpl_named(&mut out, r, rule);
                            continue;
                        }
                    }
                    rule_seq += 1;
                    emit_rsmpl(&mut out, r, rule_seq);
                }
            }
        }
    }
    out
}

/// True when file section `sec` holds an RSmPL hunk named `rule`.
fn file_contains_rule(sec: &FileSection, rule: &str) -> bool {
    sec.hunks.iter().any(|h| match h {
        Hunk::Rsmpl(r) => r.header.rule.as_deref() == Some(rule),
        _ => false,
    })
}

/// WI-7b definition heuristic from hunk bytes alone (no detector info):
/// an item-header minus line (`fn`/`struct`/…/`pub`/`V`/`Q`/`impl`)
/// is a definition; calls, uses, and macros are use sites.
fn is_definition_hunk(r: &RsmplHunk) -> bool {
    const PREFIXES: &[&str] = &[
        "fn ", "struct ", "enum ", "trait ", "type ", "mod ", "union ", "const ", "static ",
        "impl ", "pub", "V ", "Q ",
    ];
    for line in &r.lines {
        if line.is_removal() {
            let t = line.text.trim_start();
            return PREFIXES.iter().any(|p| t.starts_with(p));
        }
    }
    false
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

/// WI-7b: emit an RSmPL hunk under its preserved shared name.
fn emit_rsmpl_named(out: &mut String, r: &RsmplHunk, rule: &str) {
    let h = &r.header;
    let (start, len) = if r.scope() == Scope::File && h.old_len != 0 {
        (1, 1)
    } else {
        (h.old_start, h.old_len)
    };
    out.push_str(&format!("@@ -{start},{len} @@ rule={rule}\n"));
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
    use super::{canonicalize, canonicalize_with_spans};
    use crate::parser::parse_patch;
    use std::collections::HashMap;

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

    #[test]
    fn singleton_rename_name_still_renumbers() {
        // WI-7b: a lone `rename_*` rule (one hunk) is not a shared
        // intent — it renumbers to positional `r1` as before.
        let text =
            "diff --git src/lib.rs\n@@ -1,1 @@ rule=rename_foo\n# lang=rust\n- fn foo\n+ fn bar\n";
        let got = canon(text);
        assert!(got.contains("rule=r1\n"), "got:\n{got}");
        assert!(!got.contains("rename_foo"), "got:\n{got}");
        assert_eq!(got, canon(&got));
    }

    #[test]
    fn shared_rename_name_preserved_and_definition_first() {
        // WI-7b canonical decision: a `rename_*` rule in ≥2 hunks keeps
        // its name, and files order definition-first (z.rs def after
        // a.rs use in input → z.rs leads), fixed-point stable.
        let text = "diff --git src/a.rs\n@@ -1,1 @@ rule=rename_Old_to_New\n# allow=many\n- Old(E0)\n+ New(E0)\ndiff --git src/z.rs\n@@ -1,1 @@ rule=rename_Old_to_New\n# lang=rust\n- struct Old\n+ struct New\n";
        let got = canon(text);
        assert_eq!(
            got.matches("rule=rename_Old_to_New").count(),
            2,
            "got:\n{got}"
        );
        let z = got.find("diff --git src/z.rs").expect("z.rs present");
        let a = got.find("diff --git src/a.rs").expect("a.rs present");
        assert!(z < a, "definition z.rs must lead:\n{got}");
        assert_eq!(got, canon(&got), "shared-rule form must be fixed-point");
    }

    type SpanMap = HashMap<(String, String), Vec<(u32, u32)>>;
    type SpanEntry<'a> = (&'a str, &'a str, Vec<(u32, u32)>);
    fn spans(pairs: &[SpanEntry<'_>]) -> SpanMap {
        pairs
            .iter()
            .map(|(f, r, v)| ((f.to_string(), r.to_string()), v.clone()))
            .collect()
    }

    #[test]
    fn with_spans_sweep_tightens_to_union() {
        // WI-1a: an authored sweep over two sites becomes the tight
        // ranged form covering their union.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let got =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(2, 2), (5, 7)])]))
                .unwrap();
        assert!(
            got.contains("@@ -2,6 @@ rule=r1\n# allow=many\n"),
            "union 2..7 → len 6:\n{got}"
        );
        assert!(!got.contains("# sweep"), "got:\n{got}");
    }

    #[test]
    fn with_spans_ranged_widens_to_sweep() {
        // The reverse direction: a tight range whose sites actually
        // spread past 64 lines becomes the sweep dummy.
        let text = "diff --git src/lib.rs\n@@ -2,3 @@ rule=s\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let got = canonicalize_with_spans(
            text,
            &spans(&[("src/lib.rs", "s", vec![(2, 2), (100, 100)])]),
        )
        .unwrap();
        assert!(got.contains("@@ -1,1 @@ rule=r1\n# sweep\n"), "got:\n{got}");
    }

    #[test]
    fn with_spans_sixty_four_is_tight_sixty_five_is_sweep() {
        // D.5 boundary: ≤64 lines tight, 65+ sweep.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let tight =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(1, 1), (64, 64)])]))
                .unwrap();
        assert!(tight.contains("@@ -1,64 @@"), "got:\n{tight}");
        let wide =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(1, 1), (65, 65)])]))
                .unwrap();
        assert!(wide.contains("@@ -1,1 @@"), "got:\n{wide}");
        assert!(wide.contains("# sweep"), "got:\n{wide}");
    }

    #[test]
    fn with_spans_unknown_key_keeps_spelling() {
        // No spans for this hunk (optional zero-match, unapplied tree):
        // the authored locality survives verbatim.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let got =
            canonicalize_with_spans(text, &spans(&[("src/other.rs", "s", vec![(2, 2)])])).unwrap();
        assert!(got.contains("@@ -1,1 @@ rule=r1\n# sweep\n"), "got:\n{got}");
    }

    #[test]
    fn with_spans_anchor_and_one_untouched() {
        // Insertion anchors (SPEC 10.5) and allow=one hunks are never
        // re-ranged: their coordinates are anchors, not search scopes.
        let text = "diff --git src/lib.rs\n@@ -7,0 @@ rule=cut\n# scope=file\n# use=H\n+ H\ndiff --git src/other.rs\n@@ -3,9 @@ rule=one\n- foo(E0)\n+ bar(E0)\n";
        let got = canonicalize_with_spans(
            text,
            &spans(&[
                ("src/lib.rs", "cut", vec![(2, 2)]),
                ("src/other.rs", "one", vec![(2, 2), (5, 5)]),
            ]),
        )
        .unwrap();
        assert!(got.contains("@@ -7,0 @@"), "anchor kept:\n{got}");
        assert!(got.contains("@@ -3,9 @@"), "allow=one kept:\n{got}");
    }

    #[test]
    fn with_spans_non_file_scope_untouched() {
        // `anchor`/`overlap`/`exact` scopes are load-bearing for
        // matching — only contained/file hunks normalize.
        let text = "diff --git src/lib.rs\n@@ -2,3 @@ rule=s\n# allow=many\n# scope=anchor\n- foo(E0)\n+ bar(E0)\n";
        let got = canonicalize_with_spans(
            text,
            &spans(&[("src/lib.rs", "s", vec![(10, 10), (12, 12)])]),
        )
        .unwrap();
        assert!(got.contains("@@ -2,3 @@"), "got:\n{got}");
        assert!(got.contains("# scope=anchor"), "got:\n{got}");
    }

    #[test]
    fn with_spans_is_idempotent_under_pure_form() {
        // Tree-aware output re-parses and is a pure-canonical fixed
        // point (the normalized range is itself canonical).
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let once =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(2, 2), (5, 5)])]))
                .unwrap();
        parse_patch(&once).expect("tree-aware output must re-parse");
        assert_eq!(once, canon(&once), "must be pure-canonical fixed point");
    }

    #[test]
    fn wi8_sweep_single_collapses_to_directive_free_tight() {
        // WI-8: an authored sweep over one site becomes the diet single
        // (directive-free tight actual range) — the form the precision
        // singles emit.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let got =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(2, 2)])])).unwrap();
        assert!(
            got.contains("@@ -2,1 @@ rule=r1\n- foo(E0)\n+ bar(E0)\n"),
            "single sweep → directive-free tight:\n{got}"
        );
        assert!(!got.contains("# allow=many"), "got:\n{got}");
        assert!(!got.contains("# sweep"), "got:\n{got}");
        assert_eq!(got, canon(&got), "must be pure-canonical fixed point");
    }

    #[test]
    fn wi8_allow_many_single_collapses_to_directive_free_tight() {
        // WI-8: a ranged `allow == many` hunk over one site likewise
        // collapses (both authored directions land on the same bytes).
        let text = "diff --git src/lib.rs\n@@ -2,3 @@ rule=s\n# allow=many\n- foo(E0)\n+ bar(E0)\n";
        let got =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(2, 2)])])).unwrap();
        assert!(
            got.contains("@@ -2,1 @@ rule=r1\n- foo(E0)\n+ bar(E0)\n"),
            "got:\n{got}"
        );
        assert!(!got.contains("# allow=many"), "got:\n{got}");
    }

    #[test]
    fn wi8_stale_allow_one_reranges_to_actual() {
        // WI-8: a stale-cited single re-ranges to its proven span (the
        // 51 pattern with an identical body).
        let stale = "diff --git src/lib.rs\n@@ -1,3 @@ rule=s\n- fn foo\n+ fn bar\n";
        let actual = "diff --git src/lib.rs\n@@ -6,3 @@ rule=s\n# allow=many\n- fn foo\n+ fn bar\n";
        let a =
            canonicalize_with_spans(stale, &spans(&[("src/lib.rs", "s", vec![(6, 8)])])).unwrap();
        let b =
            canonicalize_with_spans(actual, &spans(&[("src/lib.rs", "s", vec![(6, 8)])])).unwrap();
        assert_eq!(a, b, "stale:\n{a}\nactual:\n{b}");
        assert!(
            a.contains("@@ -6,3 @@ rule=r1\n"),
            "actual range wins:\n{a}"
        );
        assert!(!a.contains("# allow=many"), "single → directive-free:\n{a}");
    }

    #[test]
    fn wi8_different_site_sets_never_unify() {
        // Negative pin: same body, disjoint proven sites → different
        // ranges, never one form.
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let a =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(2, 2)])])).unwrap();
        let b =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(9, 9)])])).unwrap();
        assert_ne!(a, b, "site sets differ:\n{a}\n{b}");
        assert!(a.contains("@@ -2,1 @@"), "got:\n{a}");
        assert!(b.contains("@@ -9,1 @@"), "got:\n{b}");
    }

    #[test]
    fn wi9_follow_allow_one_multi_tightens_like_many() {
        // WI-9: an authored ranged follow under the default
        // `allow == one` (the 46 fixture spelling) and an inferred
        // sweep follow over the same def + follow sites share the
        // tight actual + `# allow=many` form (comparison-only; the
        // engine proved the spans on both legs).
        let ranged =
            "diff --git src/lib.rs\n@@ -1,3 @@ rule=s\n# follow=uses\n- fn foo\n+ fn bar\n";
        let sweep = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n# follow=uses\n- fn foo\n+ fn bar\n";
        let sites = spans(&[("src/lib.rs", "s", vec![(1, 1), (5, 5), (6, 6)])]);
        let a = canonicalize_with_spans(ranged, &sites).unwrap();
        let b = canonicalize_with_spans(sweep, &sites).unwrap();
        assert_eq!(a, b, "ranged:\n{a}\nsweep:\n{b}");
        assert!(
            a.contains("@@ -1,6 @@ rule=r1\n# allow=many\n# follow=uses\n"),
            "tight actual + allow=many, follow kept:\n{a}"
        );
        assert_eq!(a, canon(&a), "must be pure-canonical fixed point");
    }

    #[test]
    fn wi9_follow_wide_spread_becomes_sweep_dummy() {
        // WI-9: the same exemption past 64 lines lands on the shared
        // sweep dummy instead of the tight range.
        let ranged =
            "diff --git src/lib.rs\n@@ -1,3 @@ rule=s\n# follow=uses\n- fn foo\n+ fn bar\n";
        let got = canonicalize_with_spans(
            ranged,
            &spans(&[("src/lib.rs", "s", vec![(1, 1), (100, 100)])]),
        )
        .unwrap();
        assert!(
            got.contains("@@ -1,1 @@ rule=r1\n# sweep\n# follow=uses\n"),
            "got:\n{got}"
        );
    }

    #[test]
    fn wi8_multi_site_single_span_each_side_stays_many() {
        // Multi-site `allow == many` keeps today's tight form (no
        // directive collapse for N ≥ 2).
        let text = "diff --git src/lib.rs\n@@ -1,1 @@ rule=s\n# sweep\n- foo(E0)\n+ bar(E0)\n";
        let got =
            canonicalize_with_spans(text, &spans(&[("src/lib.rs", "s", vec![(2, 2), (5, 5)])]))
                .unwrap();
        assert!(
            got.contains("@@ -2,4 @@ rule=r1\n# allow=many\n"),
            "got:\n{got}"
        );
    }
}
