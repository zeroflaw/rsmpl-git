//! WI-5: name-hole (`N` / `# ident`) plus-side verification (SPEC 24.6).
//!
//! A name hole replays the bound name, so the plus side carries no concrete
//! ident for the WI-1.4 needle check (`fn $N` never occurs as text). The
//! name itself is preserved by construction (`plan_sig_edits` emits no
//! name edit for `$`-names); what must verify is the evolved slots. This
//! module re-matches the authored plus `FnPattern` structurally at each
//! reported span with the same strict matcher the engine uses
//! (`matches_item_fn` + `minus_matches_strict`), over free fns and
//! `impl`/`trait` methods in the post-image. Any span with no
//! slot-matching overlapping fn fails the run loud via the standard
//! instantiation error (never a substring, never `reparse-only`).
//!
//! Sibling of `postcheck` under the D5 file-size gate (`postcheck.rs`
//! stays untouched apart from the two call sites).

use rsmpl_patch::MetavarKind;
use syn::spanned::Spanned;

use crate::lines::LineIndex;

/// True when the plus-side fn name is a name hole: the stripped name is
/// declared as `# ident` (diet `N` infers to `Ident` in name position).
pub(crate) fn plus_name_is_hole(
    compiled: &rsmpl_pattern::CompiledPattern,
    plus_name_raw: &str,
) -> bool {
    let bare = plus_name_raw
        .trim()
        .trim_start_matches('$')
        .trim_matches('`');
    compiled.table.kind_of(bare) == Some(MetavarKind::Ident)
}

/// A post-image function candidate: free fns (file + inline mods) and
/// `impl`/`trait` methods share the slot planner, so verification covers
/// both. Trait methods carry inherited visibility, mirroring the
/// matcher's synthetic-ItemFn discipline.
struct PostFn {
    vis: syn::Visibility,
    sig: syn::Signature,
    attrs: Vec<syn::Attribute>,
    start_line: u32,
    end_line: u32,
}

fn push_fn(
    out: &mut Vec<PostFn>,
    vis: syn::Visibility,
    sig: &syn::Signature,
    attrs: &[syn::Attribute],
    span: proc_macro2::Span,
) {
    out.push(PostFn {
        vis,
        sig: sig.clone(),
        attrs: attrs.to_vec(),
        start_line: span.start().line as u32,
        end_line: span.end().line as u32,
    });
}

fn collect_from_items(items: &[syn::Item], out: &mut Vec<PostFn>) {
    for it in items {
        match it {
            syn::Item::Fn(f) => push_fn(out, f.vis.clone(), &f.sig, &f.attrs, f.span()),
            syn::Item::Impl(imp) => {
                for item in &imp.items {
                    if let syn::ImplItem::Fn(m) = item {
                        push_fn(out, m.vis.clone(), &m.sig, &m.attrs, m.span());
                    }
                }
            }
            syn::Item::Trait(t) => {
                let inherited = syn::Visibility::Inherited;
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item {
                        push_fn(out, inherited.clone(), &f.sig, &f.attrs, f.span());
                    }
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_from_items(inner, out);
                }
            }
            _ => {}
        }
    }
}

/// Verify the plus fn slots at each reported span (WI-5 analogue of the
/// WI-1.4 needle check). Returns `None` when the plus pattern cannot be
/// parsed (caller downgrades to `reparse-only`); otherwise `Some(true)`
/// iff every reported span overlaps a slot-matching post-image fn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verify_fn_plus_slots(
    rh: &rsmpl_patch::RsmplHunk,
    compiled: &rsmpl_pattern::CompiledPattern,
    post_parsed: &syn::File,
    post_text: &str,
    post_index: &LineIndex,
    spans: &[(u32, u32)],
    matches: usize,
) -> Option<bool> {
    let (vis, qual, gen, wh, ty) = crate::matcher_fn::slot_metas(compiled);
    let ident = compiled.table.ident_slot_metas();
    let full_plus: String = rh
        .lines
        .iter()
        .filter(|l| l.is_addition() || l.is_context())
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let block = match &compiled.kind {
        rsmpl_pattern::PatternKind::ImplContext { .. } => {
            crate::matcher_fn::extract_fn_block(&full_plus, &[vis.clone(), qual.clone()].concat())
        }
        _ => full_plus,
    };
    let plus_pat =
        rsmpl_pattern::parse_fn_pattern(&block, &vis, &qual, &gen, &wh, &ty, &ident, false).ok()?;
    let mut candidates = Vec::new();
    collect_from_items(&post_parsed.items, &mut candidates);
    // An empty dummy block for the `matches_item_fn` receiver (the
    // matcher only inspects vis/sig/attrs, never the body).
    let dummy_block: syn::Block = syn::parse_str("{}").ok()?;
    let mut hits = 0;
    for sp in spans {
        let ok = candidates.iter().any(|c| {
            if c.start_line > sp.1 || sp.0 > c.end_line {
                return false;
            }
            let synth = syn::ItemFn {
                attrs: c.attrs.clone(),
                vis: c.vis.clone(),
                sig: c.sig.clone(),
                block: Box::new(dummy_block.clone()),
            };
            if !plus_pat.matches_item_fn(&synth) {
                return false;
            }
            if !crate::matcher_fn::minus_matches_strict(
                &plus_pat, &c.vis, &c.sig, post_text, post_index,
            ) {
                return false;
            }
            if rh.returns_none() && !matches!(c.sig.output, syn::ReturnType::Default) {
                return false;
            }
            if !plus_pat.attrs_written.is_empty()
                && !crate::matcher_fn::attrs_equal(&plus_pat.attrs_written, &c.attrs)
            {
                return false;
            }
            true
        });
        if ok {
            hits += 1;
        }
    }
    Some(hits >= matches.max(1))
}
