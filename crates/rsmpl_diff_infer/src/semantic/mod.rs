//! v1 semantic inference: fn/item rename + slot-sweep + call-sweep +
//! array-head-peel call-migration sweep (T441) + method-to-function,
//! method-to-method, &-receiver strip, use-path prefix,
//! and whole-item move (T430) fan-outs. Every emission clears a token-win guard (T375): the
//! semantic hunk is kept only when byte-smaller than the ordinary
//! hunk(s) it replaces (bytes proxy tokens), else the ordinary text
//! stays — small singles and tiny sweeps never cost more than the
//! passthrough.
//!
//! Pure detectors over a binary-stripped diff. Whole-file first (cross-hunk
//! sweeps); else per balanced hunk (T349), leftovers ordinary in order
//! (pairwise-disjoint, SPEC 10.6 option-1). Every per-hunk emission uses
//! `# scope=file` + `@@ -1,1 @@`; probe-verified via `applyrs --check`.
//! Whole-item moves (T430 pre-pass) cite real ranges/anchors instead — a
//! scope=file cut would match every item in the file.
mod methods;
mod moves;
mod peel;

pub(crate) use methods::{
    try_method_same_literal_sweep, try_method_to_fn_sweep, try_method_to_method_sweep,
    try_ref_method_strip_sweep,
};
pub(crate) use peel::{try_call_peel_section, try_call_peel_sweep};

/// Try to compress binary-stripped `sections` (one full
/// `diff --git …` section each) into semantic hunks. Returns the
/// compressed patch, or `None` when no section compressed (caller
/// keeps the ordinary passthrough).
pub fn infer_semantic(sections: &[String]) -> Option<String> {
    infer_semantic_with_pre_images(sections, &std::collections::HashMap::new())
}

/// `infer_semantic` with pre-image file contents (T403 totality):
/// `pre` maps collapsed b-side paths to the old file text (assembled
/// by the caller, which owns the repo). Legacy `infer_semantic` passes
/// an empty map (section-visible evidence only).
pub fn infer_semantic_with_pre_images(
    sections: &[String],
    pre: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let mut out = String::new();
    let mut used_rules: Vec<String> = Vec::new();
    let mut compressed_any = false;
    // T430 whole-patch move pre-pass (cross-section bind/use pairs);
    // per-section detectors run on the rewritten sections below.
    let sections_owned: Vec<String>;
    let sections: &[String] = match moves::try_move_sweep(sections, pre, &mut used_rules) {
        Some(rewritten) => {
            compressed_any = true;
            sections_owned = rewritten;
            &sections_owned
        }
        None => sections,
    };
    for section in sections {
        match compress_section(section, &mut used_rules, pre) {
            Some(semantic) => {
                compressed_any = true;
                out.push_str(&semantic);
            }
            None => out.push_str(section),
        }
    }
    if compressed_any {
        Some(out)
    } else {
        None
    }
}

fn compress_section(
    section: &str,
    used_rules: &mut Vec<String>,
    pre: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let header = section.lines().next()?;
    // T385: CRLF content stays ordinary passthrough — detectors strip
    // `\r` via `str::lines` and would emit hunks that cannot match
    // the worktree file.
    if !header.starts_with("diff --git ") || section.contains('\r') {
        return None;
    }
    // T377: rename sections stay ordinary passthrough. Detectors key
    // emissions to the b-side path, which would seed from the wrong
    // file; rename passthrough is already near-minimal.
    if is_rename_header(header) {
        return None;
    }
    let path = super::b_side_path(header.strip_prefix("diff --git ")?)?;
    if !path.ends_with(".rs") {
        return None;
    }
    // T403 totality input: the pre-image text for this path, if the
    // caller supplied it (None = section-visible evidence only).
    let file_old = pre.get(&path).map(String::as_str);
    // Group changed lines per `@@` hunk: unified diff lists each
    // hunk's `-` block before its `+` block, so global order pairing
    // is only sound when every hunk is a pure modification.
    let mut minuses: Vec<&str> = Vec::new();
    let mut pluses: Vec<&str> = Vec::new();
    let mut hunk_minus = 0usize;
    let mut hunk_plus = 0usize;
    let mut balanced = true;
    let mut in_hunk = false;
    let mut saw_hunk = false;
    for line in section.lines().skip(1) {
        if line.starts_with("@@") {
            if in_hunk && hunk_minus != hunk_plus {
                balanced = false;
            }
            in_hunk = true;
            saw_hunk = true;
            hunk_minus = 0;
            hunk_plus = 0;
            continue;
        }
        if !in_hunk {
            balanced &= !(line.starts_with("old mode ") || line.starts_with("new mode ")); // T401
            continue;
        }
        if line.starts_with('-') && !line.starts_with("---") {
            minuses.push(&line[1..]);
            hunk_minus += 1;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            pluses.push(&line[1..]);
            hunk_plus += 1;
        }
    }
    if in_hunk && hunk_minus != hunk_plus {
        balanced = false;
    }
    // Whole-section attempt first (preserves cross-hunk sweeps): it
    // needs every hunk balanced and equal non-zero global counts, so
    // global order pairing stays sound. Gate failures skip straight
    // to the per-hunk fallback (T349).
    let whole = balanced && saw_hunk && minuses.len() == pluses.len() && !minuses.is_empty();
    // T375: every whole-section candidate clears the token-win guard
    // (byte-smaller than the ordinary section, same prelude either
    // way); a losing shape truncates its reserved rule name and falls
    // through to the per-hunk retry so one losing shape never vetoes
    // winning runs in other hunks.
    let checkpoint = used_rules.len();
    // T411: old-side changed lines for the line-aware opaque veto.
    let changed = crate::totality::changed_old_lines(section);
    if whole && minuses.len() == 1 {
        // Single pair: fn/item rename only (a lone call rewrite has
        // no token win over the ordinary hunk; a lone heterogeneous
        // fn header likewise costs more as a slot hunk than ordinary).
        if let Some(rename) = try_item_rename(minuses[0], pluses[0], &path, used_rules) {
            if rename.len() < section.len() {
                return Some(rename);
            }
            used_rules.truncate(checkpoint);
        }
        return compress_section_per_hunk(section, &path, file_old, used_rules);
    }
    if !whole {
        // T441: hetero-arg peel cross-hunk sweep before verbatim
        // (claims hetero classes; pure-identical classes stay for the
        // tighter verbatim sweep, which ignores the single-pair peel
        // bodies — pinned by `peel_body_verbatim_still_fires`). A peel
        // rewrite already compresses, so it is kept even when verbatim
        // and per-hunk find nothing further (returning `None` here
        // would discard the rewrite).
        if let Some(peeled) = try_call_peel_section(section, &path, used_rules, file_old) {
            if let Some(verbatim) = super::try_verbatim_sweep(&peeled, &path, used_rules, file_old)
            {
                return Some(verbatim);
            }
            return Some(peeled);
        }
        // T403: non-whole sections try the cross-hunk verbatim sweep
        // before per-hunk (whole-section detectors need balance).
        if let Some(verbatim) = super::try_verbatim_sweep(section, &path, used_rules, file_old) {
            return Some(verbatim);
        }
        return compress_section_per_hunk(section, &path, file_old, used_rules);
    }
    // N >= 2 heterogeneous fn/method renames compress to one slot
    // sweep (T344); fall through to the call sweep otherwise.
    if let Some(slot) = try_fn_slot_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if slot.len() < section.len() {
            return Some(slot);
        }
        used_rules.truncate(checkpoint);
    }
    // N >= 2 method-to-function fan-outs (T345), method-to-method
    // renames (T347), `&`-receiver strips (T359), and use-path prefix
    // renames (T363); fall through to the bare-call sweep otherwise.
    if let Some(m2f) =
        try_method_to_fn_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if m2f.len() < section.len() {
            return Some(m2f);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(m2m) =
        try_method_to_method_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if m2m.len() < section.len() {
            return Some(m2m);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(same) =
        try_method_same_literal_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if same.len() < section.len() {
            return Some(same);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(strip) =
        try_ref_method_strip_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if strip.len() < section.len() {
            return Some(strip);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(uses) =
        try_use_prefix_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if uses.len() < section.len() {
            return Some(uses);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(sweep) = try_call_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules) {
        if sweep.len() < section.len() {
            return Some(sweep);
        }
        used_rules.truncate(checkpoint);
    }
    // T441: hetero-arg array-head-peel call migration
    // (`f(D, &[H, M, T…])` → `g(D, M, &[T…])` per-tail-arity groups);
    // all-identical classes stay for the tighter verbatim sweep below.
    if let Some(peel) =
        try_call_peel_sweep(&minuses, &pluses, &path, file_old, &changed, used_rules)
    {
        if peel.len() < section.len() {
            return Some(peel);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(verbatim) = super::try_verbatim_sweep(section, &path, used_rules, file_old) {
        return Some(verbatim);
    }
    // T403: verbatim sweep after the whole-section detectors fail.
    // Per-hunk fallback (T349): pairwise-disjoint hunks merge by
    // construction (SPEC 10.6 option-1).
    compress_section_per_hunk(section, &path, file_old, used_rules)
}

/// Per-hunk fallback for `compress_section`: compress each balanced
/// native hunk independently with the same detectors, keep the rest as
/// ordinary hunks in original order. Returns `None` when no hunk
/// compressed (caller keeps the ordinary passthrough).
fn compress_section_per_hunk(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    // Split the prelude (`diff --git`/`index`/`---`/`+++`) from the
    // native hunks; a section without hunks has nothing to compress.
    let mut prelude = String::new();
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        match current.as_mut() {
            Some(h) => {
                h.push_str(line);
                h.push('\n');
            }
            None => {
                prelude.push_str(line);
                prelude.push('\n');
            }
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.is_empty() {
        return None;
    }
    let mut out = prelude;
    let mut compressed_any = false;
    for hunk in &hunks {
        let mut minuses: Vec<&str> = Vec::new();
        let mut pluses: Vec<&str> = Vec::new();
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(&line[1..]);
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(&line[1..]);
            }
        }
        // One balanced hunk through the shared detector chain
        // (`compress_balanced_pair_body` — same shapes the verbatim
        // sweep retries leftovers with); ordinary hunks pass through.
        let changed = crate::totality::changed_old_lines(hunk);
        match super::compress_balanced_pair_body(
            &minuses,
            &pluses,
            path,
            file_old,
            &changed,
            hunk.len(),
            used_rules,
        ) {
            Some(body) => {
                compressed_any = true;
                out.push_str(&body);
            }
            None => out.push_str(hunk),
        }
    }
    if compressed_any {
        Some(out)
    } else {
        None
    }
}

/// One-ident-delta item rename: exactly one `-`/`+` line pair, same
/// kind/prefix/suffix, idents differ. Column-0 (top-level) is
/// enforced: indented lines are methods or block items a bare
/// pattern cannot match.
pub(crate) fn try_item_rename(
    old_line: &str,
    new_line: &str,
    path: &str,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let old = old_line.trim_end();
    let new = new_line.trim_end();
    let (old_prefix, old_kind, old_ident, old_suffix) = split_item(old)?;
    let (new_prefix, new_kind, new_ident, new_suffix) = split_item(new)?;
    if old_kind != new_kind || old_prefix != new_prefix || old_suffix != new_suffix {
        return None;
    }
    if old_ident == new_ident {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_ident}_to_{new_ident}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# scope=file\n",
        header = super::git_header(path)
    );
    if old_kind == "fn" {
        // Plain top-level fns only: the canonical `fn Old(...)`
        // pattern matches nothing else (SPEC 14.1), and generics /
        // return types / where-clauses need explicit slots v1 does
        // not emit.
        if !old_prefix.is_empty() {
            return None;
        }
        let suffix = old_suffix.trim_start();
        if !suffix.starts_with('(') || !old.trim_end().ends_with('{') {
            return None;
        }
        if suffix.contains("->") || contains_word(suffix, "where") {
            return None;
        }
        hunk.push_str("# lang=rust\n");
        hunk.push_str(&format!(
            "- fn {old_ident}(...) {{\n+ fn {new_ident}(...) {{\n     ...\n  }}\n"
        ));
    } else {
        hunk.push_str(&format!(
            "- {old_prefix}{old_kind} {old_ident}\n+ {new_prefix}{new_kind} {new_ident}\n"
        ));
    }
    Some(hunk)
}

/// Split `fn foo() {`-style header lines into
/// (vis prefix, kind, ident, suffix). Prefix is `""` or `"pub … "`
/// (with trailing space); suffix is everything after the ident.
fn split_item(line: &str) -> Option<(String, String, String, String)> {
    const KINDS: &[&str] = &[
        "fn", "struct", "enum", "trait", "type", "mod", "union", "const", "static",
    ];
    let trimmed = line.trim_start();
    // Column-0 is enforced by the caller; a trimmed start here only
    // tolerates nothing — require the line itself unindented.
    if trimmed.len() != line.len() {
        return None;
    }
    let (prefix, rest) = if let Some(r) = trimmed.strip_prefix("pub") {
        // `pub`, `pub(…)`, each followed by whitespace.
        if r.is_empty() || r.starts_with([' ', '\t']) {
            ("pub ".to_string(), r.trim_start().to_string())
        } else if r.starts_with('(') {
            let end = r.find(')')?;
            let after = &r[end + 1..];
            if after.is_empty() || !after.starts_with([' ', '\t']) {
                return None;
            }
            (
                format!("pub{} ", &r[..=end]),
                after.trim_start().to_string(),
            )
        } else {
            return None;
        }
    } else {
        (String::new(), trimmed.to_string())
    };
    let kind_end = rest.find([' ', '\t'])?;
    let kind = &rest[..kind_end];
    if !KINDS.contains(&kind) {
        return None;
    }
    let after_kind = rest[kind_end..].trim_start();
    let ident_len = ident_len(after_kind)?;
    let ident = &after_kind[..ident_len];
    let suffix = after_kind[ident_len..].to_string();
    if ident.is_empty() || suffix.is_empty() {
        return None;
    }
    Some((prefix, kind.to_string(), ident.to_string(), suffix))
}

/// Length of a plain (non-raw) Rust ident prefix.
fn ident_len(s: &str) -> Option<usize> {
    let mut chars = s.char_indices();
    let (i, c) = chars.next()?;
    if c != '_' && !c.is_ascii_alphabetic() {
        return None;
    }
    let mut len = i + c.len_utf8();
    for (i, c) in chars {
        if c == '_' || c.is_ascii_alphanumeric() {
            len = i + c.len_utf8();
        } else {
            break;
        }
    }
    Some(len)
}

fn contains_word(haystack: &str, word: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == word)
}

/// N-site (N ≥ 2) heterogeneous fn/method rename with a
/// callee-only delta: every pair is a single-line `{`-terminated
/// fn header differing in exactly the fn ident (vis/qual/generics/
/// params/return/where byte-equal per site, heterogeneous across
/// sites). Column-0 sites emit the free-fn slot sweep (T329 shape);
/// uniformly indented sites emit the `impl Svc` advisory-context
/// sweep (T337 shape); mixed indent stays passthrough.
pub(crate) fn try_fn_slot_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut old_name: Option<String> = None;
    let mut new_name: Option<String> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n) = fn_rename_pair(m, p)?;
        if old_name.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_name.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        old_name = Some(o);
        new_name = Some(n);
    }
    let (old_name, new_name) = (old_name?, new_name?);
    if old_name == new_name {
        return None;
    }
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let all_col0 = minuses
        .iter()
        .all(|m| !m.is_empty() && !m.starts_with([' ', '\t']));
    if !all_indented && !all_col0 {
        return None;
    }
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    let plus_col0 = pluses
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with([' ', '\t']));
    if all_indented != plus_indented || all_col0 != plus_col0 {
        return None;
    }
    if !crate::totality::slot_holds(file_old, &old_name, minuses, all_indented, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("rename_{old_name}_to_{new_name}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# scope=file\n# allow=many\n# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n",
        header = super::git_header(path)
    );
    if all_indented {
        hunk.push_str(&format!(
            "  impl Svc {{\n      ...\n-     V Q fn {old_name} G (...) W {{\n+     V Q fn {new_name} G (...) W {{\n          ...\n      }}\n  }}\n"
        ));
    } else {
        hunk.push_str(&format!(
            "- V Q fn {old_name} G (...) W {{\n+ V Q fn {new_name} G (...) W {{\n      ...\n  }}\n"
        ));
    }
    Some(hunk)
}

/// A `-`/`+` line pair as a rename-only fn header: both trimmed
/// lines end with `{`, both parse as fn-shaped (`ItemFn` or
/// `ImplItemFn`), and they differ in exactly the fn ident.
fn fn_rename_pair(old_line: &str, new_line: &str) -> Option<(String, String)> {
    let o = old_line.trim();
    let p = new_line.trim();
    if o.is_empty() || p.is_empty() {
        return None;
    }
    if !o.ends_with('{') || !p.ends_with('{') {
        return None;
    }
    if !is_fn_shaped(o) || !is_fn_shaped(p) {
        return None;
    }
    let (o_pre, o_ident, o_suf) = split_fn_ident(o)?;
    let (p_pre, p_ident, p_suf) = split_fn_ident(p)?;
    if o_pre != p_pre || o_suf != p_suf {
        return None;
    }
    if o_ident == p_ident || o_ident.is_empty() || p_ident.is_empty() {
        return None;
    }
    if o_suf.is_empty() {
        return None;
    }
    let tail = o_suf.trim_start();
    if !(tail.starts_with('(') || tail.starts_with('<')) {
        return None;
    }
    Some((o_ident, p_ident))
}

fn is_fn_shaped(trimmed: &str) -> bool {
    let probe = format!("{trimmed} }}");
    syn::parse_str::<syn::ItemFn>(&probe).is_ok()
        || syn::parse_str::<syn::ImplItemFn>(&probe).is_ok()
}

/// Split `... fn Ident<...>(...)` into (prefix-before-ident, ident,
/// suffix-after-ident). The `fn` keyword must be word-bounded.
fn split_fn_ident(text: &str) -> Option<(String, String, String)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if bytes[i..i + 2] == *b"fn"
            && (i == 0 || !is_ident_char(bytes[i - 1] as char))
            && (i + 2 < bytes.len() && bytes[i + 2].is_ascii_whitespace())
        {
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let start = j;
            let len = ident_len(&text[j..])?;
            let end = start + len;
            return Some((
                text[..start].to_string(),
                text[start..end].to_string(),
                text[end..].to_string(),
            ));
        }
        i += 1;
    }
    None
}

fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric()
}

/// Strip one trailing statement/arg terminator (`;`, else `,`) for
/// shape parsing; the emitted hunk matches sub-expressions, so the
/// terminator never reaches the pattern.
pub(crate) fn strip_one_terminator(text: &str) -> Option<&str> {
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        return Some(stripped.trim_end());
    }
    if let Some(stripped) = text.strip_suffix(',') {
        return Some(stripped.trim_end());
    }
    Some(text)
}

/// Strip one leading struct-literal field label (`label:`) for shape
/// parsing (T416): `text: f(&sub)` sweeps on the inner `f(&sub)` — the
/// engine general-expr matcher visits field values as sub-expressions,
/// so a Call pattern rewrites them with the label preserved
/// (probe-verified on c48a6e6). Returns the label and the remainder.
/// `a::b(...)` paths never strip (colon followed by another colon).
pub(crate) fn strip_field_label(text: &str) -> Option<(&str, &str)> {
    let (label, rest) = text.split_once(':')?;
    if rest.starts_with(':') {
        return None;
    }
    let label = label.trim();
    if label.is_empty() || !label.chars().all(is_ident_char) {
        return None;
    }
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    Some((label, rest))
}

/// Whitespace-insensitive text for detection-time containment and
/// replacement checks (the engine re-verifies structurally at apply).
pub(crate) fn flat_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

pub(crate) fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

/// N-site (N ≥ 2) use-path prefix rename fan-out (T363, SPEC 14.9):
/// every pair parses as `use <P_old>::<tail>` → `use <P_new>::<tail>`
/// with per-site textually identical non-empty tails and one shared
/// parent + one shared renamed last segment (the tight "one segment
/// renamed" formulation — the direct analog of the call sweep's
/// callee-only delta). Leading `pub…`/`::` and trailing `;` are
/// normalized away (the engine ignores them, so mixed `use`/`pub use`
/// sites share one hunk, probe-verified); tails may be heterogeneous
/// across sites (plain, grouped, glob, `as` — the engine preserves
/// them). Emits one `# allow=many` + `# scope=file` hunk
/// (`- use P_old` / `+ use P_new`, no metavars). Single-segment
/// shared prefixes (`use crate`), unequal prefix lengths, divergent
/// parents/tails, and lone pairs stay ordinary passthrough.
pub(crate) fn try_use_prefix_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut minus_paths: Vec<Vec<String>> = Vec::new();
    let mut plus_paths: Vec<Vec<String>> = Vec::new();
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        minus_paths.push(parse_use_path(m)?);
        plus_paths.push(parse_use_path(p)?);
    }
    let old_prefix = common_prefix(&minus_paths);
    let new_prefix = common_prefix(&plus_paths);
    // Tight formulation: equal lengths, at least two segments (a lone
    // `use crate` matches too broadly to infer from N lines of
    // evidence), shared parent, renamed last segment.
    if old_prefix.len() != new_prefix.len() || old_prefix.len() < 2 {
        return None;
    }
    let k = old_prefix.len();
    if old_prefix[..k - 1] != new_prefix[..k - 1] {
        return None;
    }
    if old_prefix[k - 1] == new_prefix[k - 1] {
        return None;
    }
    for (m, p) in minus_paths.iter().zip(plus_paths.iter()) {
        if m.len() <= k || p.len() <= k {
            return None;
        }
        if m[k..] != p[k..] {
            return None;
        }
    }
    if !crate::totality::use_holds(file_old, &old_prefix, minuses, changed) {
        return None;
    }
    let rule = unique_rule(
        used_rules,
        &format!("use_{}_to_{}", old_prefix[k - 1], new_prefix[k - 1]),
    );
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n",
        header = super::git_header(path)
    );
    hunk.push_str(&format!(
        "- use {}\n+ use {}\n",
        old_prefix.join("::"),
        new_prefix.join("::")
    ));
    Some(hunk)
}

/// Parse one diff line as a `use` path: strip one trailing `;`,
/// leading visibility, the `use` keyword, and one leading `::`; split
/// the rest on `::`. The emitted pattern is the bare common prefix,
/// which is what the engine requires (SPEC 14.9 rejects braces, globs,
/// and `as` in the pattern — those may only appear in tails, where
/// per-site equality keeps them verbatim).
fn parse_use_path(line: &str) -> Option<Vec<String>> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    // Leading visibility (`pub`, `pub(…)`) is normalized away — the
    // pattern carries none and the engine matches both sides.
    if let Some(rest) = text.strip_prefix("pub") {
        if rest.is_empty() || rest.starts_with([' ', '\t']) {
            text = rest.trim_start();
        } else if rest.starts_with('(') {
            let end = rest.find(')')?;
            let after = &rest[end + 1..];
            if after.is_empty() || !after.starts_with([' ', '\t']) {
                return None;
            }
            text = after.trim_start();
        } else {
            return None;
        }
    }
    let mut text = text.strip_prefix("use")?;
    // `use` must be the keyword, not an ident prefix (`useful::x`).
    if text.is_empty() || !text.starts_with([' ', '\t']) {
        return None;
    }
    text = text.trim_start();
    // Tolerate one leading `::` (`use ::crate::old::Thing`).
    if let Some(rest) = text.strip_prefix("::") {
        text = rest.trim_start();
    }
    if text.is_empty() {
        return None;
    }
    let segs: Vec<String> = text.split("::").map(|s| s.trim().to_string()).collect();
    if segs.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some(segs)
}

/// Longest common leading segments across all paths.
fn common_prefix(paths: &[Vec<String>]) -> Vec<String> {
    let mut out = paths.first().cloned().unwrap_or_default();
    for p in paths.iter().skip(1) {
        let n = out.iter().zip(p.iter()).take_while(|(a, b)| a == b).count();
        out.truncate(n);
    }
    out
}

/// N-site (N ≥ 2) bare-call rewrite with a callee-only delta:
/// every pair parses as `Old(args)` → `New(args)` with identical
/// old/new callees, uniform arity, and per-site textually identical
/// args. Emits one `# allow=many` + `# scope=file` hunk.
pub(crate) fn try_call_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut old_cal: Option<String> = None;
    let mut new_cal: Option<String> = None;
    let mut arity: Option<usize> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n, a) = parse_call_pair(m, p)?;
        if old_cal.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_cal.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        old_cal = Some(o);
        new_cal = Some(n);
        arity = Some(a);
    }
    let (old_cal, new_cal, arity) = (old_cal?, new_cal?, arity?);
    if old_cal == new_cal {
        return None;
    }
    if !crate::totality::call_holds(file_old, &old_cal, arity, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_cal}_to_{new_cal}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n",
        header = super::git_header(path)
    );
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        hunk.push_str(&format!("# expr {v}\n"));
        vars.push(v);
    }
    let args = vars.join(", ");
    hunk.push_str(&format!("- {old_cal}({args})\n+ {new_cal}({args})\n"));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as bare calls; return
/// (old callee, new callee, arity) when the delta is callee-only.
fn parse_call_pair(old_line: &str, new_line: &str) -> Option<(String, String, usize)> {
    let old_expr = parse_call_expr(old_line)?;
    let new_expr = parse_call_expr(new_line)?;
    let (o_cal, o_args) = call_parts(&old_expr)?;
    let (n_cal, n_args) = call_parts(&new_expr)?;
    if o_args.len() != n_args.len() {
        return None;
    }
    for (a, b) in o_args.iter().zip(n_args.iter()) {
        use quote::ToTokens;
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    Some((o_cal, n_cal, o_args.len()))
}

fn parse_call_expr(line: &str) -> Option<syn::Expr> {
    let mut text = line.trim();
    if text.is_empty() {
        return None;
    }
    // Call sites end the statement with `;`.
    if let Some(stripped) = text.strip_suffix(';') {
        text = stripped.trim_end();
    }
    syn::parse_str::<syn::Expr>(text).ok()
}

/// Bare single-segment call callee + args; rejects method calls,
/// paths, turbofish, and non-ident callees.
pub(crate) fn call_parts(expr: &syn::Expr) -> Option<(String, Vec<syn::Expr>)> {
    if let syn::Expr::Call(call) = expr {
        if let syn::Expr::Path(path) = call.func.as_ref() {
            if path.qself.is_some() || path.path.leading_colon.is_some() {
                return None;
            }
            if path.path.segments.len() != 1 {
                return None;
            }
            let seg = &path.path.segments[0];
            if !matches!(seg.arguments, syn::PathArguments::None) {
                return None;
            }
            let name = seg.ident.to_string();
            if ident_len(&name)? != name.len() {
                return None;
            }
            return Some((name, call.args.iter().cloned().collect()));
        }
    }
    None
}

pub(crate) fn unique_rule(used: &mut Vec<String>, base: &str) -> String {
    if !used.iter().any(|r| r == base) {
        used.push(base.to_string());
        return base.to_string();
    }
    let mut i = 2;
    loop {
        let candidate = format!("{base}_{i}");
        if !used.iter().any(|r| r == &candidate) {
            used.push(candidate.clone());
            return candidate;
        }
        i += 1;
    }
}

/// True when a `diff --git` header names a rename (differing real
/// sides, T377) via the shared space-aware splitter.
fn is_rename_header(header: &str) -> bool {
    let header = header.lines().next().unwrap_or(header);
    let Some(rest) = header.strip_prefix("diff --git ") else {
        return false;
    };
    let Some(parts) = rsmpl_patch::split_git_paths(rest) else {
        return false;
    };
    if parts.len() != 2 {
        return false;
    }
    let old = parts[0].strip_prefix("a/").unwrap_or(&parts[0]);
    let new = parts[1].strip_prefix("b/").unwrap_or(&parts[1]);
    if old == "/dev/null" || old == "dev/null" || new == "/dev/null" || new == "dev/null" {
        return false;
    }
    old != new
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    #[test]
    fn rename_sections_stay_ordinary_passthrough() {
        // T377: rename sections never compress -- detectors key
        // emissions to the b-side path, which would seed from the
        // wrong file. The content below is the `fn_rename_compresses`
        // shape (a proven winner), yet the rename header vetoes it.
        let raw = "diff --git a/old.rs b/new.rs\nsimilarity index 89%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n";
        assert!(infer_semantic(&[raw.to_string()]).is_none());
        assert!(is_rename_header("diff --git a/old.rs b/new.rs"));
        assert!(!is_rename_header("diff --git a/f.rs b/f.rs"));
        assert!(!is_rename_header(
            "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644"
        ));
    }

    #[test]
    fn space_path_rs_sections_compress() {
        // T380: the old `splitn(2, ' ')` b-side mis-split unquoted
        // space paths (real git never quotes them), so a space-path
        // `.rs` section silently never compressed. Shared splitter
        // recovers the proven-winner shape.
        let raw = "diff --git a/my file.rs b/my file.rs\n--- a/my file.rs\n+++ b/my file.rs\n@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n";
        let pre = pre_map(&[(
            "my file.rs",
            "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n1\n}\n",
        )]);
        let out = infer_semantic(&[raw.to_string()]).expect("must compress");
        validated_live(&out, &pre);
        assert!(out.contains("diff --git my file.rs\n"), "got: {out}");
        assert!(!out.contains("diff --git a/"), "got: {out}");
        assert!(out.contains("- fn foo(...) {"), "got: {out}");
    }

    #[test]
    fn fn_rename_compresses() {
        // Long-signature rename: the `fn …(...)` pattern lines are far
        // shorter than the ordinary lines, so the semantic hunk wins
        // (T375 guard). Short headers stay ordinary (see below).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n+fn bar(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n     1\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn foo(first_argument: VeryLongTypeName, second_argument: AnotherLongTypeName) {\n1\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn foo(...) {"), "got: {out}");
        assert!(out.contains("+ fn bar(...) {"), "got: {out}");
        assert!(out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("-fn foo"), "got: {out}");
    }

    #[test]
    fn short_fn_rename_stays_passthrough() {
        // T375 token-win guard: a short-header single rename costs more
        // as a semantic hunk (rule + scope + lang + body) than the one
        // ordinary hunk it replaces.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo() {\n+fn bar() {\n     1\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn struct_rename_stays_passthrough() {
        // T375: non-fn singles can never win — the pattern lines are
        // verbatim copies of the ordinary lines, so the rule + scope
        // overhead always exceeds the replaced `@@` header.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-struct Old {\n+struct New {\n     x: i32,\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn pub_struct_rename_stays_passthrough() {
        // Same T375 math with a vis prefix: verbatim lines, pure overhead.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-pub struct Old {\n+pub struct New {\n     x: i32,\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn indented_fn_stays_passthrough() {
        // Methods live indented; a bare fn pattern cannot match them.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n impl T {\n-    fn foo(&self) {\n+    fn bar(&self) {\n     }\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn generic_fn_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo<T>(x: T) {\n+fn bar<T>(x: T) {\n     1\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_sweep_three_sites_one_hunk() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n-    foo(3);\n+    bar(1);\n+    bar(2);\n+    bar(3);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(1);\n    foo(2);\n    foo(3);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        // One hunk for all three sites.
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn call_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo();\n-    foo();\n+    bar();\n+    bar();\n }\n",
        );
        let pre = pre_map(&[("f.rs", "fn m() {\n    foo();\n    foo();\n}\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo()\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn call_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    old_function(first, second);\n-    old_function(third, fourth);\n-    old_function(fifth, sixth);\n+    new_function(first, second);\n+    new_function(third, fourth);\n+    new_function(fifth, sixth);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_function(first, second);\n    old_function(third, fourth);\n    old_function(fifth, sixth);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- old_function(E0, E1)\n"), "got: {out}");
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
    }

    #[test]
    fn call_sweep_split_hunks_pair_in_order() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-    old_function(first_argument);\n+    new_function(first_argument);\n@@ -9,2 +9,2 @@\n-    old_function(second_argument);\n+    new_function(second_argument);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    old_function(first_argument);\n    old_function(second_argument);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn single_call_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    foo(1);\n+    bar(1);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn mixed_callees_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    baz(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn changed_args_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(3);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_sweep_j0_hetero_fires() {
        // T441 slice 1: 05de4d1 miniature — hetero `M` (ident,
        // literal, call) and hetero method-chain receivers share one
        // 2-element head-peel class.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n-    let b = run(dir.path(), &[\"--patch\", \"\"]);\n-    let c = run(other.path(), &[\"--patch\", rename_patch()]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n+    let b = run_stdin(dir.path(), \"\", &[]);\n+    let c = run_stdin(other.path(), rename_patch(), &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(dir.path(), &[\"--patch\", patch]);\n    let b = run(dir.path(), &[\"--patch\", \"\"]);\n    let c = run(other.path(), &[\"--patch\", rename_patch()]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("rule=calls_run_to_run_stdin_peel\n"),
            "got: {out}"
        );
        assert!(out.contains("# expr D\n# expr M\n"), "got: {out}");
        assert!(!out.contains("# expr T0"), "got: {out}");
        assert!(out.contains("- run(D, &[\"--patch\", M])\n"), "got: {out}");
        assert!(out.contains("+ run_stdin(D, M, &[])\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
        // Native sites consumed into the pattern.
        assert!(
            !out.contains("run_stdin(dir.path(), patch, &[])"),
            "got: {out}"
        );
    }

    #[test]
    fn call_peel_sweep_j1_hetero_fires() {
        // T441 slice 1: one shared tail position replayed into the new
        // trailing slice; both the peeled value and the tail are hetero.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n-    let b = run(other.path(), &[\"--patch\", \"\", \"--format\"]);\n+    let a = run_stdin(dir.path(), patch, &[\"--check\"]);\n+    let b = run_stdin(other.path(), \"\", &[\"--format\"]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n    let b = run(other.path(), &[\"--patch\", \"\", \"--format\"]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("rule=calls_run_to_run_stdin_peel\n"),
            "got: {out}"
        );
        assert!(
            out.contains("# expr D\n# expr M\n# expr T0\n"),
            "got: {out}"
        );
        assert!(
            out.contains("- run(D, &[\"--patch\", M, T0])\n"),
            "got: {out}"
        );
        assert!(out.contains("+ run_stdin(D, M, &[T0])\n"), "got: {out}");
    }

    #[test]
    fn call_peel_sweep_bare_call_statement_fires() {
        // The statement form does not matter: bare call-statements peel
        // exactly like `let` inits (the engine rewrites the call).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    run(dir.path(), &[\"--patch\", a]);\n-    run(dir.path(), &[\"--patch\", b]);\n+    run_stdin(dir.path(), a, &[]);\n+    run_stdin(dir.path(), b, &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    run(dir.path(), &[\"--patch\", a]);\n    run(dir.path(), &[\"--patch\", b]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("rule=calls_run_to_run_stdin_peel\n"),
            "got: {out}"
        );
    }

    #[test]
    fn call_peel_single_pair_stays_passthrough() {
        // T375: one pair can never win as a sweep (the whole-section
        // single path only tries item rename; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_all_identical_stays_for_verbatim() {
        // An all-identical class compresses tighter without metavars:
        // peel declines, verbatim claims it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n-    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n+    run_stdin(dir.path(), some_long_patch_value, &[]);\n+    run_stdin(dir.path(), some_long_patch_value, &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n    run(dir.path(), &[\"--patch\", some_long_patch_value]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(!out.contains("peel"), "got: {out}");
        assert!(out.contains("rule=verbatim\n"), "got: {out}");
    }

    #[test]
    fn call_peel_tail_drift_stays_passthrough() {
        // Different tail lengths are different classes: no class reaches
        // N=2, and singletons stay ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", x]);\n-    let b = run(dir.path(), &[\"--patch\", y, \"--check\"]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[\"--check\"]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_head_drift_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", x]);\n-    let b = run(dir.path(), &[\"--check\", y]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_non_string_head_stays_passthrough() {
        // The head replays verbatim: only a string literal qualifies.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[head(), x]);\n-    let b = run(dir.path(), &[head(), y]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_plus_tail_mismatch_stays_passthrough() {
        // Minus carries one tail, plus replays none: not a peel.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(dir.path(), &[\"--patch\", x, \"--check\"]);\n-    let b = run(dir.path(), &[\"--patch\", y, \"--check\"]);\n+    let a = run_stdin(dir.path(), x, &[]);\n+    let b = run_stdin(dir.path(), y, &[]);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn call_peel_totality_overcount_vetoes() {
        // An unchanged same-shape site in the pre-image breaks the
        // count gate: the sweep would silently rewrite it.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let a = run(d, &[\"--patch\", x]);\n-    let b = run(d, &[\"--patch\", y]);\n+    let a = run_stdin(d, x, &[]);\n+    let b = run_stdin(d, y, &[]);\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(d, &[\"--patch\", x]);\n    let b = run(d, &[\"--patch\", y]);\n    let c = run(d, &[\"--patch\", z]);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn call_peel_section_multi_class_fires_twice() {
        // T441 cross-hunk driver: a non-whole section (pure-addition
        // hunk) with two hetero classes emits one hunk per class.
        let raw = section(
            "f.rs",
            "@@ -1,0 +1,1 @@\n+// marker\n@@ -10,2 +11,2 @@\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n-    let b = run(dir.path(), &[\"--patch\", \"\"]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n+    let b = run_stdin(dir.path(), \"\", &[]);\n@@ -20,2 +22,2 @@\n-    let c = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n-    let d = run(dir.path(), &[\"--patch\", other, \"--check\"]);\n+    let c = run_stdin(dir.path(), patch, &[\"--check\"]);\n+    let d = run_stdin(dir.path(), other, &[\"--check\"]);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(dir.path(), &[\"--patch\", patch]);\n    let b = run(dir.path(), &[\"--patch\", \"\"]);\n    let c = run(dir.path(), &[\"--patch\", patch, \"--check\"]);\n    let d = run(dir.path(), &[\"--patch\", other, \"--check\"]);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- run(D, &[\"--patch\", M])\n"), "got: {out}");
        assert!(
            out.contains("- run(D, &[\"--patch\", M, T0])\n"),
            "got: {out}"
        );
        assert!(out.contains("+ run_stdin(D, M, &[])\n"), "got: {out}");
        assert!(out.contains("+ run_stdin(D, M, &[T0])\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 2, "got: {out}");
        // The pure addition stays ordinary in place.
        assert!(out.contains("+// marker"), "got: {out}");
    }

    #[test]
    fn call_peel_body_verbatim_still_fires() {
        // Composition pin: peel bodies are `@@ -1,1 @@` single-pattern
        // hunks, so the later verbatim pass groups them at n=1 (skipped)
        // and still compresses the remaining identical group.
        let raw = section(
            "f.rs",
            "@@ -1,0 +1,1 @@\n+// marker\n@@ -10,2 +11,2 @@\n-    let a = run(dir.path(), &[\"--patch\", patch]);\n-    let b = run(dir.path(), &[\"--patch\", \"\"]);\n+    let a = run_stdin(dir.path(), patch, &[]);\n+    let b = run_stdin(dir.path(), \"\", &[]);\n@@ -20,2 +22,2 @@\n-    compute(some_long_state_value);\n-    compute(some_long_state_value);\n+    compute_fast(some_long_state_value);\n+    compute_fast(some_long_state_value);\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let a = run(dir.path(), &[\"--patch\", patch]);\n    let b = run(dir.path(), &[\"--patch\", \"\"]);\n    compute(some_long_state_value);\n    compute(some_long_state_value);\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("calls_run_to_run_stdin_peel\n"), "got: {out}");
        assert!(out.contains("rule=verbatim\n"), "got: {out}");
        assert!(
            out.contains("- compute(some_long_state_value)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn method_calls_divergent_new_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar(1);\n+    b.baz(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn non_rust_file_stays_passthrough() {
        let raw = section("notes.txt", "@@ -1 +1 @@\n-foo\n+bar\n");
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn deletion_only_stays_passthrough() {
        let raw = section("f.rs", "@@ -1,3 +1,2 @@\n fn m() {\n-    foo(1);\n }\n");
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn new_file_section_stays_passthrough() {
        let raw = section(
            "f.rs",
            "new file mode 100644\n--- /dev/null\n+++ b/f.rs\n@@ -0,0 +1,1 @@\n+fn n() {}\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn free_slot_sweep_heterogeneous_compresses() {
        // Heterogeneous vis/qual across sites, rename-only per site.
        // Long headers: the slot pattern beats the ordinary lines
        // (T375 guard); `foo`/`bar`-scale headers stay ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n-async fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n+fn new_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n+async fn new_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n    1\n}\nasync fn old_function_name(first_argument: FirstLongType, second_argument: SecondLongType) {\n    2\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("# visibility V\n"), "got: {out}");
        assert!(out.contains("# qualifier_list Q\n"), "got: {out}");
        assert!(
            out.contains("- V Q fn old_function_name G (...) W {\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ V Q fn new_function_name G (...) W {\n"),
            "got: {out}"
        );
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
        assert!(
            !out.contains("impl Svc"),
            "free sweep needs no impl wrapper, got: {out}"
        );
    }

    #[test]
    fn short_slot_sweep_stays_passthrough() {
        // T375: two short headers cost more as a slot sweep (four slot
        // directives + wrapper) than as two ordinary hunks.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn foo(x: i32) -> i32 {\n-async fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n+async fn bar(x: i32) -> i32 {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_slot_sweep_compresses_with_impl_wrapper() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n impl T {\n-    fn old_method_name(&self, first_argument: FirstLongType) {\n-    pub fn old_method_name(&self, first_argument: FirstLongType) {\n+    fn new_method_name(&self, first_argument: FirstLongType) {\n+    pub fn new_method_name(&self, first_argument: FirstLongType) {\n     }\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "impl T {\n    fn old_method_name(&self, first_argument: FirstLongType) {\n        1\n    }\n    pub fn old_method_name(&self, first_argument: FirstLongType) {\n        2\n    }\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("impl Svc {\n"), "got: {out}");
        assert!(
            out.contains("-     V Q fn old_method_name G (...) W {\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+     V Q fn new_method_name G (...) W {\n"),
            "got: {out}"
        );
    }

    #[test]
    fn mixed_indent_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) {\n-    fn foo(&self) {\n+fn bar(x: i32) {\n+    fn bar(&self) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn divergent_renames_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) {\n-fn foo(y: i32) {\n+fn bar(x: i32) {\n+fn baz(y: i32) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn param_edit_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo(x: i32) {\n-fn foo(y: i32) {\n+fn bar(x: u32) {\n+fn bar(y: i32) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn generic_single_stays_passthrough() {
        // N >= 2 only: a lone heterogeneous header costs more as a
        // slot hunk than as an ordinary hunk.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo<T>(x: T) {\n+fn bar<T>(x: T) {\n     1\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn rule_names_unique_across_files() {
        let a = section(
            "a.rs",
            "@@ -1,4 +1,4 @@\n-    old_function(argument1);\n-    old_function(argument2);\n-    old_function(argument3);\n+    new_function(argument1);\n+    new_function(argument2);\n+    new_function(argument3);\n",
        );
        let b = section(
            "b.rs",
            "@@ -1,4 +1,4 @@\n-    old_function(argument1);\n-    old_function(argument2);\n-    old_function(argument3);\n+    new_function(argument1);\n+    new_function(argument2);\n+    new_function(argument3);\n",
        );
        let pre = pre_map(&[
            ("a.rs", "fn m() {\n    old_function(argument1);\n    old_function(argument2);\n    old_function(argument3);\n}\n"),
            ("b.rs", "fn m() {\n    old_function(argument1);\n    old_function(argument2);\n    old_function(argument3);\n}\n"),
        ]);
        let out = infer_semantic(&[a, b]).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("rule=calls_old_function_to_new_function\n"),
            "got: {out}"
        );
        assert!(
            out.contains("rule=calls_old_function_to_new_function_2\n"),
            "got: {out}"
        );
    }

    #[test]
    fn per_hunk_sweep_compresses_beside_ordinary_hunk() {
        // Mixed file: hunk 1 is a 3-site call sweep, hunk 2 a body
        // edit. The whole-section detectors stay passthrough; the
        // per-hunk fallback (T349) compresses hunk 1 and keeps hunk 2
        // ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    foo(argument1);\n-    foo(argument2);\n-    foo(argument3);\n+    bar(argument1);\n+    bar(argument2);\n+    bar(argument3);\n }\n@@ -10,3 +10,3 @@\n fn n() {\n-    1\n+    2\n }\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(argument1);\n    foo(argument2);\n    foo(argument3);\n}\n// pad6\n// pad7\n// pad8\n// pad9\nfn n() {\n    1\n}\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ bar(E0)\n"), "got: {out}");
        assert!(out.contains("@@ -10,3 +10,3 @@\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn per_hunk_unbalanced_hunk_stays_ordinary() {
        // A pure-addition hunk beside a balanced sweep: the addition
        // stays ordinary, the sweep still compresses (T349).
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    foo(argument1);\n-    foo(argument2);\n-    foo(argument3);\n+    bar(argument1);\n+    bar(argument2);\n+    bar(argument3);\n }\n@@ -6,0 +7,2 @@\n+fn added() {}\n+// new\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    foo(argument1);\n    foo(argument2);\n    foo(argument3);\n}\n// tail\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- foo(E0)\n"), "got: {out}");
        assert!(out.contains("+fn added() {}\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn per_hunk_mixed_file_without_sweep_stays_passthrough() {
        // No compressible hunk: `None`, so the caller keeps the
        // ordinary passthrough (T349).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    1\n+    2\n }\n@@ -10,3 +10,3 @@\n fn n() {\n-    3\n+    4\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_basic_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::Thing;\n- use crate::old::Other;\n+ use crate::new::Thing;\n+ use crate::new::Other;\n",
        );
        let pre = pre_map(&[("f.rs", "use crate::old::Thing;\nuse crate::old::Other;\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=use_old_to_new"), "got: {out}");
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_heterogeneous_tails_compress() {
        // Plain, grouped, glob, `as`, and `pub use` tails share one
        // hunk — the engine preserves tails verbatim (SPEC 14.9).
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n- use crate::old::Thing;\n- use crate::old::{A, B};\n- use crate::old::*;\n- use crate::old::Widget as W;\n- pub use crate::old::Reexport;\n+ use crate::new::Thing;\n+ use crate::new::{A, B};\n+ use crate::new::*;\n+ use crate::new::Widget as W;\n+ pub use crate::new::Reexport;\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use crate::old::Thing;\nuse crate::old::{A, B};\nuse crate::old::*;\nuse crate::old::Widget as W;\npub use crate::old::Reexport;\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=use_old_to_new"), "got: {out}");
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
        assert!(!out.contains('{'), "pattern must stay bare, got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_pub_normalized() {
        // Leading `pub`/`pub(…)` is normalized away — mixed vis sites
        // share one hunk.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n- use crate::old::A;\n- pub use crate::old::B;\n- pub(crate) use crate::old::C;\n+ use crate::new::A;\n+ pub use crate::new::B;\n+ pub(crate) use crate::new::C;\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "use crate::old::A;\npub use crate::old::B;\npub(crate) use crate::old::C;\n",
        )]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=use_old_to_new"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_leading_colons_tolerated() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use ::crate::old::A;\n- use ::crate::old::B;\n+ use ::crate::new::A;\n+ use ::crate::new::B;\n",
        );
        let pre = pre_map(&[("f.rs", "use ::crate::old::A;\nuse ::crate::old::B;\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_single_site_stays_passthrough() {
        // A lone pair costs more as a semantic hunk than ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n- use crate::old::Thing;\n+ use crate::new::Thing;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_single_segment_stays_passthrough() {
        // `use foo` matches too broadly to infer from N lines of
        // evidence — v1 needs a two-or-more-segment prefix.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use foo::A;\n- use foo::B;\n+ use bar::A;\n+ use bar::B;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_parent_mismatch_stays_passthrough() {
        // Shared last segment under divergent parents is not a tight
        // rename delta.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::A;\n- use crate::old::B;\n+ use other::old::A;\n+ use other::old::B;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_divergent_tail_stays_passthrough() {
        // Prefix-only changes keep per-site tails byte-equal.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::A;\n- use crate::old::B;\n+ use crate::new::A;\n+ use crate::new::C;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_vis_only_stays_passthrough() {
        // No ident delta — equal prefixes with equal tails.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::x::A;\n- use crate::x::B;\n+ pub use crate::x::A;\n+ pub use crate::x::B;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_non_use_stays_passthrough() {
        // `useful::x` is an ident prefix, not the `use` keyword; bare
        // calls never parse as use paths.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::A;\n- useful::x;\n+ use crate::new::A;\n+ useful::y;\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_spaced_separators_normalized() {
        // Spaces around `::` are trimmed per segment; the emitted
        // pattern is the canonical `::`-joined prefix.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate :: old :: A;\n- use crate :: old :: B;\n+ use crate::new::A;\n+ use crate::new::B;\n",
        );
        let pre = pre_map(&[("f.rs", "use crate :: old :: A;\nuse crate :: old :: B;\n")]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
    }

    #[test]
    fn strip_field_label_edge_cases() {
        // T417: T416 shipped the helper with integration tests only;
        // pin the guards directly (the `::` guard is load-bearing).
        assert_eq!(
            strip_field_label("text: normalize(&sub)"),
            Some(("text", "normalize(&sub)"))
        );
        assert_eq!(strip_field_label("a::b()"), None);
        assert_eq!(strip_field_label("::foo: bar"), None);
        assert_eq!(strip_field_label("\"http://x\""), None);
        assert_eq!(strip_field_label("text:"), None);
        assert_eq!(strip_field_label("text : foo()"), Some(("text", "foo()")));
        assert_eq!(strip_field_label("_t1: foo()"), Some(("_t1", "foo()")));
    }
}
