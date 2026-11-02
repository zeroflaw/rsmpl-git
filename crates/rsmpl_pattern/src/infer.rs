//! WI-1c: inferred metavariables (SPEC 15.8).
//!
//! An undeclared identifier matching `[A-Z][A-Za-z0-9_]*` in a metavar
//! position infers its kind from that position. Explicit declarations
//! remain legal and must agree; backtick literals never infer; sigiled
//! mode only infers `$Name`.

use rsmpl_patch::{ErrorCode, MetavarKind, StructuredError};
use std::collections::{HashMap, HashSet};

use crate::metavar::MetavarTable;
use crate::quote_utils::{mask_literal_spans, qualifier_leads_to_fn};

use super::infer_control::{assign_expr_text, assign_pat_ast};

/// True for diet candidates: leading `A-Z`, rest alnum/`_`.
pub fn is_candidate(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// WI-5b-follow: `ELLIPSIS*` never infers — the placeholder would read as a
/// `...` marker (`__RSMPL_ELLIPSIS_FOO`, or byte-identical to marker 0 for
/// `ELLIPSIS_0`). Declared names hit the same fence in `directives.rs`.
fn check_reserved_ellipsis(name: &str) -> Result<(), StructuredError> {
    if name == "ELLIPSIS" || name.starts_with("ELLIPSIS_") {
        return Err(StructuredError::new(
            ErrorCode::PatternConflict,
            format!("reserved name `{name}` (`ELLIPSIS*` names the `...` marker; rename the metavariable, e.g. `E`)"),
        )
        .with_repair(StructuredError::fix_pattern_repair(
            "infer-reserved-template",
            "rename the hole (e.g. `E`): `ELLIPSIS` / `ELLIPSIS_*` names the `...` marker and never infers (SPEC 15.8)",
        )));
    }
    Ok(())
}

/// Scan ident-like tokens (mirrors `metavar.rs` tokenization).
fn scan_idents(text: &str, sigiled: bool) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        let starts = if sigiled {
            c == '$'
        } else {
            c == '$' || c.is_ascii_alphabetic() || c == '_'
        };
        if starts {
            let mut j = i + 1;
            while j < bytes.len() {
                let d = bytes[j] as char;
                if d.is_ascii_alphanumeric() || d == '_' {
                    j += 1;
                } else {
                    break;
                }
            }
            let tok = &text[i..j];
            if sigiled {
                if let Some(bare) = tok.strip_prefix('$') {
                    if !bare.is_empty() {
                        out.push(bare.to_string());
                    }
                }
            } else {
                out.push(tok.trim_start_matches('$').to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Inference view of pattern text: string literals are blanked and
/// attribute/doc lines are blanked so their literal words (e.g.
/// `Changed` in `/// Changed thing.`) never infer as metavariables.
/// v1 has no attribute-list abstraction and no string-interior holes.
fn infer_view(masked: &str) -> String {
    masked
        .lines()
        .map(|line| {
            let t = line.trim_start();
            if t.starts_with("///")
                || t.starts_with("//!")
                || t.starts_with("/**")
                || t.starts_with("/*")
                || t.starts_with("//")
                || t.starts_with('*')
                || t.starts_with("#[")
            {
                return String::new();
            }
            blank_strings(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Replace `"..."` string contents (escape-aware) with empty so uppercase
/// words inside them never infer.
fn blank_strings(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
                out.push('"');
            }
        } else if b == b'"' {
            // Emit exactly one quote for the opener; the closer emits
            // one more, so `"..."` blanks to `""` (WI-2: two pushes here
            // left `"""`, an unbalanced string that broke `balanced_args`
            // and silently un-inferred every metavar sharing the call).
            in_str = true;
            out.push('"');
        } else {
            out.push(b as char);
        }
        i += 1;
    }
    out
}

/// Extend `base` with inferred kinds from pattern text.
///
/// `minus_raw`/`plus_raw` are the full minus/plus texts (already
/// quote-validated). Returns the extended table or a
/// `RSMPL_PATTERN_CONFLICT` on ambiguity/disagreement.
pub fn infer_table(
    mut base: MetavarTable,
    minus_raw: &str,
    plus_raw: &str,
    root: Option<&str>,
) -> Result<MetavarTable, StructuredError> {
    if base.sigiled {
        return infer_sigiled(base, minus_raw, plus_raw, root);
    }
    // Mask backtick literals so `` `E` `` never infers (SPEC 15.4).
    let (masked_minus, _) = mask_literal_spans(minus_raw)?;
    let (masked_plus, _) = mask_literal_spans(plus_raw)?;
    // Inference view: strings + attr/doc lines blanked so literal words
    // (e.g. `Changed` in `/// Changed thing.`) never infer. v1 has no
    // attribute-list abstraction and no string-interior holes.
    let masked_minus = infer_view(&masked_minus);
    let masked_plus = infer_view(&masked_plus);
    // Symbol-declared names (`# symbol Old=...`) are bound by declaration,
    // not by position: a symbol in callee/receiver position is the verified
    // path, never an `expr` hole. Excluding them here keeps the C14
    // receiver/callee shapes compiling (their agreement gate lives in
    // `compile`, not in inference).
    let is_symbol = |n: &str| base.kind_of(n) == Some(MetavarKind::Symbol);
    // Collect candidates in first-use order (deterministic errors).
    // Declared names are included so explicit-vs-inferred agreement can
    // be checked; undeclared names that infer a kind are added later.
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tok in scan_idents(&masked_minus, false)
        .into_iter()
        .chain(scan_idents(&masked_plus, false))
    {
        if !is_candidate(&tok) || seen.contains(&tok) || is_symbol(&tok) {
            continue;
        }
        // Skip Rust keywords / literals that happen to be uppercase-start?
        // Keywords are lowercase; `...`/`_` never match `is_candidate`.
        seen.insert(tok.clone());
        order.push(tok);
    }
    // Map name -> set of inferred kinds.
    let mut kinds: HashMap<String, HashSet<MetavarKind>> = HashMap::new();
    for name in &order {
        kinds.insert(name.clone(), HashSet::new());
    }
    // Single-token item capture (`- I` alone).
    assign_item_capture(&mut kinds, &masked_minus, root)?;
    // Fn-header slots.
    assign_fn_slots(&mut kinds, &masked_minus, &masked_plus, &base)?;
    // Let shapes.
    assign_let_slots(&mut kinds, &masked_minus, &masked_plus, root)?;
    // Call/method/macro shapes (expr; sibling module per D5).
    super::infer_call::assign_call_slots(&mut kinds, &masked_minus, &masked_plus, &base)?;
    // General fallback for names with no kind yet (sibling module per D5).
    super::infer_call::assign_general_fallback(
        &mut kinds,
        &masked_minus,
        &masked_plus,
        root,
        &base,
    )?;

    // Commit: ambiguity + agreement checks.
    for name in order {
        let set = kinds.remove(&name).unwrap_or_default();
        if set.is_empty() {
            continue; // literal position only
        }
        check_reserved_ellipsis(&name)?;
        // Explicit declaration disambiguates a multi-kind set when it
        // names one member (e.g. `# visibility V` for single pre-`fn`
        // `V`, which is otherwise `{visibility, qualifier_list}`).
        if let Some(prev) = base.kind_of(&name) {
            if set.contains(&prev) {
                continue;
            }
            let mut v: Vec<String> = set.iter().map(|k| k.as_str().to_string()).collect();
            v.sort();
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "metavariable `{name}` declared as {} but used as {}",
                    prev.as_str(),
                    v.join(" vs ")
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "infer-declared-kind-template",
                "fix the declaration to the inferred kind (e.g. `# type E` when `E` is used as `Vec<E>`), or rename one side's hole so each hole holds one kind (SPEC 15.8)",
            )));
        }
        if set.len() > 1 {
            let mut v: Vec<String> = set.iter().map(|k| k.as_str().to_string()).collect();
            v.sort();
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "metavariable `{name}` occurs in incompatible positions ({})",
                    v.join(" vs ")
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "infer-incompatible-positions-template",
                "use one hole per kind (e.g. `foo(E)` + `Vec<T>` instead of `E` in both), or add a matching explicit declaration (e.g. `# expr E`); one hole never spans `expr` vs `type` (SPEC 15.8)",
            )));
        }
        let inferred = set.into_iter().next().unwrap();
        base.kinds.insert(name.clone(), inferred);
        base.declared.push((inferred, name));
    }
    Ok(base)
}

/// Shared commit for the sigiled path (same disambiguation rule).
fn commit_sigiled(
    mut base: MetavarTable,
    mut kinds: HashMap<String, HashSet<MetavarKind>>,
    order: Vec<String>,
) -> Result<MetavarTable, StructuredError> {
    for name in order {
        let set = kinds.remove(&name).unwrap_or_default();
        if set.is_empty() {
            continue;
        }
        check_reserved_ellipsis(&name)?;
        if let Some(prev) = base.kind_of(&name) {
            if set.contains(&prev) {
                continue;
            }
            let mut v: Vec<String> = set.iter().map(|k| k.as_str().to_string()).collect();
            v.sort();
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "metavariable `{name}` declared as {} but used as {}",
                    prev.as_str(),
                    v.join(" vs ")
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "infer-sigiled-declared-kind-template",
                "fix the sigiled declaration to the inferred kind (e.g. `# expr $E` when `$E` is used as `foo($E)`), or rename one side's hole so each hole holds one kind (SPEC 15.8)",
            )));
        }
        if set.len() > 1 {
            let mut v: Vec<String> = set.iter().map(|k| k.as_str().to_string()).collect();
            v.sort();
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                format!(
                    "metavariable `{name}` occurs in incompatible positions ({})",
                    v.join(" vs ")
                ),
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "infer-sigiled-incompatible-positions-template",
                "use one sigiled hole per kind (e.g. `-> $T` + `foo($E)` instead of `$T` in both), or add a matching explicit declaration; one hole never spans `type` vs `expr` (SPEC 15.8)",
            )));
        }
        let inferred = set.into_iter().next().unwrap();
        base.kinds.insert(name.clone(), inferred);
        base.declared.push((inferred, name));
    }
    Ok(base)
}

/// Sigiled mode: only `$Name` counts; bare `Name` is literal.
fn infer_sigiled(
    base: MetavarTable,
    minus_raw: &str,
    plus_raw: &str,
    root: Option<&str>,
) -> Result<MetavarTable, StructuredError> {
    let (masked_minus, _) = mask_literal_spans(minus_raw)?;
    let (masked_plus, _) = mask_literal_spans(plus_raw)?;
    let masked_minus = infer_view(&masked_minus);
    let masked_plus = infer_view(&masked_plus);
    let is_symbol = |n: &str| base.kind_of(n) == Some(MetavarKind::Symbol);
    let mut order: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for tok in scan_idents(&masked_minus, true)
        .into_iter()
        .chain(scan_idents(&masked_plus, true))
    {
        if !is_candidate(&tok) || seen.contains(&tok) || is_symbol(&tok) {
            continue;
        }
        seen.insert(tok.clone());
        order.push(tok);
    }
    let mut kinds: HashMap<String, HashSet<MetavarKind>> = HashMap::new();
    for name in &order {
        kinds.insert(name.clone(), HashSet::new());
    }
    assign_item_capture_sigiled(&mut kinds, &masked_minus, root)?;
    assign_fn_slots(&mut kinds, &masked_minus, &masked_plus, &base)?;
    assign_let_slots(&mut kinds, &masked_minus, &masked_plus, root)?;
    super::infer_call::assign_call_slots(&mut kinds, &masked_minus, &masked_plus, &base)?;
    super::infer_call::assign_general_fallback(
        &mut kinds,
        &masked_minus,
        &masked_plus,
        root,
        &base,
    )?;
    commit_sigiled(base, kinds, order)
}

pub(crate) fn add_kind(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    name: &str,
    kind: MetavarKind,
) {
    if is_candidate(name) {
        // WI-5: a bare `N` infers only as the fn-name hole (`Ident`, added
        // by the fn-name assigner below) or as a type hole in type
        // positions (`Type` — pinned by the T145 turbofish diet test, so
        // `foo::<N>(E)` keeps binding). Everywhere else it stays a
        // literal ident (diet stability: `foo(N)` / `let N = E` keep
        // meaning the literal `N`). Explicit declarations still bind
        // (the declaration itself lives in `base.kinds` — inference just
        // refuses to invent a further kind).
        let bare = name.trim_start_matches('$');
        if bare == "N" && kind != MetavarKind::Ident && kind != MetavarKind::Type {
            return;
        }
        // `kinds` keys are bare names; sigiled `$N` strips to `N`.
        let key = bare.to_string();
        if let Some(set) = kinds.get_mut(&key) {
            set.insert(kind);
        } else if let Some(set) = kinds.get_mut(name) {
            set.insert(kind);
        }
    }
}

/// `- I` alone (single line, whole text is the candidate) → `item`,
/// unless `# root=` forces another kind for that token.
fn assign_item_capture(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    root: Option<&str>,
) -> Result<(), StructuredError> {
    let t = masked_minus.trim();
    if t.lines().count() != 1 || t.contains(char::is_whitespace) || *t == *"..." || t.is_empty() {
        return Ok(());
    }
    let name = t.trim_start_matches('$');
    if !is_candidate(name) || !kinds.contains_key(name) {
        return Ok(());
    }
    let kind = match root {
        Some("type") => MetavarKind::Type,
        Some("expr") => MetavarKind::Expr,
        Some("stmt") => MetavarKind::Stmt,
        _ => MetavarKind::Item,
    };
    add_kind(kinds, name, kind);
    Ok(())
}

fn assign_item_capture_sigiled(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    root: Option<&str>,
) -> Result<(), StructuredError> {
    // In sigiled mode the masked text holds `$I`; strip it for the check.
    let t = masked_minus.trim();
    if t.lines().count() != 1 || t.is_empty() || *t == *"..." {
        return Ok(());
    }
    // Single `$Name` token (no whitespace inside after strip).
    let inner = t.trim();
    if inner.contains(char::is_whitespace) {
        return Ok(());
    }
    let Some(name) = inner.strip_prefix('$') else {
        return Ok(());
    };
    if !is_candidate(name) || !kinds.contains_key(name) {
        return Ok(());
    }
    let kind = match root {
        Some("type") => MetavarKind::Type,
        Some("expr") => MetavarKind::Expr,
        Some("stmt") => MetavarKind::Stmt,
        _ => MetavarKind::Item,
    };
    add_kind(kinds, name, kind);
    Ok(())
}

/// Is this (masked) side fn-like? For inference any header line carrying
/// `fn ` counts — including leading slot metas (`V fn foo`) that the
/// table-aware `looks_like_fn` cannot see before inference runs.
/// (Shared with the `infer_call` sibling per D5.)
pub(crate) fn fn_like(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        t.starts_with("fn ")
            || t.contains(" fn ")
            || t.starts_with("pub ")
            // WI-6 slice 1: `async {` / `unsafe {` open block expressions,
            // not fn headers — only route when a fn/trait/impl follows.
            || (t.starts_with("async ") && qualifier_leads_to_fn(t))
            || (t.starts_with("unsafe ") && qualifier_leads_to_fn(t))
            || t.starts_with("const ")
            || t.starts_with("extern ")
            || t.starts_with("pub(")
            || text.contains("\nfn ")
            || text.trim_start().starts_with("fn ")
    })
}

/// Tokenize an fn header line cheaply for slot detection.
fn fn_tokens(text: &str) -> Vec<String> {
    // First line carrying `fn` (headers are single-line for inference;
    // multi-line `where` tails are handled by the `W` scan below).
    let line = text.lines().find(|l| l.contains("fn ")).unwrap_or("");
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
        } else if "(){}<>[],;#".contains(c) {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
            out.push(c.to_string());
        } else if c == ':' && chars.peek() == Some(&':') {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
            chars.next();
            out.push("::".to_string());
        } else if c == '-' && chars.peek() == Some(&'>') {
            if !cur.is_empty() {
                out.push(cur.clone());
                cur.clear();
            }
            chars.next();
            out.push("->".to_string());
        } else if c == '.' && chars.peek() == Some(&'.') {
            // `...` single token
            chars.next();
            if chars.peek() == Some(&'.') {
                chars.next();
                if !cur.is_empty() {
                    out.push(cur.clone());
                    cur.clear();
                }
                out.push("...".to_string());
            } else {
                cur.push('.');
                cur.push('.');
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn assign_fn_slots(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
    base: &MetavarTable,
) -> Result<(), StructuredError> {
    for text in [masked_minus, masked_plus] {
        if !fn_like(text) {
            continue;
        }
        let toks = fn_tokens(text);
        let Some(fn_pos) = toks.iter().position(|t| t == "fn") else {
            continue;
        };
        // Pre-`fn` tokens (skip attribute lines already peeled at a higher
        // level; here tokens before `fn` on the header line).
        let pre: Vec<String> = toks[..fn_pos].to_vec();
        // Filter out `pub(...)` contents: `pub ( ... )` tokenizes with
        // parens; a candidate inside is not a slot meta.
        let mut pre_names: Vec<String> = Vec::new();
        let mut k = 0;
        while k < pre.len() {
            if pre[k] == "pub" && pre.get(k + 1).is_some_and(|n| n == "(") {
                let mut depth = 0;
                k += 1;
                while k < pre.len() {
                    if pre[k] == "(" {
                        depth += 1;
                    }
                    if pre[k] == ")" {
                        depth -= 1;
                        k += 1;
                        break;
                    }
                    k += 1;
                }
                let _ = depth;
                continue;
            }
            if pre[k] == "pub"
                || ["const", "async", "unsafe", "extern"].contains(&pre[k].as_str())
                || pre[k].starts_with('"')
            {
                k += 1;
                continue;
            }
            pre_names.push(pre[k].clone());
            k += 1;
        }
        // Keep only inference candidates present in our map.
        let pre_cands: Vec<String> = pre_names
            .into_iter()
            .map(|t| t.trim_start_matches('$').to_string())
            .filter(|n| kinds.contains_key(n))
            .collect();
        match pre_cands.len() {
            1 => {
                // Ambiguous: could be `visibility` or `qualifier_list`.
                // Record both; the commit step fails with both names
                // unless an explicit declaration disambiguates.
                let n = pre_cands[0].clone();
                add_kind(kinds, &n, MetavarKind::Visibility);
                add_kind(kinds, &n, MetavarKind::QualifierList);
            }
            2.. => {
                add_kind(kinds, &pre_cands[0], MetavarKind::Visibility);
                add_kind(kinds, &pre_cands[1], MetavarKind::QualifierList);
                for extra in pre_cands.iter().skip(2) {
                    add_kind(kinds, extra, MetavarKind::QualifierList);
                }
            }
            _ => {}
        }
        // WI-5 name hole: `N` binds the fn name per site (diet), and any
        // `# ident X` name binds in this position when declared. Any other
        // name stays concrete (rename ident).
        if let Some(name_tok) = toks.get(fn_pos + 1) {
            let bare = name_tok.trim_start_matches('$').to_string();
            if kinds.contains_key(&bare)
                && is_candidate(&bare)
                && (bare == "N" || base.kind_of(&bare) == Some(MetavarKind::Ident))
            {
                add_kind(kinds, &bare, MetavarKind::Ident);
            }
        }
        // `G` between name and `(`.
        if let Some(name_pos) = Some(fn_pos + 1).filter(|p| *p < toks.len()) {
            let after_name = name_pos + 1;
            if after_name < toks.len() {
                let t = toks[after_name].trim_start_matches('$').to_string();
                if t != "(" && t != "<" && kinds.contains_key(&t) && is_candidate(&t) {
                    add_kind(kinds, &t, MetavarKind::GenericParams);
                }
            }
        }
        // `W` after params/return, before `{`/`;`: scan tokens after the
        // first balanced `(...)` group for a lone candidate.
        if let Some(open) = toks.iter().position(|t| t == "(") {
            let mut depth = 0;
            let mut close = None;
            for (idx, t) in toks.iter().enumerate().skip(open) {
                if t == "(" {
                    depth += 1;
                }
                if t == ")" {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(idx);
                        break;
                    }
                }
            }
            if let Some(c) = close {
                let mut j = c + 1;
                // Optional `-> Ret` single token.
                if toks.get(j).is_some_and(|t| t == "->") {
                    j += 1;
                    if let Some(ret) = toks.get(j) {
                        let rn = ret.trim_start_matches('$').to_string();
                        // Single-token returns infer only when preserved
                        // (`-> T` on both sides): a concrete `-> String`
                        // rename stays literal.
                        let preserved = occurs_in(masked_minus, &rn) && occurs_in(masked_plus, &rn);
                        if kinds.contains_key(&rn)
                            && preserved
                            && toks
                                .get(j + 1)
                                .is_none_or(|n| n == "where" || n == "{" || n == ";" || n == "...")
                        {
                            // Single-token return type → `type`.
                            add_kind(kinds, &rn, MetavarKind::Type);
                        } else if kinds.contains_key(&rn) && preserved {
                            // Multi-token return head: still a type hole.
                            add_kind(kinds, &rn, MetavarKind::Type);
                        }
                        // Skip the full return span (until `where`/`{`/`;`).
                        while j < toks.len()
                            && toks[j] != "where"
                            && toks[j] != "{"
                            && toks[j] != ";"
                        {
                            j += 1;
                        }
                    }
                }
                if toks.get(j).is_some_and(|t| t == "where") {
                    j += 1;
                    // `W` alone before `{`/`;` → `where_clause`.
                    if let Some(w) = toks.get(j) {
                        let wn = w.trim_start_matches('$').to_string();
                        if kinds.contains_key(&wn)
                            && toks.get(j + 1).is_none_or(|n| n == "{" || n == ";")
                        {
                            add_kind(kinds, &wn, MetavarKind::WhereClause);
                        }
                    }
                } else if j < toks.len() {
                    let t = toks[j].trim_start_matches('$').to_string();
                    if kinds.contains_key(&t)
                        && (toks.get(j + 1).is_none_or(|n| n == "{" || n == ";"))
                    {
                        // Bare `W` with no `where` keyword (e.g. `fn foo G (...) W`).
                        add_kind(kinds, &t, MetavarKind::WhereClause);
                    }
                }
            }
        }
    }
    Ok(())
}

/// True when `name` occurs as an ident token in `text` (bare or `$`-prefixed).
pub(crate) fn occurs_in(text: &str, name: &str) -> bool {
    scan_idents(text, false)
        .into_iter()
        .any(|t| t.trim_start_matches('$') == name)
}

/// Split a `let` line into (pattern, type?, init?) spans textually.
fn split_let(line: &str) -> Option<(String, Option<String>, String)> {
    let t = line.trim().trim_end_matches(';').trim().to_string();
    let rest = t.strip_prefix("let")?;
    if !(rest.starts_with(char::is_whitespace) || rest.starts_with('(') || rest.starts_with('[')) {
        return None;
    }
    let rest = rest.trim();
    // Split at top-level `=` (skip `=>`, `==`, strings).
    let (left, init) = split_top_eq(rest)?;
    // Split left at top-level `:` (skip `::`).
    let (pat, ty) = split_top_colon(&left);
    Some((
        pat.trim().to_string(),
        ty.map(|s| s.trim().to_string()),
        init.trim().to_string(),
    ))
}

fn split_top_eq(s: &str) -> Option<(String, String)> {
    let mut in_str = false;
    let mut esc = false;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'=' {
            let next = bytes.get(i + 1).copied().unwrap_or(b' ');
            let prev = if i > 0 { bytes[i - 1] } else { b' ' };
            if next == b'>'
                || next == b'='
                || prev == b'='
                || prev == b'!'
                || prev == b'<'
                || prev == b'>'
            {
                i += 1;
                continue;
            }
            return Some((s[..i].to_string(), s[i + 1..].to_string()));
        }
        i += 1;
    }
    None
}

pub(crate) fn split_top_colon(s: &str) -> (String, Option<String>) {
    // WI-3: brace/paren/bracket-depth-aware — a struct-pattern field
    // colon (`S { f: P }`) must not split the ascription. Only a
    // depth-0 `:` separates `P: T`.
    let mut in_str = false;
    let mut esc = false;
    let mut depth: i32 = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'(' || b == b'[' || b == b'{' {
            depth += 1;
        } else if (b == b')' || b == b']' || b == b'}') && depth > 0 {
            depth -= 1;
        } else if b == b':' && depth == 0 {
            let next = bytes.get(i + 1).copied().unwrap_or(b' ');
            let prev = if i > 0 { bytes[i - 1] } else { b' ' };
            if next == b':' || prev == b':' {
                i += 1;
                continue;
            }
            return (s[..i].to_string(), Some(s[i + 1..].to_string()));
        }
        i += 1;
    }
    (s.to_string(), None)
}

pub(crate) fn names_in(text: &str, kinds: &HashMap<String, HashSet<MetavarKind>>) -> Vec<String> {
    scan_idents(text, false)
        .into_iter()
        .map(|t| t.trim_start_matches('$').to_string())
        .filter(|n| kinds.contains_key(n))
        .collect()
}

fn assign_let_slots(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
    root: Option<&str>,
) -> Result<(), StructuredError> {
    for text in [masked_minus, masked_plus] {
        for line in text.lines() {
            let Some((pat, ty, init)) = split_let(line) else {
                continue;
            };
            // Whole-pattern `P` or `P: T` parts → `pat`/`type`.
            let pat_names = names_in(&pat, kinds);
            if pat_names.len() == 1 && pat.trim() == pat_names[0] {
                add_kind(kinds, &pat_names[0], MetavarKind::Pat);
            } else if let Ok(syn::Stmt::Local(local)) =
                syn::parse_str::<syn::Stmt>(&format!("let {pat} = _;"))
            {
                // WI-3 structural composites: bare idents in bindable
                // leaf positions infer `pat` (mirrors the engine's
                // `match_pat` arms + the C8 gate). Unparseable spans
                // stay literal so the compile gate fails loud.
                assign_pat_ast(&local.pat, kinds);
            } else {
                // A composite pattern with holes is not a v1 `let` shape —
                // leave uninferred (literal) so `check_let_pat` fails
                // with its own needle rather than a spurious infer.
                for n in pat_names {
                    // Only infer `pat` when the pattern span is exactly
                    // `P` or `P: T` handled below; otherwise skip.
                    let _ = n;
                }
            }
            if let Some(ty) = ty {
                // `P: T` where both are single candidates.
                let pat_t = pat.trim();
                let ty_t = ty.trim();
                if is_candidate(pat_t)
                    && kinds.contains_key(pat_t)
                    && pat_t.lines().count() == 1
                    && !pat_t.contains(char::is_whitespace)
                {
                    add_kind(kinds, pat_t, MetavarKind::Pat);
                }
                // Type constructors stay literal (`Option` in
                // `Option<i32>` never infers); only arg-position names
                // (`T` in `Vec<T>`, bare `T`) infer `type` — the same
                // head-literal rule as the general fallback's
                // `names_in_type_args` (WI-3: a holed `Option` head hit
                // C8 instead of matching structurally).
                for n in names_in_type_args(ty_t, kinds) {
                    add_kind(kinds, &n, MetavarKind::Type);
                }
            }
            // With `# root=stmt` the init is still `expr`; without root
            // the same holds. No branch needed on `root` here.
            let _ = root;
            // WI-5b: AST-aware init (a `|P| E` / `match` / `for` init
            // would claim `P` as `expr` under the text fallback,
            // contradicting `# pat P`).
            assign_expr_text(&init, kinds);
        }
    }
    Ok(())
}

pub(crate) fn rfind_dot(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut last = None;
    let mut i = 0;
    let mut in_str = false;
    let mut esc = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'.' {
            last = Some(i);
        }
        i += 1;
    }
    last
}

/// Names inside `(...)` argument groups (all groups, e.g. `foo(E) + bar(F)`
/// yields `E, F`); the callee prefix stays literal.
pub(crate) fn names_in_paren_groups(
    line: &str,
    kinds: &HashMap<String, HashSet<MetavarKind>>,
) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut depth = 0;
            let mut j = i;
            let mut in_str = false;
            let mut esc = false;
            while j < bytes.len() {
                let b = bytes[j];
                if in_str {
                    if esc {
                        esc = false;
                    } else if b == b'\\' {
                        esc = true;
                    } else if b == b'"' {
                        in_str = false;
                    }
                } else if b == b'"' {
                    in_str = true;
                } else if b == b'(' {
                    depth += 1;
                } else if b == b')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if depth == 0 && j < bytes.len() {
                for n in names_in(&line[i + 1..j], kinds) {
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Names inside type arguments: text from the first `<`/`[` onward, so the
/// leading constructor (`Vec`/`Box`/`Result`) stays literal. Bare `T`
/// (no brackets) yields `T` itself.
pub(crate) fn names_in_type_args(
    line: &str,
    kinds: &HashMap<String, HashSet<MetavarKind>>,
) -> Vec<String> {
    let t = line.trim();
    if let Some(pos) = t.find(['<', '[']) {
        names_in(&t[pos..], kinds)
    } else {
        names_in(t, kinds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsmpl_patch::{Directive, MetavarKind};

    fn base(dirs: Vec<Directive>) -> MetavarTable {
        MetavarTable::from_directives(&dirs).unwrap()
    }

    fn infer(minus: &str, plus: &str) -> MetavarTable {
        infer_table(base(vec![]), minus, plus, None).unwrap()
    }

    #[test]
    fn single_uppercase_arg_infers_expr() {
        let t = infer("foo(E)", "bar(E)");
        assert_eq!(t.kind_of("E"), Some(MetavarKind::Expr));
    }

    #[test]
    fn wi23_field_base_infers_expr() {
        // WI-23: diet field-base hole (`E0.timeout` → `E0.timeout2`).
        let t = infer("E0.timeout", "E0.timeout2");
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
    }

    #[test]
    fn wi23_dotted_operand_infers_expr() {
        // WI-23 residue: any dotted operand missed the same way
        // (`has_call_shape` + no paren groups). Binary/reference
        // wrappers around a field base infer through the AST walk.
        let t = infer("E0.a + 1", "E0.a + 2");
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
        let t = infer("&E0.f", "&E0.g");
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
    }

    #[test]
    fn wi23_explicit_field_decl_agrees() {
        // The explicit-declaration fallback keeps compiling: declared
        // `expr` is a member of the inferred set (agreement, not
        // conflict).
        let b = base(vec![Directive::MetaVar {
            kind: MetavarKind::Expr,
            names: vec!["E0".to_string()],
        }]);
        let t = infer_table(b, "E0.timeout", "E0.timeout2", None).unwrap();
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
    }

    #[test]
    fn wi23_sigiled_field_base_infers_expr() {
        // Sigiled mode: `$E0.timeout` strips to a parseable field for
        // inference (bare `E0` stays literal there).
        let b = base(vec![Directive::SigiledMetavars(true)]);
        let t = infer_table(b, "foo(E)", "bar(E)", None).unwrap();
        assert_eq!(t.kind_of("E"), None);
        let b2 = base(vec![Directive::SigiledMetavars(true)]);
        let t2 = infer_table(b2, "$E0.timeout", "$E0.timeout2", None).unwrap();
        assert_eq!(t2.kind_of("E0"), Some(MetavarKind::Expr));
    }

    #[test]
    fn wi23_concrete_dotted_stays_literal() {
        // Fully-concrete dotted lines carry no holes.
        let t = infer("order.timeout", "order.timeout2");
        assert_eq!(t.kind_of("order"), None);
    }

    #[test]
    fn wi23_qself_qualifier_stays_literal() {
        // Qself qualifiers are token-compared by the engine (no holes
        // inside `qself`): the AST walk skips them so the qualified
        // call sweep keeps firing (fixture 94). Args still infer.
        let t = infer(
            "<TargetType as ConversionTrait>::old_handler_name(E0)",
            "<TargetType as ConversionTrait>::new_handler_name(E0)",
        );
        assert_eq!(t.kind_of("TargetType"), None);
        assert_eq!(t.kind_of("ConversionTrait"), None);
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
    }

    #[test]
    fn ellipsis_name_never_infers() {
        // WI-5b-follow: an undeclared `ELLIPSIS_*` in a hole position fails
        // loud instead of placeholder-substituting into the marker
        // namespace (declared names hit the `directives.rs` fence).
        for (m, p) in [
            ("foo(ELLIPSIS_0)", "bar(ELLIPSIS_0)"),
            ("foo(ELLIPSIS_FOO)", "bar(ELLIPSIS_FOO)"),
        ] {
            assert!(infer_table(base(vec![]), m, p, None).is_err(), "{m}");
        }
    }

    #[test]
    fn multi_char_uppercase_receiver_infers_expr() {
        let t = infer("Receiver.old_method(Arg)", "Receiver.new_method(Arg)");
        assert_eq!(t.kind_of("Receiver"), Some(MetavarKind::Expr));
        assert_eq!(t.kind_of("Arg"), Some(MetavarKind::Expr));
    }

    #[test]
    fn lowercase_never_infers() {
        let t = infer("foo(bar)", "baz(bar)");
        assert_eq!(t.kind_of("bar"), None);
    }

    #[test]
    fn backtick_literal_never_infers() {
        let t = infer("foo(`E`)", "bar(`E`)");
        assert_eq!(t.kind_of("E"), None);
    }

    #[test]
    fn metavar_beside_string_literal_still_infers() {
        // WI-2: `blank_strings` once blanked `"old"` to `"""` (three
        // quotes), leaving an unbalanced string that broke
        // `balanced_args` — every metavar sharing the call silently
        // stayed a concrete literal and the hunk never matched.
        let t = infer("- run(E0, \"old\")", "+ run(E0, \"new\")");
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
        let t = infer("- run(\"old\", E0)", "+ run(\"new\", E0)");
        assert_eq!(t.kind_of("E0"), Some(MetavarKind::Expr));
    }

    #[test]
    fn single_token_minus_infers_item() {
        let t = infer("I", "");
        // Plus empty: only minus scanned; `I` is the whole minus.
        assert_eq!(t.kind_of("I"), Some(MetavarKind::Item));
    }

    #[test]
    fn let_split_infers_pat_type_expr() {
        let t = infer("let P: T = E;", "let P2: T = E;");
        assert_eq!(t.kind_of("P"), Some(MetavarKind::Pat));
        assert_eq!(t.kind_of("T"), Some(MetavarKind::Type));
        assert_eq!(t.kind_of("E"), Some(MetavarKind::Expr));
        // `P2` is plus-only: still infers from its plus position.
        assert_eq!(t.kind_of("P2"), Some(MetavarKind::Pat));
    }

    #[test]
    fn type_position_infers_type() {
        let t = infer("Vec<T>", "Box<[T]>");
        assert_eq!(t.kind_of("T"), Some(MetavarKind::Type));
    }

    #[test]
    fn parenthesized_fn_args_infer_type() {
        // WI-6c-diet: `T` inside `Fn(T) -> T` infers `type` (input and
        // output), so the diet spelling needs no `# type T`. The call
        // pass must not claim the paren-group names as `expr` on
        // type-parseable lines (that plus the type fallback's `type`
        // was an incompatible-positions conflict).
        let t = infer("Box<dyn Fn(T) -> T>", "Box<dyn Fn(T) -> Box<T>>");
        assert_eq!(t.kind_of("T"), Some(MetavarKind::Type));
    }

    #[test]
    fn fn_generic_where_infer() {
        let t = infer("fn foo G (...) W { ... }", "fn bar G (...) W { ... }");
        assert_eq!(t.kind_of("G"), Some(MetavarKind::GenericParams));
        assert_eq!(t.kind_of("W"), Some(MetavarKind::WhereClause));
    }

    #[test]
    fn single_prefn_is_ambiguous() {
        let err = infer_table(
            base(vec![]),
            "V fn foo(...) { ... }",
            "V fn bar(...) { ... }",
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("incompatible positions"), "unexpected: {err}");
        assert!(err.contains("visibility"), "unexpected: {err}");
        assert!(err.contains("qualifier_list"), "unexpected: {err}");
    }

    #[test]
    fn explicit_disambiguates_prefn() {
        let b = base(vec![Directive::MetaVar {
            kind: MetavarKind::Visibility,
            names: vec!["V".to_string()],
        }]);
        let t = infer_table(b, "V fn foo(...) { ... }", "V fn bar(...) { ... }", None).unwrap();
        assert_eq!(t.kind_of("V"), Some(MetavarKind::Visibility));
    }

    #[test]
    fn two_prefn_tokens_infer_vis_and_qual() {
        let t = infer("V Q fn foo(...) { ... }", "V Q fn bar(...) { ... }");
        assert_eq!(t.kind_of("V"), Some(MetavarKind::Visibility));
        assert_eq!(t.kind_of("Q"), Some(MetavarKind::QualifierList));
    }

    #[test]
    fn ambiguity_across_kinds_fails() {
        // `E` as call arg (expr) and as `Vec<E>` element (type) in one patch.
        let err = infer_table(base(vec![]), "foo(E)\nVec<E>", "bar(E)\nBox<[E]>", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("incompatible positions"), "unexpected: {err}");
    }

    #[test]
    fn disagreement_with_explicit_fails() {
        let b = base(vec![Directive::MetaVar {
            kind: MetavarKind::Expr,
            names: vec!["T".to_string()],
        }]);
        let err = infer_table(b, "Vec<T>", "Box<[T]>", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("declared as expr but used as type"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn explicit_agreement_passes() {
        let b = base(vec![Directive::MetaVar {
            kind: MetavarKind::Expr,
            names: vec!["E".to_string()],
        }]);
        let t = infer_table(b, "foo(E)", "bar(E)", None).unwrap();
        assert_eq!(t.kind_of("E"), Some(MetavarKind::Expr));
    }

    #[test]
    fn sigiled_bare_is_literal() {
        let b = base(vec![Directive::SigiledMetavars(true)]);
        let t = infer_table(b, "foo(E)", "bar(E)", None).unwrap();
        assert_eq!(t.kind_of("E"), None);
        let b2 = base(vec![Directive::SigiledMetavars(true)]);
        let t2 = infer_table(b2, "foo($E)", "bar($E)", None).unwrap();
        assert_eq!(t2.kind_of("E"), Some(MetavarKind::Expr));
    }

    #[test]
    fn hetero_method_with_dotted_arg_infers_both() {
        // WI-2 follow-up: `E.push(F.clone())` mis-split at the inner
        // `F.clone` dot (`rfind_dot` ignored paren depth), so `E`
        // (outside parens) never inferred and the diet hunk never
        // matched. Depth-aware split infers both as `expr`.
        let t = infer("E.push(F.clone())", "E.push2(F.clone())");
        assert_eq!(t.kind_of("E"), Some(MetavarKind::Expr));
        assert_eq!(t.kind_of("F"), Some(MetavarKind::Expr));
    }

    #[test]
    fn concrete_method_with_dotted_arg_infers_nothing() {
        // Fully-concrete `order.push(rel.clone())` carries no holes.
        let t = infer("order.push(rel.clone())", "order.push2(rel.clone())");
        assert_eq!(t.kind_of("order"), None);
        assert_eq!(t.kind_of("rel"), None);
    }

    #[test]
    fn infer_method_dot_skips_string_and_arg_dots() {
        // Dots inside strings and inside the arg list never split.
        // (Helper lives in the `infer_call` sibling per D5.)
        use crate::infer_call::infer_method_dot;
        assert_eq!(
            infer_method_dot("Receiver.contains(\"a.b\")"),
            Some("Receiver".len())
        );
        // `foo(a.b(c))` has no depth-0 dot: plain call, not a method.
        assert_eq!(infer_method_dot("foo(a.b(c))"), None);
        // `(a.b).c(d)`: inner dot is nested, outer dot splits.
        assert_eq!(infer_method_dot("(a.b).c(d)"), Some("(a.b)".len()));
    }

    #[test]
    fn bare_n_in_fn_name_infers_ident() {
        // WI-5 diet: `N` in fn name position is the name hole.
        let t = infer("fn N(...) { ... }", "pub fn N(...) { ... }");
        assert_eq!(t.kind_of("N"), Some(MetavarKind::Ident));
    }

    #[test]
    fn explicit_ident_declares_name_hole() {
        // WI-5 explicit: `# ident Foo` makes `Foo` a hole in name position.
        let b = base(vec![Directive::MetaVar {
            kind: MetavarKind::Ident,
            names: vec!["Foo".to_string()],
        }]);
        let t = infer_table(b, "fn Foo(...) { ... }", "pub fn Foo(...) { ... }", None).unwrap();
        assert_eq!(t.kind_of("Foo"), Some(MetavarKind::Ident));
    }

    #[test]
    fn bare_n_elsewhere_stays_literal() {
        // WI-5: diet `N` outside name/type positions never infers — call
        // args, let inits, and method callees keep meaning the literal `N`.
        assert_eq!(infer("foo(N)", "bar(N)").kind_of("N"), None);
        assert_eq!(infer("let x = N;", "let x = N;").kind_of("N"), None);
        assert_eq!(infer("E.N()", "E.M()").kind_of("N"), None);
    }

    #[test]
    fn bare_n_in_type_position_still_infers_type() {
        // T145 pin survives WI-5: turbofish/type-arg `N` is a type hole
        // (diet `# type N`), so `foo::<N>(E)` keeps binding.
        let t = infer("foo::<N>(E)", "bar::<N>(E)");
        assert_eq!(t.kind_of("N"), Some(MetavarKind::Type));
    }

    #[test]
    fn sigiled_name_hole() {
        // Sigiled mode: bare `N` is literal even in name position; `$N`
        // is the hole.
        let b = base(vec![Directive::SigiledMetavars(true)]);
        let t = infer_table(b, "fn N(...) { ... }", "pub fn N(...) { ... }", None).unwrap();
        assert_eq!(t.kind_of("N"), None);
        let b2 = base(vec![Directive::SigiledMetavars(true)]);
        let t2 = infer_table(b2, "fn $N(...) { ... }", "pub fn $N(...) { ... }", None).unwrap();
        assert_eq!(t2.kind_of("N"), Some(MetavarKind::Ident));
    }

    fn repair_code(err: &StructuredError) -> String {
        err.repair
            .as_ref()
            .expect("repair")
            .get("code")
            .and_then(|v| v.as_str())
            .expect("code str")
            .to_string()
    }

    fn repair_template(err: &StructuredError) -> String {
        err.repair
            .as_ref()
            .expect("repair")
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str")
            .to_string()
    }

    #[test]
    fn wi18_slice8_reserved_template_repair_is_specific() {
        // `ELLIPSIS` as a hole never infers; the repair names the rename.
        let err = infer_table(base(vec![]), "foo(ELLIPSIS)", "bar(ELLIPSIS)", None)
            .expect_err("reserved hole must fail");
        assert_eq!(repair_code(&err), "infer-reserved-template");
        // Reason byte-identical to HEAD (fence text unchanged).
        assert!(err.to_string().contains("reserved name `ELLIPSIS`"));
        let w = repair_template(&err);
        assert!(w.contains("rename the hole"), "got: {w}");
        assert!(w.contains("SPEC 15.8"), "got: {w}");
    }

    #[test]
    fn wi18_slice8_declared_kind_template_repair_is_specific() {
        // Declared `# expr E` but used as `Vec<E>` (`type`): the repair
        // names the declaration fix, not the generic filler.
        let b = base(vec![Directive::MetaVar {
            kind: MetavarKind::Expr,
            names: vec!["E".to_string()],
        }]);
        let err = infer_table(b, "Vec<E>", "Vec<E>", None).expect_err("declared-kind must fail");
        assert_eq!(repair_code(&err), "infer-declared-kind-template");
        assert!(err
            .to_string()
            .contains("declared as expr but used as type"));
        let w = repair_template(&err);
        assert!(w.contains("# type E"), "got: {w}");
    }

    #[test]
    fn wi18_slice8_incompatible_positions_template_repair_is_specific() {
        // `E` in both `foo(E)` (`expr`) and `Vec<E>` (`type`): one hole
        // never spans two kinds; the repair names the split.
        let err = infer_table(base(vec![]), "foo(E)\nVec<E>", "bar(E)\nVec<E>", None)
            .expect_err("cross-kind hole must fail");
        assert_eq!(repair_code(&err), "infer-incompatible-positions-template");
        assert!(err.to_string().contains("incompatible positions"));
        let w = repair_template(&err);
        assert!(w.contains("one hole per kind"), "got: {w}");
    }

    #[test]
    fn wi18_slice8_sigiled_declared_kind_template_repair_is_specific() {
        // Sigiled twin of the declared-kind gate (`$E` spelling): declared
        // `# type E` but used as a call arg (`expr`).
        let b = base(vec![
            Directive::SigiledMetavars(true),
            Directive::MetaVar {
                kind: MetavarKind::Type,
                names: vec!["E".to_string()],
            },
        ]);
        let err = infer_table(b, "foo($E)", "bar($E)", None)
            .expect_err("sigiled declared-kind must fail");
        assert_eq!(repair_code(&err), "infer-sigiled-declared-kind-template");
        assert!(err
            .to_string()
            .contains("declared as type but used as expr"));
        let w = repair_template(&err);
        assert!(w.contains("# expr $E"), "got: {w}");
    }

    #[test]
    fn wi18_slice8_sigiled_incompatible_positions_template_repair_is_specific() {
        // Sigiled twin of the cross-kind gate: `$T` as an fn return type
        // (`type`) and a call arg (`expr`) on each side.
        let b = base(vec![Directive::SigiledMetavars(true)]);
        let err = infer_table(
            b,
            "fn foo() -> $T\nfoo($T)",
            "fn foo() -> $T\nbar($T)",
            None,
        )
        .expect_err("sigiled cross-kind hole must fail");
        assert_eq!(
            repair_code(&err),
            "infer-sigiled-incompatible-positions-template"
        );
        assert!(err.to_string().contains("incompatible positions"));
        let w = repair_template(&err);
        assert!(w.contains("foo($E)"), "got: {w}");
    }
}
