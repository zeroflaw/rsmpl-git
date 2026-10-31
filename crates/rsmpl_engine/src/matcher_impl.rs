//! `impl`/`trait`-context method matching (SPEC 13.4).
//!
//! Split from `matcher_fn` under the 2000-line gate (T426): the shared
//! fn-signature planner (`minus_fn_pattern`, `plan_sig_edits`, …) stays in
//! `matcher_fn` and is reused here via `pub(crate)` imports.

use syn::spanned::Spanned;

use rsmpl_patch::{AttrsPolicy, ErrorCode, RsmplHunk, Scope, StructuredError};
use rsmpl_pattern::{CompiledPattern, PatternKind};

use crate::edits::TextEdit;
use crate::lines::LineIndex;
use crate::matcher_fn::{
    attrs_equal, extract_fn_block, minus_fn_pattern, minus_matches_strict, norm_slot,
    plan_sig_edits, slot_metas,
};

/// Rename a function inside an `impl` or `trait` context block (SPEC 29.9).
pub fn apply_impl_context_hunk(
    parsed: &syn::File,
    original: &str,
    index: &LineIndex,
    hunk: &RsmplHunk,
    compiled: &CompiledPattern,
    edits: &mut Vec<TextEdit>,
) -> Result<usize, StructuredError> {
    let impl_header = match &compiled.kind {
        PatternKind::ImplContext { impl_header } => impl_header.clone(),
        _ => {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "expected impl-context pattern".to_string(),
            ))
        }
    };
    // WI-4a: the header is a filter in every scope (SPEC 13.4). It is
    // parsed once into an `ImplHeaderFilter` and compared
    // structurally against each candidate block — `quote!`-normalized,
    // so authored spacing never matters. The old advisory fallbacks
    // (token-containment mention, range-overlap acceptance, the
    // `# scope=file` skip) are deleted: only the named block matches,
    // and an unparseable header matches nothing (loud zero-match).
    let filter = rsmpl_pattern::parse_impl_header(&impl_header);
    let rule = hunk.rule_name();
    // Method-context covers `impl` blocks (existing behavior) and `trait`
    // blocks (T182: `;` declarations + default bodies via `TraitItem::Fn`).
    // The pattern header picks the half: an `impl` header never matches a
    // trait block and vice versa (enforced inside `impl_block_matches`).
    let want_trait = rsmpl_pattern::is_trait_header(&impl_header);
    let mut matched = 0;
    for it in &parsed.items {
        // Resolve each item to a borrowable method list; other items (incl.
        // the opposite block kind) are skipped. `inherited` backs the
        // trait-method view (trait methods carry no `vis` field).
        let inherited = syn::Visibility::Inherited;
        let mut methods: Vec<(
            &syn::Visibility,
            &syn::Signature,
            &[syn::Attribute],
            proc_macro2::Span,
        )> = Vec::new();
        match it {
            syn::Item::Impl(imp) if !want_trait => {
                // WI-4a: strict filter — no advisory fallback, no
                // range-overlap acceptance. Only the named block matches.
                let pass = filter
                    .as_ref()
                    .is_some_and(|f| rsmpl_pattern::impl_block_matches(f, it));
                if !pass {
                    continue;
                }
                for item in &imp.items {
                    if let syn::ImplItem::Fn(m) = item {
                        methods.push((&m.vis, &m.sig, &m.attrs, m.span()));
                    }
                }
            }
            syn::Item::Trait(t) if want_trait => {
                // WI-4a: strict filter, impl parity — only the named
                // trait block matches; garbage headers match nothing.
                let pass = filter
                    .as_ref()
                    .is_some_and(|f| rsmpl_pattern::impl_block_matches(f, it));
                if !pass {
                    continue;
                }
                for item in &t.items {
                    if let syn::TraitItem::Fn(f) = item {
                        methods.push((&inherited, &f.sig, &f.attrs, f.span()));
                    }
                }
            }
            _ => continue,
        }
        // Match methods inside (same slot-evolution semantics as free
        // functions, via the shared planner).
        let minus_pat = minus_fn_pattern(hunk, compiled).map_err(|e| e.with_rule(&rule))?;
        let (pvis, pqual, pgen, pwh, pty) = slot_metas(compiled);
        let pident = compiled.table.ident_slot_metas();
        let full_plus_all: String = hunk
            .lines
            .iter()
            .filter(|l| l.is_addition() || l.is_context())
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let plus_pat = rsmpl_pattern::parse_fn_pattern(
            &extract_fn_block(&full_plus_all, &[pvis.clone(), pqual.clone()].concat()),
            &pvis,
            &pqual,
            &pgen,
            &pwh,
            &pty,
            &pident,
            false,
        )
        .map_err(|e| e.with_rule(&rule))?;
        if !minus_pat.body_ellipsis || !plus_pat.body_ellipsis {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "v1 function hunks require `...` bodies on both sides (body evolution is not supported; the original body is always preserved)",
            )
            .with_rule(&rule));
        }
        // Attribute mirror gates (same contract as `apply_fn_hunk`: the
        // plus side must mirror the minus side, never invent or drop).
        if !plus_pat.attrs_written.is_empty() && minus_pat.attrs_written.is_empty() {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "replacement writes attributes the pattern did not match; write the same attributes in the `-` pattern or drop them",
            )
            .with_rule(&rule)
            .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
            .with_repair(StructuredError::fix_pattern_repair(
                "C11",
                "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
            )));
        }
        if hunk.attrs_policy() != AttrsPolicy::Strip
            && !minus_pat.attrs_written.is_empty()
            && minus_pat
                .attrs_written
                .iter()
                .map(|a| norm_slot(a))
                .collect::<Vec<_>>()
                != plus_pat
                    .attrs_written
                    .iter()
                    .map(|a| norm_slot(a))
                    .collect::<Vec<_>>()
        {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "replacement attributes differ from matched attributes; v1 refuses ambiguous attribute merges",
            )
            .with_rule(&rule)
            .with_suggestion("mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`")
            .with_repair(StructuredError::fix_pattern_repair(
                "C12",
                "mirror the `#[...]` lines in `-` and `+`, or use `# attrs=strip`",
            )));
        }
        let minus_name = pattern_name(&minus_pat.name);
        let plus_name = pattern_name(&plus_pat.name);
        // T182: trait methods never carry visibility qualifiers, so a `+`
        // side writing `pub` could only emit invalid Rust (the reparse
        // backstop would catch it with a confusing message). Fail fast with
        // the repair; the minus side needs no gate (explicit `pub` simply
        // never matches inherited trait visibility).
        if want_trait && matches!(plus_pat.visibility, rsmpl_pattern::VisPat::Concrete(_)) {
            return Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "trait methods cannot declare visibility: drop `pub` (or `pub(...)`) on the `+` side",
            )
            .with_rule(&rule));
        }
        for (vis, sig, attrs, span) in methods {
            let name_ok = if minus_name.starts_with('$') {
                true
            } else {
                sig.ident == minus_name.as_str()
            };
            if !name_ok {
                continue;
            }
            // Scope: method span within old range.
            let ms = span.start().line as u32;
            let me = span.end().line as u32;
            let end = hunk.header.old_start + hunk.header.old_len.saturating_sub(1);
            let inside = match hunk.scope() {
                Scope::File => true,
                Scope::Contained => {
                    ms >= hunk.header.old_start && me <= end.max(hunk.header.old_start)
                }
                _ => ms >= hunk.header.old_start && ms <= end.max(hunk.header.old_start),
            };
            if !inside {
                continue;
            }
            // Qualifier/visibility strictness via minus pattern on a
            // synthetic ItemFn. The synth carries the method's real
            // visibility: `Signature` alone renders inherited vis, which
            // would fail every explicit-`pub` pattern (T175).
            let synth: syn::ItemFn =
                syn::parse_str(&format!("{} {{}}", full_sig_without_body(vis, sig))).map_err(
                    |_| {
                        StructuredError::new(
                            ErrorCode::PatternConflict,
                            "cannot analyze method signature",
                        )
                        .with_rule(&rule)
                    },
                )?;
            if !minus_pat.matches_item_fn(&synth) {
                continue;
            }
            if !minus_matches_strict(&minus_pat, vis, sig, original, index) {
                continue;
            }
            // Written attributes constrain under default preserve, like
            // free functions (site-level filter, not an error).
            if hunk.attrs_policy() == AttrsPolicy::Preserve
                && !minus_pat.attrs_written.is_empty()
                && !attrs_equal(&minus_pat.attrs_written, attrs)
            {
                continue;
            }
            if hunk.returns_none() && !matches!(sig.output, syn::ReturnType::Default) {
                continue;
            }
            // Signature evolution: name plus any explicitly changed slots.
            // WI-5: name holes report already-conforming methods with a
            // byte-identical name edit (same posture as free functions).
            let name_hole = crate::matcher_fn::is_ident_hole(compiled, &minus_name);
            let mut new_edits = plan_sig_edits(
                vis,
                sig,
                &minus_pat,
                &plus_pat,
                &minus_name,
                &plus_name,
                original,
                index,
                &rule,
                name_hole,
            )?;
            edits.append(&mut new_edits);
            matched += 1;
        }
    }
    Ok(matched)
}

/// Pattern function name as match text (`$Meta` stays sigiled).
fn pattern_name(name: &rsmpl_pattern::NamePat) -> String {
    match name {
        rsmpl_pattern::NamePat::Concrete(n) => n.clone(),
        rsmpl_pattern::NamePat::Meta(n) => format!("${n}"),
    }
}

/// Render a signature without its body for synthetic parsing, carrying the
/// item's real visibility: `Signature` alone has none, so without `vis` the
/// synth would read as inherited and fail every explicit-`pub` pattern.
fn full_sig_without_body(vis: &syn::Visibility, sig: &syn::Signature) -> String {
    let mut s = quote::quote!(#sig).to_string();
    // `quote` renders `fn foo (x : i32)` etc.; parse needs no trailing body.
    // Drop anything from the first `{` (shouldn't be present for Signature).
    if let Some(pos) = s.find('{') {
        s.truncate(pos);
    }
    // Ensure it starts with `fn` (methods have receiver; keep as-is for
    // qualifier checks by wrapping in a trait? No: parse as ItemFn requires
    // no `self`. Replace `self` params textually for the check.)
    // NOTE: the `self` rewrite stays scoped to the sig text -- it runs
    // before the vis prepend so `pub(in self::...)` paths are untouched.
    s = replace_self_receiver(&s);
    // T190: `quote!(#sig)` renders qualifiers (`const fn …`, `async fn …`,
    // `unsafe fn …`, `extern "ABI" fn …`), so gating the defensive `fn `
    // prepend on a bare-`fn` prefix turned every qualified method synth into
    // `fn const fn …` (unparsable → bare `cannot analyze method signature`).
    // Accept the qualifier prefixes; the prepend stays for whatever
    // non-`fn` rendering needed it.
    let t = s.trim_start();
    if !t.starts_with("fn ")
        && !t.starts_with("const ")
        && !t.starts_with("async ")
        && !t.starts_with("unsafe ")
        && !t.starts_with("extern ")
    {
        s = format!("fn {s}");
    }
    let vis_text = quote::quote!(#vis).to_string();
    if vis_text.trim().is_empty() {
        s
    } else {
        format!("{vis_text} {s}")
    }
}

/// Replace the `self` receiver for synthetic parsing, word-aware (T221).
/// The impl/trait-context presence gate parses a synthetic free `fn`
/// built from the method signature, and a bare `self` receiver does not
/// parse there -- so the receiver is rewritten to a placeholder param.
/// The old blind `str::replace("self", …)` broke three valid spellings:
/// lifetime receivers (`&'a self` -- reference patterns take no
/// lifetime), idents merely containing `self` (`myself`), and typed
/// receivers (`self: Box<Self>`). All three failed with the bare
/// `cannot analyze method signature` needle. Scan rules: `& 'lt [mut]
/// self` drops the lifetime (reference pattern + `: u8` type, as
/// before); a `self` word followed by `:` (but not `::`) keeps its type
/// (`self: Box<Self>` becomes the placeholder with the same type); any
/// other `self` word becomes the placeholder param. `self::` paths and
/// `Self` are never touched.
fn replace_self_receiver(s: &str) -> String {
    fn is_ident(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < b.len() {
        // `& 'lt [mut] self`: drop the lifetime, keep `& [mut]`.
        if b[i] == b'&' {
            let mut j = i + 1;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b'\'' {
                j += 1;
                while j < b.len() && is_ident(b[j]) {
                    j += 1;
                }
                while j < b.len() && b[j] == b' ' {
                    j += 1;
                }
                let mut has_mut = false;
                if s[j..].starts_with("mut") && (j + 3 >= b.len() || !is_ident(b[j + 3])) {
                    has_mut = true;
                    j += 3;
                    while j < b.len() && b[j] == b' ' {
                        j += 1;
                    }
                }
                if s[j..].starts_with("self") && (j + 4 >= b.len() || !is_ident(b[j + 4])) {
                    out.push_str("& ");
                    if has_mut {
                        out.push_str("mut ");
                    }
                    out.push_str("_self_placeholder : u8");
                    i = j + 4;
                    continue;
                }
            }
            out.push('&');
            i += 1;
            continue;
        }
        if s[i..].starts_with("self")
            && (i == 0 || !is_ident(b[i - 1]))
            && (i + 4 >= b.len() || !is_ident(b[i + 4]))
        {
            // Typed receiver (`self: Type`, not `self::path`): keep the type.
            let mut j = i + 4;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b':' {
                if j + 1 >= b.len() || b[j + 1] != b':' {
                    out.push_str("_self_placeholder");
                    i += 4;
                    continue;
                }
                // `self::…`: not a receiver; leave untouched.
                out.push_str("self");
                i += 4;
                continue;
            }
            out.push_str("_self_placeholder : u8");
            i += 4;
            continue;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}
