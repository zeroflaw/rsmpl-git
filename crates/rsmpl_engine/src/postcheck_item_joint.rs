//! WI-5 joint postconditions (SPEC 24.6), split from `postcheck.rs`
//! (D5 line gate).
//!
//! The struct and enum rename+evolution joints verify the renamed
//! container ident AND the evolved member region directly in the
//! post-image (the two edits report two spans but the pure-evo plus
//! re-match owns only the member span — a shared re-match would fail
//! the rename span). Rename-posture counting (WI-1.4, not all-spans):
//! follow edits ride the same hunk's span list, so each arm requires
//! one span on the renamed ident and one on the evolved region.

use rsmpl_pattern::VariantPayload;

use crate::postcheck::spans_overlap;
use crate::postcheck_defs::span_lines;

/// Verify a `StructRenameFieldEvo` hunk in the post-image.
/// `None` when the post-image is unparseable (caller downgrades to
/// `reparse-only`); `Some(false)` fails loud as `RSMPL_MATCH_CONFLICT`.
pub(crate) fn verify_struct_joint(
    new_container: &str,
    field: &str,
    plus_raw: &str,
    post_parsed: Option<&syn::File>,
    spans: &[(u32, u32)],
) -> Option<bool> {
    let parsed = post_parsed?;
    let want_ty: syn::Type = syn::parse_str(plus_raw).ok()?;
    let want_norm = quote::quote!(#want_ty)
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut ident_spans: Vec<(u32, u32)> = Vec::new();
    let mut ty_spans: Vec<(u32, u32)> = Vec::new();
    let mut found = false;
    for it in &parsed.items {
        if let syn::Item::Struct(s) = it {
            if s.ident == *new_container {
                let (a, b) = span_lines(&s.ident);
                ident_spans.push((a as u32, b as u32));
                if let syn::Fields::Named(fields) = &s.fields {
                    for f in &fields.named {
                        if f.ident.as_ref().is_some_and(|i| i == field) {
                            let fty = &f.ty;
                            let got = quote::quote!(#fty)
                                .to_string()
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ");
                            if got == want_norm {
                                found = true;
                                let (a, b) = span_lines(&f.ty);
                                ty_spans.push((a as u32, b as u32));
                            }
                        }
                    }
                }
            }
        }
    }
    if !found {
        return Some(false);
    }
    let on_ident = spans
        .iter()
        .filter(|sp| ident_spans.iter().any(|pr| spans_overlap(**sp, *pr)))
        .count();
    let on_ty = spans
        .iter()
        .filter(|sp| ty_spans.iter().any(|pr| spans_overlap(**sp, *pr)))
        .count();
    Some(on_ident >= 1 && on_ty >= 1)
}

/// Verify an `EnumRenameVariantEvo` hunk in the post-image.
/// Same contract as `verify_struct_joint`.
pub(crate) fn verify_enum_joint(
    new_container: &str,
    variant: &str,
    payload: &VariantPayload,
    post_parsed: Option<&syn::File>,
    spans: &[(u32, u32)],
) -> Option<bool> {
    let parsed = post_parsed?;
    let mut ident_spans: Vec<(u32, u32)> = Vec::new();
    let mut evo_spans: Vec<(u32, u32)> = Vec::new();
    let mut found = false;
    for it in &parsed.items {
        if let syn::Item::Enum(e) = it {
            if e.ident == *new_container {
                let (a, b) = span_lines(&e.ident);
                ident_spans.push((a as u32, b as u32));
                for v in &e.variants {
                    if v.ident == *variant {
                        match payload {
                            VariantPayload::Tuple { plus_raws, .. } => {
                                if let syn::Fields::Unnamed(u) = &v.fields {
                                    let tys: Vec<&syn::Type> =
                                        u.unnamed.iter().map(|f| &f.ty).collect();
                                    if tys.len() == plus_raws.len() {
                                        let mut ok = true;
                                        let mut tmp = Vec::new();
                                        for (t, want) in tys.iter().zip(plus_raws.iter()) {
                                            let want_ty: syn::Type = syn::parse_str(want).ok()?;
                                            let got = quote::quote!(#t)
                                                .to_string()
                                                .split_whitespace()
                                                .collect::<Vec<_>>()
                                                .join(" ");
                                            let want_norm = quote::quote!(#want_ty)
                                                .to_string()
                                                .split_whitespace()
                                                .collect::<Vec<_>>()
                                                .join(" ");
                                            if got != want_norm {
                                                ok = false;
                                                break;
                                            }
                                            let (a, b) = span_lines(t);
                                            tmp.push((a as u32, b as u32));
                                        }
                                        if ok {
                                            found = true;
                                            evo_spans.extend(tmp);
                                        }
                                    }
                                }
                            }
                            VariantPayload::Struct {
                                fields, plus_raws, ..
                            } => {
                                if let syn::Fields::Named(n) = &v.fields {
                                    let have: Vec<String> = n
                                        .named
                                        .iter()
                                        .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                                        .collect();
                                    if have == *fields && n.named.len() == plus_raws.len() {
                                        let mut ok = true;
                                        let mut tmp = Vec::new();
                                        for (f, want) in n.named.iter().zip(plus_raws.iter()) {
                                            let want_ty: syn::Type = syn::parse_str(want).ok()?;
                                            let fty = &f.ty;
                                            let got = quote::quote!(#fty)
                                                .to_string()
                                                .split_whitespace()
                                                .collect::<Vec<_>>()
                                                .join(" ");
                                            let want_norm = quote::quote!(#want_ty)
                                                .to_string()
                                                .split_whitespace()
                                                .collect::<Vec<_>>()
                                                .join(" ");
                                            if got != want_norm {
                                                ok = false;
                                                break;
                                            }
                                            let (a, b) = span_lines(fty);
                                            tmp.push((a as u32, b as u32));
                                        }
                                        if ok {
                                            found = true;
                                            evo_spans.extend(tmp);
                                        }
                                    }
                                }
                            }
                            VariantPayload::Discriminant { plus_raw, .. } => {
                                if let Some((_, e)) = &v.discriminant {
                                    let want: syn::Expr = syn::parse_str(plus_raw).ok()?;
                                    let got = quote::quote!(#e)
                                        .to_string()
                                        .split_whitespace()
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    let want_norm = quote::quote!(#want)
                                        .to_string()
                                        .split_whitespace()
                                        .collect::<Vec<_>>()
                                        .join(" ");
                                    if got == want_norm {
                                        found = true;
                                        let (a, b) = span_lines(e);
                                        evo_spans.push((a as u32, b as u32));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if !found {
        return Some(false);
    }
    let on_ident = spans
        .iter()
        .filter(|sp| ident_spans.iter().any(|pr| spans_overlap(**sp, *pr)))
        .count();
    let on_evo = spans
        .iter()
        .filter(|sp| evo_spans.iter().any(|pr| spans_overlap(**sp, *pr)))
        .count();
    Some(on_ident >= 1 && on_evo >= 1)
}
