//! WI-5 slice 1: `fn` rename + single vis/qual joint helpers.
//!
//! The detection branch lives in the item-intent pre-pass (it owns the
//! pair/hunk tables); this module holds only the pure helpers so the
//! intent file stays under the D5 line gate: the joint classifier (one
//! evolving header slot alongside the name), the column-0 `fn`-name
//! twin counter, and the joint follow emission (concrete evolved terse
//! pair, `# sweep` + `# follow=uses` — never a `V`/`Q` hole, per the
//! WI-1 law: the intent proved one concrete evolution).
//!
//! Only the `fn` arm lands here: `ItemRename` rejects prefix deltas at
//! pattern compile (`item rename prefixes differ`), and `matcher_item`
//! only replaces the ident span, so bare-item rename+vis/qual joints
//! need engine prefix-evolution support first (WI-5-R2). Unknown
//! qualifier tokens beyond `const`/`async`/`unsafe`/`extern ["ABI"]`
//! stay ordinary (future-qualifier residue); multi-line headers classify
//! on their first line under the same one-slot law (WI-5-R3).

use std::collections::{HashMap, HashSet};

use super::generalized::parse_sig_slots;
use super::generalized::SigSlots;

/// A validated `fn` rename + single-slot-evolution def pair: raw
/// prefixes (original text before the `fn` keyword, trailing space
/// included, `""` when bare) plus the old/new names.
pub(crate) struct FnVisqualJoint {
    pub(crate) old_prefix: String,
    pub(crate) new_prefix: String,
    pub(crate) old: String,
    pub(crate) new: String,
}

/// Classify a native pair as a `fn` rename + exactly one evolving
/// header slot ({vis} xor {qualifier-list}): `Some` when both lines name
/// differing column-0 `fn`s with all other slots token-equal, else `None`.
/// Single-line `{`-terminated headers compare full signature slots;
/// multi-line headers (WI-5-R3) classify on their first line — the pure
/// arm already raises `split_delta` first-lines, so the joint takes the
/// same law with the vis/qual evolution, and the post-name first-line
/// tails must be token-equal (the D13 joint probe owns the remainder
/// lines). Two evolving slots, any generics/params/return/`where` delta,
/// unknown qualifier tokens, self-referential vis, indented (method)
/// headers, `;` decls, reflowed headers (brace parity mixed), and twin
/// definitions all decline (never a partial claim).
pub(crate) fn classify_fn_visqual_joint(minus: &str, plus: &str) -> Option<FnVisqualJoint> {
    if minus.trim_end() == plus.trim_end() {
        return None;
    }
    // WI-19c posture: indented `fn` lines are methods, never intents.
    if minus.starts_with([' ', '\t']) || plus.starts_with([' ', '\t']) {
        return None;
    }
    let m_brace = minus.contains('{');
    let p_brace = plus.contains('{');
    if m_brace != p_brace {
        // Reflowed header (one side wrapped, the other not), never a joint.
        return None;
    }
    if !m_brace {
        // WI-5-R3 fragment law: braceless first lines (`fn old(` …). `;`
        // decls are never a def first line; the tails after the name
        // stand in for the slot-equality checks on the visible fragment.
        if minus.contains(';') || plus.contains(';') {
            return None;
        }
        let (old_prefix, old, old_tail) = split_fn_needle(minus.trim_end())?;
        let (new_prefix, new, new_tail) = split_fn_needle(plus.trim_end())?;
        if old.is_empty() || new.is_empty() || old == new {
            return None;
        }
        if norm(&old_tail) != norm(&new_tail) {
            return None;
        }
        return finish_fn_visqual_joint(old_prefix, new_prefix, old, new);
    }
    let m = header_slots(minus)?;
    let p = header_slots(plus)?;
    if m.name.is_empty() || p.name.is_empty() || m.name == p.name {
        return None;
    }
    let vis_diff = norm(&m.vis) != norm(&p.vis);
    let qual_diff = norm(&m.qual) != norm(&p.qual);
    // Exactly one evolving slot: zero is the pure-rename path (never
    // claimed here), two vetoes to the signature domain / ordinary.
    // (Checked again inside `finish_fn_visqual_joint` on the raw
    // prefixes; the slot compare here is the cheap structural gate.)
    if vis_diff == qual_diff {
        return None;
    }
    if norm(&m.generics) != norm(&p.generics) {
        return None;
    }
    if norm(&m.params) != norm(&p.params) {
        return None;
    }
    if norm(&m.ret) != norm(&p.ret) {
        return None;
    }
    if norm(&m.where_) != norm(&p.where_) {
        return None;
    }
    let old_prefix = raw_fn_prefix(minus.trim_end(), &m.name)?;
    let new_prefix = raw_fn_prefix(plus.trim_end(), &p.name)?;
    finish_fn_visqual_joint(old_prefix, new_prefix, m.name, p.name)
}

/// Shared one-slot law over raw header prefixes: exactly one of {vis,
/// qualifier-list} evolves (zero is the pure-rename path, never claimed
/// here; two vetoes to the signature domain / ordinary); both vis
/// spellings stay in the WI-1 closed set, qualifier tokens stay inside
/// `const`/`async`/`unsafe`/`extern ["ABI"]`, and self-referential vis
/// (`pub(in old)`) vetoes (WI-4 domain). The caller proved the remaining
/// slots token-equal (slot compare on `{`-headers, tail compare on
/// first-line fragments).
fn finish_fn_visqual_joint(
    old_prefix: String,
    new_prefix: String,
    old: String,
    new: String,
) -> Option<FnVisqualJoint> {
    let (old_vis, old_qual) = split_vis_qual_prefix(&old_prefix)?;
    let (new_vis, new_qual) = split_vis_qual_prefix(&new_prefix)?;
    if (norm(&old_vis) != norm(&new_vis)) == (norm(&old_qual) != norm(&new_qual)) {
        return None;
    }
    // Both vis spellings stay in the WI-1 closed set; qualifier tokens
    // stay inside `const`/`async`/`unsafe`/`extern ["ABI"]` (unknown
    // future qualifiers decline).
    if !is_fn_vis_prefix(&old_vis) || !is_fn_vis_prefix(&new_vis) {
        return None;
    }
    if !qual_tokens_ok(&old_qual) || !qual_tokens_ok(&new_qual) {
        return None;
    }
    // Self-referential vis (`pub(in old)`) is an evolution, never this
    // joint (WI-4 domain): either name inside either prefix vetoes.
    if contains_ident(&old_prefix, &old)
        || contains_ident(&old_prefix, &new)
        || contains_ident(&new_prefix, &old)
        || contains_ident(&new_prefix, &new)
    {
        return None;
    }
    Some(FnVisqualJoint {
        old_prefix,
        new_prefix,
        old,
        new,
    })
}

/// Parse one single-line `{`-terminated fn header into signature slots.
/// One-line definitions (`fn foo() {}`) truncate to their header;
/// `;` decls and multi-line headers decline (`None`).
fn header_slots(line: &str) -> Option<SigSlots> {
    let brace = line.find('{')?;
    let header = line[..=brace].trim();
    if header.is_empty() {
        return None;
    }
    parse_sig_slots(header)
}

/// Raw text before the `fn` keyword naming `name` (ident-boundary on
/// both sides), or `None` when the line holds no such header.
fn raw_fn_prefix(line: &str, name: &str) -> Option<String> {
    let needle = format!("fn {name}");
    let mut search = 0;
    while let Some(rel) = line[search..].find(needle.as_str()) {
        let pos = search + rel;
        let before_ok = pos == 0
            || line[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| !super::is_ident_char(c));
        let after = &line[pos + needle.len()..];
        let after_ok = after.is_empty()
            || after
                .chars()
                .next()
                .is_none_or(|c| !super::is_ident_char(c));
        if before_ok && after_ok {
            return Some(line[..pos].to_string());
        }
        search = pos + 1;
    }
    None
}

/// Split a braceless first line at its `fn name` needle (WI-5-R3):
/// the first `fn` keyword preceded by start/whitespace and followed by
/// whitespace, with its ident name. Returns (raw prefix, name, tail).
/// The whitespace-before law keeps macro-paren text (`!(fn old(`) and
/// path text from ever splitting; the caller validates the prefix.
fn split_fn_needle(line: &str) -> Option<(String, String, String)> {
    let mut search = 0;
    while let Some(rel) = line[search..].find("fn") {
        let pos = search + rel;
        let before_ok = pos == 0
            || line[..pos]
                .chars()
                .next_back()
                .is_none_or(|c| c == ' ' || c == '\t');
        let after_kw = &line[pos + 2..];
        if before_ok && after_kw.starts_with([' ', '\t']) {
            let ident = after_kw.trim_start();
            let len = super::ident_len(ident)?;
            let name = &ident[..len];
            let tail = &ident[len..];
            return Some((line[..pos].to_string(), name.to_string(), tail.to_string()));
        }
        search = pos + 1;
    }
    None
}

/// Split a raw fn prefix into `(vis, qualifiers)` under the WI-1 vis
/// grammar (`""`, `pub`, `pub(…)` — each followed by whitespace);
/// anything else is not a joint prefix. The vis keeps its trailing
/// space so the closed-set gate runs on the exact authored spelling.
fn split_vis_qual_prefix(raw: &str) -> Option<(String, String)> {
    let mut rest = raw;
    let mut vis = String::new();
    if let Some(r) = rest.strip_prefix("pub") {
        if r.is_empty() || r.starts_with([' ', '\t']) {
            vis = "pub ".to_string();
            rest = r.trim_start();
        } else if r.starts_with('(') {
            let end = r.find(')')?;
            let after = &r[end + 1..];
            if after.is_empty() || !after.starts_with([' ', '\t']) {
                return None;
            }
            vis = format!("pub{} ", &r[..=end]);
            rest = after.trim_start();
        } else {
            return None;
        }
    }
    Some((vis, rest.to_string()))
}

/// True when every qualifier token is engine-supported: `const` /
/// `async` / `unsafe`, bare `extern`, or `extern "ABI"` (any single
/// double-quoted literal, `""` included). Anything else — future
/// qualifier tokens, unclosed literals — declines.
fn qual_tokens_ok(qual: &str) -> bool {
    let mut toks = qual.split_whitespace().peekable();
    while let Some(tok) = toks.next() {
        if matches!(tok, "const" | "async" | "unsafe") {
            continue;
        }
        if tok == "extern" {
            if let Some(next) = toks.peek() {
                if next.starts_with('"') {
                    let lit = toks.next().unwrap();
                    if lit.len() < 2 || !lit.ends_with('"') {
                        return false;
                    }
                    continue;
                }
            }
            continue;
        }
        return false;
    }
    true
}

/// WI-1 closed vis set the terse `fn` rename accepts (SPEC 14.1 diet
/// form — vis rides literally, never a `V` hole). Mirrors the
/// item-intent gate (local copy per the per-module convention).
fn is_fn_vis_prefix(prefix: &str) -> bool {
    if prefix.is_empty() || prefix == "pub " {
        return true;
    }
    if prefix == "pub(crate) " || prefix == "pub(super) " {
        return true;
    }
    let Some(inner) = prefix
        .strip_prefix("pub(in ")
        .and_then(|r| r.strip_suffix(") "))
    else {
        return false;
    };
    let mut segs = inner.split("::").peekable();
    let mut first = true;
    while let Some(seg) = segs.next() {
        if seg.is_empty() {
            return false;
        }
        let head_ok =
            first && matches!(seg, "crate" | "self" | "super" | "$crate") && segs.peek().is_some();
        if !head_ok && super::ident_len(seg) != Some(seg.len()) {
            return false;
        }
        first = false;
    }
    !first
}

/// Ident-boundary containment (never substring; local copy per the
/// per-module convention).
fn contains_ident(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut token = String::new();
    for c in haystack.chars().chain(std::iter::once(' ')) {
        if c == '_' || c.is_ascii_alphanumeric() {
            token.push(c);
        } else {
            if token == needle {
                return true;
            }
            token.clear();
        }
    }
    false
}

/// Whitespace-insensitive slot comparison (the engine compares slots
/// whitespace-insensitively against sliced target text).
fn norm(s: &str) -> String {
    super::flat_ws(s)
}

/// Count column-0 `fn`-header definitions of `old` across the involved
/// pre-images (any vis/qual spelling — `split_item` misses
/// qualifier-bearing headers, so the joint path cannot reuse the
/// `count_kind_defs` twin veto). Single-line `{`-headers count via
/// `header_slots`; multi-line headers (WI-5-R3) count via their braceless
/// first line (same needle the classifier claims; comment/attribute/
/// string openers never count). Exactly one must exist.
pub(crate) fn count_fn_defs(
    pre: &HashMap<String, String>,
    pairs: &[super::item_intent::Pair],
    old: &str,
) -> usize {
    let mut seen: HashSet<String> = HashSet::new();
    let mut count = 0usize;
    for pair in pairs {
        if !seen.insert(pair.path.clone()) {
            continue;
        }
        if let Some(text) = pre.get(&pair.path) {
            for line in text.lines() {
                if line.starts_with([' ', '\t']) {
                    continue;
                }
                if let Some(slots) = header_slots(line) {
                    if slots.name == old {
                        count += 1;
                    }
                } else if !line.contains('{')
                    && !line.contains(';')
                    && !line.starts_with(['/', '*', '#', '"'])
                    && split_fn_needle(line).is_some_and(|(_, name, _)| name == old)
                {
                    count += 1;
                }
            }
        }
    }
    count
}

/// Joint follow emission: sweep dummy + follow + the concrete evolved
/// terse def pair (never a `V`/`Q` hole — the intent proved one
/// concrete evolution).
pub(crate) fn joint_emit(
    path: &str,
    rule: &str,
    old_prefix: &str,
    old: &str,
    new_prefix: &str,
    new: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!("- {old_prefix}fn {old}\n+ {new_prefix}fn {new}\n"));
    out
}

/// A bare-item rename + single vis-evolution def pair: raw prefixes
/// (original text before the kind keyword, trailing space included,
/// `""` when bare) plus the kind and the old/new names.
pub(crate) struct BareVisJoint {
    pub(crate) old_prefix: String,
    pub(crate) new_prefix: String,
    pub(crate) kind: String,
    pub(crate) old: String,
    pub(crate) new: String,
}

/// Classify a native pair as a bare-item rename + exactly one vis step:
/// `Some` when both lines are `split_item` definitions of the same
/// non-`fn` vis-bearing kind with differing names, differing closed-set
/// vis prefixes, and equal suffixes -- else `None`. Pure renames (equal
/// prefixes), `fn` headers (the fn joint owns them), `mod` (WI-15c owns
/// mod intents), `impl` / `macro_rules!` (no vis slot), `unsafe`/`mut`
/// lines (never a bare vis prefix, so they decline at `split_item`),
/// suffix deltas, unknown vis, and self-referential vis all decline
/// (never a partial claim).
pub(crate) fn classify_bare_vis_joint(minus: &str, plus: &str) -> Option<BareVisJoint> {
    if minus.trim_end() == plus.trim_end() {
        return None;
    }
    let (old_prefix, old_kind, old_ident, old_suffix) = super::split_item(minus.trim_end())?;
    let (new_prefix, new_kind, new_ident, new_suffix) = super::split_item(plus.trim_end())?;
    if old_kind == "fn" || old_kind != new_kind {
        return None;
    }
    if matches!(old_kind.as_str(), "impl" | "macro_rules!" | "mod") {
        return None;
    }
    if old_ident.is_empty() || new_ident.is_empty() || old_ident == new_ident {
        return None;
    }
    // Equal prefixes are the pure-rename path (never claimed here);
    // suffix deltas are a second slot.
    if old_prefix == new_prefix || old_suffix != new_suffix {
        return None;
    }
    if !is_fn_vis_prefix(&old_prefix) || !is_fn_vis_prefix(&new_prefix) {
        return None;
    }
    // Self-referential vis (`pub(in old)`) is an evolution, never this
    // joint: either name inside either prefix vetoes.
    if contains_ident(&old_prefix, &old_ident)
        || contains_ident(&old_prefix, &new_ident)
        || contains_ident(&new_prefix, &old_ident)
        || contains_ident(&new_prefix, &new_ident)
    {
        return None;
    }
    Some(BareVisJoint {
        old_prefix,
        new_prefix,
        kind: old_kind,
        old: old_ident,
        new: new_ident,
    })
}

/// Bare-item joint follow emission: sweep dummy + follow + the concrete
/// evolved terse def pair (never a hole -- the intent proved one concrete
/// vis step).
pub(crate) fn bare_joint_emit(
    path: &str,
    rule: &str,
    kind: &str,
    old_prefix: &str,
    old: &str,
    new_prefix: &str,
    new: &str,
) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&format!("@@ -1,1 @@ rule={rule}\n"));
    out.push_str("# sweep\n");
    out.push_str("# follow=uses\n");
    out.push_str(&format!(
        "- {old_prefix}{kind} {old}\n+ {new_prefix}{kind} {new}\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joint(minus: &str, plus: &str) -> Option<FnVisqualJoint> {
        classify_fn_visqual_joint(minus, plus)
    }

    #[test]
    fn vis_joint_fires() {
        let j = joint(
            "pub fn foo(x: i32) -> i32 {",
            "pub(crate) fn bar(x: i32) -> i32 {",
        )
        .expect("vis joint must classify");
        assert_eq!(j.old_prefix, "pub ");
        assert_eq!(j.new_prefix, "pub(crate) ");
        assert_eq!(j.old, "foo");
        assert_eq!(j.new, "bar");
    }

    #[test]
    fn vis_joint_removal_fires() {
        // Narrowing (`pub(crate)` → bare) is the same one-slot shape.
        let j = joint("pub(crate) fn foo() {", "fn bar() {").expect("vis removal must classify");
        assert_eq!(j.old_prefix, "pub(crate) ");
        assert_eq!(j.new_prefix, "");
    }

    #[test]
    fn qual_joint_fires() {
        let j = joint("fn foo() {", "async fn bar() {").expect("qual joint must classify");
        assert_eq!(j.old_prefix, "");
        assert_eq!(j.new_prefix, "async ");
    }

    #[test]
    fn vis_plus_qual_double_vetoes() {
        // Vis AND qual evolving is two slots, never one joint.
        assert!(joint("pub fn foo() {", "pub(crate) async fn bar() {").is_none());
        // Qual preserved while vis evolves is the one-slot vis joint.
        let j = joint("pub async fn foo() {", "pub(crate) async fn bar() {")
            .expect("vis joint with preserved qual must classify");
        assert_eq!(j.old_prefix, "pub async ");
        assert_eq!(j.new_prefix, "pub(crate) async ");
    }

    #[test]
    fn one_line_def_is_pure_path() {
        // Same-vis one-line rename belongs to `split_delta`, never here.
        assert!(joint("fn foo(x: i32) -> i32 { x }", "fn bar(x: i32) -> i32 { x }").is_none());
    }

    #[test]
    fn method_and_decl_shapes_decline() {
        assert!(joint("    fn foo() {", "    fn bar() {").is_none());
        assert!(joint("\tasync fn foo() {", "\tasync fn bar() {").is_none());
        assert!(joint("fn foo();", "fn bar();").is_none());
        assert!(joint("    pub fn foo(", "    pub(crate) fn bar(").is_none());
    }

    #[test]
    fn double_slot_vetoes() {
        // vis + qual both evolving is never one slot.
        assert!(joint("fn foo() {", "pub async fn bar() {").is_none());
        // vis + return: two slots.
        assert!(joint("pub fn foo() {", "pub(crate) fn bar() -> i32 {").is_none());
        // vis + params: two slots.
        assert!(joint("fn foo(x: i32) {", "pub fn bar(x: u32) {").is_none());
        // qual + generics: two slots.
        assert!(joint("fn foo<T>(x: T) {", "async fn bar<T>(x: T) -> T {").is_none());
        // return-only delta with a rename is the fn_joint domain.
        assert!(joint("fn foo() {", "fn bar() -> i32 {").is_none());
    }

    #[test]
    fn self_referential_vis_vetoes() {
        assert!(joint("pub(in foo) fn foo() {", "pub(in foo) fn bar() {").is_none());
        assert!(joint("pub(in bar) fn foo() {", "pub(in bar) fn bar() {").is_none());
    }

    #[test]
    fn extern_qual_fires() {
        // Qualifier-addition is the one evolving slot.
        let j =
            joint("fn foo() {", "extern \"C\" fn bar() {").expect("extern-add joint must classify");
        assert_eq!(j.old_prefix, "");
        assert_eq!(j.new_prefix, "extern \"C\" ");
        // Preserved `extern "C"` with a pure rename is the pure path.
        assert!(joint("extern \"C\" fn foo() {", "extern \"C\" fn bar() {").is_none());
        // An ABI change is the same one qual slot evolving.
        let a = joint("extern \"C\" fn foo() {", "extern \"system\" fn bar() {")
            .expect("ABI-change joint must classify");
        assert_eq!(a.old_prefix, "extern \"C\" ");
        assert_eq!(a.new_prefix, "extern \"system\" ");
        // Preserved extern qual with vis evolution is the vis slot.
        let v = joint(
            "extern \"C\" fn foo() {",
            "pub(crate) extern \"C\" fn bar() {",
        )
        .expect("vis joint with preserved extern must classify");
        assert_eq!(v.old_prefix, "extern \"C\" ");
        assert_eq!(v.new_prefix, "pub(crate) extern \"C\" ");
        // Bare `extern` behaves the same.
        assert!(joint("fn foo() {", "extern fn bar() {").is_some());
        // Vis AND extern-qual evolving is two slots, never one joint.
        assert!(joint("fn foo() {", "pub extern \"C\" fn bar() {").is_none());
        // Preserved `const` with a pure rename is the pure path.
        assert!(joint("const fn foo() {", "const fn bar() {").is_none());
    }
    #[test]
    fn unknown_qual_still_declines() {
        assert!(joint("gen fn foo() {", "gen fn bar() {").is_none());
        assert!(joint("fn foo() {", "gen fn bar() {").is_none());
        // Unclosed ABI literal is malformed, never a joint.
        assert!(joint("fn foo() {", "extern \"C fn bar() {").is_none());
        // A bare quoted token without `extern` is never a qualifier.
        assert!(joint("fn foo() {", "\"C\" fn bar() {").is_none());
    }
    #[test]
    fn qual_tokenizer_matrix() {
        assert!(super::qual_tokens_ok(""));
        assert!(super::qual_tokens_ok("async "));
        assert!(super::qual_tokens_ok("extern \"C\" "));
        assert!(super::qual_tokens_ok("extern"));
        assert!(super::qual_tokens_ok("extern \"\""));
        assert!(super::qual_tokens_ok("const async unsafe extern \"C\""));
        assert!(!super::qual_tokens_ok("gen"));
        assert!(!super::qual_tokens_ok("async gen"));
        assert!(!super::qual_tokens_ok("extern \"C"));
        assert!(!super::qual_tokens_ok("\"C\""));
    }

    #[test]
    fn multiline_first_line_vis_joint_fires() {
        // WI-5-R3: multi-line headers classify on their first line under
        // the same one-slot law; the D13 probe owns the remainder lines.
        let j =
            joint("pub fn foo(", "pub(crate) fn bar(").expect("multi-line vis joint must classify");
        assert_eq!(j.old_prefix, "pub ");
        assert_eq!(j.new_prefix, "pub(crate) ");
        assert_eq!((j.old.as_str(), j.new.as_str()), ("foo", "bar"));
        // Vis removal / qualifier addition on first lines.
        let r =
            joint("pub(crate) fn foo(", "fn bar(").expect("multi-line vis removal must classify");
        assert_eq!(
            (r.old_prefix.as_str(), r.new_prefix.as_str()),
            ("pub(crate) ", "")
        );
        let q = joint("fn foo(", "async fn bar(").expect("multi-line qual joint must classify");
        assert_eq!(
            (q.old_prefix.as_str(), q.new_prefix.as_str()),
            ("", "async ")
        );
        // Generic first-line tails compare token-equal.
        assert!(joint("pub fn foo<T>(", "pub(crate) fn bar<T>(").is_some());
        // Preserved qualifiers with vis evolution fire.
        let v = joint("pub async fn foo(", "pub(crate) async fn bar(")
            .expect("multi-line vis joint with preserved qual must classify");
        assert_eq!(
            (v.old_prefix.as_str(), v.new_prefix.as_str()),
            ("pub async ", "pub(crate) async ")
        );
    }

    #[test]
    fn multiline_first_line_vetoes() {
        // Pure renames belong to `split_delta`, never here.
        assert!(joint("pub fn foo(", "pub fn bar(").is_none());
        assert!(joint("fn foo(", "fn bar(").is_none());
        // First-line tail deltas are a second slot.
        assert!(joint("pub fn foo(", "pub(crate) fn bar<T>(").is_none());
        assert!(joint("pub fn foo<T>(", "pub(crate) fn bar<T, U>(").is_none());
        // Reflowed headers (brace parity mixed) are never a joint.
        assert!(joint("pub fn foo() {", "pub(crate) fn bar(").is_none());
        assert!(joint("pub fn foo(", "pub(crate) fn bar() {").is_none());
        // `;` decls are never a def first line.
        assert!(joint("pub fn foo(", "pub(crate) fn bar();").is_none());
        assert!(joint("fn foo();", "async fn bar();").is_none());
        // Same-name, self-ref, unknown-qual, and method postures carry over.
        assert!(joint("pub fn foo(", "pub(crate) fn foo(").is_none());
        assert!(joint("pub(in foo) fn foo(", "pub(in foo) fn bar(").is_none());
        assert!(joint("fn foo(", "gen fn bar(").is_none());
        assert!(joint("    pub fn foo(", "    pub(crate) fn bar(").is_none());
        // Vis AND qual evolving is two slots, even on first lines.
        assert!(joint("fn foo(", "pub(crate) async fn bar(").is_none());
    }

    #[test]
    fn raw_prefix_spelling_matrix() {
        assert_eq!(
            raw_fn_prefix("pub(crate) async fn bar(x: i32) {", "bar"),
            Some("pub(crate) async ".to_string())
        );
        assert_eq!(raw_fn_prefix("fn foo() {", "foo"), Some(String::new()));
        assert_eq!(raw_fn_prefix("pub fn foobar() {", "foo"), None);
        assert_eq!(raw_fn_prefix("pub fn foo() {", "foobar"), None);
    }

    #[test]
    fn vis_splitter_matrix() {
        assert_eq!(
            split_vis_qual_prefix("pub(crate) async "),
            Some(("pub(crate) ".to_string(), "async ".to_string()))
        );
        assert_eq!(
            split_vis_qual_prefix(""),
            Some((String::new(), String::new()))
        );
        assert_eq!(
            split_vis_qual_prefix("pub "),
            Some(("pub ".to_string(), String::new()))
        );
        assert!(split_vis_qual_prefix("pubx ").is_none());
        assert!(split_vis_qual_prefix("pub(in crate::x) ").is_some());
    }

    #[test]
    fn twin_counter_matrix() {
        use std::collections::HashMap;
        let pair = |path: &str| super::super::item_intent::Pair {
            sec: 0,
            path: path.to_string(),
            minus: String::new(),
            plus: String::new(),
            site: 1,
        };
        let pre: HashMap<String, String> = HashMap::from([
            (
                "a.rs".to_string(),
                "async fn foo() {\n}\nextern \"C\" fn other() {\n}\n".to_string(),
            ),
            ("b.rs".to_string(), "fn foo() {\n}\n".to_string()),
        ]);
        let pairs = vec![pair("a.rs"), pair("b.rs")];
        // Qualifier-bearing twin counts: `split_item` would see only one.
        assert_eq!(count_fn_defs(&pre, &pairs, "foo"), 2);
        assert_eq!(count_fn_defs(&pre, &pairs, "other"), 1);
        assert_eq!(count_fn_defs(&pre, &pairs, "missing"), 0);
        // Indented (method) headers never count.
        let pre_m: HashMap<String, String> = HashMap::from([(
            "c.rs".to_string(),
            "impl S {\n    fn foo() {}\n}\n".to_string(),
        )]);
        let pairs_m = vec![pair("c.rs")];
        assert_eq!(count_fn_defs(&pre_m, &pairs_m, "foo"), 0);
        // WI-5-R3: multi-line first lines count; comments, decls, and
        // braceless non-headers never count.
        let pre_f: HashMap<String, String> = HashMap::from([(
            "d.rs".to_string(),
            "pub fn foo(\n    x: i32,\n) -> i32 {\n    x\n}\n// fn foo(\nfn decl();\n".to_string(),
        )]);
        let pairs_f = vec![pair("d.rs")];
        assert_eq!(count_fn_defs(&pre_f, &pairs_f, "foo"), 1);
        // A multi-line twin vetoes like a single-line twin.
        let pre_t: HashMap<String, String> = HashMap::from([
            (
                "d.rs".to_string(),
                "pub fn foo(\n    x: i32,\n) -> i32 {\n    x\n}\n".to_string(),
            ),
            (
                "e.rs".to_string(),
                "async fn foo(\n    x: i32,\n) {\n}\n".to_string(),
            ),
        ]);
        let pairs_t = vec![pair("d.rs"), pair("e.rs")];
        assert_eq!(count_fn_defs(&pre_t, &pairs_t, "foo"), 2);
    }

    #[test]
    fn multiline_emission_applies_jointly() {
        // WI-5-R3: the unchanged terse emission lowers onto multi-line
        // headers — vis + name rewrite, signature lines byte-verbatim.
        let pre: HashMap<String, String> = HashMap::from([(
            "a.rs".to_string(),
            "pub fn foo(\n    x: i32,\n) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n}\n"
                .to_string(),
        )]);
        let emission = joint_emit("a.rs", "r1", "pub ", "foo", "pub(crate) ", "bar");
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emission must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("emission must apply");
        assert!(
            files["a.rs"].contains("pub(crate) fn bar(\n    x: i32,\n) -> i32 {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("let a = bar(1);"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["a.rs"].contains("foo"), "got: {}", files["a.rs"]);
    }

    #[test]
    fn emission_applies_jointly() {
        let pre: HashMap<String, String> = HashMap::from([(
            "a.rs".to_string(),
            "pub fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n}\n"
                .to_string(),
        )]);
        let emission = joint_emit("a.rs", "r1", "pub ", "foo", "pub(crate) ", "bar");
        assert!(emission.contains("- pub fn foo\n"));
        assert!(emission.contains("+ pub(crate) fn bar\n"));
        assert!(emission.contains("# follow=uses"));
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emission must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("emission must apply");
        assert!(
            files["a.rs"].contains("pub(crate) fn bar(x: i32) -> i32 {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("let a = bar(1);"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["a.rs"].contains("foo"), "got: {}", files["a.rs"]);
    }
    #[test]
    fn extern_emission_applies_jointly() {
        let pre: HashMap<String, String> = HashMap::from([(
            "a.rs".to_string(),
            "extern \"C\" fn foo(x: i32) -> i32 {\n    x + 1\n}\nfn m() {\n    let a = foo(1);\n}\n"
                .to_string(),
        )]);
        let emission = joint_emit("a.rs", "r1", "extern \"C\" ", "foo", "extern \"C\" ", "bar");
        assert!(emission.contains("- extern \"C\" fn foo\n"));
        assert!(emission.contains("+ extern \"C\" fn bar\n"));
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emission must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("emission must apply");
        assert!(
            files["a.rs"].contains("extern \"C\" fn bar(x: i32) -> i32 {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("let a = bar(1);"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["a.rs"].contains("foo"), "got: {}", files["a.rs"]);
    }

    fn bare(minus: &str, plus: &str) -> Option<BareVisJoint> {
        classify_bare_vis_joint(minus, plus)
    }

    #[test]
    fn bare_vis_joint_fires() {
        let j =
            bare("pub struct Old {", "pub(crate) struct New {").expect("vis joint must classify");
        assert_eq!(j.old_prefix, "pub ");
        assert_eq!(j.new_prefix, "pub(crate) ");
        assert_eq!(j.kind, "struct");
        assert_eq!((j.old.as_str(), j.new.as_str()), ("Old", "New"));
        // Vis addition / removal / further vis-bearing kinds.
        assert!(bare("struct Old;", "pub struct New;").is_some());
        assert!(bare("pub(crate) enum Old {", "enum New {").is_some());
        assert!(bare("pub trait Old {", "pub(crate) trait New {").is_some());
        assert!(bare("type Old = u8;", "pub type New = u8;").is_some());
        // Indented nested items ride (split_item tolerates their indent).
        assert!(bare("    pub struct Old;", "    pub(crate) struct New;").is_some());
    }

    #[test]
    fn bare_vis_joint_declines() {
        // Pure renames belong to split_delta, never here.
        assert!(bare("pub struct Old {", "pub struct New {").is_none());
        assert!(bare("struct Old;", "struct New;").is_none());
        // `fn` headers belong to the fn joint; `mod` to WI-15c; `impl`
        // and `macro_rules!` take no visibility.
        assert!(bare("pub fn foo() {", "pub(crate) fn bar() {").is_none());
        assert!(bare("pub mod old {", "pub(crate) mod new {").is_none());
        assert!(bare("impl Old {", "impl New {").is_none());
        // Suffix deltas are a second slot.
        assert!(bare("pub struct Old {", "pub(crate) struct New(i32);").is_none());
        assert!(bare("type Old = u8;", "pub type New = u16;").is_none());
        // Unknown vis spellings veto.
        assert!(bare("pub(foo) struct Old {", "pub struct New {").is_none());
        // Self-referential vis is an evolution, never this joint.
        assert!(bare("pub(in Foo) struct Foo {", "pub struct Bar {").is_none());
        assert!(bare("pub struct Foo {", "pub(in Bar) struct Bar {").is_none());
        // `unsafe`/`mut` lines parse with shifted idents, so no joint
        // claims them.
        assert!(bare("unsafe trait Old {", "unsafe trait New {").is_none());
        assert!(bare("static mut Old;", "static mut New;").is_none());
        // Indented assoc items decline (split_item posture).
        assert!(bare("    type Old = u8;", "    pub type New = u8;").is_none());
    }

    #[test]
    fn bare_joint_emission_applies_jointly() {
        let pre: HashMap<String, String> = HashMap::from([(
            "a.rs".to_string(),
            "pub struct Old {\n    x: i32,\n}\nfn f(s: Old) -> i32 {\n    s.x\n}\n".to_string(),
        )]);
        let emission = bare_joint_emit("a.rs", "r1", "struct", "pub ", "Old", "pub(crate) ", "New");
        assert!(emission.contains("- pub struct Old\n"));
        assert!(emission.contains("+ pub(crate) struct New\n"));
        assert!(emission.contains("# follow=uses"));
        let parsed = rsmpl_patch::parse_patch(&emission).expect("emission must parse");
        let mut files = pre.clone();
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed).expect("emission must apply");
        assert!(
            files["a.rs"].contains("pub(crate) struct New {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(
            files["a.rs"].contains("fn f(s: New) -> i32 {"),
            "got: {}",
            files["a.rs"]
        );
        assert!(!files["a.rs"].contains("Old"), "got: {}", files["a.rs"]);
    }
}
