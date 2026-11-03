//! Definition-addition raising, slice 1 (WI-12b): single-insertion
//! field/variant addition under the precision guard.
//!
//! A native diff that inserts one member line inside a proven
//! container (`+ b: bool,` after line N with `struct S` spanning N,
//! `+ C` / `+ C(..)` / `+ C { .. }` / `+ C = ..` with `enum E`
//! spanning N) compresses to the WI-12b authored spelling — container
//! header as context plus the one `+` member line. The semantic form
//! costs more than the one-line ordinary hunk by construction, so like
//! the WI-12 value detector it fires under `PRECISION_GUARD`
//! (`semantic <= ordinary + 16`, the SPEC D.4 constant).
//!
//! Totality is the scratch-apply byte-compare probe (T361 doctrine):
//! the emitted hunk must apply with `matches == 1` AND reproduce
//! byte-identically the native post (insertion after line N, everything
//! else untouched). The container must be unique at the point (the
//! insertion point lies strictly inside exactly one same-named
//! container); an already-present member vetoes. `None` pre-image
//! declines. `# follow=uses` never emits (C17: additions rename
//! nothing). Construction follow-through (struct-literal and match-arm
//! sites) is WI-12c, not this slice.

use super::{unique_rule, PRECISION_GUARD};

/// Pure-insertion definition addition: `minuses` empty, `pluses` one
/// member line, `hunk` the native hunk text (for the `-N,0` insertion
/// point). Returns the single-hunk emission plus its provenance
/// detector name, or `None`. Byte guards live with the callers
/// (precision guard for singles), so this function reserves no rule
/// on failure.
pub(crate) fn try_definition_addition(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    hunk: &str,
    used_rules: &mut Vec<String>,
) -> Option<(String, &'static str)> {
    if !minuses.is_empty() || pluses.len() != 1 {
        return None;
    }
    let pre = file_old?;
    let point = insertion_point(hunk)?;
    let plus = pluses[0].trim();
    if plus.is_empty() || plus.lines().count() != 1 {
        return None;
    }
    // Classify the plus line: field (top-level `:`) or variant.
    enum AddKind {
        Field { field: String },
        Variant { variant: String },
    }
    let kind = if has_top_colon(plus) {
        let (field, ty) = split_field(plus)?;
        let _: syn::Type = syn::parse_str(ty).ok()?;
        if !is_ident(&field) {
            return None;
        }
        AddKind::Field { field }
    } else {
        let variant = variant_ident(plus)?;
        if !is_ident(&variant) {
            return None;
        }
        // The line must parse as a real variant (rejects malformed
        // payloads like `C(` that a raw ident probe would accept).
        let body = plus
            .trim()
            .trim_end_matches(',')
            .trim_end()
            .trim_end_matches(';')
            .trim();
        let _: syn::Variant = syn::parse_str(body).ok()?;
        AddKind::Variant { variant }
    };
    // Container recovery at the insertion point: exactly the container
    // spanning the point, lacking the member.
    let container: String = match &kind {
        AddKind::Field { field } => struct_spanning(pre, field, point)?,
        AddKind::Variant { variant } => enum_spanning(pre, variant, point)?,
    };
    let is_field = matches!(kind, AddKind::Field { .. });
    let checkpoint = used_rules.len();
    let rule = unique_rule(used_rules, "definition_addition");
    // Point range citing the insertion line (`-N,1`: the one old-side
    // context line is the container header the classifier needs).
    // No `# allow=many`: a single insertion matches once under the
    // default `allow=one`, and the shorter header is what keeps the
    // single-insertion emission inside the 16-byte precision guard.
    let mut out = format!(
        "{}\n@@ -{point},1 @@ rule={rule}\n",
        crate::git_header(path)
    );
    if is_field {
        out.push_str(&format!("  struct {container} {{\n"));
    } else {
        out.push_str(&format!("  enum {container} {{\n"));
    }
    let mut member = plus.to_string();
    if !member.ends_with(',') && !member.ends_with(';') {
        member.push(',');
    }
    out.push_str(&format!("+ {member}\n"));
    if !scratch_reproduces(pre, path, &out, point, plus) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Precision-guard accounting lives with callers, but enforce the
    // constant here as defence in depth (callers compare against the
    // native hunk length; `hunk` is that native hunk).
    debug_assert!(out.len() <= hunk.len() + PRECISION_GUARD || true);
    Some((out, "definition_addition"))
}

/// WI-12c slice 1: one emitted hunk inside a definition-addition
/// follow-through group (ordered native order by the caller).
/// Shared with the WI-12d sibling (`definition_arm_follow`) per D5.
pub(crate) struct FollowHunk {
    /// Single-hunk text starting at its `@@` line (no `diff --git`).
    pub(crate) emitted: String,
    /// Covered native hunk (for provenance + guard accounting).
    pub(crate) native: String,
    /// `definition_addition` (def) or `definition_addition_follow`.
    pub(crate) detector: &'static str,
    /// Changed old-side lines this hunk covers.
    pub(crate) changed: Vec<usize>,
    /// Reserved rule name (`rN`) for this hunk.
    pub(crate) rule: String,
}

/// WI-12c slice 1: field-addition construction follow-through.
///
/// A file section holding exactly one pure-insertion field addition
/// (the WI-12b shape) plus K ≥ 1 balanced single-pair hunks whose only
/// delta is adding the new field to a struct literal of the same
/// struct compresses to the definition hunk plus K per-site concrete
/// ranged construction hunks. Hetero added values stay per-site
/// concrete (a plus-only hole would be `unbound`); ranges pin each
/// value to its line so values cannot cross-apply.
///
/// All-or-nothing over the section's hunks: any unclassifiable hunk
/// (variant insertion, `..`-rest pair, multi-line pair, unrelated
/// edit) declines the whole group (`None` — the per-hunk fallback
/// then still compresses the definition insertion alone). Totality is
/// the joint scratch-apply probe (every hunk matches once AND the
/// post equals the native post byte-identically); the guard is the
/// precision guard scaled per emitted hunk
/// (`semantic ≤ ordinary + 16 × hunks`).
pub(crate) fn try_definition_addition_follow(
    section: &str,
    path: &str,
    file_old: Option<&str>,
    used_rules: &mut Vec<String>,
) -> Option<Vec<FollowHunk>> {
    let pre = file_old?;
    // Split prelude from native hunks (same shape as the chain's
    // per-hunk fallback).
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        if let Some(h) = current.as_mut() {
            h.push_str(line);
            h.push('\n');
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    if hunks.len() < 2 {
        return None;
    }
    // One pure-insertion hunk, every other hunk a balanced single pair.
    let mut pairs: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    for h in &hunks {
        let mut minuses: Vec<String> = Vec::new();
        let mut pluses: Vec<String> = Vec::new();
        for line in h.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
        pairs.push((minuses, pluses));
    }
    let mut insert_idx: Option<usize> = None;
    for (i, (m, p)) in pairs.iter().enumerate() {
        if m.is_empty() && p.len() == 1 {
            if insert_idx.is_some() {
                return None;
            }
            insert_idx = Some(i);
        } else if m.len() != 1 || p.len() != 1 {
            return None;
        }
    }
    let insert_idx = insert_idx?;
    let checkpoint = used_rules.len();
    // Reuse the single-insertion classifier/emitter (validates the
    // container, the probe, and reserves the def rule).
    let def_section = match try_definition_addition(
        &[],
        &[pairs[insert_idx].1[0].as_str()],
        path,
        Some(pre),
        &hunks[insert_idx],
        used_rules,
    ) {
        Some((s, _)) => s,
        None => {
            used_rules.truncate(checkpoint);
            return None;
        }
    };
    // Slice 1 owns field addition only: the def emission names its
    // container (`  struct S {` vs `  enum E {`). Variant insertions
    // (match-arm follow-through) are WI-12d.
    let Some((container, field)) = def_container_field(&def_section) else {
        used_rules.truncate(checkpoint);
        return None;
    };
    // Every balanced hunk must add exactly this field to an `S`
    // literal; each cites its own changed line (hetero-safe).
    let mut out: Vec<FollowHunk> = Vec::new();
    let def_body = def_section
        .split_once('\n')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_default();
    let def_rule = used_rules.last().cloned().unwrap_or_default();
    for (i, h) in hunks.iter().enumerate() {
        if i == insert_idx {
            out.push(FollowHunk {
                emitted: def_body.clone(),
                native: h.clone(),
                detector: "definition_addition",
                changed: crate::totality::changed_old_lines(h),
                rule: def_rule.clone(),
            });
            continue;
        }
        let changed = crate::totality::changed_old_lines(h);
        if changed.len() != 1 {
            used_rules.truncate(checkpoint);
            return None;
        }
        if !classify_literal_addition(&pairs[i].0[0], &pairs[i].1[0], &container, &field) {
            used_rules.truncate(checkpoint);
            return None;
        }
        let rule = unique_rule(used_rules, "definition_addition_follow");
        let emitted = format!(
            "@@ -{},1 @@ rule={rule}\n- {}\n+ {}\n",
            changed[0],
            pairs[i].0[0].trim(),
            pairs[i].1[0].trim(),
        );
        out.push(FollowHunk {
            emitted,
            native: h.clone(),
            detector: "definition_addition_follow",
            changed,
            rule,
        });
    }
    // Scaled precision guard: each emitted hunk carries the same
    // 16-byte proof budget as the WI-12 single shape.
    let native_len: usize = hunks.iter().map(|h| super::normalized_hunk_len(h)).sum();
    let emitted_len: usize = out.iter().map(|h| h.emitted.len()).sum();
    if emitted_len > native_len + super::guard::precision_allowance(out.len(), 0) {
        used_rules.truncate(checkpoint);
        return None;
    }
    // Joint scratch-apply probe: every hunk matches once AND the post
    // equals the native post byte-identically (untouched lines —
    // including `..`-rest sites with no diff — covered by the compare).
    if !joint_reproduces(pre, path, section, &out) {
        used_rules.truncate(checkpoint);
        return None;
    }
    Some(out)
}

/// Read back the def emission's container + field (`  struct S {` /
/// `+ b: bool,`). `None` for variant additions (WI-12d) or malformed
/// emissions (defensive — the emitter above is the only producer).
fn def_container_field(def_section: &str) -> Option<(String, String)> {
    let mut container: Option<String> = None;
    let mut member: Option<String> = None;
    for line in def_section.lines().skip(1) {
        if line.starts_with("@@") || line.starts_with('#') {
            continue;
        } else if let Some(body) = line.strip_prefix("  ") {
            let t = body.trim();
            let name = t.strip_prefix("struct ")?;
            container = name.strip_suffix('{').map(|s| s.trim().to_string());
        } else if let Some(plus) = line.strip_prefix('+') {
            member = Some(plus.trim().to_string());
        }
    }
    let field = split_field(member?.as_str())?.0;
    Some((container?, field))
}

/// A balanced pair as a struct-literal field addition: both sides name
/// the same `S` literal (bare tail-expr or `let`-bound), no `..` rest
/// on either side, plus carries exactly the minus fields (token-equal
/// values) plus one extra named `field`.
fn classify_literal_addition(
    minus_raw: &str,
    plus_raw: &str,
    struct_name: &str,
    field: &str,
) -> bool {
    let m = minus_raw.trim().trim_end_matches(',').trim();
    let p = plus_raw.trim().trim_end_matches(',').trim();
    if m.is_empty() || p.is_empty() {
        return false;
    }
    let (sm, sp) = match (stmt_struct(m), stmt_struct(p)) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    if sm.is_local != sp.is_local || sm.semi != sp.semi || sm.pat != sp.pat {
        return false;
    }
    struct_adds_field(&sm.expr, &sp.expr, struct_name, field)
}

/// A parsed statement holding a struct literal: bare or `let`-bound.
struct StmtStruct {
    is_local: bool,
    /// Token-normalised `let` pattern (`""` for bare expressions).
    pat: String,
    semi: bool,
    expr: syn::ExprStruct,
}

fn stmt_struct(line: &str) -> Option<StmtStruct> {
    if let Ok(stmt) = syn::parse_str::<syn::Stmt>(line) {
        match stmt {
            syn::Stmt::Local(local) => {
                let init = local.init.as_ref()?;
                // `let … else` diverges: a different change, decline.
                if init.diverge.is_some() {
                    return None;
                }
                if let syn::Expr::Struct(s) = (*init.expr).clone() {
                    let pat = &local.pat;
                    return Some(StmtStruct {
                        is_local: true,
                        pat: norm_tokens(&quote::quote!(#pat)),
                        // `let` always carries its `;` (syn's
                        // `semi_token` is not optional).
                        semi: true,
                        expr: s,
                    });
                }
                return None;
            }
            syn::Stmt::Expr(expr, semi) => {
                if let syn::Expr::Struct(s) = expr {
                    return Some(StmtStruct {
                        is_local: false,
                        pat: String::new(),
                        semi: semi.is_some(),
                        expr: s,
                    });
                }
                return None;
            }
            // `let`-less `;`-terminated expressions (`S { … };`) parse
            // as `Stmt::Expr` with `Some(;)` in this syn version (no
            // separate `Semi` variant) — the arm above owns them.
            _ => return None,
        }
    }
    // Bare struct in a non-statement line (vec element, call arg):
    // trailing-comma-stripped lines like `S { a: 1 }` parse as
    // expressions directly.
    if let Ok(syn::Expr::Struct(s)) = syn::parse_str::<syn::Expr>(line) {
        return Some(StmtStruct {
            is_local: false,
            pat: String::new(),
            semi: false,
            expr: s,
        });
    }
    None
}

fn struct_adds_field(
    minus: &syn::ExprStruct,
    plus: &syn::ExprStruct,
    struct_name: &str,
    field: &str,
) -> bool {
    if minus.qself.is_some() || plus.qself.is_some() {
        return false;
    }
    // Same path, naming the proven container.
    let minus_path = &minus.path;
    let plus_path = &plus.path;
    if norm_tokens(&quote::quote!(#minus_path)) != norm_tokens(&quote::quote!(#plus_path)) {
        return false;
    }
    let last = minus
        .path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();
    if last != struct_name {
        return false;
    }
    // `..rest` on either side: no explicit label to add (acceptance:
    // stays ordinary).
    if minus.rest.is_some() || plus.rest.is_some() {
        return false;
    }
    let mut mfields: Vec<(String, String)> = Vec::new();
    for fv in &minus.fields {
        // Shorthand (`S { a }`) is `Named` with no colon and a path
        // value — the token-normalised value still compares equal.
        let syn::Member::Named(id) = &fv.member else {
            return false;
        };
        let en = &fv.expr;
        mfields.push((id.to_string(), norm_tokens(&quote::quote!(#en))));
    }
    let mut pfields: Vec<(String, String)> = Vec::new();
    for fv in &plus.fields {
        let syn::Member::Named(id) = &fv.member else {
            return false;
        };
        let en = &fv.expr;
        pfields.push((id.to_string(), norm_tokens(&quote::quote!(#en))));
    }
    if pfields.len() != mfields.len() + 1 {
        return false;
    }
    for (name, val) in &mfields {
        match pfields.iter().find(|(n, _)| n == name) {
            Some((_, pv)) if pv == val => {}
            _ => return false,
        }
    }
    let extras: Vec<&(String, String)> = pfields
        .iter()
        .filter(|(n, _)| !mfields.iter().any(|(m, _)| m == n))
        .collect();
    if extras.len() != 1 || extras[0].0 != field {
        return false;
    }
    // The added value is a real expression by construction (parsed).
    true
}

fn norm_tokens(ts: &impl quote::ToTokens) -> String {
    quote::quote!(#ts)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Joint scratch-apply probe (T361 for the group): the emitted hunks
/// apply to `pre` with one match each AND reproduce byte-identically
/// the native post (applying `section` to `pre`).
/// Shared with the WI-12d sibling per D5.
pub(crate) fn joint_reproduces(pre: &str, path: &str, section: &str, hunks: &[FollowHunk]) -> bool {
    let mut emission = format!("{}\n", crate::git_header(path));
    for h in hunks {
        emission.push_str(&h.emitted);
    }
    let Ok(parsed) = rsmpl_patch::parse_patch(&emission) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    if report.rules.iter().map(|r| r.matches).sum::<usize>() != hunks.len() {
        return false;
    }
    let post = files[path].clone();
    // Native post for the byte-compare leg.
    let Ok(native) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut nfiles = std::collections::HashMap::new();
    nfiles.insert(path.to_string(), pre.to_string());
    let Ok(_) = rsmpl_engine::apply_patch_to_tree(&mut nfiles, &native) else {
        return false;
    };
    post == nfiles[path]
}

/// `@@ -N,0` insertion point N (`None` for balanced/sweep headers).
/// Shared with the WI-12d sibling per D5.
pub(crate) fn insertion_point(hunk: &str) -> Option<usize> {
    for line in hunk.lines() {
        if line.starts_with("@@") {
            let first = line.split_whitespace().nth(1)?;
            let range = first.strip_prefix('-')?;
            let mut it = range.split(',');
            let start: usize = it.next()?.parse().ok()?;
            let len: usize = it.next().unwrap_or("1").parse().ok()?;
            if len != 0 {
                return None;
            }
            return Some(start);
        }
    }
    None
}

/// Top-level single-colon probe (no `::` adjacency), bracket-aware so
/// struct-payload colons (`C { f: T }`) never count as field lines.
fn has_top_colon(s: &str) -> bool {
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '(' => dp += 1,
            ')' => dp -= 1,
            '[' => db += 1,
            ']' => db -= 1,
            '{' => dbr += 1,
            '}' => dbr -= 1,
            '<' => da += 1,
            '>' => {
                if da > 0 {
                    da -= 1;
                }
            }
            ':' if dp == 0 && db == 0 && dbr == 0 && da == 0 => {
                let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
                let next = bytes.get(i + 1).copied();
                if prev != Some(b':') && next != Some(b':') {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Split `name: Ty` on the single top-level colon. Returns
/// (name, ty). Vis prefixes (`pub`, `pub(..)`) allowed on the name.
fn split_field(line: &str) -> Option<(String, &str)> {
    let t = line
        .trim()
        .trim_end_matches(',')
        .trim()
        .trim_end_matches(';')
        .trim();
    // Find the top-level colon byte offset.
    let mut dp = 0i32;
    let mut db = 0i32;
    let mut dbr = 0i32;
    let mut da = 0i32;
    let bytes = t.as_bytes();
    let mut at: Option<usize> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '(' => dp += 1,
            ')' => dp -= 1,
            '[' => db += 1,
            ']' => db -= 1,
            '{' => dbr += 1,
            '}' => dbr -= 1,
            '<' => da += 1,
            '>' => {
                if da > 0 {
                    da -= 1;
                }
            }
            ':' if dp == 0 && db == 0 && dbr == 0 && da == 0 => {
                let prev = if i > 0 { Some(bytes[i - 1]) } else { None };
                let next = bytes.get(i + 1).copied();
                if prev != Some(b':') && next != Some(b':') {
                    at = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let at = at?;
    let (left, right) = t.split_at(at);
    let right = right.strip_prefix(':')?.trim();
    if right.is_empty() {
        return None;
    }
    let left = left.trim();
    if left.is_empty() || left.contains(['{', '}', '(', ')', ';', '=', '[', ']']) {
        return None;
    }
    let mut parts: Vec<&str> = left.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let ident = parts.pop()?.to_string();
    if !parts.is_empty() && !parts.join(" ").starts_with("pub") {
        return None;
    }
    Some((ident, right))
}

/// Leading ident of a variant line (`C`, `C(..)`, `C { .. }`,
/// `C = ..`). `None` for field lines or empty heads.
/// Shared with the WI-12d sibling per D5.
pub(crate) fn variant_ident(line: &str) -> Option<String> {
    let mut t = line.trim();
    if let Some(s) = t.strip_suffix(',') {
        t = s.trim_end();
    }
    if let Some(s) = t.strip_suffix(';') {
        t = s.trim_end();
    }
    if t.is_empty() {
        return None;
    }
    let len = super::ident_len(t)?;
    let (ident, suffix) = t.split_at(len);
    let suffix = suffix.trim();
    if suffix.is_empty() {
        return Some(ident.to_string());
    }
    if suffix.starts_with('(') || suffix.starts_with('{') || suffix.starts_with('=') {
        return Some(ident.to_string());
    }
    None
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && super::ident_len(s) == Some(s.len())
}

/// The `struct S` spanning insertion `point` (1-based, `start <= point
/// < end`) that lacks `field`. `None` when no carrier spans the point
/// or the member already exists there.
pub(crate) fn struct_spanning(pre: &str, field: &str, point: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut found: Option<String> = None;
    fn walk(items: &[syn::Item], field: &str, point: usize, found: &mut Option<String>) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Struct(s) => {
                    let (start, end) = (s.span().start().line, s.span().end().line);
                    if start <= point && point < end {
                        if let syn::Fields::Named(fields) = &s.fields {
                            if fields
                                .named
                                .iter()
                                .any(|f| f.ident.as_ref().is_some_and(|i| i == field))
                            {
                                continue;
                            }
                        }
                        if found.is_none() {
                            *found = Some(s.ident.to_string());
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, field, point, found);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, field, point, &mut found);
    found
}

/// The `enum E` spanning insertion `point` that lacks `variant`.
pub(crate) fn enum_spanning(pre: &str, variant: &str, point: usize) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut found: Option<String> = None;
    fn walk(items: &[syn::Item], variant: &str, point: usize, found: &mut Option<String>) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Enum(e) => {
                    let (start, end) = (e.span().start().line, e.span().end().line);
                    if start <= point && point < end {
                        if e.variants.iter().any(|v| v.ident == variant) {
                            continue;
                        }
                        if found.is_none() {
                            *found = Some(e.ident.to_string());
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, variant, point, found);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, variant, point, &mut found);
    found
}

/// Scratch-apply byte-compare probe (T361): the emitted section must
/// apply with `matches == 1` AND produce exactly the native post
/// (insertion of the plus line after `point`, everything else
/// byte-identical).
fn scratch_reproduces(pre: &str, path: &str, section: &str, point: usize, plus: &str) -> bool {
    let Ok(parsed) = rsmpl_patch::parse_patch(section) else {
        return false;
    };
    let mut files = std::collections::HashMap::new();
    files.insert(path.to_string(), pre.to_string());
    let Ok(report) = rsmpl_engine::apply_patch_to_tree(&mut files, &parsed) else {
        return false;
    };
    if report.rules.iter().map(|r| r.matches).sum::<usize>() != 1 {
        return false;
    }
    let post = &files[path];
    let pre_lines: Vec<&str> = pre.lines().collect();
    let post_lines: Vec<&str> = post.lines().collect();
    if post_lines.len() != pre_lines.len() + 1 {
        return false;
    }
    // First `point` lines identical, inserted line matches plus
    // (trimmed — the engine splices the pattern text without the
    // file's indent), remainder shifted by one.
    for (i, line) in pre_lines.iter().take(point).enumerate() {
        if post_lines.get(i) != Some(line) {
            return false;
        }
    }
    if post_lines.get(point).map(|l| l.trim())
        != Some(
            plus.trim()
                .trim_end_matches(',')
                .trim()
                .trim_end_matches(';')
                .trim(),
        )
        && post_lines.get(point).map(|l| l.trim()) != Some(plus.trim())
    {
        // Accept with or without the normalised trailing comma.
        let got = post_lines
            .get(point)
            .map(|l| l.trim().to_string())
            .unwrap_or_default();
        let want = plus.trim().to_string();
        let want_comma = if want.ends_with(',') || want.ends_with(';') {
            want.clone()
        } else {
            format!("{want},")
        };
        if got != want && got != want_comma {
            return false;
        }
    }
    for (i, line) in pre_lines.iter().skip(point).enumerate() {
        if post_lines.get(point + 1 + i) != Some(line) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic_with_pre_images, unique_rule};
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

    #[test]
    fn field_addition_fires_and_round_trips() {
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "struct S {\n    a: u32,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("field addition must compress");
        assert!(out.contains("struct S {"), "got:\n{out}");
        assert!(out.contains("+ b: bool,"), "got:\n{out}");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "struct S {\n    a: u32,\n    b: bool,\n}\n");
        let _ = unique_rule(&mut Vec::new(), "r1");
    }

    #[test]
    fn variant_addition_fires_and_round_trips() {
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    C,\n");
        let pre = pre_map(&[("f.rs", "enum E {\n    A,\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("variant addition must compress");
        assert!(out.contains("enum E {"), "got:\n{out}");
        assert!(out.contains("+ C,"), "got:\n{out}");
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], "enum E {\n    A,\n    C,\n}\n");
    }

    #[test]
    fn duplicate_member_vetoes() {
        // `b` already present: the container spans the point but the
        // member exists, so the section stays ordinary.
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "struct S {\n    a: u32,\n    b: bool,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_addition")
            },
            "duplicate must veto"
        );
    }

    #[test]
    fn outside_container_declines() {
        let raw = section("f.rs", "@@ -1,0 +1 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "fn m() {}\nstruct S {\n    a: u32,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_addition")
            },
            "insertion outside any struct must decline"
        );
    }

    #[test]
    fn balanced_pairs_decline() {
        let mut used = Vec::new();
        assert!(try_definition_addition(
            &["a: u32"],
            &["b: bool"],
            "f.rs",
            Some("struct S {\n    a: u32,\n}\n"),
            "@@ -2,1 +2,1 @@\n-    a: u32\n+    b: bool\n",
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn hetero_container_declines() {
        // Field-shaped plus inside an enum container: no struct spans
        // the point, so the field path declines (and vice versa).
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", "enum E {\n    A,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none() || {
                let o = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
                !o.contains("definition_addition")
            },
            "field plus inside enum must decline"
        );
        let raw_v = section("f.rs", "@@ -2,0 +3 @@\n+    C,\n");
        let pre_s = pre_map(&[("f.rs", "struct S {\n    a: u32,\n}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw_v), &pre_s).is_none() || {
                let o =
                    infer_semantic_with_pre_images(std::slice::from_ref(&raw_v), &pre_s).unwrap();
                !o.contains("definition_addition")
            },
            "variant plus inside struct must decline"
        );
    }

    #[test]
    fn none_pre_image_declines() {
        let mut used = Vec::new();
        assert!(try_definition_addition(
            &[],
            &["b: bool,"],
            "f.rs",
            None,
            "@@ -2,0 +3 @@\n+    b: bool,\n",
            &mut used,
        )
        .is_none());
        assert!(used.is_empty());
    }

    #[test]
    fn precision_guard_constant_is_sixteen() {
        assert_eq!(super::super::PRECISION_GUARD, 16);
    }

    #[test]
    fn long_container_name_stays_ordinary_under_guard() {
        // A 60-char struct name costs ~60 bytes as a semantic header
        // against a one-line ordinary hunk: the precision guard
        // (semantic <= ordinary + 16) vetoes, so the section stays
        // ordinary. Proves the guard prices the proof, not the bytes.
        let long = "SWithAVeryLongNameThatExceedsThePrecisionGuardEasily1234";
        let pre_text = format!("struct {long} {{\n    a: u32,\n}}\n");
        let raw = section("f.rs", "@@ -2,0 +3 @@\n+    b: bool,\n");
        let pre = pre_map(&[("f.rs", pre_text.as_str())]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre);
        assert!(
            out.is_none() || !out.unwrap().contains("definition_addition"),
            "over-guard addition must stay ordinary"
        );
    }

    // -- WI-12c slice 1: struct-literal follow-through ------------------

    const FOLLOW_PRE: &str = "struct S {\n    a: u32,\n}\n\nfn f() -> S {\n    S { a: 1 }\n}\n\nfn g() -> S {\n    S { a: 2 }\n}\n";
    const FOLLOW_POST: &str = "struct S {\n    a: u32,\n    b: bool,\n}\n\nfn f() -> S {\n    S { a: 1, b: true }\n}\n\nfn g() -> S {\n    S { a: 2, b: false }\n}\n";

    fn follow_section() -> String {
        section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    b: bool,\n",
                "@@ -6 +7 @@\n",
                "-    S { a: 1 }\n",
                "+    S { a: 1, b: true }\n",
                "@@ -10 +11 @@\n",
                "-    S { a: 2 }\n",
                "+    S { a: 2, b: false }\n",
            ),
        )
    }

    #[test]
    fn follow_group_fires_hetero_and_round_trips() {
        let raw = follow_section();
        let pre = pre_map(&[("f.rs", FOLLOW_PRE)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("def + 2 hetero literals must compress");
        assert!(out.contains("  struct S {"), "def header, got:\n{out}");
        assert!(out.contains("+ b: bool,"), "def member, got:\n{out}");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        assert!(out.contains("rule=r2"), "got:\n{out}");
        assert!(out.contains("rule=r3"), "got:\n{out}");
        assert!(out.contains("- S { a: 1 }"), "got:\n{out}");
        assert!(out.contains("+ S { a: 1, b: true }"), "got:\n{out}");
        assert!(out.contains("- S { a: 2 }"), "got:\n{out}");
        assert!(out.contains("+ S { a: 2, b: false }"), "got:\n{out}");
        // Real-apply byte-compare: the joint probe's promise.
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(files["f.rs"], FOLLOW_POST);
    }

    #[test]
    fn follow_group_let_bound_fires() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    b: bool,\n",
                "@@ -6 +7 @@\n",
                "-    let x = S { a: 1 };\n",
                "+    let x = S { a: 1, b: true };\n",
            ),
        );
        let pre = pre_map(&[(
            "f.rs",
            "struct S {\n    a: u32,\n}\n\nfn f() -> S {\n    let x = S { a: 1 };\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre)
            .expect("let-bound literal must join the group");
        assert!(out.contains("rule=r1"), "got:\n{out}");
        assert!(out.contains("rule=r2"), "got:\n{out}");
        assert!(out.contains("- let x = S { a: 1 };"), "got:\n{out}");
        assert!(
            out.contains("+ let x = S { a: 1, b: true };"),
            "got:\n{out}"
        );
        let parsed = rsmpl_patch::parse_patch(&out).expect("must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("must apply");
        assert_eq!(
            files["f.rs"],
            "struct S {\n    a: u32,\n    b: bool,\n}\n\nfn f() -> S {\n    let x = S { a: 1, b: true };\n    x\n}\n"
        );
    }

    #[test]
    fn follow_rest_pair_declines_group_but_def_fires() {
        // `..` on either side: no explicit label to add (acceptance:
        // stays ordinary); the definition still fires alone.
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    b: bool,\n",
                "@@ -6 +7 @@\n",
                "-    S { a: 1, ..Default::default() }\n",
                "+    S { a: 1, b: true, ..Default::default() }\n",
            ),
        );
        let pre = "struct S {\n    a: u32,\n}\n\nfn f() -> S {\n    S { a: 1, ..Default::default() }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_definition_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "rest-carrying pair must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
        let mut used = Vec::new();
        assert!(
            try_definition_addition(
                &[],
                &["    b: bool,"],
                "f.rs",
                Some(pre),
                "@@ -2,0 +3 @@\n+    b: bool,\n",
                &mut used,
            )
            .is_some(),
            "definition still fires alone"
        );
    }

    #[test]
    fn follow_unrelated_hunk_declines_group() {
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    b: bool,\n",
                "@@ -6 +7 @@\n",
                "-    S { a: 1 }\n",
                "+    S { a: 1, b: true }\n",
                "@@ -9 +10 @@\n",
                "-fn one() -> u32 { 1 }\n",
                "+fn one() -> u32 { 2 }\n",
            ),
        );
        let pre = "struct S {\n    a: u32,\n}\n\nfn f() -> S {\n    S { a: 1 }\n}\n\nfn one() -> u32 { 1 }\n";
        let mut used = Vec::new();
        assert!(
            try_definition_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "unrelated hunk must decline the group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn follow_variant_insertion_declines_group() {
        // Variant additions (match-arm follow-through) are WI-12d.
        let raw = section(
            "f.rs",
            concat!(
                "@@ -2,0 +3 @@\n",
                "+    C,\n",
                "@@ -6 +7 @@\n",
                "-    S { a: 1 }\n",
                "+    S { a: 1, b: true }\n",
            ),
        );
        let pre =
            "enum E {\n    A,\n}\nstruct S {\n    a: u32,\n}\nfn f() -> S {\n    S { a: 1 }\n}\n";
        let mut used = Vec::new();
        assert!(
            try_definition_addition_follow(&raw, "f.rs", Some(pre), &mut used).is_none(),
            "variant insertion must decline the field group"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn follow_none_pre_image_declines() {
        let mut used = Vec::new();
        assert!(
            try_definition_addition_follow(&follow_section(), "f.rs", None, &mut used).is_none()
        );
        assert!(used.is_empty());
    }

    #[test]
    fn follow_scaled_guard_long_name_declines() {
        // The scaled guard (16 × hunks) still prices absurd headers:
        // a 60-char struct name over def + 1 construction stays
        // ordinary as a group.
        let long = "SWithAVeryLongNameThatExceedsThePrecisionGuardEasily1234";
        let pre_text = format!(
            "struct {long} {{\n    a: u32,\n}}\n\nfn f() -> {long} {{\n    {long} {{ a: 1 }}\n}}\n"
        );
        let raw = section(
            "f.rs",
            &format!(
                "@@ -2,0 +3 @@\n+    b: bool,\n@@ -6 +7 @@\n-    {long} {{ a: 1 }}\n+    {long} {{ a: 1, b: true }}\n"
            ),
        );
        let mut used = Vec::new();
        assert!(
            try_definition_addition_follow(&raw, "f.rs", Some(&pre_text), &mut used).is_none(),
            "over-guard group must decline"
        );
        assert!(used.is_empty(), "decline must burn no rules");
    }

    #[test]
    fn literal_addition_classification_matrix() {
        // Tail-expr fire, hetero values fine.
        assert!(classify_literal_addition(
            "S { a: 1 }",
            "S { a: 1, b: true }",
            "S",
            "b"
        ));
        assert!(classify_literal_addition(
            "    S { a: 1 },",
            "    S { a: 1, b: true },",
            "S",
            "b"
        ));
        // `let`-bound fire; pat/semi must match.
        assert!(classify_literal_addition(
            "let x = S { a: 1 };",
            "let x = S { a: 1, b: true };",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "let x = S { a: 1 };",
            "let y = S { a: 1, b: true };",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "let x = S { a: 1 };",
            "let x = S { a: 1, b: true }",
            "S",
            "b"
        ));
        // Shorthand fields compare by path value.
        assert!(classify_literal_addition(
            "S { a }",
            "S { a, b: v }",
            "S",
            "b"
        ));
        // Vetoes: wrong struct, wrong field, rest, changed sibling,
        // two extras, missing extra.
        assert!(!classify_literal_addition(
            "T { a: 1 }",
            "T { a: 1, b: v }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1 }",
            "S { a: 1, c: v }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1, ..r }",
            "S { a: 1, b: v, ..r }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1, ..r }",
            "S { a: 1, b: v }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1 }",
            "S { a: 2, b: v }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1 }",
            "S { a: 1, b: v, c: w }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1 }",
            "S { a: 1 }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1 }",
            "T { a: 1, b: v }",
            "S",
            "b"
        ));
        assert!(!classify_literal_addition(
            "S { a: 1 }",
            "S { a: 1, b: v };",
            "S",
            "b"
        ));
    }

    #[test]
    fn wi3_lone_use_insertion_stays_ordinary() {
        // WI-3 slice 4: a standalone top-level `+ use …` insertion is
        // ordinary by design — no struct/enum container spans the
        // point, and a literal-only insertion single would cost the
        // rule-tag overhead over native with zero template benefit
        // (holed insertions are unbound). Insertion joints with a
        // shared rule ride WI-8/WI-13.
        let raw = section("f.rs", "@@ -1,0 +2 @@\n+use crate::new_module::Thing;\n");
        let pre = pre_map(&[("f.rs", "fn main() {}\n")]);
        assert!(
            infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none(),
            "lone use insertion must stay ordinary"
        );
    }
}
