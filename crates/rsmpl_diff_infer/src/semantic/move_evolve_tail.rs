//! WI-8 slices 4-8: `type`, `const`, `static`, `mod`, and `trait`
//! move + header evolution pairing (split from `move_evolve` for the
//! 2000-line gate; re-exported there so callers keep their paths).
//!
//! Same mechanism as `move_evolve`: pairs items whose bodies (aliased
//! types, const/static type+expr, empty-or-absent mod content, empty
//! trait items) are token-equal but whose header prefix (vis +
//! qualifiers, verbatim) and/or name evolve, and builds the single-line
//! header template the engine's evolution paste splices the captured
//! body under (`static mut` mutability and `unsafe mod`/`unsafe trait`
//! ride verbatim in the prefix; `auto trait` vetoes to ordinary).

use super::move_evolve::tok;
use syn::spanned::Spanned;

/// Evolved `type` alias pair check (WI-8 slice 4): same shape modulo
/// the visibility prefix and the ident. Generics (params + where),
/// attrs, and the aliased type must be token-equal. Returns the
/// plus-side ident when the pair evolves (a vis or name delta is
/// required — the byte-exact path owns identical items).
pub(crate) fn type_evolved_match(minus: &syn::ItemType, plus: &syn::ItemType) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    if vis_same && name_same {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.ty) != tok(&plus.ty) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All evolved `type` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 4). Mirrors `find_union_evolved_hits`;
/// spans coincide with parsed `type` item spans by construction.
pub(crate) fn find_type_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemType>(minus_text) else {
        return Vec::new();
    };
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Type(plus) = it else {
            continue;
        };
        if type_evolved_match(&minus, plus).is_some() {
            let s = it.span().start().line;
            let e = it.span().end().line;
            if s < 1 || e < s || e > plus_body.len() {
                continue;
            }
            let name = plus.ident.to_string();
            let Some(prefix) = type_header_prefix(&plus_body[(s - 1)..e], &name) else {
                continue;
            };
            out.push((s - 1, e - 1, prefix, name));
            continue;
        }
        // WI-14 slice 8: type move+rename with exactly one added leading
        // attr/doc block. Same composition as the `fn`/struct arms: count
        // gate, kind-agnostic block measure, stripped inner re-parse.
        if plus.attrs.len() != minus.attrs.len() + 1 {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let span_lines: Vec<&str> = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect();
        let Some(k) = super::move_attr::added_attr_block_len(&span_lines) else {
            continue;
        };
        let Ok(inner) = syn::parse_str::<syn::ItemType>(&span_lines[k..].join("\n")) else {
            continue;
        };
        if type_evolved_match(&minus, &inner).is_none() {
            continue;
        }
        let name = inner.ident.to_string();
        let Some(prefix) = type_header_prefix(&plus_body[(s - 1 + k)..e], &name) else {
            continue;
        };
        out.push((s - 1 + k, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `type <name>` keyword on the item's
/// header line (visibility, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn type_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(t_pos) = find_type_name(text, name) {
            return Some(text[..t_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `type <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
fn find_type_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if bytes[i..].starts_with(b"type")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 4;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line type-alias evolution-paste template: the plus-side
/// visibility verbatim, `= ...;` body marker. `prefix` is trimmed;
/// an empty prefix yields a bare `type` header. The `= ...;` tail
/// is a shape marker only — the splice keeps the captured suffix
/// (generics/`= ty;`) verbatim.
pub(crate) fn type_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("type {name} = ...;")
    } else {
        format!("{pre} type {name} = ...;")
    }
}

/// Evolved `const` pair check (WI-8 slice 5): same shape modulo the
/// visibility prefix and the ident. Generics (params + where),
/// attrs, the const type, and the initializer expr must be
/// token-equal. Returns the plus-side ident when the pair evolves (a
/// vis or name delta is required — the byte-exact path owns identical
/// items).
pub(crate) fn const_evolved_match(minus: &syn::ItemConst, plus: &syn::ItemConst) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    if vis_same && name_same {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.ty) != tok(&plus.ty) {
        return None;
    }
    if tok(&minus.expr) != tok(&plus.expr) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All evolved `const` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 5). Mirrors `find_type_evolved_hits`;
/// spans coincide with parsed `const` item spans by construction.
pub(crate) fn find_const_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemConst>(minus_text) else {
        return Vec::new();
    };
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Const(plus) = it else {
            continue;
        };
        if const_evolved_match(&minus, plus).is_some() {
            let s = it.span().start().line;
            let e = it.span().end().line;
            if s < 1 || e < s || e > plus_body.len() {
                continue;
            }
            let name = plus.ident.to_string();
            let Some(prefix) = const_header_prefix(&plus_body[(s - 1)..e], &name) else {
                continue;
            };
            out.push((s - 1, e - 1, prefix, name));
            continue;
        }
        // WI-14 slice 8: const move+rename with exactly one added leading
        // attr/doc block. Mirrors the type arm above.
        if plus.attrs.len() != minus.attrs.len() + 1 {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let span_lines: Vec<&str> = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect();
        let Some(k) = super::move_attr::added_attr_block_len(&span_lines) else {
            continue;
        };
        let Ok(inner) = syn::parse_str::<syn::ItemConst>(&span_lines[k..].join("\n")) else {
            continue;
        };
        if const_evolved_match(&minus, &inner).is_none() {
            continue;
        }
        let name = inner.ident.to_string();
        let Some(prefix) = const_header_prefix(&plus_body[(s - 1 + k)..e], &name) else {
            continue;
        };
        out.push((s - 1 + k, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `const <name>` keyword on the item's
/// header line (visibility, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn const_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(c_pos) = find_const_name(text, name) {
            return Some(text[..c_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `const <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
fn find_const_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"const")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 5;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line const evolution-paste template: the plus-side
/// visibility verbatim, `= ...;` body marker. `prefix` is trimmed;
/// an empty prefix yields a bare `const` header. The `= ...;` tail
/// is a shape marker only — the splice keeps the captured suffix
/// (generics/`: ty = expr;`) verbatim.
pub(crate) fn const_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("const {name} = ...;")
    } else {
        format!("{pre} const {name} = ...;")
    }
}

/// Evolved `static` pair check (WI-8 slice 6): same shape modulo the
/// visibility prefix, the ident, and `mut` mutability. Generics do not
/// exist on free `static` items; attrs, the value type, and the
/// initializer expr must be token-equal. Returns the plus-side ident
/// when the pair evolves (a vis, name, or mutability delta is required
/// — the byte-exact path owns identical items). Mutability rides
/// verbatim in the template like `fn` qualifiers ride the prefix.
pub(crate) fn static_evolved_match(
    minus: &syn::ItemStatic,
    plus: &syn::ItemStatic,
) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    let minus_mut = matches!(minus.mutability, syn::StaticMutability::Mut(_));
    let plus_mut = matches!(plus.mutability, syn::StaticMutability::Mut(_));
    let mut_same = minus_mut == plus_mut;
    if vis_same && name_same && mut_same {
        return None;
    }
    if tok(&minus.ty) != tok(&plus.ty) {
        return None;
    }
    if tok(&minus.expr) != tok(&plus.expr) {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All evolved `static` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 6). Mirrors `find_const_evolved_hits`;
/// spans coincide with parsed `static` item spans by construction.
/// The trailing flag carries the plus-side `mut` so emission rebuilds
/// it verbatim in the template.
pub(crate) fn find_static_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String, bool)> {
    let Ok(minus) = syn::parse_str::<syn::ItemStatic>(minus_text) else {
        return Vec::new();
    };
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Static(plus) = it else {
            continue;
        };
        if static_evolved_match(&minus, plus).is_some() {
            let s = it.span().start().line;
            let e = it.span().end().line;
            if s < 1 || e < s || e > plus_body.len() {
                continue;
            }
            let name = plus.ident.to_string();
            let Some(prefix) = static_header_prefix(&plus_body[(s - 1)..e], &name) else {
                continue;
            };
            out.push((
                s - 1,
                e - 1,
                prefix,
                name,
                matches!(plus.mutability, syn::StaticMutability::Mut(_)),
            ));
            continue;
        }
        // WI-14 slice 8: static move+rename with exactly one added leading
        // attr/doc block. Mirrors the const arm (mutability rides the
        // inner match verbatim like vis).
        if plus.attrs.len() != minus.attrs.len() + 1 {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let span_lines: Vec<&str> = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect();
        let Some(k) = super::move_attr::added_attr_block_len(&span_lines) else {
            continue;
        };
        let Ok(inner) = syn::parse_str::<syn::ItemStatic>(&span_lines[k..].join("\n")) else {
            continue;
        };
        if static_evolved_match(&minus, &inner).is_none() {
            continue;
        }
        let name = inner.ident.to_string();
        let Some(prefix) = static_header_prefix(&plus_body[(s - 1 + k)..e], &name) else {
            continue;
        };
        out.push((
            s - 1 + k,
            e - 1,
            prefix,
            name,
            matches!(inner.mutability, syn::StaticMutability::Mut(_)),
        ));
    }
    out
}

/// The verbatim text before the `static <name>` keyword on the item's
/// header line (visibility, leading whitespace stripped).
/// Attr/doc lines never match: the header is always a code line.
pub(crate) fn static_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(s_pos) = find_static_name(text, name) {
            return Some(text[..s_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `static <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
/// An optional word-bounded `mut` between the keyword and the ident
/// (the `static mut` form) is skipped.
fn find_static_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if bytes[i..].starts_with(b"static")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 6).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 6;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            // Optional `mut` qualifier before the ident.
            let mut k = j;
            while bytes.get(k).is_some_and(|c| is_ident(*c)) {
                k += 1;
            }
            if line.get(j..k) == Some("mut")
                && bytes.get(k).is_some_and(|c| c.is_ascii_whitespace())
            {
                j = k;
                while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                    j += 1;
                }
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line static evolution-paste template: the plus-side
/// visibility verbatim, the plus-side `mut` verbatim, `= ...;` body
/// marker. `prefix` is trimmed; an empty prefix yields a bare
/// `static` header. The `= ...;` tail is a shape marker only — the
/// splice keeps the captured suffix (`: ty = expr;`) verbatim.
pub(crate) fn static_evolution_template(prefix: &str, name: &str, is_mut: bool) -> String {
    let pre = prefix.trim_end();
    let mut_word = if is_mut { "mut " } else { "" };
    if pre.is_empty() {
        format!("static {mut_word}{name} = ...;")
    } else {
        format!("{pre} static {mut_word}{name} = ...;")
    }
}

/// Evolved `mod` pair check (WI-8 slice 7): same shape modulo the
/// visibility prefix (which carries an optional `unsafe` qualifier)
/// and the ident. Attrs must be token-equal; content must be absent on
/// both sides (`mod foo;` declarations) or empty on both sides
/// (`mod foo {}` — a `;`-vs-braced delta vetoes, and any nested item
/// vetoes: the scalar `- I` cut cannot isolate a mod with nested
/// items, it would match them too). Returns the plus-side ident when
/// the pair evolves (a vis, name, or unsafety delta is required — the
/// byte-exact path owns identical items). `unsafe` rides verbatim in
/// the prefix like `fn` qualifiers ride the prefix.
pub(crate) fn mod_evolved_match(minus: &syn::ItemMod, plus: &syn::ItemMod) -> Option<String> {
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    let unsafety_same = minus.unsafety.is_some() == plus.unsafety.is_some();
    if vis_same && name_same && unsafety_same {
        return None;
    }
    if minus.semi.is_some() != plus.semi.is_some() {
        return None;
    }
    match (&minus.content, &plus.content) {
        (None, None) => {}
        // Only empty inline mods evolve (see above): any nested item
        // on either side vetoes, even identical content.
        (Some((_, a)), Some((_, b))) if a.is_empty() && b.is_empty() => {}
        _ => return None,
    }
    Some(plus.ident.to_string())
}

/// All evolved `mod` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 7). Mirrors `find_static_evolved_hits`;
/// spans coincide with parsed `mod` item spans by construction.
pub(crate) fn find_mod_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemMod>(minus_text) else {
        return Vec::new();
    };
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Mod(plus) = it else {
            continue;
        };
        if mod_evolved_match(&minus, plus).is_some() {
            let s = it.span().start().line;
            let e = it.span().end().line;
            if s < 1 || e < s || e > plus_body.len() {
                continue;
            }
            let name = plus.ident.to_string();
            let Some(prefix) = mod_header_prefix(&plus_body[(s - 1)..e], &name) else {
                continue;
            };
            out.push((s - 1, e - 1, prefix, name));
            continue;
        }
        // WI-14 slice 8: empty-mod move+rename with exactly one added
        // leading attr/doc block. Mirrors the other arms; the shared
        // `move_cut_eligible` gate still declines content-bearing mods.
        if plus.attrs.len() != minus.attrs.len() + 1 {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let span_lines: Vec<&str> = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect();
        let Some(k) = super::move_attr::added_attr_block_len(&span_lines) else {
            continue;
        };
        let Ok(inner) = syn::parse_str::<syn::ItemMod>(&span_lines[k..].join("\n")) else {
            continue;
        };
        if mod_evolved_match(&minus, &inner).is_none() {
            continue;
        }
        let name = inner.ident.to_string();
        let Some(prefix) = mod_header_prefix(&plus_body[(s - 1 + k)..e], &name) else {
            continue;
        };
        out.push((s - 1 + k, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `mod <name>` keyword on the item's
/// header line (visibility + optional `unsafe`, leading whitespace
/// stripped). Attr/doc lines never match: the header is always a
/// code line.
pub(crate) fn mod_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(m_pos) = find_mod_name(text, name) {
            return Some(text[..m_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `mod <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
fn find_mod_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 3 <= bytes.len() {
        if bytes[i..].starts_with(b"mod")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 3).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 3;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line mod evolution-paste template: the plus-side
/// visibility (+ optional `unsafe`) verbatim, `{ ... }` body marker.
/// `prefix` is trimmed; an empty prefix yields a bare `mod` header.
/// The `{ ... }` tail is a shape marker only — the splice keeps the
/// captured suffix (`;` or `{}`) verbatim, so the declaration and
/// empty-inline forms round-trip.
pub(crate) fn mod_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("mod {name} {{ ... }}")
    } else {
        format!("{pre} mod {name} {{ ... }}")
    }
}

/// Evolved `trait` pair check (WI-8 slice 8 + E2 slice 1): same shape
/// modulo the visibility prefix (which carries an optional `unsafe`
/// qualifier) and the ident. Attrs, generics, supertraits, AND trait
/// items must be token-equal; a vis, name, or unsafety delta is
/// required (the byte-exact path owns identical items). E2 slice 1
/// admits method-bearing traits whose methods are token-equal (the
/// engine's scalar cut isolates the whole trait via `outermost_spans`
/// and `splice_trait_header` keeps the body verbatim, both probed
/// `verified`); any method delta (rename/add/remove/signature) vetoes
/// (needs the `trait Old as New` splice + method joint, slice 2).
/// Any `auto` token or `impl Trait for` restriction on either side
/// vetoes (mirroring the authoring-side `auto trait` rejection).
/// `unsafe` rides verbatim in the prefix like `fn` qualifiers /
/// `unsafe mod`.
pub(crate) fn trait_evolved_match(minus: &syn::ItemTrait, plus: &syn::ItemTrait) -> Option<String> {
    if minus.auto_token.is_some() || plus.auto_token.is_some() {
        return None;
    }
    if minus.restriction.is_some() || plus.restriction.is_some() {
        return None;
    }
    if minus.attrs.len() != plus.attrs.len()
        || minus
            .attrs
            .iter()
            .zip(plus.attrs.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    let vis_same = tok(&minus.vis) == tok(&plus.vis);
    let name_same = minus.ident == plus.ident;
    let unsafety_same = minus.unsafety.is_some() == plus.unsafety.is_some();
    if vis_same && name_same && unsafety_same {
        return None;
    }
    if tok(&minus.generics) != tok(&plus.generics) {
        return None;
    }
    if tok(&minus.supertraits) != tok(&plus.supertraits) {
        return None;
    }
    // E2 slice 1: methods must be token-equal (any method delta vetoes
    // — rename/add/remove/signature all stay ordinary pending slice 2).
    if minus.items.len() != plus.items.len()
        || minus
            .items
            .iter()
            .zip(plus.items.iter())
            .any(|(a, b)| tok(a) != tok(b))
    {
        return None;
    }
    Some(plus.ident.to_string())
}

/// All evolved `trait` hits of one minus item text in a pure-plus
/// hunk body (WI-8 slice 8). Mirrors `find_mod_evolved_hits`;
/// spans coincide with parsed `trait` item spans by construction.
pub(crate) fn find_trait_evolved_hits(
    minus_text: &str,
    plus_body: &[String],
) -> Vec<(usize, usize, String, String)> {
    let Ok(minus) = syn::parse_str::<syn::ItemTrait>(minus_text) else {
        return Vec::new();
    };
    let fragment: String = plus_body
        .iter()
        .map(|l| l.get(1..).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(file) = syn::parse_str::<syn::File>(&fragment) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for it in &file.items {
        let syn::Item::Trait(plus) = it else {
            continue;
        };
        if trait_evolved_match(&minus, plus).is_some() {
            let s = it.span().start().line;
            let e = it.span().end().line;
            if s < 1 || e < s || e > plus_body.len() {
                continue;
            }
            let name = plus.ident.to_string();
            let Some(prefix) = trait_header_prefix(&plus_body[(s - 1)..e], &name) else {
                continue;
            };
            out.push((s - 1, e - 1, prefix, name));
            continue;
        }
        // WI-14 slice 9: empty-trait move+rename with exactly one added
        // leading attr/doc block (`trait Old` -> `#[derive] trait New`).
        // Mirrors the slice-8 type/const/static/mod arms: count gate,
        // kind-agnostic block measure, stripped inner re-parse through
        // the same evolved matcher (vis-or-name-or-unsafety delta
        // required, so pure attr-added never double-claims the
        // byte-identical axis; method-bearing inners veto inside
        // `trait_evolved_match`), hit past the block so the attr rides
        // the existing T432b ordinary prefix.
        if plus.attrs.len() != minus.attrs.len() + 1 {
            continue;
        }
        let s = it.span().start().line;
        let e = it.span().end().line;
        if s < 1 || e < s || e > plus_body.len() {
            continue;
        }
        let span_lines: Vec<&str> = plus_body[(s - 1)..e]
            .iter()
            .map(|l| l.get(1..).unwrap_or(""))
            .collect();
        let Some(k) = super::move_attr::added_attr_block_len(&span_lines) else {
            continue;
        };
        let Ok(inner) = syn::parse_str::<syn::ItemTrait>(&span_lines[k..].join("\n")) else {
            continue;
        };
        if trait_evolved_match(&minus, &inner).is_none() {
            continue;
        }
        let name = inner.ident.to_string();
        let Some(prefix) = trait_header_prefix(&plus_body[(s - 1 + k)..e], &name) else {
            continue;
        };
        out.push((s - 1 + k, e - 1, prefix, name));
    }
    out
}

/// The verbatim text before the `trait <name>` keyword on the item's
/// header line (visibility + optional `unsafe`, leading whitespace
/// stripped). Attr/doc lines never match: the header is always a
/// code line.
pub(crate) fn trait_header_prefix(item_lines: &[String], name: &str) -> Option<String> {
    for line in item_lines {
        let text = line.get(1..).unwrap_or("");
        let t = text.trim_start();
        if t.starts_with('#') || t.starts_with('/') || t.starts_with('*') {
            continue;
        }
        if let Some(t_pos) = find_trait_name(text, name) {
            return Some(text[..t_pos].trim_start().to_string());
        }
    }
    None
}

/// Byte offset of a word-bounded `trait <name>` pair (`None` when
/// the line holds no such pair: comments, strings, or other idents).
/// An `auto trait <name>` line still yields a hit here — the pair
/// veto happens in `trait_evolved_match`, not the scanner.
fn find_trait_name(line: &str, name: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i..].starts_with(b"trait")
            && (i == 0 || !is_ident(bytes[i - 1]))
            && bytes.get(i + 5).is_some_and(|c| c.is_ascii_whitespace())
        {
            let mut j = i + 5;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            let start = j;
            while bytes.get(j).is_some_and(|c| is_ident(*c)) {
                j += 1;
            }
            if line.get(start..j) == Some(name) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Single-line trait evolution-paste template: the plus-side
/// visibility (+ optional `unsafe`) verbatim, `{ ... }` body marker.
/// `prefix` is trimmed; an empty prefix yields a bare `trait` header.
/// The `{ ... }` tail is a shape marker only — the splice keeps the
/// captured suffix (generics/supertraits/`{...items}`) verbatim.
pub(crate) fn trait_evolution_template(prefix: &str, name: &str) -> String {
    let pre = prefix.trim_end();
    if pre.is_empty() {
        format!("trait {name} {{ ... }}")
    } else {
        format!("{pre} trait {name} {{ ... }}")
    }
}
