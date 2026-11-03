//! Signature-evolution sweep (WI-4c) — fn-header pairs with one uniform slot delta.
//
//! Split from `generalized.rs` under the 2000-line gate (D5): pure move,
//! no behavior change. `generalized.rs` keeps the let/type/sequence
//! sweeps; this module owns `try_sig_evolution_sweep` plus its slot
//! vocabulary (`SlotClass`/`Emit`/`SigSlots` and helpers).
//
//! Method (indented) N = 1 vis/qual/return singles pay a 64-byte method-run
//! price (four guards) for the mandatory WI-4a filter block (a proof
//! anchor, D10, not bloat): the whole-section arm allows 32 over its
//! prelude-carrying section (`chain.rs`), the per-hunk fallback
//! (`lib.rs`) allows 64 over the bare pair hunk, and the subhunk scaled
//! guard prices three extra guards per method run beyond its base.
//! Each budget additionally prices the verbatim `impl` filter header
//! per emission (`block_headers::filter_header_excess`): headers longer
//! than the measured baseline contribute exactly their surplus, so a
//! long enclosing block never vetoes a proved single on bytes alone.
//! Totality is the engine-count check plus the scratch-apply
//! byte-compare in every path.

use super::unique_rule;
use quote::ToTokens;

/// N-site (N ≥ 2) signature-evolution sweep (WI-4c): fn-header pairs
/// where the name plus any of vis/qualifiers/generics/params/return/
/// where carry one uniform concrete delta, bodies untouched. Each pair
/// is a single-line `{`-terminated header (`ItemFn`/`ImplItemFn`
/// probe); uniformly column-0 sites emit the free-fn sweep, uniformly
/// indented sites one hunk per enclosing `impl` block with its
/// verbatim header line (WI-4a: the engine header filter matches only
/// the named block); mixed indent stays passthrough. Every non-name slot is
/// either preserved (per-site minus == plus) or evolving (one uniform
/// plus text; the base may be hetero). Emission spells uniform bases
/// concrete (absent → omitted), hetero preserved bases as wildcards
/// (`V`/`Q`/`G`/`W`, params `(...)`, return omitted = `Any`), hetero
/// evolving bases as wildcards on the minus side with the plus side
/// setting the slot wholesale, and removals as minus-concrete/
/// plus-omitted (hetero-base removal is engine-inexpressible and
/// declines). Divergent plus sides stay ordinary: the engine's slots
/// are all-or-nothing (`...`, concrete, or omitted), so a
/// per-site-varying target is inexpressible. The rename-only zero-delta case never reaches here
/// (the slot sweep runs first and keeps those classes), hence ≥ 1
/// evolving non-name slot is required. A concrete plus-side return
/// drops the `where` wildcard (`-> T W` is unparsable); every other
/// wildcard/concrete mix is probed legal. Totality is the engine-count
/// check over the emitted minus header (`template_fn_holds` for free
/// fns, one `probe_block_hunk` per block for methods):
/// wildcard/`Any` slots match broadly, so an extra same-name fn
/// anywhere in the file vetoes (the `slot_holds` posture). Detector
/// name `sig_evolution`.
pub(crate) fn try_sig_evolution_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    // WI-19b: N=1 slot-evolution singles (uniform-by-construction) ride
    // the same exactly-1-evolving gate + engine-count totality under
    // the precision guard (callers). Disjoint from `try_item_rename`
    // (suffix-equal gate) and the slot sweep (rename-only gate). N=1
    // requires a pre-image (same wide-minus doctrine as the slot sweep).
    if minuses.is_empty() || minuses.len() != pluses.len() {
        return None;
    }
    if minuses.len() == 1 && file_old.is_none() {
        return None;
    }
    if changed.len() != minuses.len() {
        return None;
    }
    // WI-19b: the all-identical veto is an N≥2 verbatim-territory rule;
    // at N=1 it is trivially true and must not fire.
    if minuses.len() > 1
        && minuses.iter().all(|m| *m == minuses[0])
        && pluses.iter().all(|p| *p == pluses[0])
    {
        return None;
    }
    let all_indented = minuses.iter().all(|m| m.starts_with([' ', '\t']));
    let all_col0 = minuses
        .iter()
        .all(|m| !m.is_empty() && !m.starts_with([' ', '\t']));
    if !all_indented && !all_col0 {
        return None;
    }
    let plus_indented = pluses.iter().all(|p| p.starts_with([' ', '\t']));
    let plus_col0 = pluses
        .iter()
        .all(|p| !p.is_empty() && !p.starts_with([' ', '\t']));
    if all_indented != plus_indented || all_col0 != plus_col0 {
        return None;
    }
    let mut minus_slots = Vec::with_capacity(minuses.len());
    let mut plus_slots = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        minus_slots.push(parse_sig_slots(m)?);
        plus_slots.push(parse_sig_slots(p)?);
    }
    let old_name = minus_slots[0].name.clone();
    let new_name = plus_slots[0].name.clone();
    for (m, p) in minus_slots.iter().zip(plus_slots.iter()) {
        if m.name != old_name || p.name != new_name {
            return None;
        }
    }
    let vis = classify_slot(
        slot_col(&minus_slots, |s| &s.vis),
        slot_col(&plus_slots, |s| &s.vis),
        false,
    )?;
    let qual = classify_slot(
        slot_col(&minus_slots, |s| &s.qual),
        slot_col(&plus_slots, |s| &s.qual),
        false,
    )?;
    let generics = classify_slot(
        slot_col(&minus_slots, |s| &s.generics),
        slot_col(&plus_slots, |s| &s.generics),
        false,
    )?;
    let params = classify_slot(
        slot_col(&minus_slots, |s| &s.params),
        slot_col(&plus_slots, |s| &s.params),
        true,
    )?;
    let ret = classify_slot(
        slot_col(&minus_slots, |s| &s.ret),
        slot_col(&plus_slots, |s| &s.ret),
        false,
    )?;
    let where_ = classify_slot(
        slot_col(&minus_slots, |s| &s.where_),
        slot_col(&plus_slots, |s| &s.where_),
        true,
    )?;
    // WI-19b: N=1 owns vis/qual evolutions (single-token slots,
    // formatting-stable under rewrite) plus formatting-stable return
    // evolutions (WI-2 slice 3: `-> i32` → `-> i64` re-renders
    // byte-identically, so the 113 vis+ret shape and the ret-only twin
    // compress; complex returns decline via `ret_slot_stable` below).
    // A params/generics/where evolution belongs to the type-sweep
    // family (strict N≥2 by design): the sig rewrite would re-render
    // the type with pretty spacing (`Box<[i32]>` → `Box <[ i32 ]>`,
    // 117 precedent), losing to the ordinary hunk it claims to compress.
    if minuses.len() == 1 {
        for c in [&params, &generics, &where_] {
            if matches!(c, SlotClass::Evolving { .. }) {
                return None;
            }
        }
        if !ret_slot_stable(&ret) {
            return None;
        }
    }
    // WI-14 slice 3: at N=1 non-empty preserved long slots keep the
    // WI-19b wildcard (concrete pretty-printing would re-render types
    // like `&str`/`Option<String>` and fail the D13 byte-compare, as
    // probed on `b_side_path`); empty preserved slots stay concrete
    // (`()`/omission instead of `G (...) W`), which is what loses the
    // precision guard on short headers. N≥2 keeps the full wildcard
    // diet.
    let (generics, params, mut where_) = if minuses.len() == 1 {
        wildcard_preserved_nonempty_1(generics, params, where_)
    } else {
        (generics, params, where_)
    };
    // WI-14 slice 3 (fixup restore): the `-> T W` pair below needs a
    // `W` on any side with a concrete return, or the minus side carries
    // the full pretty return and loses the guard (probed: `b_side_path`
    // `-> Option<String>`). At N=1 a preserved or stable-evolving
    // return (complex evolving returns decline above) with a non-empty
    // preserved return forces a wildcard `where` (matching the pre-fixup
    // WI-19b shape: minus omits the return and keeps `W`, plus sets it).
    if minuses.len() == 1
        && matches!(
            &ret,
            SlotClass::Preserved { uniform: Some(t) } if !t.is_empty()
        )
        && matches!(
            &where_,
            SlotClass::Preserved { uniform: Some(t) } if t.is_empty()
        )
    {
        where_ = SlotClass::Preserved { uniform: None };
    }
    // The rename-only zero-delta class belongs to the slot sweep (which
    // runs first and keeps those classes); this detector owns classes
    // with ≥ 1 evolving non-name slot, including pure evolutions whose
    // name never changes.
    let any_evolving = [&vis, &qual, &generics, &params, &ret, &where_]
        .iter()
        .any(|c| matches!(c, SlotClass::Evolving { .. }));
    if !any_evolving {
        return None;
    }
    let (vis_m, vis_p) = vis.plain(Emit::Wild)?;
    let (qual_m, qual_p) = qual.plain(Emit::Wild)?;
    let (gen_m, gen_p) = generics.plain(Emit::Wild)?;
    let (params_m, params_p) = params.params();
    let (ret_m, ret_p) = ret.ret()?;
    let (where_m, where_p) = where_.plain(Emit::Wild)?;
    // `V Q` pairing rule (probed): a lone `V`/`Q` is
    // inference-ambiguous (`{visibility, qualifier_list}` — needs an
    // explicit `# visibility V` to disambiguate), while concrete-vis +
    // `Q` (or `V` + concrete-qual) is rejected outright. Any side
    // wildcarding either slot therefore emits the full `V Q` pair
    // (dropping that side's concrete text — the totality probe stays
    // sound by counting the broader match).
    let (vis_m, qual_m) = vq_pair(vis_m, qual_m);
    let (vis_p, qual_p) = vq_pair(vis_p, qual_p);
    // `-> T W` never parses (the `W` token absorbs into the return
    // slot, producing garbage rewrites — probed): a concrete return
    // forces the same-side `W` out. Minus side keeps `W` and widens
    // the return to omission (`Any` still matches the concrete base,
    // so totality stays sound); plus side keeps the concrete return
    // and omits `where` (omission preserves). Concrete-`where` plus
    // concrete-return (`-> T where ...`) is plain Rust and stays.
    let (ret_m, where_m) = if ret_m.is_some() && matches!(where_m, Emit::Wild) {
        (None, where_m)
    } else {
        (ret_m, where_m)
    };
    let (ret_p, where_p) = if ret_p.is_some() && matches!(where_p, Emit::Wild) {
        (ret_p, Emit::Omit)
    } else {
        (ret_p, where_p)
    };
    let minus_header = build_sig_header(
        &old_name,
        vis_m.text("V").as_deref(),
        qual_m.text("Q").as_deref(),
        gen_m.text("G").as_deref(),
        params_m.as_deref(),
        ret_m.as_deref(),
        where_m.text("W").as_deref(),
    );
    let plus_header = build_sig_header(
        &new_name,
        vis_p.text("V").as_deref(),
        qual_p.text("Q").as_deref(),
        gen_p.text("G").as_deref(),
        params_p.as_deref(),
        ret_p.as_deref(),
        where_p.text("W").as_deref(),
    );
    let rule = unique_rule(used_rules, "sig_evolution");
    // WI-14 slice 3: N=1 singles ride default `one` (exactly-one-set
    // proof via the count probe below) — `# allow=many` would waste 13
    // bytes and lose the precision guard on short headers.
    let single = minuses.len() == 1;
    if all_indented {
        // WI-4a: the engine header filter matches only the named block,
        // so the old single advisory `impl Svc` hunk could no longer
        // apply. Emit one hunk per enclosing `impl` block with its
        // verbatim header line (slot.rs posture); each hunk clears its
        // own engine probe (twin headers veto to ordinary).
        let pre = file_old?;
        let groups = super::block_headers::group_sites_by_impl_block(file_old, changed)?;
        let mut out = String::new();
        for g in &groups {
            if !super::block_headers::probe_block_hunk(
                path,
                pre,
                &g.header_line,
                &minus_header,
                &plus_header,
                g.sites.len(),
            ) {
                return None;
            }
            let header = crate::semantic::item_emission_header(path, &rule, file_old, &g.sites);
            let header = if single {
                header.replace("# allow=many\n", "")
            } else {
                header
            };
            out.push_str(&header);
            out.push_str(&super::block_headers::block_hunk_body(
                &g.header_line,
                &minus_header,
                &plus_header,
            ));
        }
        return Some(out);
    }
    if !crate::totality::template_fn_holds(path, file_old, &minus_header, None, minuses.len()) {
        return None;
    }
    // Item-shape emission (slot.rs posture): fn patterns match whole
    // multi-line items, so the range must cover every changed item —
    // a header-line range could never match under Contained scope.
    let mut hunk = crate::semantic::item_emission_header(path, &rule, file_old, changed);
    if single {
        hunk = hunk.replace("# allow=many\n", "");
    }
    hunk.push_str(&format!(
        "- {minus_header}\n+ {plus_header}\n      ...\n  }}\n"
    ));
    Some(hunk)
}
/// One fn-header slot class.
///
/// Preserved slots are per-site minus == plus: `uniform` carries the
/// shared pretty text when every site agrees (`Some("")` = uniformly
/// absent) and is `None` when the slot is hetero across sites.
/// Evolving slots have one uniform plus text; `minus` is `Some` when
/// the base is uniform (stays concrete for a tighter totality) and
/// `None` when hetero (the minus side wildcards/omits and the plus
/// side sets the slot wholesale — the engine's all-or-nothing slot
/// semantics). An absent side is `""` (addition/removal).
#[derive(Debug)]
pub(crate) enum SlotClass {
    Preserved { uniform: Option<String> },
    Evolving { minus: Option<String>, plus: String },
}

impl SlotClass {
    /// (minus, plus) emission for a plain slot (vis/qual/generics/
    /// where): uniform bases stay concrete (absent → omitted), hetero
    /// bases ride the wildcard on both sides; removal (empty plus) needs
    /// a uniform base, else the class declines (hetero-base removal is
    /// engine-inexpressible — minus-wildcard/plus-omitted preserves).
    pub(crate) fn plain(&self, wildcard: Emit) -> Option<(Emit, Emit)> {
        match self {
            SlotClass::Preserved { uniform } => match uniform {
                Some(t) if t.is_empty() => Some((Emit::Omit, Emit::Omit)),
                Some(t) => Some((Emit::Concrete(t.clone()), Emit::Concrete(t.clone()))),
                None => Some((wildcard.clone(), wildcard.clone())),
            },
            SlotClass::Evolving { minus, plus } => {
                if plus.is_empty() && minus.is_none() {
                    return None;
                }
                let m = match minus {
                    Some(t) if t.is_empty() => Emit::Omit,
                    Some(t) => Emit::Concrete(t.clone()),
                    None => wildcard.clone(),
                };
                let p = if plus.is_empty() {
                    Emit::Omit
                } else {
                    Emit::Concrete(plus.clone())
                };
                Some((m, p))
            }
        }
    }

    /// (minus, plus) emission for params: uniform bases stay concrete
    /// (possibly the empty `()`), hetero bases ride `(...)` on both
    /// sides; the plus side is concrete always (a plus `...` after a
    /// concrete minus is a second spelling of deletion — never emit
    /// it).
    pub(crate) fn params(&self) -> (Option<String>, Option<String>) {
        match self {
            SlotClass::Preserved { uniform } => match uniform {
                Some(t) => (Some(t.clone()), Some(t.clone())),
                None => (None, None),
            },
            SlotClass::Evolving { minus, plus } => (minus.clone(), Some(plus.clone())),
        }
    }

    /// (minus, plus) emission for the return slot: omission is
    /// `RetPat::Any` (matches any return, preserves it), so hetero
    /// preserved bases ride omission on both sides and hetero evolving
    /// bases omit on the minus side; removal (empty plus) needs a
    /// uniform present base (minus-concrete/plus-omitted deletes).
    pub(crate) fn ret(&self) -> Option<(Option<String>, Option<String>)> {
        match self {
            SlotClass::Preserved { uniform } => match uniform {
                Some(t) if t.is_empty() => Some((None, None)),
                Some(t) => Some((Some(t.clone()), Some(t.clone()))),
                None => Some((None, None)),
            },
            SlotClass::Evolving { minus, plus } => {
                if plus.is_empty() && minus.is_none() {
                    return None;
                }
                let m = match minus {
                    Some(t) if t.is_empty() => None,
                    Some(t) => Some(t.clone()),
                    None => None,
                };
                let p = if plus.is_empty() {
                    None
                } else {
                    Some(plus.clone())
                };
                Some((m, p))
            }
        }
    }
}

/// One side of one plain slot in a header under construction.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Emit {
    Omit,
    Concrete(String),
    Wild,
}

impl Emit {
    pub(crate) fn text(&self, wildcard: &str) -> Option<String> {
        match self {
            Emit::Omit => None,
            Emit::Concrete(t) => Some(t.clone()),
            Emit::Wild => Some(wildcard.to_string()),
        }
    }
}

/// WI-14 slice 3 helper: wildcard the non-empty preserved long slots
/// of a single-site evolution (WI-19b posture for types: concrete
/// pretty-printing would re-render `&str`/`Option<String>` and fail the
/// D13 byte-compare). Empty slots stay concrete (`()`/omission — the
/// diet fix for short headers). Evolving slots pass through untouched.
pub(crate) fn wildcard_preserved_nonempty_1(
    generics: SlotClass,
    params: SlotClass,
    where_: SlotClass,
) -> (SlotClass, SlotClass, SlotClass) {
    fn wild(c: SlotClass) -> SlotClass {
        match c {
            SlotClass::Preserved { uniform } => match uniform {
                Some(t) if t.is_empty() => SlotClass::Preserved { uniform: Some(t) },
                Some(_) => SlotClass::Preserved { uniform: None },
                None => SlotClass::Preserved { uniform: None },
            },
            e => e,
        }
    }
    (wild(generics), wild(params), wild(where_))
}

/// One side's vis/qual pair under the `V Q` pairing rule: any side
/// wildcarding either slot emits the full pair (a lone `V`/`Q` is
/// inference-ambiguous and concrete+wildcard mixes are rejected —
/// probed). Concrete texts on a paired side are dropped; the totality
/// probe counts the broader match, so soundness holds.
pub(crate) fn vq_pair(vis: Emit, qual: Emit) -> (Emit, Emit) {
    if vis == Emit::Wild || qual == Emit::Wild {
        (Emit::Wild, Emit::Wild)
    } else {
        (vis, qual)
    }
}

/// Pretty (single-space) slot texts of one parsed header.
#[derive(Debug)]
pub(crate) struct SigSlots {
    pub(crate) name: String,
    pub(crate) vis: String,
    pub(crate) qual: String,
    pub(crate) generics: String,
    pub(crate) params: String,
    pub(crate) ret: String,
    pub(crate) where_: String,
}

pub(crate) fn slot_col(slots: &[SigSlots], f: impl Fn(&SigSlots) -> &String) -> Vec<String> {
    slots.iter().map(|s| f(s).clone()).collect()
}

/// WI-2 slice 3: formatting-stability gate for an N=1 evolving
/// return slot. Both sides must be absent or single-token. Slot texts
/// are already pretty-printed (`parse_sig_slots` renders via `pretty`,
/// which single-space-joins tokens), so any multi-token type would
/// emit spaced text the native tree never contained (`Option<String>`
/// pretty-prints as `Option < String >`) and break the S′→N
/// byte-identity leg the cycle harness enforces. Unstable shapes
/// decline honestly to ordinary instead of emitting a lossy rewrite.
/// `Preserved` slots never re-render (wildcarded or concrete-preserved)
/// and always pass.
fn ret_slot_stable(ret: &SlotClass) -> bool {
    match ret {
        SlotClass::Preserved { .. } => true,
        SlotClass::Evolving { minus, plus } => {
            ret_text_stable(minus.as_deref().unwrap_or("")) && ret_text_stable(plus)
        }
    }
}

/// One return-slot text is formatting-stable: absent (`""`, addition /
/// removal of a bare return) or a `-> Ty` whose type is one token (no
/// whitespace in the pretty text ⟺ single token ⟺ re-renders
/// byte-identically; a lone ident like `i32`/`String`/`Self`
/// pretty-prints to itself). Anything wider declines so the emission
/// never writes spaced text the native post tree never contained.
fn ret_text_stable(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let ty = match text.strip_prefix("->") {
        Some(t) => t.trim(),
        None => return false,
    };
    if ty.is_empty() || ty.contains(char::is_whitespace) {
        return false;
    }
    syn::parse_str::<syn::Type>(ty).is_ok()
}

/// Whitespace-insensitive slot comparison (the engine compares slots
/// whitespace-insensitively against sliced target text).
fn flat_norm(s: &str) -> String {
    super::flat_ws(s)
}

/// List-slot comparison: `flat_norm` plus trailing-comma tolerance,
/// mirroring the engine's `norm_list_slot` (params, `where`
/// predicates — a rustfmt trailing comma is never an evolution).
fn flat_list(s: &str) -> String {
    flat_norm(s).trim_end_matches(',').to_string()
}

pub(crate) fn classify_slot(
    minus_pretty: Vec<String>,
    plus_pretty: Vec<String>,
    is_list: bool,
) -> Option<SlotClass> {
    let norm = if is_list { flat_list } else { flat_norm };
    let minus_flat: Vec<String> = minus_pretty.iter().map(|s| norm(s)).collect();
    let plus_flat: Vec<String> = plus_pretty.iter().map(|s| norm(s)).collect();
    if minus_flat.iter().zip(plus_flat.iter()).all(|(m, p)| m == p) {
        // Preserved: record the shared text when every site agrees
        // (tighter totality), else hetero (rides the wildcard).
        let uniform = minus_flat
            .iter()
            .all(|m| m == &minus_flat[0])
            .then(|| minus_pretty[0].clone());
        return Some(SlotClass::Preserved { uniform });
    }
    // Some site evolves: the plus side must be one uniform concrete
    // text (the engine sets the slot wholesale — no holes). The minus
    // base may be hetero (emitted as omission/`(...)`, matching any);
    // a uniform base stays concrete for a tighter totality.
    // Divergent plus sides stay ordinary.
    let p0 = &plus_flat[0];
    if !plus_flat.iter().all(|p| p == p0) {
        return None;
    }
    let minus_uniform = minus_flat.iter().all(|m| m == &minus_flat[0]);
    Some(SlotClass::Evolving {
        minus: minus_uniform.then(|| minus_pretty[0].clone()),
        plus: plus_pretty[0].clone(),
    })
}

/// Collapse a token stream to single-space pretty text for emission
/// (valid Rust: `async fn`, `pub ( crate )`, `x : i32 , y : u32` —
/// the engine compacts spacing at compare time).
fn pretty<T: ToTokens>(node: &T) -> String {
    node.to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse one single-line `{`-terminated fn header into signature
/// slots. `;`-terminated trait decls never reach here (the `{` gate
/// excludes them, same as the slot sweep). WI-14 slice 3: single-line
/// `}`-terminated items (`fn foo() {}`, `fn foo() { 1 }`) parse via
/// their header prefix (text up to the first `{`); the body after the
/// header never enters the slots and the scaffold emission rewrites
/// the header only (engine preserves the body verbatim, probed).
pub(crate) fn parse_sig_slots(line: &str) -> Option<SigSlots> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    // Multi-line header form: trailing `{` (unchanged fast path).
    if t.ends_with('{') {
        let probe = format!("{t} }}");
        return parse_sig_probe(&probe);
    }
    // Single-line item form: header prefix up to the first `{` plus a
    // synthetic close. No `{` at all (or a `;` decl) still declines.
    if t.ends_with('}') {
        let brace = t.find('{')?;
        let prefix = t[..=brace].trim_end();
        if !prefix.ends_with('{') {
            return None;
        }
        let probe = format!("{prefix} }}");
        return parse_sig_probe(&probe);
    }
    None
}

/// Shared `syn` header parse for one `{`-terminated header probe.
fn parse_sig_probe(probe: &str) -> Option<SigSlots> {
    let (sig, vis) = if let Ok(f) = syn::parse_str::<syn::ItemFn>(probe) {
        (f.sig, f.vis)
    } else if let Ok(m) = syn::parse_str::<syn::ImplItemFn>(probe) {
        (m.sig, m.vis)
    } else {
        return None;
    };
    let vis_text = match vis {
        syn::Visibility::Inherited => String::new(),
        _ => pretty(&vis),
    };
    let mut qual_parts = Vec::new();
    if sig.constness.is_some() {
        qual_parts.push("const".to_string());
    }
    if sig.asyncness.is_some() {
        qual_parts.push("async".to_string());
    }
    if sig.unsafety.is_some() {
        qual_parts.push("unsafe".to_string());
    }
    if let Some(abi) = &sig.abi {
        qual_parts.push(pretty(abi));
    }
    let generics_text = if sig.generics.params.is_empty() {
        String::new()
    } else {
        let params = sig
            .generics
            .params
            .iter()
            .map(pretty)
            .collect::<Vec<_>>()
            .join(", ");
        format!("<{params}>")
    };
    let params_text = sig.inputs.iter().map(pretty).collect::<Vec<_>>().join(", ");
    let ret_text = match &sig.output {
        syn::ReturnType::Default => String::new(),
        syn::ReturnType::Type(_, ty) => format!("-> {}", pretty(ty)),
    };
    let where_text = sig
        .generics
        .where_clause
        .as_ref()
        .map(pretty)
        .unwrap_or_default();
    Some(SigSlots {
        name: sig.ident.to_string(),
        vis: vis_text,
        qual: qual_parts.join(" "),
        generics: generics_text,
        params: params_text,
        ret: ret_text,
        where_: where_text,
    })
}

/// Build one `fn` header line (trailing ` {` included): evolving slots
/// concrete, preserved slots omitted — params `(...)`. `generics`
/// abuts the name (`fn f<T>`); every other present slot is
/// single-space separated (plain Rust spacing, as probed).
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_sig_header(
    name: &str,
    vis: Option<&str>,
    qual: Option<&str>,
    generics: Option<&str>,
    params: Option<&str>,
    ret: Option<&str>,
    where_: Option<&str>,
) -> String {
    let mut h = String::new();
    for s in [vis, qual].into_iter().flatten() {
        h.push_str(s);
        h.push(' ');
    }
    h.push_str("fn ");
    h.push_str(name);
    if let Some(g) = generics {
        // Concrete `<...>` abuts the name (`fn f<T>`); the `G`
        // wildcard is a separate token (`fn f G`). A concrete generic
        // list abuts the params (`<T>(...)`); the wildcard keeps the
        // slot-sweep spacing (`G (...)`).
        if !g.starts_with('<') {
            h.push(' ');
        }
        h.push_str(g);
        if !g.ends_with('>') {
            h.push(' ');
        }
    }
    h.push('(');
    h.push_str(params.unwrap_or("..."));
    h.push(')');
    for s in [ret, where_].into_iter().flatten() {
        h.push(' ');
        h.push_str(s);
    }
    h.push_str(" {");
    h
}
