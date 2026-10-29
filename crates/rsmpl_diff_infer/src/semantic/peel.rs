//! T441 slice 1: hetero-arg array-head-peel call-migration sweep.
//! Split from `super` (T441) to hold the 2000-line hygiene gate.

use super::{call_parts, flat_ws, unique_rule};

/// One hetero-arg array-head-peel class (T441 slice 1): every pair
/// parses as `f(D, &[H, M, T…])` → `g(D, M, &[T…])` with shared
/// callees, shared string-literal head `H`, and shared tail length.
/// `D`, `M`, and the tails bind positionally per site (hetero values
/// are the point); `H` replays verbatim.
#[derive(Debug, PartialEq, Eq)]
struct PeelClass {
    old_cal: String,
    new_cal: String,
    head_text: String,
    head_value: String,
    tails: usize,
}

/// N-site (N ≥ 2) hetero-arg array-head-peel call migration (T441):
/// one `# allow=many` + `# scope=file` hunk per class. Whole-section
/// and per-hunk callers pass their full pair lists (exactly one class
/// must hold, mirroring `try_call_sweep`); the cross-hunk section
/// driver passes one class's pairs. All-identical classes stay for the
/// tighter verbatim sweep (no metavars beats metavars there).
pub(crate) fn try_call_peel_sweep(
    minuses: &[&str],
    pluses: &[&str],
    path: &str,
    file_old: Option<&str>,
    changed: &[usize],
    used_rules: &mut Vec<String>,
) -> Option<String> {
    if minuses.len() < 2 {
        return None;
    }
    let mut class: Option<PeelClass> = None;
    for (m, p) in minuses.iter().zip(pluses.iter()) {
        let c = parse_peel_pair(m, p)?;
        if class.as_ref().is_some_and(|k| *k != c) {
            return None;
        }
        class = Some(c);
    }
    let c = class?;
    // Hetero required: an all-identical class compresses tighter as a
    // verbatim hunk (no `# expr` directives), so leave it for verbatim.
    if minuses
        .iter()
        .zip(pluses.iter())
        .all(|(m, p)| flat_ws(m) == flat_ws(minuses[0]) && flat_ws(p) == flat_ws(pluses[0]))
    {
        return None;
    }
    if !crate::totality::peel_holds(
        file_old,
        &c.old_cal,
        &c.head_value,
        c.tails + 2,
        minuses.len(),
        minuses,
        changed,
    ) {
        return None;
    }
    let rule = unique_rule(
        used_rules,
        &format!("calls_{}_to_{}_peel", c.old_cal, c.new_cal),
    );
    let mut hunk = format!(
        "{header}\n@@ -1,1 @@ rule={rule}\n# sweep\n",
        header = crate::git_header(path)
    );
    let mut tails = Vec::new();
    for i in 0..c.tails {
        let v = format!("T{i}");
        tails.push(v);
    }
    let tail_list = tails.join(", ");
    let minus_inner = if tail_list.is_empty() {
        "M".to_string()
    } else {
        format!("M, {tail_list}")
    };
    hunk.push_str(&format!(
        "- {f}(D, &[{h}, {minus_inner}])\n+ {g}(D, M, &[{tail_list}])\n",
        f = c.old_cal,
        g = c.new_cal,
        h = c.head_text,
    ));
    Some(hunk)
}

/// Cross-hunk peel driver for non-whole sections (T441): mirrors the
/// `try_verbatim_sweep` skeleton — group balanced-hunk pairs by peel
/// class, emit one hunk per passing hetero class, leave everything
/// else ordinary for verbatim + per-hunk to retry. Peel bodies are
/// `@@ -1,1 @@` single-pattern hunks, so the later verbatim pass
/// groups them at n=1 (skipped) and passes them through unchanged.
pub(crate) fn try_call_peel_section(
    section: &str,
    _path: &str,
    used_rules: &mut Vec<String>,
    file_old: Option<&str>,
) -> Option<String> {
    let (prelude, hunks) = split_section_hunks(section);
    if hunks.is_empty() {
        return None;
    }
    let hunk_changed: Vec<Vec<usize>> = hunks
        .iter()
        .map(|h| crate::totality::changed_old_lines(h))
        .collect();
    // Balanced-hunk pairs in order; unbalanced hunks stay leftovers.
    let mut pairs: Vec<(usize, String, String)> = Vec::new();
    let mut hunk_balanced = vec![false; hunks.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        let mut minuses = Vec::new();
        let mut pluses = Vec::new();
        for line in hunk.lines().skip(1) {
            if line.starts_with('-') && !line.starts_with("---") {
                minuses.push(line[1..].to_string());
            } else if line.starts_with('+') && !line.starts_with("+++") {
                pluses.push(line[1..].to_string());
            }
        }
        if minuses.len() == pluses.len() && !minuses.is_empty() {
            hunk_balanced[i] = true;
            pairs.extend(minuses.into_iter().zip(pluses).map(|(m, p)| (i, m, p)));
        }
    }
    // Group pair indices by peel class, first-occurrence order.
    let mut classes: Vec<(PeelClass, Vec<usize>)> = Vec::new();
    for (idx, (.., m, p)) in pairs.iter().enumerate() {
        let Some(c) = parse_peel_pair(m, p) else {
            continue;
        };
        match classes.iter_mut().find(|(k, _)| *k == c) {
            Some((_, members)) => members.push(idx),
            None => classes.push((c, vec![idx])),
        }
    }
    let checkpoint = used_rules.len();
    // Fire at most one hunk per hetero class (all-identical classes
    // stay for verbatim, which is tighter there).
    let mut passing: Vec<(usize, String)> = Vec::new();
    let mut covered_pair = vec![false; pairs.len()];
    for (class, members) in &classes {
        if members.len() < 2 {
            continue;
        }
        let first = &pairs[members[0]];
        if members.iter().all(|&idx| {
            flat_ws(&pairs[idx].1) == flat_ws(&first.1)
                && flat_ws(&pairs[idx].2) == flat_ws(&first.2)
        }) {
            continue;
        }
        let ms: Vec<&str> = members.iter().map(|&idx| pairs[idx].1.as_str()).collect();
        let mut group_changed: Vec<usize> = Vec::new();
        for &idx in members {
            for &l in &hunk_changed[pairs[idx].0] {
                if !group_changed.contains(&l) {
                    group_changed.push(l);
                }
            }
        }
        if !crate::totality::peel_holds(
            file_old,
            &class.old_cal,
            &class.head_value,
            class.tails + 2,
            members.len(),
            &ms,
            &group_changed,
        ) {
            continue;
        }
        let rule = unique_rule(
            used_rules,
            &format!("calls_{}_to_{}_peel", class.old_cal, class.new_cal),
        );
        let mut body = format!("@@ -1,1 @@ rule={rule}\n# sweep\n");
        let mut tails = Vec::new();
        for i in 0..class.tails {
            let v = format!("T{i}");
            tails.push(v);
        }
        let tail_list = tails.join(", ");
        let minus_inner = if tail_list.is_empty() {
            "M".to_string()
        } else {
            format!("M, {tail_list}")
        };
        body.push_str(&format!(
            "- {f}(D, &[{h}, {minus_inner}])\n+ {g}(D, M, &[{tail_list}])\n",
            f = class.old_cal,
            g = class.new_cal,
            h = class.head_text,
        ));
        for &idx in members {
            covered_pair[idx] = true;
        }
        passing.push((members[0], body));
    }
    if passing.is_empty() {
        used_rules.truncate(checkpoint);
        return None;
    }
    // A hunk is covered iff balanced and every pair sits in a passing
    // class; leftover hunks stay ordinary in place.
    let covered_hunk: Vec<bool> = hunk_balanced
        .iter()
        .enumerate()
        .map(|(i, b)| {
            *b && pairs
                .iter()
                .enumerate()
                .filter(|(_, (j, _, _))| *j == i)
                .all(|(idx, _)| covered_pair[idx])
        })
        .collect();
    // T375 over the peel-covered set: the leftover retry is
    // fallback-identical, so the section beats the fallback exactly
    // when the peel bodies beat the covered ordinary hunks.
    let covered_len: usize = hunks
        .iter()
        .zip(&covered_hunk)
        .filter(|(_, b)| **b)
        .map(|(h, _)| h.len())
        .sum();
    if passing.iter().map(|(_, b)| b.len()).sum::<usize>() >= covered_len {
        used_rules.truncate(checkpoint);
        return None;
    }
    let mut out = prelude;
    let mut emitted = vec![false; passing.len()];
    for (i, hunk) in hunks.iter().enumerate() {
        if covered_hunk[i] {
            for (gi, (first_idx, body)) in passing.iter().enumerate() {
                if !emitted[gi] && pairs[*first_idx].0 == i {
                    out.push_str(body);
                    emitted[gi] = true;
                }
            }
            continue;
        }
        out.push_str(hunk);
    }
    Some(out)
}

/// Split a section into its prelude lines and native hunks (the same
/// split the per-hunk fallback uses; each part keeps trailing newlines).
fn split_section_hunks(section: &str) -> (String, Vec<String>) {
    let mut prelude = String::new();
    let mut hunks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in section.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            current = Some(String::new());
        }
        match current.as_mut() {
            Some(h) => {
                h.push_str(line);
                h.push('\n');
            }
            None => {
                prelude.push_str(line);
                prelude.push('\n');
            }
        }
    }
    if let Some(h) = current.take() {
        hunks.push(h);
    }
    (prelude, hunks)
}

/// The peeled call init of one `-`/`+` line: a `let` with a bare-call
/// init (any pattern — only the init is rewritten), or a bare
/// call-statement. `let-else` never qualifies (the engine never moves
/// into diverge targets).
fn peel_init(line: &str) -> Option<syn::Expr> {
    match syn::parse_str::<syn::Stmt>(line.trim()) {
        Ok(syn::Stmt::Local(local)) => {
            let init = local.init?;
            if init.diverge.is_some() {
                return None;
            }
            Some(*init.expr)
        }
        Ok(syn::Stmt::Expr(expr, _)) => Some(expr),
        _ => None,
    }
}

/// Parse a `-`/`+` line pair as an array-head-peel migration:
/// `f(D, &[H, M, T…])` → `g(D, M, &[T…])` with a shared string-literal
/// head `H` and equal tail lengths. Both sides parse statement-level
/// (the engine rewrites the call sub-expression in place, including
/// inside `let` inits — probe-verified); the head is a string literal
/// so it replays verbatim, and `D`/`M`/tails bind positionally.
fn parse_peel_pair(old_line: &str, new_line: &str) -> Option<PeelClass> {
    let old_init = peel_init(old_line)?;
    let new_init = peel_init(new_line)?;
    let (old_cal, old_args) = call_parts(&old_init)?;
    let (new_cal, new_args) = call_parts(&new_init)?;
    if old_args.len() != 2 || new_args.len() != 3 {
        return None;
    }
    let syn::Expr::Reference(old_ref) = &old_args[1] else {
        return None;
    };
    if old_ref.mutability.is_some() {
        return None;
    }
    let syn::Expr::Array(old_arr) = old_ref.expr.as_ref() else {
        return None;
    };
    if old_arr.elems.len() < 2 {
        return None;
    }
    let mut old_elems = old_arr.elems.iter();
    let head = old_elems.next()?;
    let syn::Expr::Lit(head_lit) = head else {
        return None;
    };
    let syn::Lit::Str(head_str) = &head_lit.lit else {
        return None;
    };
    let syn::Expr::Reference(new_ref) = &new_args[2] else {
        return None;
    };
    if new_ref.mutability.is_some() {
        return None;
    }
    let syn::Expr::Array(new_arr) = new_ref.expr.as_ref() else {
        return None;
    };
    let tails = old_arr.elems.len() - 2;
    if new_arr.elems.len() != tails {
        return None;
    }
    use quote::ToTokens;
    Some(PeelClass {
        old_cal,
        new_cal,
        head_text: flat_ws(&head.to_token_stream().to_string()),
        head_value: head_str.value(),
        tails,
    })
}
