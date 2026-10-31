//! Generalized anti-unification sweeps (WI-4a first slice, WI-4b type slice).
//!
//! Thin detectors over [`super::antiunify`]: `try_let_sweep` for let
//! type-addition (`let PAT = INIT;` → `let PAT: T = INIT;` with
//! per-site preserved `PAT`/`INIT` bound as inferred `P0`/`E0` and a
//! uniform concrete `T`), plus `try_type_sweep` for type rewrites
//! (`Vec<T>` → `Box<[T]>` with hetero `T` bound as inferred `T0…`).
//! Signature evolution (WI-4c, `try_sig_evolution_sweep` below) is the
//! third slice: it needs no anti-unification core (fn slots are
//! all-or-nothing in the engine — `...`/concrete/omitted — so every
//! evolving slot is uniform-concrete by construction) and lives here
//! per the IMPROVE.md WI-4 detector layout.

use super::antiunify::{
    antiunify_let_template, antiunify_let_type_add, antiunify_pat_swap, antiunify_sequence,
    antiunify_types, antiunify_untyped_let, TypedSite,
};
use super::unique_rule;
use quote::ToTokens;
use syn::spanned::Spanned;

/// N-site (N ≥ 2) let type-addition sweep: every pair adds the same
/// concrete type ascription to a preserved pattern and initializer,
/// both varying across sites (hence the `P0`/`E0` holes). Emits one
/// `# allow=many` hunk (tight original range, SPEC D.5) with diet
/// inferred holes — no `# pat`/`# expr` declarations (SPEC 15.8).
pub(crate) fn try_let_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let ty = antiunify_let_type_add(minuses, pluses)?;
    if !crate::totality::let_untyped_holds(file_old, minuses, changed) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_type_add");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- let P0 = E0;\n+ let P0: {ty} = E0;\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) type rewrite sweep: every pair carries the same type
/// delta with per-site varying subtrees bound as inferred `T0…` holes
/// (e.g. `Vec<T>` → `Box<[T]>`). Line shapes: bare `syn::Type` lines
/// (one `;`/`,` terminator tolerated), or `Stmt::Local` pairs with a
/// type ascription on both sides and per-site token-equal patterns and
/// initializers (a per-site init/pattern delta is a let-statement
/// template, not a type template — that is WI-4's `try_let_sweep`
/// generalization territory, not this detector). All-identical line
/// classes decline (verbatim stays the tighter winner, T422).
/// Totality is the generic engine-count check
/// (`template_type_holds`): the emitted minus template must match
/// exactly N type subterms in the pre-image — macro/string twins never
/// count (the engine never enters them), nested matches count
/// outer-only (contained skips), exactly like the real sweep rewrites.
pub(crate) fn try_type_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    let mut minus_sites = Vec::with_capacity(minuses.len());
    let mut plus_sites = Vec::with_capacity(pluses.len());
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let (mt, pt, m_text, p_text) = extract_type_pair(m, p)?;
        minus_sites.push(TypedSite {
            ty: mt,
            text: m_text,
        });
        plus_sites.push(TypedSite {
            ty: pt,
            text: p_text,
        });
    }
    let (minus_tpl, plus_tpl) = antiunify_types(&minus_sites, &plus_sites)?;
    if !crate::totality::template_type_holds(path, file_old, &minus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "type_sweep");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) joint let-template sweep (WI-5 slice 1): typed→typed
/// `let` pairs with per-site preserved pattern and type plus one
/// uniform initializer delta (e.g. `let P0: T0 = E0;` →
/// `let P0: T0 = E0.wrapping_add(1);` with hetero `P0`/`T0`/`E0`
/// across sites). Strictly generalizes the type-add-only slice (which
/// owns minus-untyped classes) and the type slice (which owns
/// per-site-preserved-init classes) — disjoint from both by
/// construction: this needs ascriptions on both sides plus a per-site
/// init delta. Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted minus must match exactly N
/// typed lets in the pre-image — an untouched twin vetoes, exactly
/// like the real sweep rewrites. Detector name `let_template`.
pub(crate) fn try_let_template_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    let (minus_tpl, plus_tpl) = antiunify_let_template(minuses, pluses)?;
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) untyped joint let-template sweep (WI-8 slice 1):
/// untyped→untyped `let` pairs with per-site preserved patterns plus
/// one uniform initializer delta (`let Some(P0) = E0;` →
/// `let Some(P0) = E0.unwrap_or(0);`, hetero `P0`/`E0` across
/// sites). The typed core's sibling without ascriptions (disjoint by
/// construction: this declines any ascribed side; the typed detector
/// above owns those classes). Uniform concrete deltas
/// (`let v = const { 1 + 2 };` → `… { 1 + 3 };`,
/// `let v = _;` → `let v = 0;`) ride the same core — WI-8's
/// const/infer init support is the `shape.rs` parity gate, not a
/// second template. Totality is the real-pair engine-count check
/// (`template_pair_holds`): the emitted minus must match exactly N
/// untyped lets in the pre-image — an untouched twin vetoes.
/// Detector name `let_template` (same family as the typed core).
pub(crate) fn try_untyped_let_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    let (minus_tpl, plus_tpl) = antiunify_untyped_let(minuses, pluses)?;
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-site (N ≥ 2) tuple-position swap sweep (WI-8 slice 1):
/// `let (P0, P1) = E0;` → `let (P1, P0) = E0;` with per-site
/// preserved inits (a per-site init delta belongs to the untyped
/// core above, never guessed here). Totality is the same real-pair
/// engine-count check. Detector name `let_template` (same family).
pub(crate) fn try_pat_swap_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    let (minus_tpl, plus_tpl) = antiunify_pat_swap(minuses, pluses)?;
    if !crate::totality::template_pair_holds(path, file_old, &minus_tpl, &plus_tpl, minuses.len()) {
        return None;
    }
    let rule = unique_rule(used_rules, "let_template");
    let mut hunk = crate::emission_header(path, &rule, changed);
    hunk.push_str(&format!("- {minus_tpl}\n+ {plus_tpl}\n"));
    Some(hunk)
}

/// N-pair (N ≥ 4) repeated-block sequence sweep (WI-5c): N flat pairs
/// split block-major into ≥ 2 blocks × K = 2–4 `let` statements, each
/// position anti-unified across blocks (hetero positions ride holes,
/// identical ones stay concrete anchors). Emits one `# allow=many`
/// hunk (tight original range, SPEC D.5) with K minus/plus lines.
/// Contiguity gate: each block's old lines must be consecutive (block
/// bodies are consecutive statements — the detector only fuses runs the
/// engine matches as windows). Totality is the real-pair engine-count
/// check (`template_sequence_holds`): the emitted minus must match
/// exactly N blocks in the pre-image — an untouched twin window vetoes,
/// exactly like the real sweep rewrites. Detector name `sequence`.
pub(crate) fn try_sequence_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    let n = minuses.len();
    if n != pluses.len() || n < 4 || changed.len() != n {
        return None;
    }
    // All-identical whole classes stay for the tighter verbatim sweep
    // (T422 precedent).
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
        return None;
    }
    // Largest K first: the most specific template wins the byte guard
    // naturally.
    for k in [4, 3, 2] {
        if !n.is_multiple_of(k) || n / k < 2 {
            continue;
        }
        let mut contiguous = true;
        for b in 0..n / k {
            let run = &changed[b * k..(b + 1) * k];
            for w in run.windows(2) {
                if w[1] != w[0] + 1 {
                    contiguous = false;
                    break;
                }
            }
            if !contiguous {
                break;
            }
        }
        if !contiguous {
            continue;
        }
        let (minus_tpls, plus_tpls) = antiunify_sequence(minuses, pluses, k)?;
        if !crate::totality::template_sequence_holds(path, file_old, &minus_tpls, &plus_tpls, n / k)
        {
            continue;
        }
        let rule = unique_rule(used_rules, "sequence");
        let mut hunk = crate::emission_header(path, &rule, changed);
        for l in &minus_tpls {
            hunk.push_str(&format!("- {l}\n"));
        }
        for l in &plus_tpls {
            hunk.push_str(&format!("+ {l}\n"));
        }
        return Some(hunk);
    }
    None
}

/// One pair's `(minus, plus)` types plus their original spellings: the
/// ascribed types of a `let` pair whose pattern and initializer are
/// token-equal (plus `let-else` diverge agreement — the engine never
/// moves into diverge targets). Bare-type full lines never occur in
/// `-U0` diffs of valid Rust (every type sits inside a larger line),
/// so there is deliberately no bare-`syn::Type` path: every pair site
/// is provably a type position, and the expression-domain hazard class
/// cannot arise. The texts are line-sliced via the types' own spans
/// and re-parsed, so every span inside them slices the text — template
/// rendering inherits the first site's exact spacing.
fn extract_type_pair(m: &str, p: &str) -> Option<(syn::Type, syn::Type, String, String)> {
    let (minus, plus) = match (
        syn::parse_str::<syn::Stmt>(m.trim()),
        syn::parse_str::<syn::Stmt>(p.trim()),
    ) {
        (Ok(syn::Stmt::Local(a)), Ok(syn::Stmt::Local(b))) => (a, b),
        _ => return None,
    };
    let (syn::Pat::Type(minus_pt), syn::Pat::Type(plus_pt)) = (&minus.pat, &plus.pat) else {
        return None;
    };
    if minus_pt.pat.to_token_stream().to_string() != plus_pt.pat.to_token_stream().to_string() {
        return None;
    }
    match (&minus.init, &plus.init) {
        (Some(a), Some(b)) => {
            if a.diverge.is_some() || b.diverge.is_some() {
                return None;
            }
            if a.expr.to_token_stream().to_string() != b.expr.to_token_stream().to_string() {
                return None;
            }
        }
        (None, None) => {}
        _ => return None,
    }
    let m_text = slice_type_text(m.trim(), &minus_pt.ty)?;
    let p_text = slice_type_text(p.trim(), &plus_pt.ty)?;
    let mt = syn::parse_str::<syn::Type>(&m_text).ok()?;
    let pt = syn::parse_str::<syn::Type>(&p_text).ok()?;
    Some((mt, pt, m_text, p_text))
}

/// Slice a type's original spelling from its single-line parent snippet
/// via the type's own span. Multi-line or out-of-bounds spans veto.
fn slice_type_text(line: &str, ty: &syn::Type) -> Option<String> {
    let span = ty.span();
    if span.start().line != 1 || span.end().line != 1 {
        return None;
    }
    // Span columns are Unicode scalars (SPEC 31.3), not bytes.
    let (s, e) = (
        rsmpl_pattern::char_col_to_byte(line, span.start().column),
        rsmpl_pattern::char_col_to_byte(line, span.end().column),
    );
    if s >= e || e > line.len() || !line.is_char_boundary(s) || !line.is_char_boundary(e) {
        return None;
    }
    Some(line[s..e].to_string())
}

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
    if minuses.len() != pluses.len() || minuses.len() < 2 {
        return None;
    }
    if minuses.iter().all(|m| *m == minuses[0]) && pluses.iter().all(|p| *p == pluses[0]) {
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
            out.push_str(&crate::semantic::item_emission_header(
                path, &rule, file_old, &g.sites,
            ));
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
enum SlotClass {
    Preserved { uniform: Option<String> },
    Evolving { minus: Option<String>, plus: String },
}

impl SlotClass {
    /// (minus, plus) emission for a plain slot (vis/qual/generics/
    /// where): uniform bases stay concrete (absent → omitted), hetero
    /// bases ride the wildcard on both sides; removal (empty plus) needs
    /// a uniform base, else the class declines (hetero-base removal is
    /// engine-inexpressible — minus-wildcard/plus-omitted preserves).
    fn plain(&self, wildcard: Emit) -> Option<(Emit, Emit)> {
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
    fn params(&self) -> (Option<String>, Option<String>) {
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
    fn ret(&self) -> Option<(Option<String>, Option<String>)> {
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
enum Emit {
    Omit,
    Concrete(String),
    Wild,
}

impl Emit {
    fn text(&self, wildcard: &str) -> Option<String> {
        match self {
            Emit::Omit => None,
            Emit::Concrete(t) => Some(t.clone()),
            Emit::Wild => Some(wildcard.to_string()),
        }
    }
}

/// One side's vis/qual pair under the `V Q` pairing rule: any side
/// wildcarding either slot emits the full pair (a lone `V`/`Q` is
/// inference-ambiguous and concrete+wildcard mixes are rejected —
/// probed). Concrete texts on a paired side are dropped; the totality
/// probe counts the broader match, so soundness holds.
fn vq_pair(vis: Emit, qual: Emit) -> (Emit, Emit) {
    if vis == Emit::Wild || qual == Emit::Wild {
        (Emit::Wild, Emit::Wild)
    } else {
        (vis, qual)
    }
}

/// Pretty (single-space) slot texts of one parsed header.
#[derive(Debug)]
struct SigSlots {
    name: String,
    vis: String,
    qual: String,
    generics: String,
    params: String,
    ret: String,
    where_: String,
}

fn slot_col(slots: &[SigSlots], f: impl Fn(&SigSlots) -> &String) -> Vec<String> {
    slots.iter().map(|s| f(s).clone()).collect()
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

fn classify_slot(
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
/// excludes them, same as the slot sweep).
fn parse_sig_slots(line: &str) -> Option<SigSlots> {
    let t = line.trim();
    if t.is_empty() || !t.ends_with('{') {
        return None;
    }
    let probe = format!("{t} }}");
    let (sig, vis) = if let Ok(f) = syn::parse_str::<syn::ItemFn>(&probe) {
        (f.sig, f.vis)
    } else if let Ok(m) = syn::parse_str::<syn::ImplItemFn>(&probe) {
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
fn build_sig_header(
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

#[cfg(test)]
mod tests {
    use super::super::{infer_semantic, infer_semantic_with_pre_images};
    use std::collections::HashMap;

    fn section(path: &str, body: &str) -> String {
        format!("diff --git a/{path} b/{path}\n{body}")
    }

    fn pre_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn validated_live(patch: &str, pre: &HashMap<String, String>) {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
    }

    const PRE_THREE: &str = "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n}\n";

    fn three_pair_body() -> String {
        "@@ -1,5 +1,5 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n-    let gamma_value = compute_third(extra);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u32 = compute_second(other);\n+    let gamma_value: u32 = compute_third(extra);\n }\n"
            .to_string()
    }

    #[test]
    fn let_type_add_fires() {
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[("f.rs", PRE_THREE)]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        // Consumed: no native let lines survive.
        assert!(!out.contains("compute_first(input);"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        assert!(!out.contains("# pat"), "got: {out}");
    }

    #[test]
    fn let_type_add_extra_untyped_let_vetoes() {
        // Totality: an untouched fourth untyped `let` in the pre-image
        // would be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n    let untouched_guard = compute_fourth(more);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn let_type_add_typed_twin_does_not_veto() {
        // An already-typed `let` never matches `let P0 = E0;` (ascription
        // agreement is strict), so it is not counted and never vetoes.
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value = compute_first(input);\n    let beta_value = compute_second(other);\n    let gamma_value = compute_third(extra);\n    let already: u32 = 1;\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn let_type_add_varying_type_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u64 = compute_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_init_delta_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n-    let beta_value = compute_second(other);\n+    let alpha_value: u32 = compute_first(input);\n+    let beta_value: u32 = changed_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_single_vetoes() {
        // Singles never reach the pair chain (whole-section singles try
        // item rename only; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let alpha_value = compute_first(input);\n+    let alpha_value: u32 = compute_first(input);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn let_type_add_unparseable_pre_vetoes() {
        let raw = section("f.rs", &three_pair_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    const PRE_TYPE: &str = "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_count: u32 = compute_third(extra);\n}\n";

    fn three_type_body() -> String {
        "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n-    let beta_value: Vec<String> = compute_second(other);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n+    let beta_value: Box<[String]> = compute_second(other);\n }\n"
            .to_string()
    }

    #[test]
    fn type_sweep_fires() {
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[("f.rs", PRE_TYPE)]);
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(!out.contains("# scope=file"), "got: {out}");
        // Templates inherit the sites' exact spacing (span-sliced).
        assert!(out.contains("- Vec<T0>\n"), "got: {out}");
        assert!(out.contains("+ Box<[T0]>\n"), "got: {out}");
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# type"), "got: {out}");
        // Consumed: no native type lines survive.
        assert!(!out.contains("Vec<u32>"), "got: {out}");
        assert!(
            out.len() < three_type_body().len() + 100,
            "type sweep must beat native bytes, got: {out}"
        );
    }

    #[test]
    fn type_sweep_fn_bound_fires() {
        // WI-6d: parenthesized `Fn` bound delta at depth 3 (Box angle →
        // trait bound → path args) raises through the full pipeline:
        // hetero inputs hole as `T0`, the uniform `u8` → `String`
        // output delta stays concrete, pre-image engine-count
        // totality holds (exactly 4 subterms). Same shape as
        // applyrs bench 27, which proves the authored direction.
        let pre = "fn a() {\n    let f: Box<dyn Fn(u8) -> u8> = build();\n    f\n}\nfn b() {\n    let f: Box<dyn Fn(u16) -> u8> = build();\n    f\n}\nfn c() {\n    let f: Box<dyn Fn(u32) -> u8> = build();\n    f\n}\nfn d() {\n    let f: Box<dyn Fn(bool) -> u8> = build();\n    f\n}\n";
        let body = "@@ -2 +2 @@\n-    let f: Box<dyn Fn(u8) -> u8> = build();\n+    let f: Box<dyn Fn(u8) -> String> = build();\n@@ -6 +6 @@\n-    let f: Box<dyn Fn(u16) -> u8> = build();\n+    let f: Box<dyn Fn(u16) -> String> = build();\n@@ -10 +10 @@\n-    let f: Box<dyn Fn(u32) -> u8> = build();\n+    let f: Box<dyn Fn(u32) -> String> = build();\n@@ -14 +14 @@\n-    let f: Box<dyn Fn(bool) -> u8> = build();\n+    let f: Box<dyn Fn(bool) -> String> = build();\n".to_string();
        let raw = section("f.rs", &body);
        let pres = pre_map(&[("f.rs", pre)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pres).unwrap();
        validated_live(&out, &pres);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- Box<dyn Fn(T0) -> u8>\n"), "got: {out}");
        assert!(out.contains("+ Box<dyn Fn(T0) -> String>\n"), "got: {out}");
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# type"), "got: {out}");
        // Consumed: no native type lines survive.
        assert!(!out.contains("Box<dyn Fn(u8) -> u8>"), "got: {out}");
        assert!(out.len() < body.len(), "type sweep must beat native bytes");
    }

    #[test]
    fn type_sweep_extra_type_site_vetoes() {
        // Totality: a third `Vec<..>` ascription in the pre-image would
        // be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let gamma_extra: Vec<u8> = compute_third(extra);\n    let gamma_count: u32 = done();\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn type_sweep_untyped_let_does_not_veto() {
        // An untyped `let` holds no type subterm, so the template never
        // matches it — unlike `let P0 = E0;`, it is invisible here.
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn m() {\n    let alpha_value: Vec<u32> = compute_first(input);\n    let beta_value: Vec<String> = compute_second(other);\n    let plain = compute_third(extra);\n    let gamma_count: u32 = done();\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1"), "got: {out}");
    }

    #[test]
    fn type_sweep_init_delta_vetoes() {
        // A per-site initializer delta is a let-statement template, not
        // a type template.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n-    let beta_value: Vec<String> = compute_second(other);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n+    let beta_value: Box<[String]> = changed_second(other);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn type_sweep_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn m() {\n-    let alpha_value: Vec<u32> = compute_first(input);\n+    let alpha_value: Box<[u32]> = compute_first(input);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn type_sweep_unparseable_pre_vetoes() {
        let raw = section("f.rs", &three_type_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    const PRE_EVO: &str =
        "fn alpha(x: i32) -> i32 {\n    x + 1\n}\nfn alpha(y: String) -> String {\n    y\n}\n";

    // Realistic `-U0` shape: each header-only change is its own hunk
    // (headers at old lines 1 and 4); whole-section pairing joins them.
    fn evo_return_body() -> String {
        "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32) -> u32 {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+fn beta(y: String) -> u32 {\n"
            .to_string()
    }

    #[test]
    fn sig_evo_rename_return_fires() {
        // Rename plus a uniform return-type addition; params hetero
        // across sites ride `(...)`, return rides omission→concrete.
        // The covering item range spans both items.
        let raw = section("f.rs", &evo_return_body());
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("@@ -1,6 @@ rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+ fn beta(...) -> u32 {"), "got: {out}");
        assert!(!out.contains("-fn alpha(x: i32)"), "got: {out}");
    }

    #[test]
    fn sig_evo_vis_params_mixed_fire() {
        // Hetero preserved vis rides the `V Q` pair (lone `V` is
        // inference-ambiguous — probed) while uniform params evolve
        // concrete; the uniform return stays concrete.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-pub fn alpha(x: i32) -> i32 {\n+pub fn beta(x: i32, y: u32) -> i32 {\n@@ -4,1 +4,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32, y: u32) -> i32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "pub fn alpha(x: i32) -> i32 {\n    x\n}\nfn alpha(x: i32) -> i32 {\n    x\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(
            out.contains("- V Q fn alpha(x : i32) -> i32 {"),
            "got: {out}"
        );
        assert!(
            out.contains("+ V Q fn beta(x : i32, y : u32) -> i32 {"),
            "got: {out}"
        );
    }

    #[test]
    fn sig_evo_pure_evolution_no_rename_fires() {
        // Same name, uniform return addition — no rename involved.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn alpha(x: i32) -> u32 {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+fn alpha(y: String) -> u32 {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+ fn alpha(...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_generics_where_fire() {
        // Uniform `<T>` + `where` addition with rename; generics/where
        // hetero-preserved sites would ride omission instead.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta<T>(x: i32) -> i32 where T: Clone {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+fn beta<T>(y: String) -> String where T: Clone {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(
            out.contains("+ fn beta<T>(...) where T : Clone {"),
            "got: {out}"
        );
    }

    #[test]
    fn sig_evo_impl_context_fires() {
        // Uniformly indented methods evolve under the block's verbatim
        // header (WI-4a filter: the header is literally `impl Svc`,
        // so the verbatim emission reuses it).
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    fn alpha(x: i32) -> i32 {\n+    fn beta(x: i32) -> u32 {\n@@ -5,1 +5,1 @@\n-    fn alpha(y: String) -> String {\n+    fn beta(y: String) -> u32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "impl Svc {\n    fn alpha(x: i32) -> i32 {\n        x\n    }\n    fn alpha(y: String) -> String {\n        y\n    }\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("impl Svc"), "got: {out}");
        assert!(out.contains("-     fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+     fn beta(...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_name_only_declines() {
        // Rename-only classes belong to the slot sweep (which runs
        // first) — a direct call proves this detector declines them.
        let minuses = [
            "fn alpha(x: i32) -> i32 {",
            "fn alpha(y: String) -> String {",
        ];
        let pluses = ["fn beta(x: i32) -> i32 {", "fn beta(y: String) -> String {"];
        assert!(super::try_sig_evolution_sweep(
            &minuses,
            &pluses,
            "f.rs",
            None,
            &[1, 2],
            &mut Vec::new()
        )
        .is_none());
    }

    #[test]
    fn sig_evo_divergent_evolution_vetoes() {
        // Different returns per site are not one uniform delta.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn alpha(x: i32) -> i32 {\n-fn alpha(y: String) -> String {\n+fn beta(x: i32) -> u32 {\n+fn beta(y: String) -> u64 {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sig_evo_extra_site_vetoes() {
        // Totality: a third untouched `fn alpha` in the pre-image would
        // be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &evo_return_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 {\n    x + 1\n}\nfn alpha(y: String) -> String {\n    y\n}\nfn alpha(z: u8) -> u8 {\n    z\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn sig_evo_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32) -> u32 {\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sig_evo_unparseable_pre_vetoes() {
        let raw = section("f.rs", &evo_return_body());
        let pre = pre_map(&[("f.rs", "not rust {{{")]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn sig_evo_qual_evolution_fires() {
        // Uniform `async` addition with rename; qual rides concrete
        // while vis stays omitted on both sides (`V Q` pairs only when
        // a wildcard is needed — here neither slot needs one).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+async fn beta(x: i32) -> i32 {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String {\n+async fn beta(y: String) -> String {\n",
        );
        let pre = pre_map(&[("f.rs", PRE_EVO)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(out.contains("+ async fn beta(...) {"), "got: {out}");
    }

    #[test]
    fn sig_evo_where_hetero_return_evolve_fires() {
        // Hetero preserved `where` rides `W` (lone-`W` is legal,
        // probed) while a uniform return is added — the plus side
        // drops `W` (`-> T W` is unparsable).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 where T: Clone {\n+fn beta(x: i32) -> u32 where T: Clone {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> String where U: Debug {\n+fn beta(y: String) -> u32 where U: Debug {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 where T: Clone {\n    x\n}\nfn alpha(y: String) -> String where U: Debug {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) W {"), "got: {out}");
        assert!(out.contains("+ fn beta(...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_generics_hetero_return_evolve_fires() {
        // Hetero preserved generics ride `G` while a uniform return is
        // added — the `G`-without-`W` plus shape with a concrete
        // return.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha<T>(x: T) -> T {\n+fn beta<T>(x: T) -> u32 {\n@@ -4,1 +4,1 @@\n-fn alpha<U>(y: U) -> U {\n+fn beta<U>(y: U) -> u32 {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha<T>(x: T) -> T {\n    x\n}\nfn alpha<U>(y: U) -> U {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha G (...) {"), "got: {out}");
        assert!(out.contains("+ fn beta G (...) -> u32 {"), "got: {out}");
    }

    #[test]
    fn sig_evo_return_removal_fires() {
        // Uniform return removal: minus concrete, plus omitted
        // (= delete, engine semantics); hetero-base removal declines.
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) -> i32 {\n+fn beta(x: i32) {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) -> i32 {\n+fn beta(y: String) {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) -> i32 {\n    x\n}\nfn alpha(y: String) -> i32 {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) -> i32 {"), "got: {out}");
        assert!(out.contains("+ fn beta(...) {"), "got: {out}");
    }

    #[test]
    fn sig_evo_hetero_base_removal_vetoes() {
        // Return removal over a hetero base (one site already
        // return-less) would need minus-`Any`/plus-omitted, which
        // preserves instead of deleting — decline.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n-fn alpha(x: i32) -> i32 {\n-fn alpha(y: String) {\n+fn beta(x: i32) {\n+fn beta(y: String) {\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    const PRE_JOINT: &str =
        "fn f() {\n    let a: i32 = 0;\n    let b: u32 = 1;\n    let c: i64 = 2;\n}\n";

    fn three_joint_body() -> String {
        "@@ -1,4 +1,4 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n-    let c: i64 = 2;\n+    let a: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(1);\n+    let c: i64 = 2.wrapping_add(1);\n }\n"
            .to_string()
    }

    #[test]
    fn joint_let_wrapping_fires() {
        // WI-5 slice 1: hetero pat/ty across sites ride P0/T0, the lit
        // inits ride E0, and the uniform `.wrapping_add(1)` delta is the
        // anchor. Consumed lines must vanish; diet holes need no
        // declarations (SPEC 15.8).
        let raw = section("f.rs", &three_joint_body());
        let pre = pre_map(&[("f.rs", PRE_JOINT)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- let P0: T0 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P0: T0 = E0.wrapping_add(1);\n"),
            "got: {out}"
        );
        assert!(
            !out.contains("-    let a: i32"),
            "changed lines must be consumed, got: {out}"
        );
        assert!(!out.contains("# pat"), "got: {out}");
        assert!(!out.contains("# type"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
    }

    #[test]
    fn joint_let_extra_twin_vetoes() {
        // Totality: an untouched fourth typed `let` in the pre-image
        // would be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &three_joint_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let a: i32 = 0;\n    let b: u32 = 1;\n    let c: i64 = 2;\n    let guard: u8 = 9;\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn joint_let_untyped_minus_vetoes() {
        // Minus-untyped classes belong to the type-add slice (which
        // runs first and keeps them); a direct call proves this
        // detector declines them.
        let minuses = ["let a = f(x);", "let b = f(y);"];
        let pluses = ["let a: u32 = f(x);", "let b: u32 = f(y);"];
        assert!(super::try_let_template_sweep(
            &minuses,
            &pluses,
            "f.rs",
            None,
            &[1, 2],
            &mut Vec::new()
        )
        .is_none());
    }

    #[test]
    fn joint_let_per_site_pat_delta_vetoes() {
        // A per-site pattern rename is not a joint template (rename
        // detectors own it) — never guessed here.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n+    let ax: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(1);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn joint_let_divergent_plus_vetoes() {
        // Different per-site deltas are not one uniform delta.
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n+    let a: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(2);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn joint_let_single_vetoes() {
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn f() {\n-    let a: i32 = 0;\n+    let a: i32 = 0.wrapping_add(1);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    // WI-5c detector proving shape: 2 blocks × 2 hetero positions no
    // single-class detector covers — position 0 is a type-addition,
    // position 1 a joint init-delta template (hetero pat/ty ride P1/T1).
    const PRE_SEQ: &str = "fn first() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\n\nfn second() {\n    let delta = compute_second(other);\n    let epsilon: u64 = wrap(delta);\n}\n";

    fn seq_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta: u32 = wrap(alpha);\n+    let alpha: u32 = compute_first(input);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n@@ -7,2 +7,2 @@\n-    let delta = compute_second(other);\n-    let epsilon: u64 = wrap(delta);\n+    let delta: u32 = compute_second(other);\n+    let epsilon: u64 = wrap(delta).wrapping_add(1);\n"
            .to_string()
    }

    #[test]
    fn sequence_two_blocks_fire() {
        // WI-5c canonical shape: 2 blocks × 2 hetero positions. One
        // shared header amortized over four lines.
        let raw = section("f.rs", &seq_body());
        let pre = pre_map(&[("f.rs", PRE_SEQ)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("rule=r1\n"), "got: {out}");
        assert!(out.contains("# allow=many\n"), "got: {out}");
        assert!(out.contains("- let P0 = E0;\n"), "got: {out}");
        assert!(out.contains("- let P1: T1 = E1;\n"), "got: {out}");
        assert!(out.contains("+ let P0: u32 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P1: T1 = E1.wrapping_add(1);\n"),
            "got: {out}"
        );
        // Inferred holes need no declarations (SPEC 15.8).
        assert!(!out.contains("# pat"), "got: {out}");
        assert!(!out.contains("# type"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        // Consumed: no native changed lines survive.
        assert!(!out.contains("compute_first(input);"), "got: {out}");
    }

    #[test]
    fn sequence_extra_twin_vetoes() {
        // Totality: an untouched twin window in the pre-image would be
        // silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &seq_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn first() {\n    let alpha = compute_first(input);\n    let beta: u32 = wrap(alpha);\n}\n\nfn second() {\n    let delta = compute_second(other);\n    let epsilon: u64 = wrap(delta);\n}\n\nfn third() {\n    let guard = compute_third(more);\n    let guard2: u64 = wrap(guard);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn sequence_single_block_vetoes() {
        // One hetero block is not a repeated sequence (WI-5d owns
        // single-block fusion): the type-add position fails on the
        // typed pair, the joint position on the untyped pair — stays
        // ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let alpha = compute_first(input);\n-    let beta: u32 = wrap(alpha);\n+    let alpha: u32 = compute_first(input);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sequence_noncontiguous_vetoes() {
        // Hetero changed lines split across non-consecutive runs are not
        // block bodies: singles fail their class gates and the sequence
        // fails contiguity — stays ordinary.
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    let alpha = compute_first(input);\n+    let alpha: u32 = compute_first(input);\n@@ -4,1 +4,1 @@\n-    let beta: u32 = wrap(alpha);\n+    let beta: u32 = wrap(alpha).wrapping_add(1);\n@@ -7,1 +7,1 @@\n-    let delta = compute_second(other);\n+    let delta: u32 = compute_second(other);\n@@ -9,1 +9,1 @@\n-    let epsilon: u64 = wrap(delta);\n+    let epsilon: u64 = wrap(delta).wrapping_add(1);\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    #[test]
    fn sig_evo_return_and_where_both_evolve_fire() {
        // Concrete return plus concrete `where` on the plus side is
        // plain Rust (`-> T where ...` only bans the `W` wildcard).
        let raw = section(
            "f.rs",
            "@@ -1,1 +1,1 @@\n-fn alpha(x: i32) {\n+fn beta(x: i32) -> u32 where T: Clone {\n@@ -4,1 +4,1 @@\n-fn alpha(y: String) {\n+fn beta(y: String) -> u32 where T: Clone {\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn alpha(x: i32) {\n    x\n}\nfn alpha(y: String) {\n    y\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- fn alpha(...) {"), "got: {out}");
        assert!(
            out.contains("+ fn beta(...) -> u32 where T : Clone {"),
            "got: {out}"
        );
    }

    fn applied_post(patch: &str, pre: &HashMap<String, String>) -> HashMap<String, String> {
        let parsed = rsmpl_patch::parse_patch(patch).expect("emitted patch must parse");
        let mut files = pre.clone();
        for section in &parsed.files {
            files.entry(section.target_path.clone()).or_default();
        }
        rsmpl_engine::apply_patch_to_tree(&mut files, &parsed)
            .expect("emitted patch must apply cleanly");
        files
    }

    // WI-8 slice 1: untyped joint let template (hetero `Some(P0)`
    // patterns, uniform `E0`-hole method-wrap init delta).
    const PRE_SOME: &str = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let _ = (v, w);\n}\n";
    const POST_SOME: &str = "fn a(o: Option<i32>, p: Option<i32>) {\n    let Some(v) = o.unwrap_or(0);\n    let Some(w) = p.unwrap_or(0);\n    let _ = (v, w);\n}\n";

    fn some_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let Some(v) = o;\n-    let Some(w) = p;\n+    let Some(v) = o.unwrap_or(0);\n+    let Some(w) = p.unwrap_or(0);\n"
            .to_string()
    }

    #[test]
    fn untyped_let_some_unwrap_fires_byte_identical() {
        let raw = section("f.rs", &some_body());
        let pre = pre_map(&[("f.rs", PRE_SOME)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        assert!(out.contains("- let Some(P0) = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let Some(P0) = E0.unwrap_or(0);\n"),
            "got: {out}"
        );
        assert!(!out.contains("# pat"), "got: {out}");
        assert!(!out.contains("# expr"), "got: {out}");
        let files = applied_post(&out, &pre);
        assert_eq!(
            files["f.rs"], POST_SOME,
            "real-apply must be byte-identical"
        );
    }

    #[test]
    fn untyped_let_extra_twin_vetoes() {
        // Totality: an untouched third `Some` let in the pre-image
        // would be silently rewritten — the sweep vetoes.
        let raw = section("f.rs", &some_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn a(o: Option<i32>, p: Option<i32>, q: Option<i32>) {\n    let Some(v) = o;\n    let Some(w) = p;\n    let Some(z) = q;\n    let _ = (v, w, z);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }

    #[test]
    fn untyped_let_const_block_fires() {
        let raw = section(
            "f.rs",
            "@@ -2,1 +2,1 @@\n-    let v = const { 1 + 2 };\n+    let v = const { 1 + 3 };\n@@ -5,1 +5,1 @@\n-    let w = const { 1 + 2 };\n+    let w = const { 1 + 3 };\n",
        );
        let pre = pre_map(&[(
            "f.rs",
            "fn f() {\n    let v = const { 1 + 2 };\n}\nfn g() {\n    let w = const { 1 + 2 };\n}\n",
        )]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = const { 1 + 2 };\n"), "got: {out}");
        assert!(out.contains("+ let P0 = const { 1 + 3 };\n"), "got: {out}");
    }

    #[test]
    fn untyped_let_infer_init_fires() {
        let raw = section(
            "f.rs",
            "@@ -2,2 +2,2 @@\n-    let v = _;\n-    let w = _;\n+    let v = 0;\n+    let w = 0;\n",
        );
        let pre = pre_map(&[("f.rs", "fn f() {\n    let v = _;\n    let w = _;\n}\n")]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        validated_live(&out, &pre);
        assert!(out.contains("- let P0 = _;\n"), "got: {out}");
        assert!(out.contains("+ let P0 = 0;\n"), "got: {out}");
    }

    #[test]
    fn untyped_let_macro_init_stays_with_macro_detector() {
        // Nested-macro classes belong to the macro-interior sweep,
        // never to a whole-line let template.
        let raw = section(
            "f.rs",
            "@@ -1,4 +1,4 @@\n fn m() {\n-    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n-    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_gitrs\")));\n+    let first_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n+    let second_out = Command::new(PathBuf::from(env!(\"CARGO_BIN_EXE_applyrs\")));\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(
            !out.contains("let P0"),
            "untyped-let must decline, got: {out}"
        );
        assert!(
            out.contains("- env!(\"CARGO_BIN_EXE_gitrs\")\n"),
            "got: {out}"
        );
    }

    #[test]
    fn untyped_let_typed_class_stays_typed() {
        // Chain precedence: ascribed classes still hit the typed
        // template first (this detector declines any ascribed side).
        let raw = section(
            "f.rs",
            "@@ -1,3 +1,3 @@\n fn f() {\n-    let a: i32 = 0;\n-    let b: u32 = 1;\n+    let a: i32 = 0.wrapping_add(1);\n+    let b: u32 = 1.wrapping_add(1);\n }\n",
        );
        let out = infer_semantic(std::slice::from_ref(&raw)).unwrap();
        assert!(out.contains("- let P0: T0 = E0;\n"), "got: {out}");
        assert!(
            out.contains("+ let P0: T0 = E0.wrapping_add(1);\n"),
            "got: {out}"
        );
    }

    #[test]
    fn untyped_let_single_stays_ordinary() {
        // Singles never reach the pair chain (whole-section singles try
        // item rename only; per-hunk singles likewise).
        let raw = section(
            "f.rs",
            "@@ -1,2 +1,2 @@\n fn f(o: Option<i32>) {\n-    let Some(v) = o;\n+    let Some(v) = o.unwrap_or(0);\n }\n",
        );
        assert!(infer_semantic(std::slice::from_ref(&raw)).is_none());
    }

    // WI-8 slice 1: tuple-position swap (`(P0, P1)` → `(P1, P0)`).
    const PRE_SWAP: &str = "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (x, y) = t;\n    let (m, n) = u;\n    let _ = (x, y, m, n);\n}\n";
    const POST_SWAP: &str = "fn a(t: (i32, i32), u: (i32, i32)) {\n    let (y, x) = t;\n    let (n, m) = u;\n    let _ = (x, y, m, n);\n}\n";

    fn swap_body() -> String {
        "@@ -2,2 +2,2 @@\n-    let (x, y) = t;\n-    let (m, n) = u;\n+    let (y, x) = t;\n+    let (n, m) = u;\n"
            .to_string()
    }

    #[test]
    fn pat_swap_fires_byte_identical() {
        let raw = section("f.rs", &swap_body());
        let pre = pre_map(&[("f.rs", PRE_SWAP)]);
        let out = infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).unwrap();
        assert!(out.contains("- let (P0, P1) = E0;\n"), "got: {out}");
        assert!(out.contains("+ let (P1, P0) = E0;\n"), "got: {out}");
        let files = applied_post(&out, &pre);
        assert_eq!(
            files["f.rs"], POST_SWAP,
            "real-apply must be byte-identical"
        );
    }

    #[test]
    fn pat_swap_extra_twin_vetoes() {
        // An untouched same-shape tuple let would be silently
        // rewritten — the sweep vetoes.
        let raw = section("f.rs", &swap_body());
        let pre = pre_map(&[(
            "f.rs",
            "fn a(t: (i32, i32), u: (i32, i32), v: (i32, i32)) {\n    let (x, y) = t;\n    let (m, n) = u;\n    let (p, q) = v;\n    let _ = (x, y, m, n, p, q);\n}\n",
        )]);
        assert!(infer_semantic_with_pre_images(std::slice::from_ref(&raw), &pre).is_none());
    }
}
