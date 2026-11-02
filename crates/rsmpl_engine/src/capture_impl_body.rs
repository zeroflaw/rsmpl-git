//! WI-19 residue: inherent-`impl`-block body-evolving `as` splices
//! (D5 sibling of `capture_container_body.rs`).
//!
//! A `+ impl Old as New { ... }` paste splices the captured header's
//! self-ty to the new name and renames `Old` → `New` through
//! [`rsmpl_pattern::rename_ident_in_code`] in the captured suffix/body.
//! The WI-2 binding boundary
//! ([`rsmpl_pattern::impl_body_evolution_veto`]) applies: an assoc-item
//! name, a macro name, or an attribute interior vetoes loud instead of
//! renaming. Trait-impls decline (WI-3 owns that header).

use rsmpl_patch::{ErrorCode, StructuredError};

/// Splice a captured inherent `impl` block under a body-evolving
/// template header (`impl Old as New { ... }`, or `unsafe impl ...`
/// when the template carries the `unsafe` prefix verbatim).
pub(crate) fn splice_impl_body_evolution(
    captured: &str,
    new_prefix: &str,
    old_t: &str,
    new_t: &str,
    rule: &str,
) -> Result<String, StructuredError> {
    let fail = |msg: String| {
        StructuredError::new(ErrorCode::PatternConflict, msg)
            .with_rule(rule)
            .with_repair(StructuredError::fix_pattern_repair(
                "impl-as-paste",
                "write `+ impl <Old> as <New> { ... }` (exactly the self-ty rename, WI-2 binding law), or split the move",
            ))
    };
    let file: syn::File = syn::parse_str(captured).map_err(|e| {
        fail(format!(
            "evolution paste captured text is not a Rust file: {e}"
        ))
    })?;
    let old_name: String = match file.items.as_slice() {
        [syn::Item::Impl(imp)] if imp.trait_.is_none() => {
            self_ty_last_ident(&imp.self_ty).ok_or_else(|| {
                fail("evolution paste needs a plain-path self-ty for an `impl <Old> as <New>` template".to_string())
            })?
        }
        [syn::Item::Impl(_)] => {
            return Err(fail(
                "evolution paste needs an inherent `impl` block for an `impl <Old> as <New>` template (trait-impls stay ordinary)".to_string(),
            ));
        }
        _ => {
            return Err(fail(
                "evolution paste needs a captured `impl` block for an `impl <Old> as <New>` template".to_string(),
            ));
        }
    };
    if old_name != old_t {
        return Err(fail(format!(
            "body-evolving paste names `{old_t}` but the capture holds `impl {old_name}`"
        )));
    }
    if let Some(veto) = rsmpl_pattern::impl_body_evolution_veto(captured, &old_name) {
        return Err(fail(format!(
            "body-evolving paste vetoes: `{old_name}` has a {} occurrence at captured line {} (rename it by hand or split the move)",
            veto.kind, veto.line
        )));
    }
    // First `impl Old` header line: word-bounded match so a body
    // occurrence never wins over the header.
    let mut target: Option<usize> = None;
    for (idx, line) in captured.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        if impl_line_self_ty(line).as_deref() == Some(old_name.as_str()) {
            target = Some(idx);
            break;
        }
    }
    let idx = target.ok_or_else(|| {
        fail(format!(
            "captured text has no `impl {old_name}` header line"
        ))
    })?;
    let mut lines: Vec<String> = captured.lines().map(str::to_string).collect();
    let line = lines[idx].clone();
    let ws_len = line.len() - line.trim_start().len();
    let (ws, rest) = line.split_at(ws_len);
    // Word-bounded `impl <Old>` within the code line; the remainder
    // after the old self-ty is the suffix (generics/where/body-on-same-line),
    // renamed in place so single-line items round-trip.
    let mut kw_at = None;
    let mut search = rest;
    let mut base = 0;
    while let Some(pos) = search.find("impl") {
        let before = search.as_bytes().get(pos.wrapping_sub(1));
        let before_ok =
            pos == 0 || before.is_some_and(|c| !c.is_ascii_alphanumeric() && *c != b'_');
        let after = &search[pos + 4..];
        let after_ok = after.starts_with(|c: char| c.is_whitespace());
        if before_ok && after_ok && after.trim_start().starts_with(old_name.as_str()) {
            let tail = &after.trim_start()[old_name.len()..];
            let boundary = tail
                .chars()
                .next()
                .map(|c| c != '_' && !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            if boundary {
                kw_at = Some(base + pos);
                break;
            }
        }
        search = &search[pos + 4..];
        base += pos + 4;
    }
    let kw_at = kw_at.ok_or_else(|| {
        fail(format!(
            "captured text has no `impl {old_name}` header line"
        ))
    })?;
    let after_impl = &rest[kw_at + 4..];
    let trimmed = after_impl.trim_start();
    // The template prefix rides verbatim (`unsafe impl` stays `unsafe`;
    // a dropped/added `unsafe` never sneaks in through the suffix).
    let pre = new_prefix.trim_end();
    let head = if pre.is_empty() {
        format!("impl {new_t}")
    } else {
        format!("{pre} impl {new_t}")
    };
    let suffix = trimmed[old_name.len()..].to_string();
    let renamed_suffix = rsmpl_pattern::rename_ident_in_code(&suffix, &old_name, new_t);
    lines[idx] = format!("{ws}{head}{renamed_suffix}");
    if idx + 1 < lines.len() {
        let body = lines[idx + 1..].join("\n");
        let renamed = rsmpl_pattern::rename_ident_in_code(&body, &old_name, new_t);
        let mut new_lines: Vec<String> = renamed.split('\n').map(str::to_string).collect();
        lines.truncate(idx + 1);
        lines.append(&mut new_lines);
    }
    Ok(lines.join("\n"))
}

/// Last segment of a plain-path self-ty (`Old`, `Old<T>`, `crate::Old`).
/// Anything else (references, tuples, `dyn`, qself) is not a moveable
/// self-ty.
fn self_ty_last_ident(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(tp) = ty {
        if tp.qself.is_none() {
            return tp.path.segments.last().map(|s| s.ident.to_string());
        }
    }
    None
}

/// The self-ty ident of an `impl <Ident>` header line (`None` when the
/// line holds no word-bounded `impl` + ident pair or names a trait
/// form). Trait-form lines (`impl Tr for Ty`) return `None` — the
/// splice declines them loud downstream.
pub(crate) fn impl_line_self_ty(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"impl")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 4;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            // Optional `unsafe` never precedes `impl` on the same line
            // here (`unsafe impl` puts `unsafe` first); skip nothing.
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if start != j {
                // Trait-form guard: a `for` word after the ident means
                // `impl Tr for Ty` — not an inherent header.
                let rest = &line[j..];
                if has_word(rest, "for") {
                    return None;
                }
                return Some(line[start..j].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

fn has_word(hay: &str, word: &str) -> bool {
    let hb = hay.as_bytes();
    let wb = word.as_bytes();
    if wb.is_empty() || hb.len() < wb.len() {
        return false;
    }
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + wb.len() <= hb.len() {
        if &hb[i..i + wb.len()] == wb {
            let before_ok = i == 0 || !is_ident(hb[i - 1]);
            let after_ok = i + wb.len() >= hb.len() || !is_ident(hb[i + wb.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::capture::splice_evolved_header;

    const IMPL: &str = "impl Old {\n    fn make() -> Old {\n        Old\n    }\n}";

    #[test]
    fn impl_as_splice_renames_header_and_body() {
        let out = splice_evolved_header(IMPL, "impl Old as New { ... }", "r")
            .expect("impl as splice must fire");
        assert!(out.contains("impl New {"), "got:\n{out}");
        assert!(out.contains("-> New"), "got:\n{out}");
        assert!(!out.contains("Old"), "old name gone, got:\n{out}");
    }

    #[test]
    fn impl_as_splice_single_line_round_trips() {
        let cap = "impl Old { fn m(&self) -> Old { Old } }";
        let out = splice_evolved_header(cap, "impl Old as New { ... }", "r")
            .expect("single-line impl as splice must fire");
        assert!(out.contains("impl New"), "got:\n{out}");
        assert!(out.contains("-> New"), "got:\n{out}");
    }

    #[test]
    fn impl_as_splice_vetoes_assoc_name() {
        let cap = "impl Old {\n    fn Old(&self) {}\n}";
        let err = splice_evolved_header(cap, "impl Old as New { ... }", "r")
            .expect_err("assoc-name capture must veto");
        assert!(err.to_string().contains("assoc"), "got: {err}");
    }

    #[test]
    fn impl_as_splice_keeps_unsafe_prefix() {
        let cap = "unsafe impl Old {\n    fn m(&self) {}\n}";
        let out = splice_evolved_header(cap, "unsafe impl Old as New { ... }", "r")
            .expect("unsafe impl as splice must fire");
        assert!(out.contains("unsafe impl New {"), "got:\n{out}");
    }

    #[test]
    fn impl_as_splice_rejects_trait_impl() {
        let cap = "impl Tr for Old {\n    fn m(&self) {}\n}";
        let err = splice_evolved_header(cap, "impl Old as New { ... }", "r")
            .expect_err("trait-impl must decline");
        assert!(err.to_string().contains("inherent"), "got: {err}");
    }
}
