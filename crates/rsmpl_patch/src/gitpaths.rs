//! Git path/header helpers for RSmPL-Git patch files (SPEC section 6).
//!
//! Split from `parser.rs` (WI-3c): `diff --git` header splitting,
//! C-quote decoding/encoding, and side-prefix handling live here;
//! hunk/file-section parsing stays in `parser.rs`. Pure move, no
//! behavior change. `parser.rs` re-exports the moved public items so
//! existing import paths (`crate::parser::X`, `rsmpl_patch::X`) keep
//! working.

pub(crate) fn parse_diff_git_header(line: &str) -> Option<(String, String)> {
    // T385: tolerate a CRLF-authored header line — the trailing `\r`
    // would otherwise embed into a single-path fallback (whitespace
    // splitting already tolerates it for shaped headers).
    let line = line.strip_suffix('\r').unwrap_or(line);
    let rest = line.strip_prefix("diff --git ")?;
    // A single path (`diff --git <path>`, SPEC 6) is equivalent to the
    // duplicated two-path form; the two-path form stays for renames/compat.
    let parts = split_git_paths(rest)?;
    if parts.len() == 1 {
        return Some((parts[0].clone(), parts[0].clone()));
    }
    if parts.len() != 2 {
        return None;
    }
    Some((parts[0].clone(), parts[1].clone()))
}

/// Split the rest of a `diff --git <old> <new>` header into paths,
/// honoring quotes. Real git never quotes spaces, so unquoted paths may
/// contain them (`diff --git a/my file.txt b/my file.txt`, T379): when
/// whitespace-splitting yields more than two parts, rejoin at the first
/// `b/`-led part (the joined old side must start with `a/`), else split
/// at a `/dev/null` create/delete half, else fail (error as before).
pub fn split_git_paths(rest: &str) -> Option<Vec<String>> {
    let parts = split_paths(rest)?;
    if parts.len() == 1 {
        return Some(parts);
    }
    // Identical duplicated bare path (`diff --git A A`, no `a/`/`b/`
    // uniform) is the single path `A`, not a file literally named
    // `A A` - quote (`diff --git "A A"`) to name such a file.
    // `/dev/null` halves never collapse (create/delete keeps halves).
    if parts.len() >= 2
        && parts.len() % 2 == 0
        && !parts.iter().any(|p| is_dev_null(p))
        && parts[..parts.len() / 2] == parts[parts.len() / 2..]
    {
        return Some(vec![parts[..parts.len() / 2].join(" ")]);
    }
    if parts.len() == 2 {
        // T381: a two-part split is only a two-path header when it wears
        // git's `a/`/`b/` (or `/dev/null`) uniform. Otherwise the whole
        // remainder is one single-path (SPEC 6) file with a space —
        // previously misread as a rename of the first word into the
        // second (wrong-file hazard: `diff --git my file.rs` targeted
        // `file.rs`). Bare two-word remainders that are not git-shaped
        // (e.g. `diff --git foo bar`) are likewise one path — real git
        // always emits the `a/`/`b/` uniform for pairs.
        let old_git = parts[0].starts_with("a/") || is_dev_null(&parts[0]);
        let new_git = parts[1].starts_with("b/") || is_dev_null(&parts[1]);
        if old_git && new_git {
            return Some(parts);
        }
        return Some(vec![rest.to_string()]);
    }
    // T397: the true path itself may contain a `b/…` segment
    // (`diff --git a/a b/c d/e f.txt b/a b/c d/e f.txt`), so the
    // first `b/`-led part is not always the split. Bare git resolves
    // these to the split whose stripped sides are EQUAL (verified via
    // `apply --stat`), so prefer that candidate; differing halves
    // (renames) keep the first split — their exact paths come from
    // the `rename from/to` lines (see the metadata branch above).
    let mut first: Option<Vec<String>> = None;
    let mut saw_b = false;
    for (i, p) in parts.iter().enumerate() {
        if !p.starts_with("b/") {
            continue;
        }
        saw_b = true;
        if i == 0 || !parts[..i].join(" ").starts_with("a/") {
            continue;
        }
        let pair = vec![parts[..i].join(" "), parts[i..].join(" ")];
        if first.is_none() {
            first = Some(pair.clone());
        }
        if strip_ab_prefix(&pair[0]) == strip_ab_prefix(&pair[1]) {
            return Some(pair);
        }
    }
    if let Some(pair) = first {
        return Some(pair);
    }
    if saw_b {
        return None;
    }
    if parts.len() > 2 && is_dev_null(&parts[parts.len() - 1]) {
        return Some(vec![
            parts[..parts.len() - 1].join(" "),
            parts[parts.len() - 1].clone(),
        ]);
    }
    if is_dev_null(&parts[0]) {
        return Some(vec![parts[0].clone(), parts[1..].join(" ")]);
    }
    // T382: the T381 rule generalized — a multi-part remainder with no
    // git-shaped part at all (no `a/`/`b/` uniform, no `/dev/null` half)
    // is one single path with spaces (SPEC 6). Anything claiming a git
    // shape but malformed (e.g. `diff --git a/x y z`) still fails fast.
    if parts
        .iter()
        .any(|p| p.starts_with("a/") || p.starts_with("b/") || is_dev_null(p))
    {
        return None;
    }
    Some(vec![rest.to_string()])
}

/// Collapse a `diff --git a/P b/P` header line to the single-path
/// `diff --git P` spelling (SPEC 6 equivalence, T381) when `P` re-parses
/// as the same target; renames, `/dev/null` halves, quoted headers, and
/// paths whose single form would re-parse differently keep the two-path
/// spelling (e.g. a file literally named `a/x b/y`, or `a/b` where the
/// single form's `a/` prefix would strip to a different file).
/// Non-header lines pass through unchanged.
pub fn collapse_git_header(line: &str) -> String {
    let Some(rest) = line.strip_prefix("diff --git ") else {
        return line.to_string();
    };
    if rest.contains('"') || rest.contains('\'') {
        return line.to_string();
    }
    let parts = match split_git_paths(rest) {
        Some(p) if p.len() == 2 => p,
        _ => return line.to_string(),
    };
    if is_dev_null(&parts[0]) || is_dev_null(&parts[1]) {
        return line.to_string();
    }
    let old = strip_ab_prefix(&parts[0]);
    if old != strip_ab_prefix(&parts[1]) || is_dev_null(&old) {
        return line.to_string();
    }
    // Round-trip guard: the single form must parse back to the same
    // file (same split, non-rename, same target).
    let single = format!("diff --git {old}");
    match parse_diff_git_header(&single) {
        Some((o, n)) if o == n && target_path(&n, &o) == old => single,
        _ => line.to_string(),
    }
}

fn is_dev_null(p: &str) -> bool {
    p == "/dev/null" || p == "dev/null"
}

fn split_paths(s: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    let mut in_q = false;
    let mut q = '"';
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == q {
                in_q = false;
            } else if c == '\\' {
                // T389: git C-quotes non-ASCII bytes (`"caf\303\251.txt"`);
                // decode escapes inside double quotes so the target is
                // the real filename, not the backslash-stripped text.
                // Single-quote spans keep the legacy lenient rule
                // (backslash quotes the next char verbatim).
                if q == '"' {
                    if let Some(n) = chars.next() {
                        push_c_escape(&mut cur, &mut chars, n);
                    }
                } else if let Some(n) = chars.next() {
                    push_str_byte(&mut cur, n);
                }
            } else {
                push_str_byte(&mut cur, c);
            }
        } else if c == '"' || c == '\'' {
            in_q = true;
            q = c;
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(bytes_to_path(&cur));
                cur.clear();
            }
        } else {
            push_str_byte(&mut cur, c);
        }
    }
    if !cur.is_empty() {
        out.push(bytes_to_path(&cur));
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Decode one C-style escape after a backslash inside a double-quoted
/// git path (`\"`, `\\`, `\a\b\f\n\r\t\v`, octal `\NNN`); anything
/// else quotes the char verbatim (the pre-T389 rule).
fn push_c_escape(cur: &mut Vec<u8>, chars: &mut std::iter::Peekable<std::str::Chars>, n: char) {
    let byte = match n {
        'a' => Some(0x07),
        'b' => Some(0x08),
        'f' => Some(0x0C),
        'n' => Some(b'\n'),
        'r' => Some(b'\r'),
        't' => Some(b'\t'),
        'v' => Some(0x0B),
        '\\' => Some(b'\\'),
        '"' => Some(b'"'),
        '0'..='7' => {
            let mut val = n as u32 - '0' as u32;
            for _ in 0..2 {
                match chars.peek() {
                    Some(&d) if ('0'..='7').contains(&d) => {
                        val = val * 8 + (d as u32 - '0' as u32);
                        chars.next();
                    }
                    _ => break,
                }
            }
            Some((val & 0xFF) as u8)
        }
        _ => None,
    };
    match byte {
        Some(b) => cur.push(b),
        None => push_str_byte(cur, n),
    }
}

fn push_str_byte(cur: &mut Vec<u8>, c: char) {
    let mut buf = [0u8; 4];
    cur.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}

fn bytes_to_path(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Decode a `git status --porcelain` path token: double-quoted tokens
/// are C-quoted (`?? "caf\303\251.txt"`, T389) and decode to the real
/// filename; unquoted tokens (spaces included — git never quotes
/// those) pass through byte-exact.
pub fn decode_git_path_token(token: &str) -> String {
    let inner = match token.strip_prefix('"') {
        Some(rest) => match rest.strip_suffix('"') {
            Some(inner) => inner,
            None => return token.to_string(),
        },
        None => return token.to_string(),
    };
    let mut cur = Vec::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(n) => push_c_escape(&mut cur, &mut chars, n),
                None => cur.push(b'\\'),
            }
        } else {
            push_str_byte(&mut cur, c);
        }
    }
    bytes_to_path(&cur)
}

/// Quote a header-slot path for emission (`diff --git`/`---`/`+++`/
/// `rename from/to`, T398): bare git only parses unquoted paths when
/// they hold no whitespace/control characters, so a slot containing a
/// byte `< 0x20`, `0x7f`, `"` or `\` is emitted C-quoted — exactly
/// what git's own quotepath produces (spaces and non-ASCII stay raw:
/// the native shape, parseable unquoted per T379/T389). Every escape
/// produced here round-trips through `push_c_escape`.
pub fn quote_git_path(slot: &str) -> String {
    if !slot
        .chars()
        .any(|c| c < '\u{20}' || c == '\u{7f}' || c == '"' || c == '\\')
    {
        return slot.to_string();
    }
    let mut out = String::with_capacity(slot.len() + 2);
    out.push('"');
    for c in slot.chars() {
        match c {
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c < '\u{20}' || c == '\u{7f}' => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("\\{b:03o}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn strip_ab_prefix(p: &str) -> String {
    if let Some(s) = p.strip_prefix("a/") {
        return s.to_string();
    }
    if let Some(s) = p.strip_prefix("b/") {
        return s.to_string();
    }
    if let Some(s) = p.strip_prefix('"') {
        let _ = s;
    }
    p.to_string()
}

/// Re-attach the `diff --git` side uniform to a `rename from/to` value
/// (T397): native git emits these without a prefix (`rename to new d`),
/// while section halves wear `a/`/`b/` everywhere downstream — so
/// prepend unless already present (hand-authored prefixed values and
/// `/dev/null` pass through untouched). C-quoted values decode via the
/// shared quotepath rule; raw values (spaces included) pass byte-exact.
pub(crate) fn with_ab_prefix(p: &str, side: char) -> String {
    let decoded = decode_git_path_token(p);
    let want = if side == 'a' { "a/" } else { "b/" };
    if decoded == "/dev/null" || decoded == "dev/null" || decoded.starts_with(want) {
        return decoded;
    }
    format!("{want}{decoded}")
}

pub(crate) fn target_path(new_path: &str, old_path: &str) -> String {
    if new_path == "/dev/null" || new_path == "dev/null" {
        return strip_ab_prefix(old_path);
    }
    strip_ab_prefix(new_path)
}
