//! Item-intent joint emission (WI-21 slice 1 D5 split).
//!
//! Split from `item_intent` (which hit the 2000-line gate): the
//! definition/member/addition/deletion joint emission match. Pure move —
//! the match arms, the stacking veto (`None`), and the plain-emit
//! fallback are verbatim from the pre-pass; the caller owns rule
//! reservation release on `None`.

/// Build the item-intent emission for exactly one joint shape (or the
/// plain rename when every joint is `None`). `None` means joints stack
/// — the caller releases the reserved rule and declines.
#[allow(clippy::too_many_arguments)]
pub(crate) fn joint_emission(
    def_path: &str,
    rule: &str,
    def_kind: &str,
    def_prefix: &str,
    old_ident: &str,
    new_ident: &str,
    pairs: &[super::item_intent::Pair],
    attr_joint: Option<usize>,
    field_joint: Option<usize>,
    variant_joint: Option<usize>,
    add_joint: Option<&super::item_joint_field_add::AddJoint>,
    variant_add_joint: Option<&super::item_joint_field_add::AddJoint>,
    del_joint: Option<&super::item_joint_delete::DelJoint>,
) -> Option<String> {
    match (
        attr_joint,
        field_joint,
        variant_joint,
        add_joint,
        variant_add_joint,
        del_joint,
    ) {
        (Some(api), None, None, None, None, None) => Some(super::item_joint_attr::joint_emit(
            def_path,
            rule,
            def_kind,
            def_prefix,
            old_ident,
            new_ident,
            &pairs[api].minus,
            &pairs[api].plus,
        )),
        (None, Some(fpi), None, None, None, None) => Some(super::item_joint_field_evo::joint_emit(
            def_path,
            rule,
            def_kind,
            def_prefix,
            old_ident,
            new_ident,
            &pairs[fpi].minus,
            &pairs[fpi].plus,
        )),
        (None, None, Some(vpi), None, None, None) => Some(super::item_joint_field_evo::joint_emit(
            def_path,
            rule,
            def_kind,
            def_prefix,
            old_ident,
            new_ident,
            &pairs[vpi].minus,
            &pairs[vpi].plus,
        )),
        (None, None, None, Some(add), None, None) => Some(super::item_joint_field_add::add_emit(
            def_path,
            rule,
            def_kind,
            def_prefix,
            old_ident,
            new_ident,
            add.point,
            &add.member_plus,
        )),
        (None, None, None, None, Some(add), None) => {
            Some(super::item_joint_field_add::variant_add_emit(
                def_path,
                rule,
                def_kind,
                def_prefix,
                old_ident,
                new_ident,
                add.point,
                &add.member_plus,
            ))
        }
        (None, None, None, None, None, Some(del)) => Some(super::item_joint_delete::del_emit(
            def_path,
            rule,
            def_kind,
            def_prefix,
            old_ident,
            new_ident,
            del.point,
            &del.member_minus,
        )),
        (None, None, None, None, None, None) => Some(super::item_joint_attr::emit(
            def_path, rule, def_kind, def_prefix, old_ident, new_ident,
        )),
        _ => None,
    }
}
