//! v1 semantic inference: fn/item rename + slot-sweep + call-sweep +
//! method-to-function, method-to-method, &-receiver strip, and use-path
//! prefix fan-outs. Every emission clears a token-win guard (T375): the
//! semantic hunk is kept only when byte-smaller than the ordinary
//! hunk(s) it replaces (bytes proxy tokens), else the ordinary text
//! stays — small singles and tiny sweeps never cost more than the
//! passthrough.
//!
//! Pure detectors over a binary-stripped native diff. Whole-file
//! first: a section whose every pair shares one shape is fully
//! replaced by one semantic hunk (this preserves cross-hunk sweeps).
//! Otherwise each balanced native hunk retries the same detectors
//! independently (T349) and leftovers stay ordinary in original
//! order — native hunks are pairwise disjoint in original
//! coordinates, so a compressed hunk never overlaps a leftover hunk
//! (SPEC 10.6 option-1 merge by construction). Every emission uses
//! the `# scope=file` + `@@ -1,1 @@` dummy, so no original spans are
//! needed. Shapes are probe-verified against `applyrs --check` (see
//! PLAN.md); anything unrecognized stays ordinary passthrough.

/// Try to compress binary-stripped `sections` (one full
/// `diff --git …` section each) into semantic hunks. Returns the
/// compressed patch, or `None` when no section compressed (caller
/// keeps the ordinary passthrough).
pub fn infer_semantic(sections: &[String]) -> Option<String> {
    let mut out = String::new();
    let mut used_rules: Vec<String> = Vec::new();
    let mut compressed_any = false;
    for section in sections {
        match compress_section(section, &mut used_rules) {
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

fn compress_section(section: &str, used_rules: &mut Vec<String>) -> Option<String> {
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
    let path = b_side_path(header)?;
    if !path.ends_with(".rs") {
        return None;
    }
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
        return compress_section_per_hunk(section, &path, used_rules);
    }
    if !whole {
        return compress_section_per_hunk(section, &path, used_rules);
    }
    // N >= 2 heterogeneous fn/method renames compress to one slot
    // sweep (T344); fall through to the call sweep otherwise.
    if let Some(slot) = try_fn_slot_sweep(&minuses, &pluses, &path, used_rules) {
        if slot.len() < section.len() {
            return Some(slot);
        }
        used_rules.truncate(checkpoint);
    }
    // N >= 2 method-to-function fan-outs (T345), method-to-method
    // renames (T347), `&`-receiver strips (T359), and use-path prefix
    // renames (T363); fall through to the bare-call sweep otherwise.
    if let Some(m2f) = try_method_to_fn_sweep(&minuses, &pluses, &path, used_rules) {
        if m2f.len() < section.len() {
            return Some(m2f);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(m2m) = try_method_to_method_sweep(&minuses, &pluses, &path, used_rules) {
        if m2m.len() < section.len() {
            return Some(m2m);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(strip) = try_ref_method_strip_sweep(&minuses, &pluses, &path, used_rules) {
        if strip.len() < section.len() {
            return Some(strip);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(uses) = try_use_prefix_sweep(&minuses, &pluses, &path, used_rules) {
        if uses.len() < section.len() {
            return Some(uses);
        }
        used_rules.truncate(checkpoint);
    }
    if let Some(sweep) = try_call_sweep(&minuses, &pluses, &path, used_rules) {
        if sweep.len() < section.len() {
            return Some(sweep);
        }
        used_rules.truncate(checkpoint);
    }
    // Per-hunk fallback (T349): the whole-section detectors above need
    // every pair in the file to share one shape, so one doc tweak, one
    // pure addition, or one divergent pair anywhere vetoes compression
    // of unrelated sweep runs in other hunks. Retry hunk by hunk with
    // the same detectors: git hunks of one file are pairwise disjoint
    // in original coordinates, so a compressed hunk's changed lines
    // never overlap a leftover hunk's ranges (SPEC 10.6 option-1
    // merge by construction). Residual best-effort risk, same as the
    // whole-section path: a sweep pattern can also match unchanged
    // sites elsewhere — verify with the D.3 round-trip before trusting
    // a compression.
    compress_section_per_hunk(section, &path, used_rules)
}

/// Per-hunk fallback for `compress_section`: compress each balanced
/// native hunk independently with the same detectors, keep the rest as
/// ordinary hunks in original order. Returns `None` when no hunk
/// compressed (caller keeps the ordinary passthrough).
fn compress_section_per_hunk(
    section: &str,
    path: &str,
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
        let checkpoint = used_rules.len();
        let semantic = if minuses.len() == pluses.len() && !minuses.is_empty() {
            if minuses.len() == 1 {
                try_item_rename(minuses[0], pluses[0], path, used_rules)
            } else if let Some(slot) = try_fn_slot_sweep(&minuses, &pluses, path, used_rules) {
                Some(slot)
            } else if let Some(m2f) = try_method_to_fn_sweep(&minuses, &pluses, path, used_rules) {
                Some(m2f)
            } else if let Some(m2m) =
                try_method_to_method_sweep(&minuses, &pluses, path, used_rules)
            {
                Some(m2m)
            } else if let Some(strip) =
                try_ref_method_strip_sweep(&minuses, &pluses, path, used_rules)
            {
                Some(strip)
            } else if let Some(uses) = try_use_prefix_sweep(&minuses, &pluses, path, used_rules) {
                Some(uses)
            } else {
                try_call_sweep(&minuses, &pluses, path, used_rules)
            }
        } else {
            None
        };
        match semantic {
            Some(full) => {
                // Detectors return a full `diff --git` section; keep
                // only the hunk body (everything after the header
                // line) so the reassembled section keeps one header.
                // T375: the body must beat the ordinary hunk it
                // replaces (prelude is shared either way); else the
                // hunk stays ordinary and the rule name is released.
                let body = full.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
                if body.len() < hunk.len() {
                    compressed_any = true;
                    out.push_str(body);
                } else {
                    used_rules.truncate(checkpoint);
                    out.push_str(hunk);
                }
            }
            None => {
                // A failed detector may have reserved a rule name
                // (`try_item_rename` names before its column-0 and
                // generics guards); release it so later hunks in the
                // same file keep tight names.
                used_rules.truncate(checkpoint);
                out.push_str(hunk);
            }
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
fn try_item_rename(
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
fn try_fn_slot_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
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

/// N-site (N ≥ 2) method-to-function fan-out (T345, SPEC 18.5): every
/// pair parses as `recv.old_method(args)` → `new_fn(recv, args)` with
/// one shared method ident, one shared function callee, uniform
/// minus-arity, and a per-site receiver-prepend delta (receiver texts
/// differ per site — `Receiver` binds per site like the engine).
/// Turbofish sites, `&`-led receivers (general-expr routing), and
/// method→method pairs stay passthrough. Emits one `# allow=many` +
/// `# scope=file` hunk (`# expr Receiver` + `# expr E{i}` per arg;
/// arity 0 needs only `Receiver`).
fn try_method_to_fn_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut old_method: Option<String> = None;
    let mut new_func: Option<String> = None;
    let mut arity: Option<usize> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n, a) = parse_method_to_fn_pair(m, p)?;
        if old_method.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_func.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        old_method = Some(o);
        new_func = Some(n);
        arity = Some(a);
    }
    let (old_method, new_func, arity) = (old_method?, new_func?, arity?);
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_func}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = super::git_header(path)
    );
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        hunk.push_str(&format!("# expr {v}\n"));
        vars.push(v);
    }
    let args = vars.join(", ");
    let tail = if args.is_empty() {
        "Receiver".to_string()
    } else {
        format!("Receiver, {args}")
    };
    hunk.push_str(&format!(
        "- Receiver.{old_method}({args})\n+ {new_func}({tail})\n"
    ));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a method-to-function refactor; return
/// (method ident, function callee, minus-arity) when the delta is a
/// receiver prepend (plus `[0]` token-equals the minus receiver, rest
/// token-equal pairwise).
fn parse_method_to_fn_pair(old_line: &str, new_line: &str) -> Option<(String, String, usize)> {
    let mut old_text = old_line.trim();
    if old_text.is_empty() {
        return None;
    }
    if let Some(stripped) = old_text.strip_suffix(';') {
        old_text = stripped.trim_end();
    }
    let mut new_text = new_line.trim();
    if new_text.is_empty() {
        return None;
    }
    if let Some(stripped) = new_text.strip_suffix(';') {
        new_text = stripped.trim_end();
    }
    if old_text.starts_with('&') {
        return None;
    }
    let old_expr: syn::Expr = syn::parse_str(old_text).ok()?;
    let new_expr: syn::Expr = syn::parse_str(new_text).ok()?;
    let syn::Expr::MethodCall(old_call) = &old_expr else {
        return None;
    };
    if old_call.turbofish.is_some() {
        return None;
    }
    let (new_cal, new_args) = call_parts(&new_expr)?;
    if new_args.len() != old_call.args.len() + 1 {
        return None;
    }
    use quote::ToTokens;
    let recv_text = old_call.receiver.to_token_stream().to_string();
    if new_args.is_empty() {
        return None;
    }
    if new_args[0].to_token_stream().to_string() != recv_text {
        return None;
    }
    for (a, b) in old_call.args.iter().zip(new_args.iter().skip(1)) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    let method = old_call.method.to_string();
    if ident_len(&method)? != method.len() {
        return None;
    }
    Some((method, new_cal, old_call.args.len()))
}

/// N-site (N ≥ 2) method-to-method rename fan-out (T347): every pair
/// parses as `recv.old(args)` → `recv.new(args)` with one shared old
/// method ident, one shared new method ident, uniform arity, and a
/// per-site identical receiver and args (receiver texts differ across
/// sites — `Receiver` binds per site like the engine). Turbofish on
/// either side, `&`-led receivers (general-expr routing), and
/// same-method pairs stay passthrough. Emits one `# allow=many` +
/// `# scope=file` hunk (`# expr Receiver` + `# expr E{i}` per arg;
/// arity 0 needs only `Receiver`).
fn try_method_to_method_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut old_method: Option<String> = None;
    let mut new_method: Option<String> = None;
    let mut arity: Option<usize> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (o, n, a) = parse_method_to_method_pair(m, p)?;
        if old_method.as_ref().is_some_and(|c| *c != o) {
            return None;
        }
        if new_method.as_ref().is_some_and(|c| *c != n) {
            return None;
        }
        if arity.is_some_and(|x| x != a) {
            return None;
        }
        old_method = Some(o);
        new_method = Some(n);
        arity = Some(a);
    }
    let (old_method, new_method, arity) = (old_method?, new_method?, arity?);
    if old_method == new_method {
        return None;
    }
    let rule = unique_rule(used_rules, &format!("calls_{old_method}_to_{new_method}"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = super::git_header(path)
    );
    let mut vars = Vec::new();
    for i in 0..arity {
        let v = format!("E{i}");
        hunk.push_str(&format!("# expr {v}\n"));
        vars.push(v);
    }
    let args = vars.join(", ");
    hunk.push_str(&format!(
        "- Receiver.{old_method}({args})\n+ Receiver.{new_method}({args})\n"
    ));
    Some(hunk)
}

/// Parse a `-`/`+` line pair as a method-to-method rename; return
/// (old method, new method, arity) when receivers and args are
/// token-equal pairwise and only the method ident differs.
fn parse_method_to_method_pair(old_line: &str, new_line: &str) -> Option<(String, String, usize)> {
    let mut old_text = old_line.trim();
    if old_text.is_empty() {
        return None;
    }
    if let Some(stripped) = old_text.strip_suffix(';') {
        old_text = stripped.trim_end();
    }
    let mut new_text = new_line.trim();
    if new_text.is_empty() {
        return None;
    }
    if let Some(stripped) = new_text.strip_suffix(';') {
        new_text = stripped.trim_end();
    }
    if old_text.starts_with('&') || new_text.starts_with('&') {
        return None;
    }
    let old_expr: syn::Expr = syn::parse_str(old_text).ok()?;
    let new_expr: syn::Expr = syn::parse_str(new_text).ok()?;
    let (syn::Expr::MethodCall(old_call), syn::Expr::MethodCall(new_call)) = (&old_expr, &new_expr)
    else {
        return None;
    };
    if old_call.turbofish.is_some() || new_call.turbofish.is_some() {
        return None;
    }
    if old_call.args.len() != new_call.args.len() {
        return None;
    }
    use quote::ToTokens;
    if old_call.receiver.to_token_stream().to_string()
        != new_call.receiver.to_token_stream().to_string()
    {
        return None;
    }
    for (a, b) in old_call.args.iter().zip(new_call.args.iter()) {
        if a.to_token_stream().to_string() != b.to_token_stream().to_string() {
            return None;
        }
    }
    let old_method = old_call.method.to_string();
    let new_method = new_call.method.to_string();
    if ident_len(&old_method)? != old_method.len() {
        return None;
    }
    if ident_len(&new_method)? != new_method.len() {
        return None;
    }
    Some((old_method, new_method, old_call.args.len()))
}

/// N-site (N ≥ 2) `&`-receiver strip fan-out (T359): every pair
/// contains exactly one `&recv.method(args)` sub-expression (a shared
/// reference to a turbofish-free method call) and the plus side equals
/// the minus side with that sub-expression replaced by `recv`; the
/// method ident and the arg token-texts are shared across sites
/// (typically literals like `""`), so the emitted hunk binds them
/// verbatim and can never over-match `&dir.join("repo")`-style sites.
/// Bare arg-position lines carry a trailing `,` and call-nested sites
/// a trailing `;`: one trailing `;` (else one `,`) is stripped before
/// parsing. Emits one `# allow=many` + `# scope=file` general-expr
/// hunk (`# expr Receiver` only); a lone site stays ordinary (no token
/// win), as do turbofish, `&mut`, explicit-lifetime, divergent, and
/// plus-is-not-receiver pairs.
fn try_ref_method_strip_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut method: Option<String> = None;
    let mut args: Option<Vec<String>> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (meth, argv) = parse_ref_strip_pair(m, p)?;
        if method.as_ref().is_some_and(|c| *c != meth) {
            return None;
        }
        if args.as_ref().is_some_and(|a| *a != argv) {
            return None;
        }
        method = Some(meth);
        args = Some(argv);
    }
    let (method, args) = (method?, args?);
    let rule = unique_rule(used_rules, &format!("calls_{method}_strip"));
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n",
        header = super::git_header(path)
    );
    hunk.push_str(&format!(
        "- &Receiver.{method}({})\n+ Receiver\n",
        args.join(", ")
    ));
    Some(hunk)
}

/// Parse one `-`/`+` line pair as a `&`-receiver strip: the minus side
/// parses as an expr containing exactly one shared-reference-to-method
/// call, and the plus side is the minus with that span replaced by the
/// receiver. Returns (method ident, arg token-texts) on success.
fn parse_ref_strip_pair(old_line: &str, new_line: &str) -> Option<(String, Vec<String>)> {
    let minus = strip_one_terminator(old_line.trim())?;
    let plus = strip_one_terminator(new_line.trim())?;
    if minus.is_empty() || plus.is_empty() {
        return None;
    }
    let minus_expr: syn::Expr = syn::parse_str(minus).ok()?;
    let mut hits: Vec<RefStripHit> = Vec::new();
    collect_ref_strips(&minus_expr, &mut hits);
    if hits.len() != 1 {
        return None;
    }
    let hit = hits.pop()?;
    // Whitespace-flat compare: `ToTokens` spacing never matches source
    // spacing, and source lines carry indentation the pattern drops.
    // Explicit lifetimes (`&'a r.m(..)`) never qualify: `syn` drops the
    // lifetime, so the token text cannot prove its absence — check raw.
    if flat_ws(&hit.cand).contains("&'") {
        return None;
    }
    if count_occurrences(&flat_ws(minus), &flat_ws(&hit.cand)) != 1 {
        return None;
    }
    if flat_ws(minus).replacen(&flat_ws(&hit.cand), &flat_ws(&hit.recv), 1) != flat_ws(plus) {
        return None;
    }
    Some((hit.method, hit.args))
}

/// One `&recv.method(args)` span: token-texts for the containment
/// check plus the shared method/args for cross-site agreement.
struct RefStripHit {
    method: String,
    cand: String,
    recv: String,
    args: Vec<String>,
}

/// Walk an expr collecting every turbofish-free shared reference to a
/// method call (`&recv.method(args)`); explicit lifetimes and `&mut`
/// never qualify.
fn collect_ref_strips(expr: &syn::Expr, out: &mut Vec<RefStripHit>) {
    use quote::ToTokens;
    use syn::visit::Visit;
    struct Collector<'a>(&'a mut Vec<RefStripHit>);
    impl<'ast> syn::visit::Visit<'ast> for Collector<'_> {
        fn visit_expr_reference(&mut self, node: &'ast syn::ExprReference) {
            if node.mutability.is_none() {
                if let syn::Expr::MethodCall(call) = node.expr.as_ref() {
                    if call.turbofish.is_none() {
                        self.0.push(RefStripHit {
                            method: call.method.to_string(),
                            cand: node.to_token_stream().to_string(),
                            recv: call.receiver.to_token_stream().to_string(),
                            args: call
                                .args
                                .iter()
                                .map(|a| a.to_token_stream().to_string())
                                .collect(),
                        });
                    }
                }
            }
            syn::visit::visit_expr_reference(self, node);
        }
    }
    Collector(out).visit_expr(expr);
}

/// Strip one trailing statement/arg terminator (`;`, else `,`) for
/// shape parsing; the emitted hunk matches sub-expressions, so the
/// terminator never reaches the pattern.
fn strip_one_terminator(text: &str) -> Option<&str> {
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

/// Whitespace-insensitive text for detection-time containment and
/// replacement checks (the engine re-verifies structurally at apply).
fn flat_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
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
fn try_use_prefix_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
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
fn try_call_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
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
fn call_parts(expr: &syn::Expr) -> Option<(String, Vec<syn::Expr>)> {
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

fn unique_rule(used: &mut Vec<String>, base: &str) -> String {
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

fn b_side_path(header: &str) -> Option<String> {
    // Shared space-aware splitter (T379/T380): real git never quotes
    // spaces, so the old `splitn(2, ' ')` mis-split them (space-path
    // `.rs` files silently never compressed).
    let header = header.lines().next().unwrap_or(header);
    let rest = header.strip_prefix("diff --git ")?;
    rsmpl_patch::split_git_paths(rest).map(|parts| {
        let side = parts.last().cloned().unwrap_or_default();
        // T399: no `trim_matches('"')` — the splitter already strips
        // only the outer C-quote pair (genuine edge quotes survive).
        side.strip_prefix("b/").unwrap_or(&side).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn validated(patch: &str) {
        rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
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
        let out = infer_semantic(&[raw.to_string()]).expect("must compress");
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- foo()\n"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn call_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    old_function(first, second);\n-    old_function(third, fourth);\n-    old_function(fifth, sixth);\n+    new_function(first, second);\n+    new_function(third, fourth);\n+    new_function(fifth, sixth);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- old_function(E0, E1)\n"), "got: {out}");
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
    }

    #[test]
    fn call_sweep_split_hunks_pair_in_order() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-    old_function(first_argument);\n+    new_function(first_argument);\n@@ -9,2 +9,2 @@\n-    old_function(second_argument);\n+    new_function(second_argument);\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
    fn method_calls_divergent_new_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar(1);\n+    b.baz(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_sweep_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n+    new_function(a, 1);\n+    new_function(b, 2);\n+    new_function(c, 3);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ new_function(Receiver, E0)\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn method_to_fn_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    source.old_operation(first, second);\n-    target.old_operation(third, fourth);\n-    other.old_operation(fifth, sixth);\n+    new_operation(source, first, second);\n+    new_operation(target, third, fourth);\n+    new_operation(other, fifth, sixth);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
        assert!(
            out.contains("- Receiver.old_operation(E0, E1)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ new_operation(Receiver, E0, E1)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn method_to_fn_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    source.old_operation();\n-    target.old_operation();\n-    other.old_operation();\n+    new_operation(source);\n+    new_operation(target);\n+    new_operation(other);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- Receiver.old_operation()\n"), "got: {out}");
        assert!(out.contains("+ new_operation(Receiver)\n"), "got: {out}");
    }

    #[test]
    fn method_to_fn_divergent_method_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.other(2);\n+    new_fn(a, 1);\n+    new_fn(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_divergent_callee_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.old(2);\n+    new_fn(a, 1);\n+    other_fn(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_receiver_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1);\n-    b.old(2);\n+    new_fn(x, 1);\n+    new_fn(y, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_fn_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old::<u8>(1);\n-    b.old::<u8>(2);\n+    new_fn(a, 1);\n+    new_fn(b, 2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_sweep_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,6 +1,6 @@\n fn m() {\n-    a.old_method(1);\n-    b.old_method(2);\n-    c.old_method(3);\n-    d.old_method(4);\n+    a.new_method(1);\n+    b.new_method(2);\n+    c.new_method(3);\n+    d.new_method(4);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(out.contains("- Receiver.old_method(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_method(E0)\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn method_to_method_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_method(first, second);\n-    b.old_method(third, fourth);\n-    c.old_method(fifth, sixth);\n+    a.new_method(first, second);\n+    b.new_method(third, fourth);\n+    c.new_method(fifth, sixth);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
        assert!(
            out.contains("- Receiver.old_method(E0, E1)\n"),
            "got: {out}"
        );
        assert!(
            out.contains("+ Receiver.new_method(E0, E1)\n"),
            "got: {out}"
        );
    }

    #[test]
    fn method_to_method_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.old_operation();\n-    b.old_operation();\n-    c.old_operation();\n+    a.new_operation();\n+    b.new_operation();\n+    c.new_operation();\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- Receiver.old_operation()\n"), "got: {out}");
        assert!(out.contains("+ Receiver.new_operation()\n"), "got: {out}");
    }

    #[test]
    fn method_to_method_divergent_old_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.other(2);\n+    a.bar(1);\n+    b.bar(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_receiver_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    x.bar(1);\n+    y.bar(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_arg_mismatch_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar(1);\n+    b.bar(3);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n+    a.bar::<u8>(1);\n+    b.bar::<u8>(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn method_to_method_ref_receiver_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.foo(1);\n-    &b.foo(2);\n+    &a.bar(1);\n+    &b.bar(2);\n }\n",
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(&[a, b]).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
    fn ref_strip_sweep_bare_and_nested_compresses() {
        // T359 shape: bare arg-position lines (trailing `,`) and a
        // `write(...)`-nested site share `&R.join("")` -> `R`.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    &dir.path().join(\"\"),\n-    &dir.path().join(\"\"),\n-    write(&dir.path().join(\"\"), \"a\");\n+    dir.path(),\n+    dir.path(),\n+    write(dir.path(), \"a\");\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- &Receiver.join(\"\")\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_shared_nonempty_arg_compresses() {
        // Shared literal args are the design point, not a hazard:
        // `&d.join("repo")` -> `d` at two sites compresses.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    copy_tree(&a.join(\"repo\"), x);\n-    copy_tree(&b.join(\"repo\"), y);\n+    copy_tree(a, x);\n+    copy_tree(b, y);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- &Receiver.join(\"repo\")\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_zero_arg_compresses() {
        // Empty arg list renders as `()` with no metavars.
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n fn m() {\n-    &alpha.shutdown();\n-    &beta.shutdown();\n-    &gamma.shutdown();\n+    alpha;\n+    beta;\n+    gamma;\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- &Receiver.shutdown()\n"), "got: {out}");
        assert!(out.contains("+ Receiver\n"), "got: {out}");
    }

    #[test]
    fn ref_strip_sweep_divergent_args_stay_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join(\"\");\n-    &b.join(\"x\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_divergent_method_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join(\"\");\n-    &b.push(\"\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_plus_not_receiver_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join(\"\");\n-    &b.join(\"\");\n+    a;\n+    c;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_turbofish_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &a.join::<T>(\"\");\n-    &b.join::<T>(\"\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_mut_ref_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    &mut a.join(\"\");\n-    &mut b.join(\"\");\n+    a;\n+    b;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn ref_strip_sweep_single_site_stays_passthrough() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    &a.join(\"\");\n+    a;\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn use_prefix_sweep_basic_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use crate::old::Thing;\n- use crate::old::Other;\n+ use crate::new::Thing;\n+ use crate::new::Other;\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("rule=use_old_to_new"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn use_prefix_sweep_leading_colons_tolerated() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n- use ::crate::old::A;\n- use ::crate::old::B;\n+ use ::crate::new::A;\n+ use ::crate::new::B;\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
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
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- use crate::old\n"), "got: {out}");
        assert!(out.contains("+ use crate::new\n"), "got: {out}");
    }
}
