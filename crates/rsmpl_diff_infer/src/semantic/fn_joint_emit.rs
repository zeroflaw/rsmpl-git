//! WI-4f split: joint-emission hunk builders (D5 — `fn_joint.rs` was
//! at the 2000-line gate; tests stay inline in their owning files per
//! WI-19).
//!
//! The def hunk (the only hunk for Joint-A, which rides `# follow=uses`;
//! the header pair for Joint-B) plus the Joint-B call-template hunk
//! under the same rule live here. Single-file emission (`emit` below)
//! concatenates both in one section; the workspace Joint-B rebuild
//! (`rebuild_workspace_b` in `fn_joint_ws`) places them at the def file
//! and the call file as two sections sharing one rule.

use super::fn_joint_types::{Fire, Joint};

/// The def hunk of a joint emission (the only hunk for Joint-A, which
/// rides `# follow=uses`; the header pair for Joint-B).
pub(crate) fn def_hunk_text(rule: &str, fire: &Fire) -> String {
    let mut out = format!("@@ -1,1 @@ rule={rule}\n");
    match &fire.joint {
        Joint::A => {
            out.push_str("# sweep\n");
            out.push_str("# follow=uses\n");
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
        }
        Joint::Body { bodies } => {
            out.push_str("# sweep\n");
            out.push_str("# follow=uses\n");
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
            for (body_minus, body_plus) in bodies {
                out.push_str(&format!("- {body_minus}\n"));
                out.push_str(&format!("+ {body_plus}\n"));
            }
        }
        Joint::B1 { .. } => {
            out.push_str("# sweep\n");
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
        }
        Joint::B2 {
            attr_line,
            attr_add,
            ..
        } => {
            out.push_str("# sweep\n");
            if !*attr_add {
                out.push_str(&format!("- {attr_line}\n"));
            }
            out.push_str(&format!("- {}\n", with_body_hole(&fire.def_minus)));
            if *attr_add {
                out.push_str(&format!("+ {attr_line}\n"));
            }
            out.push_str(&format!("+ {}\n", with_body_hole(&fire.def_plus)));
        }
    }
    out
}

/// The call-template hunk of a Joint-B emission (`None` for Joint-A,
/// whose calls ride the follow). WI-4f emits it at the call file.
pub(crate) fn call_hunk_text(rule: &str, fire: &Fire) -> Option<String> {
    let (call_minus, call_plus) = match &fire.joint {
        Joint::A | Joint::Body { .. } => return None,
        Joint::B1 {
            call_minus,
            call_plus,
        } => (call_minus, call_plus),
        Joint::B2 {
            call_minus,
            call_plus,
            ..
        } => (call_minus, call_plus),
    };
    Some(format!(
        "@@ -1,1 @@ rule={rule}\n# sweep\n- {call_minus}\n+ {call_plus}\n"
    ))
}

/// Single-file joint emission: the def hunk plus — for Joint-B — the
/// call-template hunk under the same rule, in one section.
pub(crate) fn emit(path: &str, rule: &str, fire: &Fire) -> String {
    let mut out = crate::git_header(path);
    out.push('\n');
    out.push_str(&def_hunk_text(rule, fire));
    if let Some(call) = call_hunk_text(rule, fire) {
        out.push_str(&call);
    }
    out
}

/// `fn foo(x: i32) {` → `fn foo(x: i32) { ... }` (the engine's fn
/// pattern needs the `{ ... }` body hole; the header text stays
/// native so the lowering re-renders byte-identically on formatted
/// code and the probe vetoes otherwise).
pub(crate) fn with_body_hole(header_line: &str) -> String {
    let trimmed = header_line.trim_end();
    let no_brace = trimmed.strip_suffix('{').unwrap_or(trimmed).trim_end();
    format!("{no_brace} {{ ... }}")
}
