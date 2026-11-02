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
//! need engine prefix-evolution support first (WI-5-R2). Qualifier
//! tokens beyond `const`/`async`/`unsafe` (`extern "ABI"`, …) stay
//! ordinary (WI-5-R1); multi-line headers stay ordinary (WI-5-R3).

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
/// header slot ({vis} xor {qualifier-list}): `Some` when both lines are
/// column-0 `{`-terminated headers whose names differ with all other
/// slots token-equal, else `None`. Two evolving slots, any
/// generics/params/return/`where` delta, `extern` qualifiers,
/// self-referential vis, indented (method) headers, `;` decls, and
/// multi-line headers all decline (never a partial claim).
pub(crate) fn classify_fn_visqual_joint(minus: &str, plus: &str) -> Option<FnVisqualJoint> {
    if minus.trim_end() == plus.trim_end() {
        return None;
    }
    // WI-19c posture: indented `fn` lines are methods, never intents.
    if minus.starts_with([' ', '\t']) || plus.starts_with([' ', '\t']) {
        return None;
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
    let (old_vis, old_qual) = split_vis_qual_prefix(&old_prefix)?;
    let (new_vis, new_qual) = split_vis_qual_prefix(&new_prefix)?;
    // Both vis spellings stay in the WI-1 closed set; qualifier tokens
    // stay inside `const`/`async`/`unsafe` (`extern "ABI"` declines).
    if !is_fn_vis_prefix(&old_vis) || !is_fn_vis_prefix(&new_vis) {
        return None;
    }
    if !qual_tokens_ok(&old_qual) || !qual_tokens_ok(&new_qual) {
        return None;
    }
    // Self-referential vis (`pub(in old)`) is an evolution, never this
    // joint (WI-4 domain): either name inside either prefix vetoes.
    if contains_ident(&old_prefix, &m.name)
        || contains_ident(&old_prefix, &p.name)
        || contains_ident(&new_prefix, &m.name)
        || contains_ident(&new_prefix, &p.name)
    {
        return None;
    }
    Some(FnVisqualJoint {
        old_prefix,
        new_prefix,
        old: m.name,
        new: p.name,
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

/// True when every whitespace-separated qualifier token is a known
/// single-token qualifier (`const`/`async`/`unsafe`, empty included);
/// `extern "ABI"` and anything else decline.
fn qual_tokens_ok(qual: &str) -> bool {
    qual.split_whitespace()
        .all(|tok| matches!(tok, "const" | "async" | "unsafe"))
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
/// `count_kind_defs` twin veto). Exactly one must exist.
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
    fn pure_rename_declines() {
        assert!(joint("fn foo() {", "fn bar() {").is_none());
        assert!(joint("pub fn foo() {", "pub fn bar() {").is_none());
        assert!(joint("fn foo() {", "fn foo() {").is_none());
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
    fn extern_and_unknown_qual_decline() {
        assert!(joint("fn foo() {", "extern \"C\" fn bar() {").is_none());
        // Preserved `const` with a pure rename is the pure path.
        assert!(joint("const fn foo() {", "const fn bar() {").is_none());
    }

    #[test]
    fn method_and_decl_shapes_decline() {
        assert!(joint("    fn foo() {", "    fn bar() {").is_none());
        assert!(joint("\tasync fn foo() {", "\tasync fn bar() {").is_none());
        assert!(joint("fn foo();", "fn bar();").is_none());
        assert!(joint("pub fn foo(", "pub(crate) fn bar(").is_none());
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
                "async fn foo() {\n}\nfn other() {\n}\n".to_string(),
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
}
