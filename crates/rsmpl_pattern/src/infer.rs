//! WI-1c: inferred metavariables (SPEC 15.8).
//!
//! An undeclared identifier matching `[A-Z][A-Za-z0-9_]*` in a metavar
//! position infers its kind from that position. Explicit declarations
//! remain legal and must agree; backtick literals never infer; sigiled
//! mode only infers `$Name`.

use rsmpl_patch::{ErrorCode, MetavarKind, StructuredError};
use std::collections::{HashMap, HashSet};

use crate::metavar::MetavarTable;
use crate::quote_utils::mask_literal_spans;

/// True for diet candidates: leading `A-Z`, rest alnum/`_`.
pub fn is_candidate(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_uppercase() => {}
        _ => return false,
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
    assign_fn_slots(&mut kinds, &masked_minus, &masked_plus)?;
    // Let shapes.
    assign_let_slots(&mut kinds, &masked_minus, &masked_plus, root)?;
    // Call/method/macro shapes (expr).
    assign_call_slots(&mut kinds, &masked_minus, &masked_plus, &base)?;
    // General fallback for names with no kind yet.
    assign_general_fallback(&mut kinds, &masked_minus, &masked_plus, root, &base)?;

    // Commit: ambiguity + agreement checks.
    for name in order {
        let set = kinds.remove(&name).unwrap_or_default();
        if set.is_empty() {
            continue; // literal position only
        }
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
            ));
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
            ));
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
            ));
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
            ));
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
    assign_fn_slots(&mut kinds, &masked_minus, &masked_plus)?;
    assign_let_slots(&mut kinds, &masked_minus, &masked_plus, root)?;
    assign_call_slots(&mut kinds, &masked_minus, &masked_plus, &base)?;
    assign_general_fallback(&mut kinds, &masked_minus, &masked_plus, root, &base)?;
    commit_sigiled(base, kinds, order)
}

fn add_kind(kinds: &mut HashMap<String, HashSet<MetavarKind>>, name: &str, kind: MetavarKind) {
    if is_candidate(name) {
        if let Some(set) = kinds.get_mut(name) {
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
fn fn_like(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim();
        t.starts_with("fn ")
            || t.contains(" fn ")
            || t.starts_with("pub ")
            || t.starts_with("async ")
            || t.starts_with("unsafe ")
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
        // Name is concrete (rename ident); skip it.
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
fn occurs_in(text: &str, name: &str) -> bool {
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

fn split_top_colon(s: &str) -> (String, Option<String>) {
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
        } else if b == b':' {
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

fn names_in(text: &str, kinds: &HashMap<String, HashSet<MetavarKind>>) -> Vec<String> {
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
            } else {
                // `P: T` inside the pattern span already split above; a
                // composite pattern with holes is not a v1 `let` shape —
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
                for n in names_in(ty_t, kinds) {
                    add_kind(kinds, &n, MetavarKind::Type);
                }
            }
            // With `# root=stmt` the init is still `expr`; without root
            // the same holds. No branch needed on `root` here.
            let _ = root;
            for n in names_in(&init, kinds) {
                // Only bare-arg / wholesale init holes infer here; deeper
                // general-expr holes are picked up by the call/general
                // passes below (same `expr` kind, so no conflict).
                add_kind(kinds, &n, MetavarKind::Expr);
            }
        }
    }
    Ok(())
}

/// Call/method/macro argument + receiver + callee positions → `expr`.
///
/// A callee explicitly declared as `# type` (`P(T)` tuple-struct type
/// patterns) is type-owned: the whole line is left to the type fallback
/// instead of forcing an `expr` disagreement.
fn assign_call_slots(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
    base: &MetavarTable,
) -> Result<(), StructuredError> {
    for text in [masked_minus, masked_plus] {
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with("let ") || t.starts_with("fn ") {
                continue;
            }
            // Method split at the receiver/method dot: depth-aware (same
            // family as the WI-2 router fix). `rfind_dot` ignored paren
            // depth, so `E.push(F.clone())` split at the inner `F.clone`
            // dot (recv `E.push(F`, empty args) and `E` never inferred.
            // Only dots at paren-depth 0 (outside strings) split; the
            // last such dot with a `(` after it wins (WI-2 follow-up).
            if let Some(dot) = infer_method_dot(t) {
                let recv = t[..dot].trim();
                let after = t[dot + 1..].trim();
                if let Some(paren) = after.find('(') {
                    let method = after[..paren].trim();
                    // Strip turbofish `<...>` from method for the check.
                    let method_base = method.split('<').next().unwrap_or(method).trim();
                    if is_candidate(method_base) && kinds.contains_key(method_base) {
                        add_kind(kinds, method_base, MetavarKind::Expr);
                    }
                    // Turbofish args inside `method::<T>`: type holes.
                    // Only the inner args infer (`Vec` in `::<Vec<T>>`
                    // stays literal — same constructor rule as the
                    // general fallback).
                    if let Some((_, after)) = method.split_once('<') {
                        for n in names_in_type_args(after, kinds) {
                            add_kind(kinds, &n, MetavarKind::Type);
                        }
                    }
                    if recv == recv.trim() && !recv.is_empty() {
                        let r = recv.trim_start_matches('&').trim();
                        if is_candidate(r)
                            && kinds.contains_key(r)
                            && !r.contains(char::is_whitespace)
                        {
                            add_kind(kinds, r, MetavarKind::Expr);
                        }
                        for n in names_in(recv, kinds) {
                            // Composite receivers (e.g. `E.foo`) still bind
                            // whole-slot `E` as `expr`; deeper handling is
                            // the general pass's job (same kind).
                            let _ = n;
                        }
                    }
                    // Args inside `(...)`: each bare candidate arg is an
                    // `expr` hole (`...` never matches `is_candidate`).
                    if let Some(args) = balanced_args(&format!("({}", &after[paren + 1..])) {
                        for n in names_in(&args, kinds) {
                            add_kind(kinds, &n, MetavarKind::Expr);
                        }
                    }
                    continue;
                }
            }
            // Plain call `callee(args)` / macro `name!(args)`.
            if let Some(paren) = t.find('(') {
                let callee = t[..paren].trim();
                // Skip `if`/`match`/`for`/`while`/`return` keywords.
                let base_callee = callee.trim_end_matches('!').trim();
                if [
                    "if", "match", "for", "while", "loop", "return", "else", "let",
                ]
                .contains(&base_callee)
                {
                    continue;
                }
                // Type-owned lines (`P(T)` tuple-struct types with `# type P`):
                // the callee is a type constructor, not a call — leave the
                // whole line to explicit declarations (no `expr` inference).
                {
                    let last = base_callee
                        .rsplit("::")
                        .next()
                        .unwrap_or(base_callee)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if matches!(
                        base.kind_of(last),
                        Some(
                            MetavarKind::Type
                                | MetavarKind::Pat
                                | MetavarKind::Item
                                | MetavarKind::GenericParams
                                | MetavarKind::WhereClause
                                | MetavarKind::Visibility
                                | MetavarKind::Qualifier
                                | MetavarKind::QualifierList
                        )
                    ) {
                        continue;
                    }
                }
                // Callee last segment candidate → `expr` (covers `E(...)`
                // and `crate::E(...)`; concrete paths yield no candidates).
                // A turbofish callee (`id::<T>`) keeps its type args as
                // `type` holes. A line that parses as a `syn::Type`
                // (`P(T)` tuple-struct type) leaves its callee to the type
                // pass — it is a type constructor, not a call.
                if syn::parse_str::<syn::Type>(t).is_ok() {
                    // Type-owned: skip callee `expr` (the type fallback
                    // assigns `type` to preserved names only).
                } else if let Some((callee_base, turbo_inner)) = split_turbofish(base_callee) {
                    for n in names_in_type_args(turbo_inner, kinds) {
                        add_kind(kinds, &n, MetavarKind::Type);
                    }
                    let last = callee_base
                        .rsplit("::")
                        .next()
                        .unwrap_or(callee_base)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if is_candidate(last) && kinds.contains_key(last) {
                        add_kind(kinds, last, MetavarKind::Expr);
                    }
                } else {
                    let last = base_callee
                        .rsplit("::")
                        .next()
                        .unwrap_or(base_callee)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if is_candidate(last) && kinds.contains_key(last) {
                        add_kind(kinds, last, MetavarKind::Expr);
                    }
                }
                // Macro invocations (`foo!(...)`) carry token-tree bodies,
                // not expression args: leave their holes to explicit
                // `# macro`/`# token_tree` declarations (diet does not
                // infer macro interiors in WI-1c). Macro names stay
                // literal (explicit `# macro M` binds via placeholders).
                if callee.ends_with('!') {
                    continue;
                }
                if let Some(args) = balanced_args(&t[paren..]) {
                    for n in names_in(&args, kinds) {
                        add_kind(kinds, &n, MetavarKind::Expr);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Split `id::<T, U>` into (`id`, `T, U`). Returns `None` when there is
/// no `::<...>` turbofish.
fn split_turbofish(callee: &str) -> Option<(&str, &str)> {
    let pos = callee.find("::<")?;
    let base = callee[..pos].trim();
    let rest = &callee[pos + 3..];
    let end = rest.find('>')?;
    Some((base, rest[..end].trim()))
}

/// Text inside the first balanced `(...)` group (without parens).
fn balanced_args(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'(') {
        return None;
    }
    let mut depth = 0;
    let mut in_str = false;
    let mut esc = false;
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
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                return Some(s[1..i].to_string());
            }
        }
        i += 1;
    }
    None
}

/// Receiver/method split dot for inference: the last `.` outside strings
/// at paren-depth 0 with a `(` after it. Mirrors `method_split_dots` in
/// `expr_pattern` (WI-2 router fix): dots inside the arg list
/// (`E.push(F.clone())` inner dot) or inside strings never split.
fn infer_method_dot(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut dots = Vec::new();
    let mut depth = 0usize;
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
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth = depth.saturating_sub(1);
        } else if b == b'.' && depth == 0 {
            dots.push(i);
        }
        i += 1;
    }
    dots.into_iter().rev().find(|&d| s[d + 1..].contains('('))
}

fn rfind_dot(s: &str) -> Option<usize> {
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

/// Fallback for names with no kind yet: `# root=` wins; `let`-led uses
/// the `let` split kinds; paren/bang/dot shapes → `expr`; `syn::Type`
/// parse → `type`; else `expr` (mirrors `try_general` order).
///
/// Per-line: a patch may mix a call line (`foo(E)` → `expr`) with a
/// type line (`Vec<E>` → `type`); whole-text heuristics would collapse
/// both to one kind and miss the ambiguity. Each line assigns only the
/// names it contains, so cross-line conflicts surface as incompatible
/// positions.
fn assign_general_fallback(
    kinds: &mut HashMap<String, HashSet<MetavarKind>>,
    masked_minus: &str,
    masked_plus: &str,
    root: Option<&str>,
    base: &MetavarTable,
) -> Result<(), StructuredError> {
    for text in [masked_minus, masked_plus] {
        // Forced roots are whole-text (the root governs the shape).
        match root {
            Some("type") => {
                for n in names_in(text, kinds) {
                    add_kind(kinds, &n, MetavarKind::Type);
                }
                continue;
            }
            Some("expr") => {
                for n in names_in(text, kinds) {
                    add_kind(kinds, &n, MetavarKind::Expr);
                }
                continue;
            }
            Some("item") => {
                for n in names_in(text, kinds) {
                    add_kind(kinds, &n, MetavarKind::Item);
                }
                continue;
            }
            Some("stmt") => continue, // handled by let pass
            _ => {}
        }
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            // Single-token item capture (`- I`) owns its token; the
            // general fallback must not also claim it as `type`.
            {
                let stripped = t.trim_start_matches('$');
                if !t.contains(char::is_whitespace)
                    && is_candidate(stripped)
                    && kinds.contains_key(stripped)
                {
                    continue;
                }
            }
            // `use`-path and item-rename headers never infer: leave pending
            // (literal) so their classifiers emit their own needles.
            if t.starts_with("use ") || t.starts_with("use(") {
                continue;
            }
            if [
                "struct", "enum", "trait", "union", "type", "const", "static", "mod", "impl",
            ]
            .iter()
            .any(|k| t.starts_with(&format!("{k} ")))
            {
                continue;
            }
            if fn_like(line) {
                continue; // fn slots handled above; leftovers are concrete
            }
            // Turbofish call lines (`id::<T>(E)`) are owned by the call
            // pass (type args → `type`, value args → `expr`); the generic
            // fallback would otherwise claim the type args as `expr`.
            if line.contains("::<") {
                continue;
            }
            // `let`-led but unmatched by the split (composite patterns):
            // leave literal.
            if t.starts_with("let") {
                continue;
            }
            // Macro invocations (`foo!(...)`) carry token-tree bodies,
            // not expression args (same rule as the call pass).
            if t.contains('!') && t.contains('(') {
                if let Some(bang) = t.find('!') {
                    let after = t[bang + 1..].trim_start();
                    if after.starts_with('(') || after.starts_with('[') || after.starts_with('{') {
                        continue;
                    }
                }
            }
            let has_call_shape = t.contains('(') || t.contains('!') || rfind_dot(t).is_some();
            // Types first: `fn(T) -> T` (fn-pointer), `Box<dyn T>`,
            // `Vec<T>` all parse as `syn::Type` despite containing
            // parens/dots — the call branch would misclaim their holes
            // as `expr`. Only preserved names infer (`T` on both sides);
            // one-sided concrete additions (`+ Send`) stay literal.
            if syn::parse_str::<syn::Type>(t).is_ok() {
                for n in names_in_type_args(t, kinds) {
                    if occurs_in(masked_minus, &n) && occurs_in(masked_plus, &n) {
                        add_kind(kinds, &n, MetavarKind::Type);
                    }
                }
                continue;
            }
            if has_call_shape {
                // Type-owned lines (`P(T)` with `# type P`): the callee is
                // a type constructor — skip expression inference entirely.
                if let Some(paren) = t.find('(') {
                    let callee = t[..paren].trim();
                    let base_callee = callee.trim_end_matches('!').trim();
                    let last = base_callee
                        .rsplit("::")
                        .next()
                        .unwrap_or(base_callee)
                        .trim();
                    let last = last.trim_start_matches('$');
                    if matches!(
                        base.kind_of(last),
                        Some(
                            MetavarKind::Type
                                | MetavarKind::Pat
                                | MetavarKind::Item
                                | MetavarKind::GenericParams
                                | MetavarKind::WhereClause
                                | MetavarKind::Visibility
                                | MetavarKind::Qualifier
                                | MetavarKind::QualifierList
                        )
                    ) {
                        continue;
                    }
                }
                // Holes live inside the argument groups; the callee prefix
                // (`Box` in `Box::new(E)`, `foo` in `foo(E)`) stays literal.
                // The call pass already bound true call args; this covers
                // general-expression tails (`foo(E) + 1`) it bypassed.
                for n in names_in_paren_groups(t, kinds) {
                    add_kind(kinds, &n, MetavarKind::Expr);
                }
                continue;
            }
            for n in names_in(t, kinds) {
                add_kind(kinds, &n, MetavarKind::Expr);
            }
        }
    }
    Ok(())
}

/// Names inside `(...)` argument groups (all groups, e.g. `foo(E) + bar(F)`
/// yields `E, F`); the callee prefix stays literal.
fn names_in_paren_groups(line: &str, kinds: &HashMap<String, HashSet<MetavarKind>>) -> Vec<String> {
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
fn names_in_type_args(line: &str, kinds: &HashMap<String, HashSet<MetavarKind>>) -> Vec<String> {
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
        assert_eq!(
            infer_method_dot("Receiver.contains(\"a.b\")"),
            Some("Receiver".len())
        );
        // `foo(a.b(c))` has no depth-0 dot: plain call, not a method.
        assert_eq!(infer_method_dot("foo(a.b(c))"), None);
        // `(a.b).c(d)`: inner dot is nested, outer dot splits.
        assert_eq!(infer_method_dot("(a.b).c(d)"), Some("(a.b)".len()));
    }
}
