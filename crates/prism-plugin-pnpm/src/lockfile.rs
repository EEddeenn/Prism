//! `pnpm-lock.yaml` parsing into a resolved graph (and canonical facts).
//!
//! pnpm's lockfile is YAML, but the part Prism needs is structurally regular, so
//! this module hand-rolls a focused, indentation-based reader (no YAML library —
//! the `prism-*` crates stay dependency-free). It reads exactly two sections:
//!
//! - `importers:` — each workspace package's *direct* dependencies (the roots).
//!   A dependency entry carries a resolved `version:` ref (e.g. `1.2.3` or
//!   `1.2.3(react@18.2.0)`); `link:`/`file:`/`workspace:` refs are local
//!   workspace edges and are skipped.
//! - `snapshots:` — the EXACT resolved graph: one entry per resolved unit, keyed
//!   `name@version(peer-context)`, listing its `dependencies` / `optionalDependencies`
//!   by resolved ref. Each peer/patch context is a distinct unit (pnpm installs
//!   each separately), so the graph reproduces pnpm's topology verbatim.
//!
//! A dependency ref `name: <ref>` resolves to the unit keyed `name@<ref>`: pnpm
//! writes the ref as exactly the version-suffix of the target's snapshot key, so
//! resolution is a string-key lookup with no version solving.
//!
//! The graph is rooted at one synthetic node depending on the union of every
//! importer's external direct deps (workspace packages are collapsed into this
//! root). It is [`FidelityTier::LockfileExact`]; parsing never resolves against a
//! registry, executes nothing, and never panics on hostile bytes.

use std::collections::{HashMap, HashSet};

use prism_model::{
    ConditionKind, DependencyKind, EdgeFlags, FidelityTier, GraphFidelity, NodeFlags, VersionId,
};
use prism_plugin_api::{
    CanonicalBatch, CanonicalEvent, LockfileParser, PackageFact, PackageName, ParseError,
    RawDocument, RequirementFact, ResolvedEdge, ResolvedGraph, VersionFact,
};

/// Reject pathologically large inputs before allocating line state.
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Upper bound on resolved units, to keep an adversarial lockfile bounded.
const MAX_UNITS: usize = 2_000_000;

/// `pnpm-lock.yaml` parser.
#[derive(Debug, Default, Clone, Copy)]
pub struct PnpmLockfile;

impl LockfileParser for PnpmLockfile {
    fn parse_lockfile(&self, raw: &RawDocument) -> Result<CanonicalBatch, ParseError> {
        let text = std::str::from_utf8(&raw.bytes)
            .map_err(|_| ParseError::new("not-utf8", "pnpm-lock.yaml is not utf-8"))?;
        parse_pnpm_batch(text)
    }
}

/// One resolved occurrence, aligned 1:1 with a [`ResolvedGraph`] node (same order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PnpmUnit {
    /// Full package name (may be `@scope/name`). The synthetic root is `<root>`.
    pub name: String,
    /// Pinned version, or `None` for the synthetic root.
    pub version: Option<String>,
}

/// One resolved dependency reference: `(declared name, resolved ref, kind)`.
type Dep = (String, String, DependencyKind);

/// The two sections of a `pnpm-lock.yaml` Prism reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PnpmDoc {
    /// Union of every importer's external direct deps (deduplicated by unit key).
    root_deps: Vec<Dep>,
    /// Snapshot units in file order: `(unit_key, its deps)`.
    snapshots: Vec<(String, Vec<Dep>)>,
}

/// Leading-space count of a line (pnpm indents with spaces only).
fn indent(s: &str) -> usize {
    s.len() - s.trim_start_matches(' ').len()
}

/// `name@ref` — the snapshot-unit key a dependency ref resolves to.
fn make_unit_key(name: &str, ref_: &str) -> String {
    let mut k = String::with_capacity(name.len() + 1 + ref_.len());
    k.push_str(name);
    k.push('@');
    k.push_str(ref_);
    k
}

/// A `link:`/`file:`/`workspace:` ref is a local workspace edge, not a registry unit.
fn is_local_ref(r: &str) -> bool {
    r.starts_with("link:") || r.starts_with("file:") || r.starts_with("workspace:")
}

/// Parse a single-quoted YAML scalar (`'...'`, with `''` escaping one quote),
/// returning the unquoted text and the remainder after the closing quote.
fn take_single_quoted(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, '\'')) => {}
        _ => return None,
    }
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        if c == '\'' {
            let mut peek = chars.clone();
            if let Some((_, '\'')) = peek.next() {
                out.push('\'');
                chars.next(); // consume the doubled quote
            } else {
                return Some((out, &s[i + c.len_utf8()..]));
            }
        } else {
            out.push(c);
        }
    }
    None // unterminated
}

/// Parse a double-quoted YAML scalar with basic backslash escapes.
fn take_double_quoted(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, '"')) => {}
        _ => return None,
    }
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + c.len_utf8()..])),
            '\\' => {
                if let Some((_, e)) = chars.next() {
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                }
            }
            _ => out.push(c),
        }
    }
    None
}

/// Unquote a scalar VALUE; `{}`/`[]`/empty collapse to the empty string (no value).
fn unquote_value(v: &str) -> String {
    if v.is_empty() || v == "{}" || v == "[]" {
        return String::new();
    }
    if v.starts_with('\'') {
        if let Some((s, _)) = take_single_quoted(v) {
            return s;
        }
    } else if v.starts_with('"') {
        if let Some((s, _)) = take_double_quoted(v) {
            return s;
        }
    }
    v.to_string()
}

/// Split a `key: value` mapping line (key may be quoted) into `(key, value)`.
/// A block/empty entry yields an empty value string.
fn split_kv(content: &str) -> Option<(String, String)> {
    let c = content.trim();
    if c.is_empty() {
        return None;
    }
    if c.starts_with('\'') || c.starts_with('"') {
        let (key, rest) = if c.starts_with('\'') {
            take_single_quoted(c)?
        } else {
            take_double_quoted(c)?
        };
        let rest = rest.trim_start().strip_prefix(':')?;
        Some((key, unquote_value(rest.trim())))
    } else {
        // An unquoted key never contains ':'; the first ':' is structural.
        let idx = c.find(':')?;
        Some((c[..idx].trim().to_string(), unquote_value(c[idx + 1..].trim())))
    }
}

/// Which top-level section a line belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Importers,
    Snapshots,
    Other,
}

/// Parse the `importers:` + `snapshots:` sections of a `pnpm-lock.yaml`.
fn parse_pnpm_doc(text: &str) -> Result<PnpmDoc, ParseError> {
    if text.len() > MAX_BYTES {
        return Err(ParseError::new(
            "too-large",
            format!("pnpm-lock.yaml is {} bytes (max {MAX_BYTES})", text.len()),
        ));
    }
    let mut root_deps: Vec<Dep> = Vec::new();
    let mut root_seen: HashSet<String> = HashSet::new();
    let mut snapshots: Vec<(String, Vec<Dep>)> = Vec::new();

    let mut section = Section::Other;
    // Importer cursor.
    let mut imp_kind: Option<DependencyKind> = None;
    let mut imp_dep: Option<String> = None;
    // Snapshot cursor.
    let mut in_unit = false;
    let mut snap_field: Option<DependencyKind> = None;

    for raw in text.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let ind = indent(raw);
        let content = &raw[ind..];

        if ind == 0 {
            section = match content.trim_end() {
                "importers:" => Section::Importers,
                "snapshots:" => Section::Snapshots,
                _ => Section::Other,
            };
            imp_kind = None;
            imp_dep = None;
            in_unit = false;
            snap_field = None;
            continue;
        }

        match section {
            Section::Importers => match ind {
                2 => {
                    imp_kind = None;
                    imp_dep = None;
                }
                4 => {
                    imp_kind = split_kv(content).and_then(|(k, _)| dep_kind(&k, true));
                    imp_dep = None;
                }
                6 => {
                    if imp_kind.is_some() {
                        imp_dep = split_kv(content).map(|(k, _)| k);
                    }
                }
                8 => {
                    if let (Some(kind), Some(dep)) = (imp_kind, imp_dep.as_deref()) {
                        if let Some((k, v)) = split_kv(content) {
                            if k == "version" && !v.is_empty() && !is_local_ref(&v) {
                                let key = make_unit_key(dep, &v);
                                if root_seen.insert(key) {
                                    root_deps.push((dep.to_string(), v, kind));
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            Section::Snapshots => match ind {
                2 => {
                    if let Some((key, _)) = split_kv(content) {
                        if snapshots.len() >= MAX_UNITS {
                            return Err(ParseError::new(
                                "too-many-units",
                                format!("pnpm-lock.yaml has over {MAX_UNITS} snapshot units"),
                            ));
                        }
                        snapshots.push((key, Vec::new()));
                        in_unit = true;
                        snap_field = None;
                    }
                }
                4 if in_unit => {
                    snap_field = split_kv(content).and_then(|(k, _)| dep_kind(&k, false));
                }
                6 if in_unit => {
                    if let Some(kind) = snap_field {
                        if let Some((name, refr)) = split_kv(content) {
                            if !refr.is_empty() && !is_local_ref(&refr) {
                                if let Some(last) = snapshots.last_mut() {
                                    last.1.push((name, refr, kind));
                                }
                            }
                        }
                    }
                }
                _ => {}
            },
            Section::Other => {}
        }
    }

    Ok(PnpmDoc {
        root_deps,
        snapshots,
    })
}

/// Map a dependency-block field name to its [`DependencyKind`]. `dev` only exists
/// in importers (snapshots carry no dev edges).
fn dep_kind(field: &str, allow_dev: bool) -> Option<DependencyKind> {
    match field {
        "dependencies" => Some(DependencyKind::Runtime),
        "optionalDependencies" => Some(DependencyKind::Optional),
        "devDependencies" if allow_dev => Some(DependencyKind::Dev),
        _ => None,
    }
}

/// Split a snapshot key `name@version(peers...)` into `(name, version)`. The name
/// may be `@scope/x`; the `(peer)` / `(patch_hash=...)` context is dropped.
fn split_unit_key(key: &str) -> (String, Option<String>) {
    let head = match key.find('(') {
        Some(i) => &key[..i],
        None => key,
    };
    match head.rfind('@') {
        // `at > 0` so a scope-only `@scope/x` (no version) keeps its leading `@`.
        Some(at) if at > 0 => (head[..at].to_string(), Some(head[at + 1..].to_string())),
        _ => (head.to_string(), None),
    }
}

/// A deterministic node ordering plus the resolved edges, shared by the graph and
/// identity builders so node `i` means the same occurrence in both.
struct Assembled {
    /// Unit keys; node `local_id` is `index + 1` (node 0 is the synthetic root).
    order: Vec<String>,
    /// Resolved edges as `(from_local, to_local, kind)`.
    edges: Vec<(u32, u32, DependencyKind)>,
}

/// Intern `key` into `order`, returning its 0-based unit index.
fn intern(order: &mut Vec<String>, idx: &mut HashMap<String, u32>, key: &str) -> u32 {
    if let Some(&i) = idx.get(key) {
        return i;
    }
    let i = order.len() as u32;
    order.push(key.to_string());
    idx.insert(key.to_string(), i);
    i
}

/// Assemble the node order and edge list from a parsed document.
fn assemble(doc: &PnpmDoc) -> Assembled {
    let mut order: Vec<String> = Vec::new();
    let mut idx: HashMap<String, u32> = HashMap::new();

    // 1) Snapshot units in file order (stable node ids).
    for (uk, _) in &doc.snapshots {
        intern(&mut order, &mut idx, uk);
    }
    // 2) Any referenced-only units (deps with no snapshot entry) appended after.
    for (_, deps) in &doc.snapshots {
        for (n, r, _) in deps {
            intern(&mut order, &mut idx, &make_unit_key(n, r));
        }
    }
    for (n, r, _) in &doc.root_deps {
        intern(&mut order, &mut idx, &make_unit_key(n, r));
    }

    // Edges. Node local_id of unit index i is i + 1 (root is 0). Parallel edges
    // (same from→to) are collapsed so the topology is simple.
    let mut edges: Vec<(u32, u32, DependencyKind)> = Vec::new();
    let mut seen: HashSet<(u32, u32)> = HashSet::new();
    for (n, r, kind) in &doc.root_deps {
        if let Some(&i) = idx.get(&make_unit_key(n, r)) {
            let to = i.saturating_add(1);
            if seen.insert((0, to)) {
                edges.push((0, to, *kind));
            }
        }
    }
    for (uk, deps) in &doc.snapshots {
        let from = match idx.get(uk) {
            Some(&i) => i.saturating_add(1),
            None => continue,
        };
        for (n, r, kind) in deps {
            if let Some(&i) = idx.get(&make_unit_key(n, r)) {
                let to = i.saturating_add(1);
                if from != to && seen.insert((from, to)) {
                    edges.push((from, to, *kind));
                }
            }
        }
    }

    Assembled { order, edges }
}

/// Parse a `pnpm-lock.yaml` into its node identity column (the synthetic root at
/// index 0, then one [`PnpmUnit`] per resolved occurrence in node order).
pub fn parse_pnpm_units(text: &str) -> Result<Vec<PnpmUnit>, ParseError> {
    let doc = parse_pnpm_doc(text)?;
    let asm = assemble(&doc);
    let mut units = Vec::with_capacity(asm.order.len() + 1);
    units.push(PnpmUnit {
        name: "<root>".to_string(),
        version: None,
    });
    for key in &asm.order {
        let (name, version) = split_unit_key(key);
        units.push(PnpmUnit { name, version });
    }
    Ok(units)
}

/// Parse a `pnpm-lock.yaml` into a [`ResolvedGraph`] at [`FidelityTier::LockfileExact`].
///
/// Node 0 is the synthetic workspace root; nodes `1..` are the resolved units in
/// `assemble` order. `assign_bfs_depths_from_roots` derives depth from the edges.
pub fn parse_pnpm_graph(text: &str) -> Result<ResolvedGraph, ParseError> {
    let doc = parse_pnpm_doc(text)?;
    let asm = assemble(&doc);

    let mut g = ResolvedGraph::new();
    // Synthetic root (version-less; interning maps it to `<root>@0.0.0`).
    g.add_node(VersionId::new(1), 0, "", NodeFlags::ROOT);
    for (i, key) in asm.order.iter().enumerate() {
        // Synthetic, deterministic version id (root took 1).
        let vid = VersionId::new((i as u64).saturating_add(2));
        g.add_node(vid, 0, key.clone(), NodeFlags::EMPTY);
    }

    let mut next_ordinal: HashMap<u32, u32> = HashMap::new();
    for (from, to, kind) in asm.edges {
        let ord = next_ordinal.entry(from).or_insert(0);
        g.add_edge(ResolvedEdge {
            from,
            to,
            ordinal: *ord,
            requirement_id: None,
            kind,
            flags: EdgeFlags::RESOLVED,
        });
        *ord = ord.saturating_add(1);
    }

    g.assign_bfs_depths_from_roots();
    g.fidelity = Some(GraphFidelity::exact(FidelityTier::LockfileExact));
    Ok(g)
}

/// Parse a `pnpm-lock.yaml` into a [`CanonicalBatch`] of package/version/requirement
/// facts (one set per resolved unit; the synthetic root carries no facts).
pub fn parse_pnpm_batch(text: &str) -> Result<CanonicalBatch, ParseError> {
    let doc = parse_pnpm_doc(text)?;
    let mut batch = CanonicalBatch::new();

    // Emit a package + version fact per distinct snapshot unit.
    let mut seen: HashSet<&str> = HashSet::new();
    for (uk, _) in &doc.snapshots {
        if !seen.insert(uk.as_str()) {
            continue;
        }
        let (name, version) = split_unit_key(uk);
        let vraw = version.unwrap_or_else(|| "0.0.0".to_string());
        batch.push(CanonicalEvent::PackageSeen(PackageFact {
            name: PackageName::new(name.clone()),
            namespace: None,
            repository_url: None,
            homepage_url: None,
        }));
        batch.push(CanonicalEvent::VersionSeen(VersionFact {
            name: PackageName::new(name),
            version_raw: vraw.clone(),
            released_at_unix: None,
            license_expr: None,
            is_prerelease: vraw.contains('-'),
        }));
    }
    // Requirement facts from each unit's resolved deps.
    for (uk, deps) in &doc.snapshots {
        let (sname, sver) = split_unit_key(uk);
        let svraw = sver.unwrap_or_else(|| "0.0.0".to_string());
        for (n, r, kind) in deps {
            batch.push(CanonicalEvent::RequirementSeen(RequirementFact {
                src_name: PackageName::new(sname.clone()),
                src_version_raw: svraw.clone(),
                declared_target: n.clone(),
                target_alias: None,
                raw_requirement: r.clone(),
                kind: *kind,
                scope: None,
                condition_kind: ConditionKind::None,
                raw_condition: None,
                features_or_extras: Vec::new(),
            }));
        }
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small but representative pnpm v9 lockfile: a root importer + one workspace
    // importer, block + inline-`{}` snapshot units, a peer-suffixed unit, an
    // optionalDependency, a devDependency (importer-only), and a `link:` edge.
    const LOCK: &str = r#"lockfileVersion: '9.0'

settings:
  autoInstallPeers: true

importers:

  .:
    dependencies:
      lit:
        specifier: 3.3.3
        version: 3.3.3
      '@scope/ui':
        specifier: 0.10.0
        version: 0.10.0(lit@3.3.3)
    devDependencies:
      typescript:
        specifier: 5.0.0
        version: 5.0.0
    optionalDependencies:
      fsevents:
        specifier: 2.3.3
        version: 2.3.3
  packages/app:
    dependencies:
      local-lib:
        specifier: workspace:*
        version: link:../local-lib

packages:

  lit@3.3.3:
    resolution: {integrity: sha512-aaa}

snapshots:

  lit@3.3.3: {}

  '@scope/ui@0.10.0(lit@3.3.3)':
    dependencies:
      lit: 3.3.3
      leftpad: 1.0.0
    optionalDependencies:
      fsevents: 2.3.3
    transitivePeerDependencies:
      - supports-color

  leftpad@1.0.0: {}

  typescript@5.0.0: {}

  fsevents@2.3.3:
    optional: true
"#;

    #[test]
    fn parses_importers_and_snapshots() {
        let doc = parse_pnpm_doc(LOCK).unwrap();
        // Root deps: lit, @scope/ui, typescript (dev), fsevents (optional);
        // the `link:` workspace dep is excluded.
        assert_eq!(doc.root_deps.len(), 4);
        assert!(doc.root_deps.iter().any(|(n, r, _)| n == "@scope/ui" && r == "0.10.0(lit@3.3.3)"));
        assert!(!doc.root_deps.iter().any(|(n, _, _)| n == "local-lib"));
        // 5 snapshot units (incl the inline-`{}` leaves).
        assert_eq!(doc.snapshots.len(), 5);
        let ui = doc.snapshots.iter().find(|(k, _)| k.starts_with("@scope/ui")).unwrap();
        // lit + leftpad (runtime) + fsevents (optional) = 3 deps; the
        // transitivePeerDependencies list is ignored.
        assert_eq!(ui.1.len(), 3);
    }

    #[test]
    fn builds_graph_with_root_and_exact_topology() {
        let g = parse_pnpm_graph(LOCK).unwrap();
        // root + {lit, @scope/ui, leftpad, typescript, fsevents} = 6 nodes.
        assert_eq!(g.node_count(), 6);
        assert!(g.nodes[0].flags.contains(NodeFlags::ROOT));
        assert_eq!(g.nodes[0].occurrence_key, "");
        // Root → 4 direct deps; @scope/ui → lit, leftpad, fsevents = 3. Total 7.
        assert_eq!(g.edge_count(), 7);
        // Depth: root=0, its direct deps=1, leftpad (only via @scope/ui)=2.
        assert_eq!(g.nodes[0].depth, 0);
        let leftpad = g.nodes.iter().find(|n| n.occurrence_key == "leftpad@1.0.0").unwrap();
        assert_eq!(leftpad.depth, 2);
        assert_eq!(g.fidelity.unwrap().tier, FidelityTier::LockfileExact);
    }

    #[test]
    fn units_align_with_graph_nodes() {
        let units = parse_pnpm_units(LOCK).unwrap();
        let g = parse_pnpm_graph(LOCK).unwrap();
        assert_eq!(units.len(), g.node_count());
        assert_eq!(units[0].name, "<root>");
        assert!(units[0].version.is_none());
        // The peer-suffixed unit keeps its base name + version, peer context dropped.
        let ui = units.iter().find(|u| u.name == "@scope/ui").unwrap();
        assert_eq!(ui.version.as_deref(), Some("0.10.0"));
    }

    #[test]
    fn peer_variants_are_distinct_units() {
        let lock = r#"importers:

  .:
    dependencies:
      a:
        specifier: 1.0.0
        version: 1.0.0(react@17.0.0)
      b:
        specifier: 1.0.0
        version: 1.0.0(react@18.0.0)

snapshots:

  'shared@2.0.0(react@17.0.0)': {}

  'shared@2.0.0(react@18.0.0)': {}

  'a@1.0.0(react@17.0.0)':
    dependencies:
      shared: 2.0.0(react@17.0.0)

  'b@1.0.0(react@18.0.0)':
    dependencies:
      shared: 2.0.0(react@18.0.0)
"#;
        let g = parse_pnpm_graph(lock).unwrap();
        // Two distinct `shared@2.0.0` occurrences (one per peer context).
        let shared: Vec<&str> = g
            .nodes
            .iter()
            .filter(|n| n.occurrence_key.starts_with("shared@2.0.0"))
            .map(|n| n.occurrence_key.as_str())
            .collect();
        assert_eq!(shared.len(), 2);
        // Both resolve as the same name@version → a duplicate-version family the
        // facade will detect (same name, two occurrence keys).
        let units = parse_pnpm_units(lock).unwrap();
        let shared_units: Vec<&PnpmUnit> = units.iter().filter(|u| u.name == "shared").collect();
        assert_eq!(shared_units.len(), 2);
        assert!(shared_units.iter().all(|u| u.version.as_deref() == Some("2.0.0")));
    }

    #[test]
    fn facts_capture_versions_and_requirements() {
        let b = parse_pnpm_batch(LOCK).unwrap();
        let versions: Vec<&str> = b
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::VersionSeen(v) => Some(v.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(versions.contains(&"lit"));
        assert!(versions.contains(&"@scope/ui"));
        let reqs: Vec<&RequirementFact> = b
            .iter()
            .filter_map(|e| match e {
                CanonicalEvent::RequirementSeen(r) => Some(r),
                _ => None,
            })
            .collect();
        // @scope/ui declares 3 requirements (lit, leftpad, fsevents).
        let from_ui = reqs.iter().filter(|r| r.src_name.as_str() == "@scope/ui").count();
        assert_eq!(from_ui, 3);
        let opt = reqs.iter().find(|r| r.declared_target == "fsevents").unwrap();
        assert_eq!(opt.kind, DependencyKind::Optional);
    }

    #[test]
    fn malformed_and_empty_inputs_do_not_panic() {
        // Not utf-8 is handled at the LockfileParser boundary; here, garbage text
        // simply yields an empty doc (no importers/snapshots sections).
        assert_eq!(parse_pnpm_graph("").unwrap().node_count(), 1); // lone root
        assert_eq!(parse_pnpm_graph("not: yaml\njust: scalars\n").unwrap().node_count(), 1);
        let doc = parse_pnpm_doc("snapshots:\n  weird line without colon\n").unwrap();
        assert!(doc.snapshots.is_empty());
    }

    #[test]
    fn single_quoted_keys_with_escapes() {
        // A doubled '' is one literal quote inside a single-quoted scalar.
        let (k, v) = split_kv("'a''b': 1.0.0").unwrap();
        assert_eq!(k, "a'b");
        assert_eq!(v, "1.0.0");
        // A scoped, peer-suffixed key with a ':' nowhere structural.
        let (k, _) = split_kv("'@scope/x@1.0.0(y@2.0.0)':").unwrap();
        assert_eq!(k, "@scope/x@1.0.0(y@2.0.0)");
    }
}
