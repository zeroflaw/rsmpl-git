//! WI-16: crate-local `# symbol` resolution index (SPEC 19).
//!
//! Built from `syn` alone (no build, no cargo): module paths from file
//! paths + inline `mod` nesting, item definitions per module, and `use`
//! imports/re-exports. Verifies that a `# symbol Old=crate::…` seed
//! resolves to a definition in this crate and that each matched call
//! site's spelled path resolves to that same definition. Anything else
//! (dependency/`std::` paths, glob-imported names, macro-hidden items)
//! is unresolvable and fails with the concrete-path repair — the honest
//! fallback is `# fallback=syntax`, never a silent downgrade.

use std::collections::{HashMap, HashSet};

/// Crate-local resolution index over a workspace snapshot.
#[derive(Debug, Default, Clone)]
pub struct CrateGraph {
    /// Canonical `crate::…` definition paths present in the crate.
    defs: HashSet<String>,
    /// File path → module path segments (without the `crate` root).
    /// `src/lib.rs` / `src/main.rs` → `[]`; `src/foo.rs` → `["foo"]`.
    file_modules: HashMap<String, Vec<String>>,
    /// File path → list of (local first segment, canonical prefix).
    /// Glob imports are recorded separately as opaque (never resolve).
    imports: HashMap<String, Vec<(String, String)>>,
    /// Files carrying a glob import (any site there is opaque).
    glob_files: HashSet<String>,
    /// `pub use` alias canonical → target canonical (one hop each;
    /// chains chased while proven, cycles cut).
    reexports: HashMap<String, String>,
}

impl CrateGraph {
    /// Build the index from a path → contents snapshot. Unparseable
    /// files are skipped (sites there cannot be proven).
    pub fn build(files: &HashMap<String, String>) -> Self {
        let mut g = CrateGraph::default();
        let mut paths: Vec<&String> = files.keys().collect();
        paths.sort();
        for path in paths {
            if !path.ends_with(".rs") {
                continue;
            }
            let text = &files[path];
            let Ok(parsed) = syn::parse_str::<syn::File>(text) else {
                continue;
            };
            let module = file_module_path(path);
            g.file_modules.insert(path.clone(), module.clone());
            collect_file(&parsed.items, &module, path, &mut g);
        }
        g
    }

    /// Canonical module path of a file (`[]` = crate root). Unknown
    /// files default to the root.
    pub fn module_of(&self, file: &str) -> Vec<String> {
        self.file_modules.get(file).cloned().unwrap_or_default()
    }

    /// Resolve a seeded `# symbol` path to its canonical definition.
    /// Returns `None` for external (`std::`, dependency), relative, or
    /// unresolvable paths.
    pub fn resolve_seed(&self, seed: &str) -> Option<String> {
        let canon = normalize_seed(seed)?;
        let resolved = self.chase_reexports(&canon);
        if self.defs.contains(&resolved) {
            Some(resolved)
        } else {
            None
        }
    }

    /// Resolve a call site's spelled callee to canonical form, relative
    /// to the file that contains it. `leading` is `::`-prefix, `segs`
    /// the written path segments. Returns `None` when unprovable
    /// (glob file, `qself`, unresolvable import, missing definition).
    pub fn resolve_call(&self, file: &str, leading: bool, segs: &[String]) -> Option<String> {
        if segs.is_empty() || self.glob_files.contains(file) {
            return None;
        }
        let module = self.module_of(file);
        // Absolute spellings.
        if leading {
            let canon = format!("crate::{}", segs.join("::"));
            return Some(self.chase_reexports(&canon)).filter(|c| self.defs.contains(c));
        }
        let first = &segs[0];
        if first == "crate" {
            let canon = format!("crate::{}", segs[1..].join("::"));
            if segs.len() == 1 {
                return None;
            }
            return Some(self.chase_reexports(&canon)).filter(|c| self.defs.contains(c));
        }
        if first == "self" || first == "super" || first == "Self" {
            // `super::`/`self::` need module-relative math; done
            // structurally in `self_join` (`Self::` never resolves).
            let canon = self_join(segs, &module)?;
            return Some(self.chase_reexports(&canon)).filter(|c| self.defs.contains(c));
        }
        // Import-first: a leading segment naming an import wins.
        if let Some(imports) = self.imports.get(file) {
            for (local, prefix) in imports {
                if local == first {
                    let mut canon = prefix.clone();
                    for s in &segs[1..] {
                        canon.push_str("::");
                        canon.push_str(s);
                    }
                    let resolved = self.chase_reexports(&canon);
                    if self.defs.contains(&resolved) {
                        return Some(resolved);
                    }
                    return None;
                }
            }
        }
        // Module-relative, then crate-root probe.
        let mut canon = String::from("crate");
        for s in &module {
            canon.push_str("::");
            canon.push_str(s);
        }
        for s in segs {
            canon.push_str("::");
            canon.push_str(s);
        }
        let resolved = self.chase_reexports(&canon);
        if self.defs.contains(&resolved) {
            return Some(resolved);
        }
        let root = format!("crate::{}", segs.join("::"));
        let resolved = self.chase_reexports(&root);
        if self.defs.contains(&resolved) {
            return Some(resolved);
        }
        None
    }

    fn chase_reexports(&self, canon: &str) -> String {
        let mut cur = canon.to_string();
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(next) = self.reexports.get(&cur) {
            if !seen.insert(cur.clone()) {
                break;
            }
            cur = next.clone();
        }
        cur
    }
}

/// File path → module segments (`src/`-rooted convention; other roots
/// best-effort by stripping directories down to the crate guess).
fn file_module_path(path: &str) -> Vec<String> {
    let p = path.replace('\\', "/");
    let stripped = p
        .strip_prefix("src/")
        .or_else(|| p.strip_prefix("./src/"))
        .unwrap_or(&p);
    let mut segs: Vec<String> = stripped.split('/').map(|s| s.to_string()).collect();
    let Some(last) = segs.pop() else {
        return Vec::new();
    };
    if last == "lib.rs" || last == "main.rs" || last == "mod.rs" {
        // `mod.rs` keeps its directory; `lib.rs`/`main.rs` are the root.
        if last == "mod.rs" {
            return segs;
        }
        return if stripped.contains('/') {
            segs
        } else {
            Vec::new()
        };
    }
    if let Some(stem) = last.strip_suffix(".rs") {
        segs.push(stem.to_string());
    }
    segs
}

/// `self`/`super` chains resolved against the file's module path.
fn self_join(segs: &[String], module: &[String]) -> Option<String> {
    let mut stack: Vec<String> = module.to_vec();
    let mut i = 0;
    while i < segs.len() {
        match segs[i].as_str() {
            "self" => {
                i += 1;
            }
            "super" => {
                stack.pop()?;
                i += 1;
            }
            "Self" => return None,
            _ => break,
        }
    }
    let mut out = String::from("crate");
    for s in &stack {
        out.push_str("::");
        out.push_str(s);
    }
    for s in &segs[i..] {
        out.push_str("::");
        out.push_str(s);
    }
    Some(out)
}

/// Normalize a seed to `crate::…` canonical (no re-export chase).
fn normalize_seed(seed: &str) -> Option<String> {
    let s = seed.trim();
    if let Some(rest) = s.strip_prefix("::") {
        return Some(format!("crate::{rest}"));
    }
    if let Some(rest) = s.strip_prefix("crate::") {
        if rest.is_empty() {
            return None;
        }
        return Some(s.to_string());
    }
    // Bare / `self::` / `super::` / dependency seeds are not seeded
    // definitions — seeded paths are always fully qualified in v0.1.
    None
}

fn collect_file(items: &[syn::Item], module: &[String], file: &str, g: &mut CrateGraph) {
    collect_items(items, module, file, g, false);
}

fn canon_path(module: &[String], name: &str) -> String {
    if module.is_empty() {
        format!("crate::{name}")
    } else {
        format!("crate::{}::{name}", module.join("::"))
    }
}

fn collect_items(
    items: &[syn::Item],
    module: &[String],
    file: &str,
    g: &mut CrateGraph,
    in_pub_use_chain: bool,
) {
    let _ = in_pub_use_chain;
    for item in items {
        match item {
            syn::Item::Fn(f) => {
                g.defs.insert(canon_path(module, &f.sig.ident.to_string()));
            }
            syn::Item::Struct(s) => {
                g.defs.insert(canon_path(module, &s.ident.to_string()));
            }
            syn::Item::Enum(e) => {
                g.defs.insert(canon_path(module, &e.ident.to_string()));
            }
            syn::Item::Trait(t) => {
                g.defs.insert(canon_path(module, &t.ident.to_string()));
            }
            syn::Item::Type(t) => {
                g.defs.insert(canon_path(module, &t.ident.to_string()));
            }
            syn::Item::Const(c) => {
                g.defs.insert(canon_path(module, &c.ident.to_string()));
            }
            syn::Item::Static(s) => {
                g.defs.insert(canon_path(module, &s.ident.to_string()));
            }
            syn::Item::Mod(m) => {
                let name = m.ident.to_string();
                g.defs.insert(canon_path(module, &name));
                if let Some((_, inner)) = &m.content {
                    let mut sub = module.to_vec();
                    sub.push(name);
                    collect_items(inner, &sub, file, g, false);
                }
            }
            syn::Item::Macro(m) => {
                if m.ident.is_some() {
                    // `macro_rules! name` definitions are addressable.
                    if let Some(id) = &m.ident {
                        g.defs.insert(canon_path(module, &id.to_string()));
                    }
                }
            }
            syn::Item::Use(u) => {
                collect_use(
                    &u.tree,
                    module,
                    file,
                    g,
                    matches!(u.vis, syn::Visibility::Public(_)),
                );
            }
            _ => {}
        }
    }
}

fn collect_use(
    tree: &syn::UseTree,
    module: &[String],
    file: &str,
    g: &mut CrateGraph,
    is_pub: bool,
) {
    // Flatten one `use` tree into (local, canonical-prefix) pairs plus
    // optional re-export aliases when `pub`.
    let mut out: Vec<(Vec<String>, Option<String>)> = Vec::new();
    flatten_use(tree, &mut Vec::new(), &mut out, &mut false);
    for (path_segs, alias) in out {
        if path_segs.iter().any(|s| s == "*") {
            g.glob_files.insert(file.to_string());
            continue;
        }
        let Some(canon_prefix) = use_prefix_to_canon(&path_segs, module, g, file) else {
            continue;
        };
        let local = alias.unwrap_or_else(|| path_segs.last().cloned().unwrap_or_default());
        if local.is_empty() || local == "self" || local == "super" || local == "crate" {
            continue;
        }
        if is_pub {
            // `pub use a::b` re-exports `crate::<module>::local` → target.
            let alias_canon = canon_path(module, &local);
            g.reexports.insert(alias_canon, canon_prefix.clone());
        }
        g.imports
            .entry(file.to_string())
            .or_default()
            .push((local, canon_prefix));
    }
}

fn use_prefix_to_canon(
    segs: &[String],
    module: &[String],
    g: &CrateGraph,
    file: &str,
) -> Option<String> {
    if segs.is_empty() {
        return None;
    }
    if segs[0] == "crate" {
        return Some(format!("crate::{}", segs[1..].join("::")));
    }
    if segs[0] == "self" || segs[0] == "super" {
        return self_join(segs, module);
    }
    // Relative root: if it names a known top-level module file, treat
    // as crate-relative; otherwise it is external (dependency/std).
    let top = &segs[0];
    let known_top = g
        .file_modules
        .values()
        .any(|m| m.first().is_some_and(|f| f == top));
    if known_top {
        return Some(format!("crate::{}", segs.join("::")));
    }
    // Same-module or crate-root items resolve without an import edge
    // (e.g. `use old_api;` inside the defining file is unusual but the
    // direct module-relative probe in `resolve_call` covers it); here a
    // bare single segment with no known top is likely external.
    if segs.len() == 1 {
        // Local probe: only claim when the defining file itself holds it.
        let _ = file;
        return None;
    }
    None
}

fn flatten_use(
    tree: &syn::UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, Option<String>)>,
    in_glob: &mut bool,
) {
    match tree {
        syn::UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            flatten_use(&p.tree, prefix, out, in_glob);
            prefix.pop();
        }
        syn::UseTree::Name(n) => {
            let mut full = prefix.clone();
            full.push(n.ident.to_string());
            out.push((full, None));
        }
        syn::UseTree::Rename(r) => {
            let mut full = prefix.clone();
            full.push(r.ident.to_string());
            out.push((full, Some(r.rename.to_string())));
        }
        syn::UseTree::Glob(_) => {
            let mut full = prefix.clone();
            full.push("*".to_string());
            out.push((full, None));
            *in_glob = true;
        }
        syn::UseTree::Group(gr) => {
            for t in &gr.items {
                flatten_use(t, prefix, out, in_glob);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn files(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect()
    }

    #[test]
    fn file_module_convention() {
        assert_eq!(file_module_path("src/lib.rs"), Vec::<String>::new());
        assert_eq!(file_module_path("src/foo.rs"), vec!["foo".to_string()]);
        assert_eq!(file_module_path("src/foo/mod.rs"), vec!["foo".to_string()]);
        assert_eq!(
            file_module_path("src/a/b.rs"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn seed_resolves_to_definition() {
        let g = CrateGraph::build(&files(&[(
            "src/lib.rs",
            "pub fn old_api(x: i32) -> i32 { x }\n",
        )]));
        assert_eq!(
            g.resolve_seed("crate::old_api"),
            Some("crate::old_api".to_string())
        );
        assert_eq!(g.resolve_seed("std::vec::Vec"), None);
        assert_eq!(g.resolve_seed("crate::missing"), None);
    }

    #[test]
    fn call_spelling_resolves_via_import() {
        let g = CrateGraph::build(&files(&[
            (
                "src/lib.rs",
                "pub mod a;\npub fn old_api(x: i32) -> i32 { x }\n",
            ),
            ("src/a.rs", "use crate::old_api;\nfn f() { old_api(1); }\n"),
        ]));
        assert_eq!(
            g.resolve_call("src/a.rs", false, &["old_api".to_string()]),
            Some("crate::old_api".to_string())
        );
        // Wrong-module copy does not resolve to the seed.
        let g2 = CrateGraph::build(&files(&[
            (
                "src/lib.rs",
                "pub mod vendored;\npub fn old_api(x: i32) -> i32 { x }\n",
            ),
            (
                "src/vendored.rs",
                "pub fn old_api(x: i32) -> i32 { x }\nfn g() { old_api(1); }\n",
            ),
        ]));
        assert_eq!(
            g2.resolve_call("src/vendored.rs", false, &["old_api".to_string()]),
            Some("crate::vendored::old_api".to_string())
        );
    }

    #[test]
    fn reexport_chain_resolves() {
        let g = CrateGraph::build(&files(&[
            ("src/lib.rs", "pub mod inner;\npub use inner::old_api;\n"),
            ("src/inner.rs", "pub fn old_api(x: i32) -> i32 { x }\n"),
        ]));
        assert_eq!(
            g.resolve_seed("crate::old_api"),
            Some("crate::inner::old_api".to_string())
        );
    }

    #[test]
    fn glob_is_opaque() {
        let g = CrateGraph::build(&files(&[
            ("src/lib.rs", "pub fn old_api(x: i32) -> i32 { x }\n"),
            ("src/a.rs", "use crate::*;\nfn f() { old_api(1); }\n"),
        ]));
        assert_eq!(
            g.resolve_call("src/a.rs", false, &["old_api".to_string()]),
            None
        );
    }

    #[test]
    fn inline_mod_nests() {
        let g = CrateGraph::build(&files(&[(
            "src/lib.rs",
            "mod m { pub fn old_api(x: i32) -> i32 { x } }\n",
        )]));
        assert_eq!(
            g.resolve_seed("crate::m::old_api"),
            Some("crate::m::old_api".to_string())
        );
    }
}
