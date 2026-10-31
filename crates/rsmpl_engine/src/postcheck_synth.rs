//! WI-16 plus-as-minus synth constructors for the postcondition gate.
//!
//! Split from `postcheck.rs` per the D5 size gate: the synthetic hunks
//! the WI-1.5 plus gate re-matches in the post-image. The WI-16 fallback
//! injection lives with the constructors: the plus re-match is a
//! syntactic identity check, never a second resolution proof (the new
//! callee may not be defined yet), so a resolved symbol hunk's synth
//! carries `# fallback=syntax` and the C13 gate cannot fail its own
//! postcondition.

use rsmpl_patch::{Allow, BodyLine, Directive, HunkHeader, RsmplHunk, Scope};

/// Plus lines as a file-scope `# allow=many` minus/plus pair set.
pub(crate) fn plus_pattern_hunk(rh: &RsmplHunk, plus_lines: &[String]) -> RsmplHunk {
    let mut directives = Vec::new();
    for d in &rh.directives {
        match d {
            rsmpl_patch::Directive::Allow(_)
            | rsmpl_patch::Directive::Scope(_)
            | rsmpl_patch::Directive::Optional(_) => {}
            _ => directives.push(d.clone()),
        }
    }
    directives.push(rsmpl_patch::Directive::Allow(Allow::Many));
    directives.push(rsmpl_patch::Directive::Scope(Scope::File));
    // WI-16: the plus re-match is syntactic identity, never a second
    // resolution proof (the new callee may not be defined yet).
    if directives
        .iter()
        .any(|d| matches!(d, rsmpl_patch::Directive::Symbol { .. }))
        && !directives
            .iter()
            .any(|d| matches!(d, rsmpl_patch::Directive::FallbackSyntax(true)))
    {
        directives.push(rsmpl_patch::Directive::FallbackSyntax(true));
    }
    let mut lines = Vec::with_capacity(plus_lines.len() * 2);
    for t in plus_lines {
        lines.push(BodyLine {
            prefix: '-',
            text: t.clone(),
        });
        lines.push(BodyLine {
            prefix: '+',
            text: t.clone(),
        });
    }
    let header = HunkHeader {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        new_elided: true,
        rule: rh.header.rule.clone(),
        raw_tail: String::new(),
    };
    RsmplHunk {
        header,
        directives,
        raw_directives: Vec::new(),
        lines,
    }
}

/// WI-11b container-header synth: the header rides as ` ` context lines,
/// the pair as `-`/`+` lines (the classifier scopes the pair that follows).
pub(crate) fn plus_pattern_hunk_with_context(
    rh: &RsmplHunk,
    context_lines: &[String],
    pair_lines: &[String],
) -> RsmplHunk {
    let mut directives = Vec::new();
    for d in &rh.directives {
        match d {
            rsmpl_patch::Directive::Allow(_)
            | rsmpl_patch::Directive::Scope(_)
            | rsmpl_patch::Directive::Optional(_) => {}
            _ => directives.push(d.clone()),
        }
    }
    directives.push(rsmpl_patch::Directive::Allow(Allow::Many));
    directives.push(rsmpl_patch::Directive::Scope(Scope::File));
    // WI-16: same syntactic-identity posture as `plus_pattern_hunk`.
    if directives
        .iter()
        .any(|d| matches!(d, rsmpl_patch::Directive::Symbol { .. }))
        && !directives
            .iter()
            .any(|d| matches!(d, rsmpl_patch::Directive::FallbackSyntax(true)))
    {
        directives.push(Directive::FallbackSyntax(true));
    }
    let mut lines = Vec::with_capacity(context_lines.len() + pair_lines.len() * 2);
    for t in context_lines {
        lines.push(BodyLine {
            prefix: ' ',
            text: t.clone(),
        });
    }
    for t in pair_lines {
        lines.push(BodyLine {
            prefix: '-',
            text: t.clone(),
        });
        lines.push(BodyLine {
            prefix: '+',
            text: t.clone(),
        });
    }
    let header = HunkHeader {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        new_elided: true,
        rule: rh.header.rule.clone(),
        raw_tail: String::new(),
    };
    RsmplHunk {
        header,
        directives,
        raw_directives: Vec::new(),
        lines,
    }
}
