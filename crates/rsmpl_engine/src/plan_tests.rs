use super::*;

fn apply_multi(
    patch_text: &str,
    initial: Vec<(&str, &str)>,
) -> Result<HashMap<String, String>, String> {
    apply_multi_report(patch_text, initial).map(|(files, _)| files)
}

fn apply_multi_report(
    patch_text: &str,
    initial: Vec<(&str, &str)>,
) -> Result<(HashMap<String, String>, crate::ApplyReport), String> {
    let patch = rsmpl_patch::parse_patch(patch_text).map_err(|e| e.to_string())?;
    let mut files: HashMap<String, String> = initial
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let report = apply_with_captures(&mut files, &patch).map_err(|e| e.render())?;
    Ok((files, report))
}

const UPWARD_MOVE: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,0 +2,0 @@ rule=paste_helper_near_top\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -6,4 +6,0 @@ rule=cut_helper_from_bottom\n# lang=rust\n# item I\n# bind=Helper\n- I\n";

#[test]
fn upward_move_with_forward_ref() {
    // Paste hunk (anchor after line 2) precedes the cut hunk in document
    // order; the engine must still resolve `Helper` (SPEC 23.3).
    let src = "fn top() {}\nfn mid() {}\nfn keep() {}\nfn other() {}\nfn tail() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\n";
    // helper spans lines 6-8; cut range 6,4 exceeds (8 lines: 6+4-1=9 > 8). Fix range below.
    let patch = UPWARD_MOVE.replace("@@ -6,4 +6,0 @@", "@@ -6,3 +6,0 @@");
    let files = apply_multi(&patch, vec![("src/lib.rs", src)]).unwrap();
    let out = &files["src/lib.rs"];
    // helper moved to after line 2.
    let helper_pos = out.find("fn helper").unwrap();
    let top_pos = out.find("fn top").unwrap();
    let mid_pos = out.find("fn mid").unwrap();
    let keep_pos = out.find("fn keep").unwrap();
    assert!(
        top_pos < mid_pos && mid_pos < helper_pos && helper_pos < keep_pos,
        "got:\n{out}"
    );
    assert_eq!(out.matches("fn helper").count(), 1);
}

#[test]
fn cross_file_move_with_import_hunk() {
    let patch = "diff --git a/src/old.rs b/src/old.rs\n--- a/src/old.rs\n+++ b/src/old.rs\n@@ -1,1 +1,0 @@ rule=cut_helper\n# lang=rust\n# item I\n# bind=Helper\n- I\n\ndiff --git a/src/new.rs b/src/new.rs\n--- a/src/new.rs\n+++ b/src/new.rs\n@@ -1,0 +1,0 @@ rule=paste_helper\n# lang=rust\n# use=Helper\n+ Helper\n\n@@ -1,1 +1,2 @@\n use crate::common;\n+use crate::old_dependency;\n";
    let files = apply_multi(
        patch,
        vec![
            ("src/old.rs", "fn helper() {}\nfn stays() {}\n"),
            ("src/new.rs", "use crate::common;\n"),
        ],
    )
    .unwrap();
    assert!(
        !files["src/old.rs"].contains("fn helper"),
        "got: {}",
        files["src/old.rs"]
    );
    assert!(
        files["src/old.rs"].contains("fn stays"),
        "got: {}",
        files["src/old.rs"]
    );
    assert!(
        files["src/new.rs"].contains("fn helper"),
        "got: {}",
        files["src/new.rs"]
    );
    assert!(
        files["src/new.rs"].contains("use crate::old_dependency;"),
        "got: {}",
        files["src/new.rs"]
    );
}

#[test]
fn unresolved_use_fails() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 +1,0 @@ rule=paste\n# lang=rust\n# use=Missing\n+ Missing\n";
    let err = apply_multi(patch, vec![("src/lib.rs", "fn a() {}\n")]).unwrap_err();
    assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
    assert!(err.contains("Missing"), "got: {err}");
}

#[test]
fn second_hunk_uses_original_lines() {
    // SPEC 29.11: the second semantic hunk keeps its original line number
    // even though the first hunk inserts a larger block before it.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,1 +3,4 @@ rule=wrap_call\n# lang=rust\n# allow=many\n# expr E\n- old_function(E)\n+ new_function_that_spans(\n+     E,\n+ )\n\n@@ -5,1 +5,1 @@ rule=rename_second_function\n# lang=rust\n- fn second_function(...) {\n+ fn renamed_function(...) {\n      ...\n  }\n";
    let src = "fn top() {}\nfn old_wrapper() {\n    old_function(1);\n}\nfn second_function() {}\n";
    let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
    let out = &files["src/lib.rs"];
    assert!(out.contains("new_function_that_spans("), "got: {out}");
    assert!(out.contains("fn renamed_function()"), "got: {out}");
}

#[test]
fn use_on_non_insertion_hunk_rejected() {
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n# expr E\n# use=Helper\n- foo(E)\n+ bar(E)\n";
    let err = apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap_err();
    assert!(err.contains("RSMPL_CAPTURE_CONFLICT"), "got: {err}");
}

#[test]
fn non_preserve_format_rejected_with_cargo_fmt_pointer() {
    // T17: `# format=rustfmt` is rejected, not deferred — the
    // suggestion must name the permanent answer (`cargo fmt`).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# format=rustfmt\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let err = apply_multi(patch, vec![("src/lib.rs", "fn foo() {}\n")]).unwrap_err();
    assert!(err.contains("RSMPL_PATTERN_CONFLICT"), "got: {err}");
    assert!(err.contains("cargo fmt"), "got: {err}");
}

#[test]
fn scope_file_dummy_range_finds_late_item() {
    // T61: `@@ -1,1 @@` + `# scope=file` renames a fn on lines 5-7
    // with no file-read roundtrip.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let src = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n";
    let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
    let out = &files["src/lib.rs"];
    assert!(out.contains("fn bar()"), "got: {out}");
    assert!(!out.contains("fn foo()"), "got: {out}");
}

#[test]
fn zero_match_unique_elsewhere_hints_scope_file() {
    // T62: tight wrong range (lines 1-1) but the only `foo` is on
    // line 5 — the suggestion must offer the zero-discovery retry.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let src = "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}\nfn foo() {}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("no match found"), "got: {err}");
    assert!(err.contains("# scope=file"), "got: {err}");
    assert!(err.contains("@@ -1,1 @@"), "got: {err}");
}

#[test]
fn zero_match_many_elsewhere_hints_widen() {
    // T62: zero in range, several matches elsewhere — suggest widening
    // plus `# allow=many` instead of the single-match retry.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- foo(E)\n+ bar(E)\n";
    let src =
        "fn a() {}\nfn b() {\n    foo(1);\n}\nfn c() {\n    foo(2);\n}\nfn d() {\n    foo(3);\n}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("multiple matches exist"), "got: {err}");
    assert!(err.contains("# allow=many"), "got: {err}");
}

#[test]
fn zero_match_nowhere_keeps_generic_hint() {
    // T62: pattern matches nothing anywhere — no scope=file retry.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn nosuchfn(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let src = "fn a() {}\nfn b() {}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("read `src/lib.rs`"), "got: {err}");
    assert!(!err.contains("scope=file"), "got: {err}");
}

#[test]
fn scope_file_whole_file_call_rewrite() {
    // T61: `allow=many` + `scope=file` rewrites calls anywhere.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# scope=file\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n";
    let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
    let out = &files["src/lib.rs"];
    assert_eq!(out.matches("bar(").count(), 2, "got: {out}");
    assert!(!out.contains("foo("), "got: {out}");
}

#[test]
fn zero_match_fn_preview_names_nearest() {
    // T67: wrong-name fn rename names the real fn + line in `reason`,
    // so the retry often needs no file read.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn nosuchfn(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let src = "fn a() {}\nfn b() {}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("no match found"), "got: {err}");
    assert!(err.contains("nearest"), "got: {err}");
    assert!(err.contains("line 1"), "got: {err}");
}

#[test]
fn zero_match_block_level_preview_names_local_item() {
    // T141: a typo'd block-level rename previews the same-kind local,
    // not the enclosing fn — the retry needs no file read.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,4 @@ rule=r\n# lang=rust\n- struct Lcoal\n+ struct Fixed\n";
    let src = "fn outer() {\n    struct Local;\n    let _ = Local;\n}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("nearest struct Local at line 2"), "got: {err}");
}

#[test]
fn zero_match_typo_prefers_similar_name_over_nearer() {
    // T142: a typo'd rename previews the most similar same-kind
    // header, not the nearest one — the retry needs no file read.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- struct Betta\n+ struct Gamma\n";
    let src = "struct Alpha;\nstruct Beta;\nfn outer() {}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("nearest struct Beta at line 2"), "got: {err}");
}

#[test]
fn zero_match_similarity_tie_breaks_on_distance() {
    // T142: equal edit distance falls back to line distance — both
    // `Zetta` and `Beta` are 1 edit from `Betta`, so the nearer line
    // (old_start=1) wins.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 @@ rule=r\n# lang=rust\n- struct Betta\n+ struct Gamma\n";
    let src = "struct Zetta;\nstruct Beta;\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("nearest struct Zetta at line 1"), "got: {err}");
}

#[test]
fn zero_match_call_preview_shows_source_line() {
    // T67 fallback tier: a call rewrite with no same-kind header walk
    // still previews the nearest non-empty source line.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
    let src = "fn a() {\n    foo(1);\n}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("nearest source at line"), "got: {err}");
}

#[test]
fn zero_match_impl_preview_names_trait_and_self_ty() {
    // T19b: an impl rename with the wrong name previews the real
    // header with both sides, so the retry needs no file read.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -3,1 @@ rule=r\n# lang=rust\n- impl Nosuch\n+ impl New\n";
    let src = "trait Old {}\nstruct Foo;\nimpl Old for Foo {}\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("nearest impl Old for Foo"), "got: {err}");
}

#[test]
fn report_carries_per_match_spans() {
    // T27: `spans` mirrors `matches` with original-source 1-based
    // inclusive line ranges — agents verify without re-reading.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,6 +1,6 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\n";
    let (files, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
    assert_eq!(report.rules.len(), 1);
    let r = &report.rules[0];
    assert_eq!(r.matches, 2);
    assert_eq!(r.spans, vec![(2, 2), (5, 5)], "spans: {:?}", r.spans);
    assert!(files["src/lib.rs"].contains("bar(1)"));
    assert!(files["src/lib.rs"].contains("bar(2)"));
}

#[test]
fn report_carries_per_site_bindings_in_span_order() {
    // WI-2b/2c: `sites` carries one entry per emitted edit with its own
    // bindings — sites 2..N are no longer evidence-free. V2 has no
    // first-site `bindings` compat (removed with `version: 2`).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,9 +1,9 @@ rule=r\n# allow=many\n# expr E\n- foo(E)\n+ bar(E)\n";
    let src = "fn a() {\n    foo(1);\n}\nfn b() {\n    foo(2);\n}\nfn c() {\n    foo(3);\n}\n";
    let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
    assert_eq!(report.rules.len(), 1);
    let r = &report.rules[0];
    assert_eq!(r.matches, 3);
    assert_eq!(r.spans, vec![(2, 2), (5, 5), (8, 8)]);
    assert_eq!(r.sites.len(), 3, "sites: {:?}", r.sites);
    for (i, site) in r.sites.iter().enumerate() {
        assert_eq!(site.span, r.spans[i]);
        let e = site
            .bindings
            .iter()
            .find(|(k, _)| k == "E")
            .expect("E binding");
        assert_eq!(e.1, format!("{}", i + 1), "site {i}: {:?}", site);
    }
    // Rename shapes bind nothing: sites omitted.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n- fn foo\n+ fn bar\n";
    let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", "fn foo() {}\n")]).unwrap();
    assert!(report.rules[0].sites.is_empty());
    assert!(report.rules[0].follow_skipped_details.is_empty());
}

#[test]
fn report_spans_cover_cut_and_paste() {
    // T27: cut hunks span the removed item lines; paste hunks span the
    // anchor line ([L, L] for zero-width insertions).
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -4,1 +4,1 @@ rule=paste_h\n# lang=rust\n# use=H\n+ H\n\n@@ -6,3 +6,0 @@ rule=cut_h\n# lang=rust\n# item I\n# bind=H\n- I\n";
    let src = "fn top() {}\nfn mid() {}\nfn keep() {}\nfn other() {}\nfn tail() {}\nfn helper(x: i32) -> i32 {\n    x + 1\n}\n";
    let (_, report) = apply_multi_report(patch, vec![("src/lib.rs", src)]).unwrap();
    assert_eq!(report.rules.len(), 2);
    // Document order: paste first, cut second (T45). The paste anchor
    // `-4,1` inserts after line 4 (SPEC 10.5), i.e. at line 5.
    assert_eq!(report.rules[0].rule, "paste_h");
    assert_eq!(report.rules[0].spans, vec![(5, 5)]);
    assert_eq!(report.rules[1].rule, "cut_h");
    assert_eq!(report.rules[1].spans, vec![(6, 8)]);
    for r in &report.rules {
        assert_eq!(r.spans.len(), r.matches, "rule {}", r.rule);
    }
}

#[test]
fn preview_truncates_long_lines() {
    // T67 caps: a 300-char call line previews at most 200 chars +
    // ellipsis (fallback tier keeps the label fixed, isolating the
    // excerpt cap; pathological idents are bound by `cap_preview`).
    let long_arg = "a".repeat(300);
    let src = format!("fn a() {{\n    foo({long_arg});\n}}\n");
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -2,1 @@ rule=r\n# lang=rust\n# expr E\n- nosuchcall(E)\n+ bar(E)\n";
    let err = apply_multi(patch, vec![("src/lib.rs", src.as_str())]).unwrap_err();
    assert!(err.contains("nearest source at line 2"), "got: {err}");
    // WI-3a: the rendered diagnostic now carries a trailing
    // `repair:` line duplicating the excerpt as `candidate` — measure
    // only the reason's first line (the T67 cap is per-excerpt).
    let tail = err.split("nearest ").nth(1).expect("preview");
    let excerpt = tail.lines().next().expect("reason line");
    assert!(excerpt.chars().count() <= 330, "got: {err}");
    assert!(excerpt.contains('…'), "got: {err}");
}

#[test]
fn ordinary_only_markdown_applies_without_rust_parse() {
    // T137: ordinary-only sections are language-agnostic — no Rust
    // parse of the original or the rewritten file.
    let patch = "diff --git a/SPEC.md b/SPEC.md\n--- a/SPEC.md\n+++ b/SPEC.md\n@@ -1,2 +1,2 @@\n # Title\n-old line\n+new line\n";
    let src = "# Title\nold line\n";
    let files = apply_multi(patch, vec![("SPEC.md", src)]).unwrap();
    assert_eq!(files["SPEC.md"], "# Title\nnew line\n");
}

#[test]
fn mixed_rust_and_markdown_sections() {
    // T137: one Rust section with an RSmPL hunk + one markdown
    // ordinary-only section apply together.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n\ndiff --git a/NOTES.md b/NOTES.md\n--- a/NOTES.md\n+++ b/NOTES.md\n@@ -1,1 +1,1 @@\n-old note\n+new note\n";
    let files = apply_multi(
        patch,
        vec![("src/lib.rs", "fn foo() {}\n"), ("NOTES.md", "old note\n")],
    )
    .unwrap();
    assert!(
        files["src/lib.rs"].contains("fn bar()"),
        "got: {}",
        files["src/lib.rs"]
    );
    assert_eq!(files["NOTES.md"], "new note\n");
}

#[test]
fn ordinary_only_invalid_rust_applies() {
    // T137 negative shape: the skip is per-file semantic presence,
    // not extension sniffing — invalid Rust with only ordinary hunks
    // still applies.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n-this is not rust {{{\n+fixed line\n";
    let files = apply_multi(patch, vec![("src/lib.rs", "this is not rust {{{\n")]).unwrap();
    assert_eq!(files["src/lib.rs"], "fixed line\n");
}

#[test]
fn semantic_hunk_on_non_rust_still_parse_conflicts() {
    // T137 negative control: an RSmPL hunk on markdown still fails
    // with RSMPL_PARSE_CONFLICT — the skip covers ordinary-only
    // files, not a blanket removal of the Rust gate.
    let patch = "diff --git a/NOTES.md b/NOTES.md\n--- a/NOTES.md\n+++ b/NOTES.md\n@@ -1,1 @@ rule=r\n# lang=rust\n- fn foo(...) {\n+ fn bar(...) {\n      ...\n  }\n";
    let err = apply_multi(patch, vec![("NOTES.md", "# notes\n")]).unwrap_err();
    assert!(err.contains("RSMPL_PARSE_CONFLICT"), "got: {err}");
    assert!(err.contains("does not parse as Rust"), "got: {err}");
}

#[test]
fn same_anchor_multi_paste_keeps_document_order_with_blanks() {
    // T249: three pastes at one anchor land in document order
    // (first hunk topmost), joined with exactly one blank line.
    // Mirrors the R2 stdin-trio move: pastes [one, two, three] must
    // read [one, two, three], not reversed, with blanks restored.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_one\n# use=One\n+ One\n\n@@ -1,0 @@ rule=paste_two\n# use=Two\n+ Two\n\n@@ -1,0 @@ rule=paste_three\n# use=Three\n+ Three\n\n@@ -2,1 @@ rule=cut_one\n# item I\n# bind=One\n- I\n\n@@ -4,1 @@ rule=cut_two\n# item I\n# bind=Two\n- I\n\n@@ -6,1 @@ rule=cut_three\n# item I\n# bind=Three\n- I\n";
    let src = "fn top() {}\nfn one() {}\nfn mid() {}\nfn two() {}\nfn keep() {}\nfn three() {}\n";
    let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
    let out = &files["src/lib.rs"];
    let p1 = out.find("fn one() {}").expect("one");
    let p2 = out.find("fn two() {}").expect("two");
    let p3 = out.find("fn three() {}").expect("three");
    assert!(p1 < p2 && p2 < p3, "document order, got:\n{out}");
    assert!(
        out.contains("fn one() {}\n\nfn two() {}\n\nfn three() {}"),
        "blank-joined, got:\n{out}"
    );
    assert_eq!(out.matches("fn one() {}").count(), 1);
    assert_eq!(out.matches("fn two() {}").count(), 1);
    assert_eq!(out.matches("fn three() {}").count(), 1);
}

#[test]
fn identity_sweep_fails_via_change_gate() {
    // WI-2c: an identity sweep (`foo -> foo`) changes no bytes —
    // loud, not an empty lowering with a clean report. (Ranged
    // fixed-points stay valid matches; only file sweeps owe the
    // change gate.)
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# expr E\n- foo(E)\n+ foo(E)\n";
    let err = apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap_err();
    assert!(err.contains("RSMPL_MATCH_CONFLICT"), "got: {err}");
    assert!(err.contains("postcondition"), "got: {err}");
}

#[test]
fn widening_sweep_passes_change_gate() {
    // WI-2c: a file sweep whose plus reintroduces the minus
    // (`foo(E)` -> `foo(foo(E))`, like the `24-pat-sweep` widening
    // `E` -> `E.wrapping_add(1)`) still changes bytes, so the change
    // gate passes — minus-zero re-matching would wrongly reject it.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 @@ rule=r\n# sweep\n# expr E\n- foo(E)\n+ foo(foo(E))\n";
    let files = apply_multi(patch, vec![("src/lib.rs", "fn a() {\n    foo(1);\n}\n")]).unwrap();
    assert!(
        files["src/lib.rs"].contains("foo(foo(1))"),
        "got: {}",
        files["src/lib.rs"]
    );
}

#[test]
fn single_paste_gains_no_extra_blanks() {
    // T249: the blank-join fires only for N>1 pastes at one anchor.
    // A lone paste keeps the historical `trim_end + "\n"` bytes.
    let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,0 @@ rule=paste_h\n# lang=rust\n# use=H\n+ H\n\n@@ -3,1 @@ rule=cut_h\n# lang=rust\n# item I\n# bind=H\n- I\n";
    let src = "fn top() {}\nfn mid() {}\nfn helper() {}\n";
    let files = apply_multi(patch, vec![("src/lib.rs", src)]).unwrap();
    let out = &files["src/lib.rs"];
    assert_eq!(out, "fn top() {}\nfn helper() {}\nfn mid() {}\n");
}

// WI-6b1: unique-definition cross-file `# follow=uses`. The `files`
// map is the workspace: the def hunk touches only `a.rs`, uses in
// `b.rs` rewrite under the same rule with a follow-only entry.

const XFOLLOW_PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\n";

#[test]
fn cross_file_follow_rewrites_other_file_uses() {
    let (files, report) = apply_multi_report(
        XFOLLOW_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
            ),
        ],
    )
    .unwrap();
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    assert_eq!(
        files["src/b.rs"],
        "use crate::New;\nfn f(x: New) -> New {\n    New\n}\n"
    );
    // One def entry (matches = definitions) plus one follow-only
    // entry for `b.rs` (matches = use-site count, spans agree).
    assert_eq!(report.rules.len(), 2, "report: {:?}", report.rules);
    let def = &report.rules[0];
    assert_eq!(
        (def.rule.as_str(), def.file.as_str()),
        ("rename", "src/a.rs")
    );
    assert_eq!(def.matches, 1);
    let extra = &report.rules[1];
    assert_eq!(
        (extra.rule.as_str(), extra.file.as_str()),
        ("rename", "src/b.rs")
    );
    assert_eq!(extra.matches, 4);
    assert_eq!(extra.spans.len(), 4);
    assert!(extra.sites.is_empty());
    assert!(extra.follow_skipped_details.is_empty());
}

#[test]
fn cross_file_follow_second_definition_vetoes() {
    let err = apply_multi_report(
        XFOLLOW_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "pub struct Old;\n"),
        ],
    )
    .unwrap_err();
    assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
    assert!(err.contains("shadowed"), "got: {err}");
    assert!(err.contains("src/b.rs"), "got: {err}");
}

#[test]
fn cross_file_follow_second_definition_skips_with_fallback() {
    let patch = XFOLLOW_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let (files, report) = apply_multi_report(
        &patch,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "pub struct Old;\nfn f(x: Old) -> Old {\n    x\n}\n",
            ),
        ],
    )
    .unwrap();
    // Definition still renames; ambiguous uses stay stale and report.
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    assert!(
        files["src/b.rs"].contains("x: Old"),
        "got: {}",
        files["src/b.rs"]
    );
    assert_eq!(report.rules.len(), 1, "report: {:?}", report.rules);
    let details = &report.rules[0].follow_skipped_details;
    assert!(
        details.iter().any(|s| s.file == "src/b.rs"),
        "report: {:?}",
        report.rules
    );
}

#[test]
fn cross_file_follow_unparseable_tripwire() {
    // `b.rs` does not parse and names `old`: without fallback the
    // hunk vetoes (never a silent stale site); with it the def still
    // renames and the file is listed.
    let err = apply_multi_report(
        XFOLLOW_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "fn broken( {{{ old\n"),
        ],
    )
    .unwrap_err();
    assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
    assert!(err.contains("src/b.rs"), "got: {err}");
    let patch = XFOLLOW_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let (files, report) = apply_multi_report(
        &patch,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "fn broken( {{{\n"),
        ],
    )
    .unwrap();
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    assert_eq!(report.rules.len(), 1, "report: {:?}", report.rules);
}

#[test]
fn cross_file_follow_glob_vetoes_then_skips_only_that_file() {
    // WI-6b2-glob: `src/b.rs` pairs `use other::*;` with `Old` uses —
    // unprovable without fallback; with it only `b.rs` stays stale.
    let err = apply_multi_report(
        XFOLLOW_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "use other::*;\nfn f(x: Old) -> Old {\n    x\n}\n",
            ),
            ("src/c.rs", "fn g(x: Old) -> Old {\n    x\n}\n"),
        ],
    )
    .unwrap_err();
    assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
    assert!(err.contains("glob"), "got: {err}");
    assert!(err.contains("src/b.rs"), "got: {err}");
    let patch = XFOLLOW_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let (files, report) = apply_multi_report(
        &patch,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "use other::*;\nfn f(x: Old) -> Old {\n    x\n}\n",
            ),
            ("src/c.rs", "fn g(x: Old) -> Old {\n    x\n}\n"),
        ],
    )
    .unwrap();
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    assert!(
        files["src/b.rs"].contains("x: Old"),
        "got: {}",
        files["src/b.rs"]
    );
    assert!(
        files["src/c.rs"].contains("x: New"),
        "got: {}",
        files["src/c.rs"]
    );
    let listed: Vec<&str> = report
        .rules
        .iter()
        .flat_map(|r| r.follow_skipped_details.iter().map(|s| s.file.as_str()))
        .collect();
    assert!(listed.contains(&"src/b.rs"), "report: {:?}", report.rules);
}

// WI-7c: plain renames report the uses they leave behind.

const XORPHAN_PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n- struct Old\n+ struct New\n";

#[test]
fn plain_rename_reports_cross_file_orphans() {
    let (files, report) = apply_multi_report(
        XORPHAN_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "use crate::Old;\nfn f(x: Old) -> Old {\n    Old\n}\n",
            ),
        ],
    )
    .unwrap();
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    assert!(files["src/b.rs"].contains("Old"), "orphans stay");
    assert_eq!(report.rules.len(), 1);
    let orphans = &report.rules[0].orphans;
    assert_eq!(orphans.len(), 3, "orphans: {orphans:?}");
    assert!(orphans.iter().all(|o| o.file == "src/b.rs"));
    assert!(report.rules[0].follow_skipped_details.is_empty());
}

#[test]
fn plain_rename_without_uses_reports_no_orphans() {
    let (_, report) =
        apply_multi_report(XORPHAN_PATCH, vec![("src/a.rs", "pub struct Old;\n")]).unwrap();
    assert!(report.rules[0].orphans.is_empty());
}

// Pass-0 follow scoping: an ordinary-only `.rs` target is parsed for
// search, so `# follow=uses` rewrites its uses alongside its ordinary
// edit. Before the fix the uses stayed silently stale (no orphans, no
// follow-skips naming them).

const XFOLLOW_ORDINARY_PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\ndiff --git a/src/b.rs b/src/b.rs\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -3,1 +3,1 @@\n-// note\n+// edited\n";

#[test]
fn ordinary_only_target_joins_follow_domain() {
    let (files, report) = apply_multi_report(
        XFOLLOW_ORDINARY_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "use crate::Old;\nfn f(x: Old) -> Old {\n// note\n    x\n}\n",
            ),
        ],
    )
    .unwrap();
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    assert_eq!(
        files["src/b.rs"],
        "use crate::New;\nfn f(x: New) -> New {\n// edited\n    x\n}\n"
    );
    // Def entry plus a follow-only entry for the ordinary-only file.
    assert_eq!(report.rules.len(), 2, "report: {:?}", report.rules);
    let extra = &report.rules[1];
    assert_eq!(
        (extra.rule.as_str(), extra.file.as_str()),
        ("rename", "src/b.rs")
    );
    assert_eq!(extra.matches, 3, "report: {extra:?}");
    assert_eq!(extra.spans.len(), 3);
}

// Ordinary-only target that does not parse: the ordinary hunk must
// address its real lines, and the unsearched `old` mention fails loud
// without `# fallback=syntax` — never silently stale.

const XFOLLOW_UNPARSEABLE_PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\ndiff --git a/src/b.rs b/src/b.rs\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -1,1 +1,1 @@\n-this is not rust {{{\n+fixed line\n";

#[test]
fn follow_unparseable_ordinary_target_fails_loud() {
    let err = apply_multi_report(
        XFOLLOW_UNPARSEABLE_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "this is not rust {{{\nOld\n"),
        ],
    )
    .unwrap_err();
    assert!(err.contains("RSMPL_SYMBOL_CONFLICT"), "got: {err}");
    assert!(err.contains("src/b.rs"), "got: {err}");
}

#[test]
fn follow_unparseable_ordinary_target_skips_with_fallback() {
    let patch =
        XFOLLOW_UNPARSEABLE_PATCH.replace("# follow=uses\n", "# follow=uses\n# fallback=syntax\n");
    let (files, report) = apply_multi_report(
        &patch,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "this is not rust {{{\nOld\n"),
        ],
    )
    .unwrap();
    assert_eq!(files["src/a.rs"], "pub struct New;\n");
    // Ordinary edit lands; the unsearched use stays and is listed.
    assert!(
        files["src/b.rs"].contains("Old"),
        "got: {}",
        files["src/b.rs"]
    );
    assert!(
        files["src/b.rs"].contains("fixed line"),
        "got: {}",
        files["src/b.rs"]
    );
    let listed: Vec<&str> = report
        .rules
        .iter()
        .flat_map(|r| r.follow_skipped_details.iter().map(|s| s.file.as_str()))
        .collect();
    assert!(listed.contains(&"src/b.rs"), "report: {:?}", report.rules);
}

#[test]
fn follow_ordinary_target_bypass_preserved() {
    // The follow-parsed file keeps the T137 plain-text law: an ordinary
    // edit that breaks the parse still applies (no new reparse veto),
    // while the follow uses still rewrite.
    let patch = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,1 @@ rule=rename\n# follow=uses\n- struct Old\n+ struct New\ndiff --git a/src/b.rs b/src/b.rs\n--- a/src/b.rs\n+++ b/src/b.rs\n@@ -4,1 +4,1 @@\n-    x\n+    x {{\n";
    let (files, _) = apply_multi_report(
        patch,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            (
                "src/b.rs",
                "use crate::Old;\nfn f(x: Old) -> Old {\n    // note\n    x\n}\n",
            ),
        ],
    )
    .unwrap();
    assert!(
        files["src/b.rs"].contains("x: New"),
        "got: {}",
        files["src/b.rs"]
    );
    assert!(
        files["src/b.rs"].contains("x {{"),
        "got: {}",
        files["src/b.rs"]
    );
}

#[test]
fn follow_hunk_carries_no_orphans() {
    let (_, report) = apply_multi_report(
        XFOLLOW_PATCH,
        vec![
            ("src/a.rs", "pub struct Old;\n"),
            ("src/b.rs", "fn f(x: Old) -> Old {\n    x\n}\n"),
        ],
    )
    .unwrap();
    for r in &report.rules {
        assert!(r.orphans.is_empty(), "follow rewrites: {r:?}");
    }
}
