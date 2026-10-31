//! WI-18 evolving-collection paste support (D5 sibling of `capture.rs`).
//!
//! A collection cut (`# bind=H` + `# allow=many`) may be pasted with a
//! name-evolution template carrying an `N`-hole (`N`, `new_N`, `N_new`,
//! `pre_N_post`; `New` stays concrete). Each captured item's header
//! replays through the template with `N` bound per item
//! (`new = pre + old + post`); bodies, attrs, and remaining slots replay
//! per item exactly as scalar evolution pastes do. Concrete (hole-less)
//! templates on collections still fail loud (would duplicate one name);
//! kind mismatches fail loud in the scalar splice.

use rsmpl_patch::{ErrorCode, StructuredError};

use crate::capture::{splice_evolved_header, PasteTarget};
use crate::edits::TextEdit;
use crate::lines::LineIndex;

/// Parse an `N`-hole collection name: `N` as a standalone
/// underscore-separated segment, exactly once. Returns the
/// `(pre, post)` affixes around the hole (`N` → `("", "")`,
/// `new_N` → `("new_", "")`, `N_new` → `("", "_new")`,
/// `pre_N_post` → `("pre_", "_post")`). `New`, `newN`, `N_N`
/// (two holes), and hole-less names return `None`.
pub(crate) fn parse_collection_name_hole(new_name: &str) -> Option<(String, String)> {
    let segs: Vec<&str> = new_name.split('_').collect();
    let holes: Vec<usize> = segs
        .iter()
        .enumerate()
        .filter(|(_, s)| **s == "N")
        .map(|(i, _)| i)
        .collect();
    if holes.len() != 1 {
        return None;
    }
    let idx = holes[0];
    let pre = if idx == 0 {
        String::new()
    } else {
        format!("{}_", segs[..idx].join("_"))
    };
    let post = if idx + 1 >= segs.len() {
        String::new()
    } else {
        format!("_{}", segs[idx + 1..].join("_"))
    };
    Some((pre, post))
}

/// Extract the `(prefix, new_name)` of any evolution-template shape
/// (`fn` abstract + concrete-signature, `struct`/`enum`/`union`/`type`/
/// `const`/`static`/`mod`/`trait`). `None` when the template is not an
/// evolution shape at all.
pub(crate) fn collection_template_parts(template: &str) -> Option<(String, String)> {
    let t = template.trim();
    if let Some((pre, name)) = rsmpl_pattern::parse_fn_full_header_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_trait_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_mod_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name, _)) = rsmpl_pattern::parse_static_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_const_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_type_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_union_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_enum_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_struct_evolution_template(t) {
        return Some((pre, name));
    }
    if let Some((pre, name)) = rsmpl_pattern::parse_evolution_template(t) {
        return Some((pre, name));
    }
    None
}

/// Old ident of a single captured item (any evolvable kind). `None`
/// for ident-less captures (impl blocks etc. — the scalar splice
/// rejects them loud downstream).
pub(crate) fn captured_old_ident(captured: &str) -> Option<String> {
    let file: syn::File = syn::parse_str(captured).ok()?;
    match file.items.as_slice() {
        [syn::Item::Fn(f)] => Some(f.sig.ident.to_string()),
        [syn::Item::Struct(s)] => Some(s.ident.to_string()),
        [syn::Item::Enum(e)] => Some(e.ident.to_string()),
        [syn::Item::Union(u)] => Some(u.ident.to_string()),
        [syn::Item::Type(t)] => Some(t.ident.to_string()),
        [syn::Item::Const(c)] => Some(c.ident.to_string()),
        [syn::Item::Static(s)] => Some(s.ident.to_string()),
        [syn::Item::Mod(m)] => Some(m.ident.to_string()),
        [syn::Item::Trait(t)] => Some(t.ident.to_string()),
        _ => None,
    }
}

/// Concretize a collection template for one item: replace the first
/// word-bounded occurrence of the hole-bearing `hole_name` (e.g.
/// `new_N`) with the concrete `new_ident` (e.g. `new_a`). Word-bounded
/// so a body occurrence never wins over the header.
pub(crate) fn concretize_template(template: &str, hole_name: &str, new_ident: &str) -> String {
    let bytes = template.as_bytes();
    let hole = hole_name.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + hole.len() <= bytes.len() {
        if &bytes[i..i + hole.len()] == hole {
            let before_ok = i == 0 || !is_ident(bytes[i - 1]);
            let after_ok = i + hole.len() >= bytes.len() || !is_ident(bytes[i + hole.len()]);
            if before_ok && after_ok {
                let mut out = template[..i].to_string();
                out.push_str(new_ident);
                out.push_str(&template[i + hole.len()..]);
                return out;
            }
        }
        i += 1;
    }
    template.to_string()
}

/// Compute the concrete new ident for `old` under `(pre, post)`.
pub(crate) fn concrete_new_ident(pre: &str, old: &str, post: &str) -> String {
    format!("{pre}{old}{post}")
}

/// WI-18 collection paste: bare name pastes verbatim (WI-6); an
/// evolution template with an `N`-hole splices per item with `N` bound
/// to that item's old ident. Concrete hole-less templates fail loud
/// (would duplicate one name); kind mismatches fail loud in the scalar
/// splice; divergent per-item templates never arise here (one template
/// for the whole collection).
pub(crate) fn paste_evolving_collection(
    texts: &[String],
    template: &str,
    target: PasteTarget<'_>,
    index: &LineIndex,
    rule: &str,
    file: &str,
    edits: &mut Vec<TextEdit>,
) -> Result<(), StructuredError> {
    let fail = |msg: String| {
        StructuredError::new(ErrorCode::CaptureConflict, msg)
            .with_rule(rule)
            .with_file(file)
            .with_repair(StructuredError::fix_pattern_repair(
                "use",
                "paste with a bare `+ Name` (`# use=<Name>`) or move items singly",
            ))
    };
    let Some((_, hole_name)) = collection_template_parts(template) else {
        return Err(fail(format!(
            "evolution paste template must be a single-line header with an `N`-hole (got `{}`)",
            template.trim()
        )));
    };
    let Some((pre, post)) = parse_collection_name_hole(&hole_name) else {
        return Err(fail(format!(
            "collection evolution template needs an `N`-hole in the name position (`N`, `new_N`, `N_new`, `pre_N_post`; got `{hole_name}`) — one header template cannot rename many items to one name",
        )));
    };
    if texts.is_empty() {
        return Ok(());
    }
    let mut items = Vec::with_capacity(texts.len());
    for captured in texts {
        let Some(old) = captured_old_ident(captured) else {
            return Err(fail(
                "evolution paste needs a named captured item (only fn/struct/enum/union/type/const/static/mod/trait headers evolve)".to_string(),
            ));
        };
        let new_ident = concrete_new_ident(&pre, &old, &post);
        let concrete = concretize_template(template, &hole_name, &new_ident);
        items.push(splice_evolved_header(captured, &concrete, rule)?);
    }
    let mut ins = items
        .iter()
        .map(|t| t.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n\n");
    ins.push('\n');
    let pos = if target.anchor == 0 {
        0
    } else {
        index
            .line_start(target.anchor + 1)
            .unwrap_or_else(|| index.eof())
    };
    edits.push(TextEdit::new(pos, pos, ins));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_hole_matrix() {
        assert_eq!(
            parse_collection_name_hole("N"),
            Some((String::new(), String::new()))
        );
        assert_eq!(
            parse_collection_name_hole("new_N"),
            Some(("new_".to_string(), String::new()))
        );
        assert_eq!(
            parse_collection_name_hole("N_new"),
            Some((String::new(), "_new".to_string()))
        );
        assert_eq!(
            parse_collection_name_hole("pre_N_post"),
            Some(("pre_".to_string(), "_post".to_string()))
        );
        assert_eq!(parse_collection_name_hole("New"), None);
        assert_eq!(parse_collection_name_hole("newN"), None);
        assert_eq!(parse_collection_name_hole("N_N"), None);
        assert_eq!(parse_collection_name_hole("plain"), None);
    }

    #[test]
    fn concretize_replaces_header_only() {
        let t = "fn new_N(...) { ... } // mentions new_N again";
        let out = concretize_template(t, "new_N", "new_a");
        assert_eq!(out, "fn new_a(...) { ... } // mentions new_N again");
    }

    #[test]
    fn template_parts_cover_fn_shapes() {
        let (pre, name) = collection_template_parts("pub fn new_N(...) { ... }").unwrap();
        assert_eq!(pre.trim(), "pub");
        assert_eq!(name, "new_N");
        assert!(collection_template_parts("fn a() {}").is_none());
    }

    #[test]
    fn captured_ident_matrix() {
        assert_eq!(captured_old_ident("fn a() {}").as_deref(), Some("a"));
        assert_eq!(captured_old_ident("struct Old;").as_deref(), Some("Old"));
        assert_eq!(captured_old_ident("fn a() {} fn b() {}"), None);
    }
}
