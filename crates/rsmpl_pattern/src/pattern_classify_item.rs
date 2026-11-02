//! Item-rename routing split from `pattern_classify` (D5): bare
//! `<kind> <Name>` renames plus the one-slot vis-evolution joint gate.
//! The caller in `pattern_classify` owns routing order; this module owns
//! the item-rename shape. No behaviour change in the split.

use rsmpl_patch::{ErrorCode, RsmplHunk, StructuredError};

use crate::pattern::PatternKind;

use super::pattern_classify::is_type_pair;

/// Classify a non-function item rename (`struct Old` -> `struct New`).
///
/// Returns `Ok(None)` when neither side is an item-rename shape (the caller
/// falls through to fn/macro/call classification). Either side being an
/// item shape while the other is not — or a kind mismatch — is a hard
/// `PatternConflict`, never a silent fallthrough.
///
/// Escape hatch (T149): when `parse_side` fails on a side but both raw
/// sides parse as Rust types (e.g. `- impl std::fmt::Display`, where the
/// multi-segment bound trips the bare-name gate), the hunk falls through
/// to the general type matcher instead of failing as a malformed rename.
/// Genuine item typos (`struct Old<T>`, `$Meta`, `auto trait`) never parse
/// as types, so they still fail here.
pub(crate) fn classify_item_rename(
    minus_raw: &str,
    plus_raw: &str,
    holes: &[String],
    hunk: &RsmplHunk,
) -> Result<Option<PatternKind>, StructuredError> {
    let minus = match crate::item_rename::parse_side(minus_raw) {
        Ok(m) => m,
        Err(e) => {
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            return Err(e);
        }
    };
    let plus = match crate::item_rename::parse_side(plus_raw) {
        Ok(p) => p,
        Err(e) => {
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            return Err(e);
        }
    };
    match (minus, plus) {
        (Some(mside), Some(pside)) => {
            let mk = mside.kind;
            let mut mn = mside.name;
            let mattrs = mside.attrs;
            let mprefix = mside.prefix;
            let pk = pside.kind;
            let mut pn = pside.name;
            let pattrs = pside.attrs;
            let pprefix = pside.prefix;
            // WI-10b: bare diet/`# ident` names hole-ify in item name
            // position (fn N-hole precedent, SPEC 15.8). Holes only ever
            // evolve attributes or visibility: both sides hole with an
            // attr delta, or both sides hole the same name with a vis
            // step — anything else fails below. `$`-prefixed spellings
            // keep the existing rejection in `parse_side`.
            let minus_hole = holes.contains(&mn);
            let plus_hole = holes.contains(&pn);
            if minus_hole {
                mn = format!("${mn}");
            }
            if plus_hole {
                pn = format!("${pn}");
            }
            if minus_hole || plus_hole {
                if hunk.follows_uses() {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "item name holes cannot be combined with `# follow=uses` in v1 (holes preserve every name; there is nothing to follow)",
                    )
                    .with_repair(StructuredError::fix_pattern_repair(
                        "attr-hole-follow",
                        "drop `# follow=uses`, or write concrete names",
                    )));
                }
                if minus_hole != plus_hole {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        "item name holes must appear on both sides (`- struct N` / `+ struct N` with an attribute delta); a hole on one side only would rename every item to one name",
                    )
                    .with_repair(StructuredError::fix_pattern_repair(
                        "attr-hole-rename",
                        "write the hole on both sides, or write concrete names",
                    )));
                }
            }
            if mk != pk {
                return Err(StructuredError::new(
                    ErrorCode::PatternConflict,
                    format!(
                        "item rename kind mismatch: minus is `{}` but plus is `{}` (kinds must match)",
                        mk.as_str(),
                        pk.as_str()
                    ),
                )
                .with_repair(StructuredError::fix_pattern_repair(
                    "rename-kind-template",
                    "write the same `<kind>` on both sides (`- struct Old` / `+ struct New`); kinds must match",
                )));
            }
            if mn == pn {
                // WI-10a: same-name pairs are attr evolution when the
                // written attr sets differ (normalized); identical names
                // *and* identical attrs *and* identical prefixes stay the
                // rename-usage error. Same-name + same-attrs + differing
                // prefixes is a pure vis-evolution candidate — fall through
                // to the vis gate below instead of erroring here.
                let norm = |ss: &Vec<String>| {
                    ss.iter()
                        .map(|a| a.split_whitespace().collect::<String>())
                        .collect::<Vec<_>>()
                };
                if norm(&mattrs) == norm(&pattrs) && mprefix == pprefix {
                    return Err(StructuredError::new(
                        ErrorCode::PatternConflict,
                        format!(
                            "item rename minus and plus names are identical (`{mn}`); write the new name on the `+` side"
                        ),
                    )
                    .with_repair(StructuredError::fix_pattern_repair(
                        "rename-new-name-template",
                        "write the new name on the `+` side (`- struct Old` / `+ struct New`)",
                    )));
                }
            }
            if mprefix != pprefix {
                // WI-5-R2: a rename may evolve exactly the vis slot (one
                // concrete vis → another, qualifiers preserved); anything
                // else stays the rename-only error via the veto reason.
                if let Some(reason) = bare_vis_evolution_veto(
                    mk,
                    &mn,
                    &pn,
                    &mattrs,
                    &pattrs,
                    &mprefix,
                    &pprefix,
                    minus_hole || plus_hole,
                ) {
                    return Err(StructuredError::new(ErrorCode::PatternConflict, reason).with_repair(
                        StructuredError::fix_pattern_repair(
                            "rename-vis-template",
                            "write the same prefix on both sides for a pure rename, or exactly one closed-set visibility step (`- struct Old` / `+ pub struct Old`, `- pub struct Old` / `+ pub(crate) struct New`)",
                        ),
                    ));
                }
            }
            Ok(Some(PatternKind::ItemRename {
                kind: mk,
                minus_name: mn,
                plus_name: pn,
                minus_attrs: mattrs,
                plus_attrs: pattrs,
                minus_prefix: mprefix,
                plus_prefix: pprefix,
            }))
        }
        (Some(_), None) => {
            // `impl Trait` type patterns (T149): a bare `impl Name` minus
            // with a non-item plus (e.g. `Box<dyn Display>`) is a general
            // type rewrite, not a malformed item rename — fall through
            // when both sides parse as Rust types. Anything else stays a
            // hard authoring error (fail over guessing).
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "item rename plus side must be a bare `<kind> <Name>` pattern matching the minus kind",
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "rename-plus-bare-template",
                "write the plus side as a bare `<kind> <Name>` (`+ struct New`) matching the minus kind, or use an ordinary hunk",
            )))
        }
        (None, Some(_)) => {
            // Symmetric with the above: a non-item minus with a bare-item
            // plus falls through only when both sides are Rust types.
            if is_type_pair(minus_raw, plus_raw) {
                return Ok(None);
            }
            Err(StructuredError::new(
                ErrorCode::PatternConflict,
                "item rename minus side must be a bare `<kind> <Name>` pattern matching the plus kind",
            )
            .with_repair(StructuredError::fix_pattern_repair(
                "rename-minus-bare-template",
                "write the minus side as a bare `<kind> <Name>` (`- struct Old`) matching the plus kind, or use an ordinary hunk",
            )))
        }
        (None, None) => Ok(None),
    }
}

/// WI-5-R2 vis-evolution gate for bare-item renames: `None` when
/// `mprefix` → `pprefix` is exactly one closed-set vis step (concrete
/// names may be equal for pure vis evolution, attrs mirror-equal, no
/// qualifier delta), else `Some(reason)` naming the veto for the author.
/// Same-hole pairs (`- struct N` / `+ pub struct N`) are pure evolution
/// (no rename) and ride the same gate; differing holes are a rename+vis
/// joint with holes and stay vetoed (WI-5 joints are concrete-only).
#[allow(clippy::too_many_arguments)]
pub(crate) fn bare_vis_evolution_veto(
    kind: crate::item_rename::ItemKind,
    minus_name: &str,
    plus_name: &str,
    minus_attrs: &[String],
    plus_attrs: &[String],
    mprefix: &str,
    pprefix: &str,
    has_hole: bool,
) -> Option<String> {
    use crate::item_rename::{is_closed_item_vis, split_vis_rest, ItemKind};
    if has_hole && minus_name != plus_name {
        return Some(
            "item name holes must match on both sides to evolve visibility (`- struct N` / `+ pub struct N`); write concrete names to rename with a visibility change (`- pub struct Old` / `+ pub(crate) struct New`)".to_string(),
        );
    }
    if kind == ItemKind::Impl || kind == ItemKind::MacroRules {
        return Some(format!(
            "`{}` blocks take no visibility (write the bare `<{}> <Name>` form)",
            kind.as_str(),
            kind.as_str()
        ));
    }
    // Same-name pairs are pure vis evolution (struct/enum vis singles):
    // fall through to the closed-set vis gate below instead of vetoing.
    // Same-name + same-vis never reaches here (prefixes equal), and
    // same-name + attr delta still vetoes at the attr gate below.
    let norm = |ss: &[String]| {
        ss.iter()
            .map(|a| a.split_whitespace().collect::<String>())
            .collect::<Vec<_>>()
    };
    if norm(minus_attrs) != norm(plus_attrs) {
        return Some(
            "rename+visibility+attribute is two evolving slots — split the attribute delta into its own hunk or use an ordinary hunk".to_string(),
        );
    }
    let (mvis, mrest) = split_vis_rest(mprefix);
    let (pvis, prest) = split_vis_rest(pprefix);
    if mrest != prest || !mrest.is_empty() {
        return Some(format!(
            "item qualifier differs or is not preserved (`{mrest}` vs `{prest}`); rename+visibility joints preserve `unsafe`/`mut` — write the same qualifier on both sides or neither"
        ));
    }
    if mvis == pvis {
        return Some(format!(
            "item rename prefixes differ (`{mprefix}` vs `{pprefix}`); item renames are rename-only — write the same prefix on both sides or neither (visibility/generics/bodies are preserved)"
        ));
    }
    if !is_closed_item_vis(&mvis) || !is_closed_item_vis(&pvis) {
        return Some(format!(
            "visibility `{mvis}` → `{pvis}` is outside the closed vis set (`\"\"`, `pub`, `pub(crate)`, `pub(super)`, `pub(in <path>)`) — use an ordinary hunk"
        ));
    }
    // Holes bind per site, so a vis spelling the hole name (`pub(in N)`
    // with an `N`-hole) is self-referential for every bound site —
    // compare the bare spelling too (`$` never matches ident tokens).
    let bare_hole = minus_name.strip_prefix('$');
    let names_self_ref = |vis: &str| {
        contains_vis_ident(vis, minus_name) || bare_hole.is_some_and(|b| contains_vis_ident(vis, b))
    };
    if names_self_ref(&mvis) || names_self_ref(&pvis) {
        return Some(
            "self-referential visibility (`pub(in <name>)` naming the renamed item) is an evolution, never a rename joint — use an ordinary hunk".to_string(),
        );
    }
    None
}

/// Ident-boundary containment for vis strings (never substring).
pub(crate) fn contains_vis_ident(haystack: &str, needle: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use rsmpl_patch::{parse_patch, ErrorCode, Hunk, RsmplHunk, StructuredError};

    use crate::pattern::compile;

    fn rename_err(minus: &str, plus: &str) -> StructuredError {
        let patch = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- {minus}\n+ {plus}\n"
        );
        let hunk: RsmplHunk = match parse_patch(&patch)
            .expect("test patch parses")
            .files
            .remove(0)
            .hunks
            .remove(0)
        {
            Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let err = compile(&hunk).expect_err("bad rename must fail");
        assert_eq!(
            err.code,
            ErrorCode::PatternConflict,
            "wrong code for `{minus}` / `{plus}`: {err:?}"
        );
        assert!(err.repair.is_some(), "WI-18 slice 4: must carry repair");
        err
    }

    fn repair_code(err: &StructuredError) -> String {
        err.repair
            .as_ref()
            .expect("repair")
            .get("code")
            .and_then(|v| v.as_str())
            .expect("code str")
            .to_string()
    }

    fn repair_template(err: &StructuredError) -> String {
        err.repair
            .as_ref()
            .expect("repair")
            .get("write_instead")
            .and_then(|v| v.as_str())
            .expect("template str")
            .to_string()
    }

    const GENERIC_FILLER: &str =
        "rewrite the pattern to a supported shape (see SPEC 18.6) or use an ordinary hunk";

    #[test]
    fn wi18_slice4_terse_template_repair_is_specific() {
        // Terse/long mix with no body context lines: the minus parses
        // terse, the plus long, and the both-sides-or-neither gate fires
        // (body context lines would route to the `fn-pattern` body gate
        // instead — pinned by its own `fn-pattern` repair, not this one).
        let patch = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@ rule=x\n# lang=rust\n- fn foo\n+ fn bar(x: i32) { ... }\n";
        let hunk: RsmplHunk = match parse_patch(patch)
            .expect("test patch parses")
            .files
            .remove(0)
            .hunks
            .remove(0)
        {
            Hunk::Rsmpl(rh) => rh,
            _ => panic!("want rsmpl hunk"),
        };
        let err = compile(&hunk).expect_err("terse/long mix must fail");
        assert_eq!(repair_code(&err), "rename-terse-template");
        let w = repair_template(&err);
        assert!(w.contains("terse on both sides"), "got: {w}");
        assert_ne!(w, GENERIC_FILLER);
    }

    #[test]
    fn wi18_slice4_kind_template_repair_is_specific() {
        let err = rename_err("struct Old", "enum New");
        assert_eq!(repair_code(&err), "rename-kind-template");
        assert!(repair_template(&err).contains("kinds must match"));
    }

    #[test]
    fn wi18_slice4_new_name_template_repair_is_specific() {
        let err = rename_err("struct Same", "struct Same");
        assert_eq!(repair_code(&err), "rename-new-name-template");
        let w = repair_template(&err);
        assert!(w.contains("`+ struct New`"), "got: {w}");
        assert_ne!(w, GENERIC_FILLER);
    }

    #[test]
    fn wi18_slice4_vis_template_repair_is_specific() {
        let err = rename_err("static Old", "static mut New");
        assert_eq!(repair_code(&err), "rename-vis-template");
        // Reason byte-identical to HEAD (veto text unchanged).
        assert!(err.to_string().contains("qualifier"), "got: {err}");
        assert_ne!(repair_template(&err), GENERIC_FILLER);
    }

    #[test]
    fn wi18_slice4_plus_bare_template_repair_is_specific() {
        let err = rename_err("struct Old", "foo(1)");
        assert_eq!(repair_code(&err), "rename-plus-bare-template");
        let w = repair_template(&err);
        assert!(w.contains("`+ struct New`"), "got: {w}");
        assert_ne!(w, GENERIC_FILLER);
    }

    #[test]
    fn wi18_slice4_minus_bare_template_repair_is_specific() {
        let err = rename_err("foo(1)", "struct New");
        assert_eq!(repair_code(&err), "rename-minus-bare-template");
        let w = repair_template(&err);
        assert!(w.contains("`- struct Old`"), "got: {w}");
        assert_ne!(w, GENERIC_FILLER);
    }
}
