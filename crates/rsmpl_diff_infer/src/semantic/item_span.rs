//! Item-span helpers for item-shape emission headers (WI-1b).
//!
//! Split from `semantic/mod.rs` (WI-3c): covering-span computation over
//! the pre-image parse tree lives here; detector registration stays in
//! `mod.rs`. Pure move, no behavior change.

/// WI-1b: covering item span for item-shape detectors (item rename,
/// slot sweep). Item patterns (`- fn Old`, slot headers with `...`
/// bodies) match whole multi-line items, so a 1-line site range can
/// never match under Contained scope — the range must cover every
/// item containing a site line. Returns the tight
/// `(start, end)` over the smallest covering items, or `None` when
/// the pre-image is absent/unparseable or a site sits outside any
/// item (callers keep `# sweep`). Spans come from `syn` (which
/// includes attrs/docs) via the workspace `span-locations` probe.
fn covering_item_span(file_old: Option<&str>, sites: &[usize]) -> Option<(usize, usize)> {
    let file: syn::File = syn::parse_str(file_old?).ok()?;
    fn push(sp: proc_macro2::Span, out: &mut Vec<(usize, usize)>) {
        let (s, e) = (sp.start().line, sp.end().line);
        if s >= 1 && e >= s {
            out.push((s, e));
        }
    }
    fn walk(items: &[syn::Item], out: &mut Vec<(usize, usize)>) {
        use syn::spanned::Spanned;
        for it in items {
            push(it.span(), out);
            match it {
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, out);
                    }
                }
                syn::Item::Impl(imp) => {
                    for i in &imp.items {
                        push(i.span(), out);
                    }
                }
                syn::Item::Trait(tr) => {
                    for i in &tr.items {
                        push(i.span(), out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut spans: Vec<(usize, usize)> = Vec::new();
    walk(&file.items, &mut spans);
    let mut start = usize::MAX;
    let mut end = 0usize;
    for &site in sites {
        let mut best: Option<(usize, usize)> = None;
        for &(s, e) in &spans {
            if s <= site && site <= e && best.is_none_or(|(bs, be)| e - s < be - bs) {
                best = Some((s, e));
            }
        }
        let (s, e) = best?;
        start = start.min(s);
        end = end.max(e);
    }
    if end == 0 {
        return None;
    }
    Some((start, end))
}

/// WI-1b: item-shape emission header. `sites` are the changed header
/// lines; the emitted range covers their items when provable, else
/// the sweep dummy (a 1-line header range could never contain the
/// multi-line match).
pub(crate) fn item_emission_header(
    path: &str,
    rule: &str,
    file_old: Option<&str>,
    sites: &[usize],
) -> String {
    match covering_item_span(file_old, sites) {
        Some((s, e)) => crate::emission_header(path, rule, &[s, e]),
        None => crate::emission_header(path, rule, &[]),
    }
}
