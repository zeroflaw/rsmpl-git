//! v1 semantic inference: fn/item rename + slot-sweep + call-sweep +
//! method-to-fn and method-to-method fan-outs.
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
    if !header.starts_with("diff --git ") {
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
    if whole && minuses.len() == 1 {
        // Single pair: fn/item rename only (a lone call rewrite has
        // no token win over the ordinary hunk; a lone heterogeneous
        // fn header likewise costs more as a slot hunk than ordinary).
        if let Some(rename) = try_item_rename(minuses[0], pluses[0], &path, used_rules) {
            return Some(rename);
        }
        return compress_section_per_hunk(section, &path, used_rules);
    }
    if !whole {
        return compress_section_per_hunk(section, &path, used_rules);
    }
    // N >= 2 heterogeneous fn/method renames compress to one slot
    // sweep (T344); fall through to the call sweep otherwise.
    if let Some(slot) = try_fn_slot_sweep(&minuses, &pluses, &path, used_rules) {
        return Some(slot);
    }
    // N >= 2 method-to-function fan-outs (T345) and method-to-method
    // renames (T347); fall through to the bare-call sweep otherwise.
    if let Some(m2f) = try_method_to_fn_sweep(&minuses, &pluses, &path, used_rules) {
        return Some(m2f);
    }
    if let Some(m2m) = try_method_to_method_sweep(&minuses, &pluses, &path, used_rules) {
        return Some(m2m);
    }
    if let Some(sweep) = try_call_sweep(&minuses, &pluses, &path, used_rules) {
        return Some(sweep);
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
            } else {
                try_call_sweep(&minuses, &pluses, path, used_rules)
            }
        } else {
            None
        };
        match semantic {
            Some(full) => {
                compressed_any = true;
                // Detectors return a full `diff --git` section; keep
                // only the hunk body (everything after the header
                // line) so the reassembled section keeps one header.
                let body = full.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
                out.push_str(body);
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
    let mut hunk = format!("diff --git a/{path} b/{path}\n@@ -1,1 @@ rule={rule}\n# scope=file\n");
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
        "diff --git a/{path} b/{path}\n@@ -1,1 @@ rule={rule}\n# scope=file\n# allow=many\n# visibility V\n# qualifier_list Q\n# generic_params G\n# where_clause W\n"
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
        "diff --git a/{path} b/{path}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n"
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
        "diff --git a/{path} b/{path}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n# expr Receiver\n"
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
        "diff --git a/{path} b/{path}\n@@ -1,1 @@ rule={rule}\n# allow=many\n# scope=file\n"
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

fn b_side_path(header: &str) -> Option<String> {
    let rest = header.strip_prefix("diff --git ")?;
    let mut parts = rest.splitn(2, ' ');
    parts.nth(1).map(|b| {
        b.strip_prefix("b/")
            .unwrap_or(b)
            .trim_matches('"')
            .to_string()
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
    fn fn_rename_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn foo() {\n+fn bar() {\n     1\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- fn foo(...) {"), "got: {out}");
        assert!(out.contains("+ fn bar(...) {"), "got: {out}");
        assert!(out.contains("# scope=file"), "got: {out}");
        assert!(!out.contains("-fn foo"), "got: {out}");
    }

    #[test]
    fn struct_rename_compresses() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-struct Old {\n+struct New {\n     x: i32,\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- struct Old\n"), "got: {out}");
        assert!(out.contains("+ struct New\n"), "got: {out}");
    }

    #[test]
    fn pub_struct_rename_echoes_vis() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-pub struct Old {\n+pub struct New {\n     x: i32,\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- pub struct Old\n"), "got: {out}");
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
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1, x);\n-    foo(2, y);\n+    bar(1, x);\n+    bar(2, y);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("- foo(E0, E1)\n"), "got: {out}");
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
    }

    #[test]
    fn call_sweep_split_hunks_pair_in_order() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n-    foo(1);\n+    bar(1);\n@@ -9,2 +9,2 @@\n-    foo(2);\n+    bar(2);\n",
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
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old(1, x);\n-    b.old(2, y);\n+    new_fn(a, 1, x);\n+    new_fn(b, 2, y);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
        assert!(out.contains("- Receiver.old(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ new_fn(Receiver, E0, E1)\n"), "got: {out}");
    }

    #[test]
    fn method_to_fn_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.old();\n-    b.old();\n+    new_fn(a);\n+    new_fn(b);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- Receiver.old()\n"), "got: {out}");
        assert!(out.contains("+ new_fn(Receiver)\n"), "got: {out}");
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
            "@@ -1,5 +1,5 @@\n fn m() {\n-    a.foo(1);\n-    b.foo(2);\n-    c.foo(3);\n+    a.bar(1);\n+    b.bar(2);\n+    c.bar(3);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# allow=many"), "got: {out}");
        assert!(out.contains("# expr Receiver\n"), "got: {out}");
        assert!(out.contains("- Receiver.foo(E0)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.bar(E0)\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
    }

    #[test]
    fn method_to_method_sweep_multi_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo(1, x);\n-    b.foo(2, y);\n+    a.bar(1, x);\n+    b.bar(2, y);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# expr E0\n# expr E1\n"), "got: {out}");
        assert!(out.contains("- Receiver.foo(E0, E1)\n"), "got: {out}");
        assert!(out.contains("+ Receiver.bar(E0, E1)\n"), "got: {out}");
    }

    #[test]
    fn method_to_method_sweep_zero_arg() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    a.foo();\n-    b.foo();\n+    a.bar();\n+    b.bar();\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(!out.contains("# expr E"), "got: {out}");
        assert!(out.contains("- Receiver.foo()\n"), "got: {out}");
        assert!(out.contains("+ Receiver.bar()\n"), "got: {out}");
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
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn foo(x: i32) -> i32 {\n-async fn foo(x: i32) -> i32 {\n+fn bar(x: i32) -> i32 {\n+async fn bar(x: i32) -> i32 {\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("# visibility V\n"), "got: {out}");
        assert!(out.contains("# qualifier_list Q\n"), "got: {out}");
        assert!(out.contains("- V Q fn foo G (...) W {\n"), "got: {out}");
        assert!(out.contains("+ V Q fn bar G (...) W {\n"), "got: {out}");
        assert_eq!(out.matches("@@ -1,1 @@").count(), 1, "got: {out}");
        assert!(
            !out.contains("impl Svc"),
            "free sweep needs no impl wrapper, got: {out}"
        );
    }

    #[test]
    fn method_slot_sweep_compresses_with_impl_wrapper() {
        let raw = section(
            "f.rs",
            "@@ -1,5 +1,5 @@\n impl T {\n-    fn foo(&self) {\n-    pub fn foo(&self) {\n+    fn bar(&self) {\n+    pub fn bar(&self) {\n     }\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated(&out);
        assert!(out.contains("impl Svc {\n"), "got: {out}");
        assert!(out.contains("-     V Q fn foo G (...) W {\n"), "got: {out}");
        assert!(out.contains("+     V Q fn bar G (...) W {\n"), "got: {out}");
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
            "@@ -1,2 +1,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        );
        let b = section(
            "b.rs",
            "@@ -1,2 +1,2 @@\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n",
        );
        let out = infer_semantic(&[a, b]).unwrap();
        validated(&out);
        assert!(out.contains("rule=calls_foo_to_bar\n"), "got: {out}");
        assert!(out.contains("rule=calls_foo_to_bar_2\n"), "got: {out}");
    }

    #[test]
    fn per_hunk_sweep_compresses_beside_ordinary_hunk() {
        // Mixed file: hunk 1 is a 2-site call sweep, hunk 2 a body
        // edit. The whole-section detectors stay passthrough; the
        // per-hunk fallback (T349) compresses hunk 1 and keeps hunk 2
        // ordinary.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n }\n@@ -10,3 +10,3 @@\n fn n() {\n-    1\n+    2\n }\n",
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
            "@@ -1,4 +1,4 @@\n fn m() {\n-    foo(1);\n-    foo(2);\n+    bar(1);\n+    bar(2);\n }\n@@ -6,0 +7,2 @@\n+fn added() {}\n+// new\n",
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
}
