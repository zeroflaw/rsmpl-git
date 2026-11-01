//! Shared pre-image carrier recovery (WI-5 slice 2c).
//!
//! Three call sites used to carry byte-identical `syn` walks answering
//! one question: which `struct`/`enum` holds member `field`/`variant`
//! at 1-based pre-image line `site`?
//!
//! - `item_joint_field_evo.rs` (`field_carrier`, `enum_of_variant`) and
//!   `definition_evolution.rs` (`struct_of_field`, `enum_of_variant`)
//!   share the exactly-one-carrier law: a second same-named member
//!   anywhere vetoes (`unique_only = true`).
//! - `member.rs` (`struct_of_field`, `enum_of_variant`) uses the WI-14
//!   relaxed law: sibling same-named members no longer veto — only the
//!   def site must resolve to exactly one container (`unique_only =
//!   false`).
//!
//! Insertion-point carriers (`definition_addition.rs`
//! `struct_spanning`/`enum_spanning`) are a different law (span, not
//! site) and stay where they are.

/// The `struct S` holding field `field` at 1-based `site`, or `None`.
///
/// `unique_only = true` enforces the exactly-one-carrier law (a second
/// same-named field anywhere vetoes); `false` keeps the WI-14 relaxed
/// law (siblings may share the name, the site must still resolve to
/// exactly one container).
pub(crate) fn struct_carrier(
    pre: &str,
    field: &str,
    site: usize,
    unique_only: bool,
) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut at_site: Vec<String> = Vec::new();
    let mut carriers = 0usize;
    fn walk(
        items: &[syn::Item],
        field: &str,
        site: usize,
        at_site: &mut Vec<String>,
        carriers: &mut usize,
    ) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Struct(s) => {
                    if let syn::Fields::Named(fields) = &s.fields {
                        let mut has = false;
                        for f in &fields.named {
                            if f.ident.as_ref().is_some_and(|i| i == field) {
                                has = true;
                                if f.span().start().line == site {
                                    at_site.push(s.ident.to_string());
                                }
                            }
                        }
                        if has {
                            *carriers += 1;
                        }
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, field, site, at_site, carriers);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, field, site, &mut at_site, &mut carriers);
    if at_site.len() != 1 {
        return None;
    }
    let veto = if unique_only {
        carriers != 1
    } else {
        carriers < 1
    };
    if veto {
        return None;
    }
    at_site.into_iter().next()
}

/// Exactly-one-carrier struct recovery (joint + definition-evolution law).
pub(crate) fn struct_carrier_unique(pre: &str, field: &str, site: usize) -> Option<String> {
    struct_carrier(pre, field, site, true)
}

/// The `enum E` holding `variant` at 1-based `site`, or `None`, under
/// the same `unique_only` law as [`struct_carrier`].
pub(crate) fn enum_carrier(
    pre: &str,
    variant: &str,
    site: usize,
    unique_only: bool,
) -> Option<String> {
    let file: syn::File = syn::parse_str(pre).ok()?;
    let mut at_site: Vec<String> = Vec::new();
    let mut carriers = 0usize;
    fn walk(
        items: &[syn::Item],
        variant: &str,
        site: usize,
        at_site: &mut Vec<String>,
        carriers: &mut usize,
    ) {
        use syn::spanned::Spanned;
        for it in items {
            match it {
                syn::Item::Enum(e) => {
                    let mut has = false;
                    for v in &e.variants {
                        if v.ident == variant {
                            has = true;
                            if v.span().start().line == site {
                                at_site.push(e.ident.to_string());
                            }
                        }
                    }
                    if has {
                        *carriers += 1;
                    }
                }
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        walk(inner, variant, site, at_site, carriers);
                    }
                }
                _ => {}
            }
        }
    }
    walk(&file.items, variant, site, &mut at_site, &mut carriers);
    if at_site.len() != 1 {
        return None;
    }
    let veto = if unique_only {
        carriers != 1
    } else {
        carriers < 1
    };
    if veto {
        return None;
    }
    at_site.into_iter().next()
}

/// Exactly-one-carrier enum recovery (joint + definition-evolution law).
pub(crate) fn enum_carrier_unique(pre: &str, variant: &str, site: usize) -> Option<String> {
    enum_carrier(pre, variant, site, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_gate_vetoes_twins_relaxed_passes() {
        let pre = "struct Old {\n    x: u32,\n    y: u32,\n}\nstruct Other {\n    x: u32,\n}\n";
        // Two same-named fields: unique vetoes, relaxed resolves the site.
        assert_eq!(struct_carrier(pre, "x", 2, true), None);
        assert_eq!(struct_carrier(pre, "x", 2, false).as_deref(), Some("Old"));
        // Unique field resolves under both laws.
        assert_eq!(struct_carrier(pre, "y", 3, true).as_deref(), Some("Old"));
        assert_eq!(struct_carrier(pre, "y", 3, false).as_deref(), Some("Old"));
        // Wrong site resolves nowhere under either law.
        assert_eq!(struct_carrier(pre, "y", 2, true), None);
        assert_eq!(struct_carrier(pre, "y", 2, false), None);
    }

    #[test]
    fn enum_twins_follow_the_same_law() {
        let pre = "enum Old {\n    A(u32),\n}\nenum Other {\n    A(u32),\n}\n";
        assert_eq!(enum_carrier(pre, "A", 2, true), None);
        assert_eq!(enum_carrier(pre, "A", 2, false).as_deref(), Some("Old"));
        let solo = "enum Old {\n    A(u32),\n    B(u32),\n}\n";
        assert_eq!(enum_carrier(solo, "B", 3, true).as_deref(), Some("Old"));
        assert_eq!(enum_carrier(solo, "B", 2, true), None);
    }

    #[test]
    fn mod_nested_carrier_resolves() {
        let pre = "mod m {\n    pub struct S {\n        pub x: u32,\n    }\n}\n";
        assert_eq!(struct_carrier(pre, "x", 3, true).as_deref(), Some("S"));
        let pre_enum = "mod m {\n    pub enum E {\n        A,\n    }\n}\n";
        assert_eq!(enum_carrier(pre_enum, "A", 3, true).as_deref(), Some("E"));
    }
}
