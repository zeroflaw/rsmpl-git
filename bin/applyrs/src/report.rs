//! JSON report evidence builders (WI-15 D5 split).
//!
//! Split from `main.rs` under the 2000-line gate (D5): pure move, no
//! behavior change. Owns the token-bounded per-site/per-rule evidence
//! builders (`sites_json`, `orphans_json`) plus their helpers. `main.rs`
//! calls them as `report::…`; unit tests pinning the builders live in
//! the `mod tests` below, inline in the owning file.

/// WI-2 digest of one full binding value (SPEC D.2 `binding_meta`).
pub(crate) fn binding_sha256_hex(full: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(full.as_bytes()))
}

/// WI-15a: pre-truncation binding-entry total across all sites
/// (the denominator `sites_truncated` omits from; additive).
pub(crate) fn sites_binding_total(sites: &[rsmpl_engine::SiteReport]) -> usize {
    sites.iter().map(|s| s.bindings.len()).sum()
}

/// WI-2b/2c + WI-1a + WI-15b/WI-15d: token-bounded per-site evidence
/// (SPEC D.2). One entry per emitted edit in span order —
/// `[{span: [s, e], bindings: {...}, origin, binding_meta?: {...}}]`
/// — with 8-entry / 80-char per-site truncation. A value cut at 80 chars
/// carries a trailing `…` (U+2026) plus a `binding_meta` digest entry
/// (`len` = full chars, `sha256` = hex of the full bytes), so cross-site
/// binding equality is assertable without re-reading the file. `full`
/// (`--bindings full`) lifts every cap: all binding entries and full
/// texts, no truncation, no meta keys. Each entry carries WI-15d
/// `origin` (`"authored"` when the patch spelled the edit,
/// `"inferred"` when the follow machinery added it — the parallel
/// `span_labels` entry is `follow`; rules without labels read
/// `"authored"`). Returns the JSON array plus the summed
/// omitted-entry count (`sites_truncated`) plus the number of cut values
/// (`bindings_truncated_chars`).
pub(crate) fn sites_json(
    sites: &[rsmpl_engine::SiteReport],
    labels: &[Option<String>],
    full: bool,
) -> (serde_json::Value, usize, usize) {
    const MAX_ENTRIES: usize = 8;
    const MAX_CHARS: usize = 80;
    let mut arr = Vec::with_capacity(sites.len());
    let mut omitted_total = 0usize;
    let mut cut_total = 0usize;
    for (i, site) in sites.iter().enumerate() {
        let mut map = serde_json::Map::new();
        let mut meta = serde_json::Map::new();
        // WI-15b: `full` lifts the 8-entry cap, not just the char cut.
        let shown = if full {
            site.bindings.len()
        } else {
            site.bindings.len().min(MAX_ENTRIES)
        };
        for (k, v) in site.bindings.iter().take(shown) {
            let cut = !full && v.chars().count() > MAX_CHARS;
            let mut shown: String = if full {
                v.clone()
            } else {
                v.chars().take(MAX_CHARS).collect()
            };
            if cut {
                shown.push('…');
                cut_total += 1;
                meta.insert(
                    k.clone(),
                    serde_json::json!({
                        "len": v.chars().count(),
                        "sha256": binding_sha256_hex(v),
                    }),
                );
            }
            map.insert(k.clone(), serde_json::Value::String(shown));
        }
        omitted_total += site.bindings.len().saturating_sub(map.len());
        // WI-15d: carried provenance — never inferred from `semantics`.
        let origin = match labels.get(i).and_then(|l| l.as_deref()) {
            Some("follow") => "inferred",
            _ => "authored",
        };
        let mut entry = serde_json::json!({
            "span": [site.span.0, site.span.1],
            "bindings": serde_json::Value::Object(map),
            "origin": origin,
        });
        if !meta.is_empty() {
            entry["binding_meta"] = serde_json::Value::Object(meta);
        }
        arr.push(entry);
    }
    (serde_json::Value::Array(arr), omitted_total, cut_total)
}

/// WI-7c + WI-1a + WI-15b: token-bounded orphan evidence (SPEC D.2).
/// Max 8 entries per rule, each `text` truncated to 80 chars with a
/// trailing `…` when cut (`full` lifts both caps: all entries, full
/// texts). Every entry carries the WI-2 `reason`.
/// Returns the JSON array plus the omitted-entry count
/// (`orphans_truncated`) plus the number of cut texts
/// (`orphans_truncated_chars`).
pub(crate) fn orphans_json(
    orphans: &[rsmpl_engine::OrphanReport],
    full: bool,
) -> (serde_json::Value, usize, usize) {
    const MAX_ORPHANS: usize = 8;
    const MAX_CHARS: usize = 80;
    // WI-15b: `full` lifts the 8-entry cap, not just the char cut.
    let shown = if full {
        orphans.len()
    } else {
        orphans.len().min(MAX_ORPHANS)
    };
    let mut arr = Vec::with_capacity(shown);
    let mut cut_total = 0usize;
    for o in orphans.iter().take(shown) {
        let cut = !full && o.text.chars().count() > MAX_CHARS;
        let mut text: String = if full {
            o.text.clone()
        } else {
            o.text.chars().take(MAX_CHARS).collect()
        };
        if cut {
            text.push('…');
            cut_total += 1;
        }
        arr.push(
            serde_json::json!({"file": o.file, "line": o.line, "reason": o.reason, "text": text}),
        );
    }
    let omitted = orphans.len().saturating_sub(arr.len());
    (serde_json::Value::Array(arr), omitted, cut_total)
}
