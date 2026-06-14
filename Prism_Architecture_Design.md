---
title: "Prism Architecture Design"
subtitle: "A lightweight, local-first dependency topology analysis system"
author: "Architecture Draft"
date: "2026-06-13"
---

# Contents {-}

- Executive Summary
- Product Thesis
- Goals and Non-Goals
- System Overview
- Canonical Domain Model
- Storage Architecture
- Fetcher, Parser, and Resolver Architecture
- Ecosystem Plugin Design
- Fetch Scheduler
- Resolution and Graph Construction
- Graph Algorithms
- TDA Architecture
- Rust API Design
- TypeScript Integration
- UI Architecture
- CLI Design
- Security Model
- Performance Strategy
- Testing Strategy
- Milestone Plan
- Architectural Risks and Mitigations
- Design Decisions
- Open Questions
- Recommended Version 0.1 Scope
- Appendices


# Executive Summary

Prism is a local-first system for analyzing software package dependency networks across multiple package ecosystems. It accepts a project, lockfile, manifest, SBOM, or explicit package list, recursively expands package dependencies, preserves multi-version relationships, builds resolved dependency graph snapshots, derives graph and risk features, and constructs topology-ready representations for TDA workflows.

The core design choice is that Prism does not use an existing database as its primary storage engine. Instead, Prism uses a purpose-built storage layer optimized for dependency analysis:

```text
append-only object segments
+ typed immutable indexes
+ content-addressed raw blobs
+ immutable CSR graph snapshots
+ range-based candidate version indexes
+ array-based TDA result files
```

Prism is not a general-purpose database, graph database, package manager, or package installer. It is a domain-specific dependency topology engine. The system is designed around the operations that matter for dependency analysis:

- find a package by canonical ecosystem name;
- list all known versions of a package;
- read declared requirements from a package version;
- compute candidate version sets under ecosystem-specific version semantics;
- construct resolved dependency graph snapshots;
- traverse forward and reverse dependency relationships;
- identify duplicate versions, cycles, strongly connected components, upgrade pressure, and conflict pressure;
- build filtrations and TDA outputs that can be mapped back to concrete packages, versions, requirements, and graph edges.

The implementation stack is Rust for core logic and TypeScript for UI and product surfaces. Rust owns storage, codecs, fetchers, parsers, resolver adapters, graph algorithms, and TDA preprocessing. TypeScript owns the desktop or web UI, interactive graph visualization, query panels, and topology result exploration.

# Product Thesis

Modern software risk often emerges from transitive dependency structure rather than from direct dependencies alone. A project may have a small manifest but a large, heterogeneous, version-fragmented dependency graph. Prism treats this graph as a first-class object and preserves enough semantic detail to ask questions such as:

- Which dependency paths introduce a vulnerable or deprecated version?
- Where do multiple versions of the same package family coexist?
- Which requirements create tight or nearly unsatisfiable version constraints?
- Which package versions act as high-centrality bridges between dependency regions?
- How does the resolved graph change under different resolver policies?
- Which topological features persist across depth, risk, age, or version-distance filtrations?
- Which original packages and edges support a persistent topological feature?

The key insight is to model dependency data in several layers, not as one flat graph:

```text
Package family layer
  e.g. npm:react, cargo:serde, pypi:requests

Package version layer
  e.g. npm:react@18.2.0

Requirement layer
  e.g. A@1.0.0 declares B >=2,<3

Candidate version layer
  e.g. B@2.0.0, B@2.1.0, B@2.5.0 satisfy the requirement

Resolved graph layer
  e.g. in this resolver run, A@1.0.0 occurrence depends on B@2.5.0 occurrence

Topological analysis layer
  e.g. filtered complexes, persistence pairs, generator support
```

# Goals and Non-Goals

## Goals

Prism should:

1. Support local-first dependency analysis without requiring PostgreSQL, Neo4j, SQLite, or another database server.
2. Preserve ecosystem-specific semantics while exposing a unified canonical model.
3. Separate declared requirements from candidate versions and resolved edges.
4. Store raw input provenance so parser and resolver results can be reproduced.
5. Support recursive dependency expansion under bounded budgets.
6. Store resolved graphs as immutable graph snapshots optimized for repeated traversal.
7. Support multiple graph snapshots for different environments, package manager versions, lockfiles, or resolution policies.
8. Provide Rust APIs for graph traversal, impact analysis, duplicate-version detection, conflict pressure analysis, and TDA preprocessing.
9. Provide TypeScript APIs for visualization and product interaction.
10. Enable TDA outputs to be traced back to original graph nodes, edges, package versions, and requirements.

## Non-Goals

Prism should not attempt to:

1. Implement a general SQL database.
2. Implement a general graph database query engine.
3. Replace package managers or perform installation.
4. Execute package build scripts, install scripts, setup scripts, or arbitrary code from packages.
5. Enumerate all possible dependency resolutions for large projects.
6. Fetch or mirror entire public registries by default.
7. Treat TDA features as direct causal explanations without mapping them back to dependency evidence.

# System Overview

Prism is organized as a pipeline from raw project or registry inputs to canonical dependency facts, graph snapshots, and topological analysis products.

```text
Input
  -> Project detector
  -> Manifest parser / lockfile parser
  -> Canonical requirement graph
  -> Registry fetcher
  -> Registry metadata parser
  -> Candidate provider
  -> Resolver adapter
  -> Resolved graph snapshot
  -> Graph feature extractor
  -> Filtration builder
  -> Persistence result mapper
  -> UI / CLI / export surfaces
```

The high-level system modules are:

```text
prism-model
  Canonical types, IDs, version constraints, conditions, package keys

prism-codec
  Binary encoding, checksums, varints, section readers and writers

prism-store
  Object segments, typed indexes, blob store, manifest, compaction

prism-textfmt
  Hand-rolled JSON + (subset) TOML parsers for offline manifest/lockfile ingestion

prism-plugin-api
  Traits for ecosystems: parse, version, requirement, condition, resolve.
  (Fetch responsibilities live inside each ecosystem plugin; the originally
  planned standalone `prism-fetch` crate has been folded into the plugins.)

prism-plugin-npm
prism-plugin-cargo
prism-plugin-bun
prism-plugin-pnpm
prism-plugin-uv
  Ecosystem implementations (npm, cargo, bun-as-npm, pnpm-as-npm, uv/PyPI).
  Other ecosystems such as maven/go can be added as new plugins implementing
  the same trait.

prism-resolve
  Native resolver adapters and synthetic resolver policies

prism-graph
  CSR graph snapshots, traversal, SCC, topological order, path search, metrics

prism-tda
  Filtration construction, simplex/cell storage, persistence result mapping

prism-api
  Rust high-level API exposed to CLI, Tauri, or Node bindings

prism-cli
  Command-line interface for ingestion, analysis, export, and debugging

prism-ui
  TypeScript UI for graph exploration and TDA visualization
```

# Canonical Domain Model

Prism uses a canonical dependency model shared across ecosystems. Ecosystem-specific details are normalized into typed fields and structured ASTs rather than flattened into strings.

## Identity and ID Model

Every identifier is an explicit newtype defined in `prism-model` (e.g. `pub struct PackageId(u64);`), never a bare alias. Widths, allocation, and stability scope are normative:

```text
ID               Width                          Scope            Allocation
StringId         u32                            store-global     intern table, monotonic
EcosystemId      u32                            process-global   static plugin registry
PackageId        u64                            store-global     monotonic counter
VersionId        u64                            store-global     monotonic counter
RequirementId    u64                            store-global     monotonic, canonicalized
EnvironmentId    u64                            store-global     content hash, deduped
SnapshotId       u64                            store-global     monotonic counter
RunId            u64                            store-global     monotonic counter
FiltrationId     u64                            store-global     monotonic counter
PairId           u64                            store-global     globalized filtration ordinal
BlobId           32 bytes (sha256)              content-address  SHA-256 of redacted bytes
HashDigest       32 bytes (sha256)              content-address  SHA-256
RecordRef        {u32 segment_id, u64 off, u32 len}  epoch-local physical location
GraphNodeId      u32                            snapshot-local   dense 0..node_count
GraphEdgeId      u32                            snapshot-local   dense, = CSR position
CellId           u64                            filtration-local dense
```

Rules:

1. Store-global integer IDs are drawn from monotonic counters whose high-water marks are recovered from the durably published `MANIFEST`, so they never regress after a crash and are never reused.
2. `GraphNodeId`/`GraphEdgeId` are dense snapshot-local indices assigned during CSR finalization. They are valid only inside their run's `.pcsr` and MUST always be carried with their `RunId` at any external boundary. `u32::MAX` is the reserved NULL sentinel. A snapshot is capped at `MAX_NODES = MAX_EDGES = u32::MAX - 1`; the CSR writer rejects graphs that exceed either bound (see `FetchBudget`).
3. `CellId` is u64 because directed-flag-complex and sampled artifacts can exceed the u32 range even when the underlying graph does not.
4. Big-endian key encoding in typed indexes (see Storage) applies only to these fixed-width unsigned integer IDs.

### External vs internal identity (Decision: adopt PURL)

PURL is the canonical *external/portable* identity for packages and versions. `Package` carries `purl_base`, `PackageVersion` carries a full `purl`, and every export and deep link uses PURL as the stable cross-system key. Internal integer IDs are store-local handles, are not portable across workspaces, and never appear as the primary key in exported artifacts.

The canonical *internal* identity of a package within an ecosystem is the tuple `(EcosystemId, namespace, canonical_name)`, indexed by `PKG_BY_NAME`. `canonical_name` is, by contract, the output of the plugin's `NameNormalizer` and is the sole package-identity key within an ecosystem. Both producers of a package (manifest parse and registry parse) MUST route the name through the same `NameNormalizer`, so `PKG_BY_NAME` and `VER_BY_PURL` cannot disagree. Per-ecosystem normalization:

```text
npm:    namespace = @scope (lowercased); unscoped name preserved case-sensitively,
        plus a case-folded collision key indexed to flag historical collisions.
PyPI:   PEP 503 (lowercase; runs of [-_.] collapse to a single '-'); no namespace.
Cargo:  crates.io name as published; namespace = source id when not the default registry.
Maven:  namespace = groupId, name = artifactId; classifier/type live on the version, not the family.
Go:     full module path; uppercase case-encoded (!u); the /vN major suffix is part of the
        module path and therefore part of family identity.
```

### RequirementId canonicalization

A `RequirementId` is allocated per canonicalized `(src_version_id, dst_package_id, constraint_hash, kind, condition_hash)` tuple, so identical declarations dedup to one requirement and `REQS_BY_DST_PACKAGE` returns one logical constraint per declaration. A `CandidateSet` is self-identifying: it carries `(requirement_id, environment_id, snapshot_id)` so a cached set is unambiguous and conflict attribution is precise.

### Occurrence identity

`GraphNode.occurrence_key` is the interned (`StringId`) canonical form of a structured `OccurrenceContext`, which is retained so occurrences stay queryable:

```rust
pub struct OccurrenceContext {
    pub path: Option<StringId>,           // npm node_modules / lockfile path
    pub feature_set_ref: Option<BlobId>,  // Cargo unified features
    pub extras_ref: Option<BlobId>,       // PyPI activated extras
    pub platform: Option<StringId>,       // target triple / marker context
    pub mediation_ref: Option<BlobId>,    // Maven dependency-path context
}
```

`occurrence_key = hash(OccurrenceContext)` for fast equality/interning; the structured fields are kept so the UI can answer "which features/platform/path distinguish this occurrence." The occurrence-identity function is ecosystem-specific and MUST match the ecosystem's real dedup behavior (see Ecosystem Plugin Design): npm = one node per dedup path; Cargo = one node per `(source, name, version)` *after* feature unification (feature context is recorded, it does not split nodes); PyPI = one node per `(distribution, version, extras-activation)`; Maven = one node per mediated coordinate; Go = one node per `(module path, selected version)`.

## Core Entities

### Package

A package is a family of versions within an ecosystem.

Examples:

```text
npm:react
cargo:serde
pypi:requests
maven:org.slf4j:slf4j-api
go:golang.org/x/net
```

Conceptual fields:

```rust
pub struct Package {
    pub id: PackageId,
    pub ecosystem: EcosystemId,
    pub namespace: Option<StringId>,
    pub name: StringId,
    pub canonical_name: StringId,
    pub purl_base: StringId,
    pub repository_url: Option<StringId>,
    pub homepage_url: Option<StringId>,
    pub metadata_ref: Option<BlobId>,
}
```

### PackageVersion

A package version is a concrete published version of a package family.

```rust
pub struct PackageVersion {
    pub id: VersionId,
    pub package_id: PackageId,
    pub version_raw: StringId,
    pub version_norm: StringId,
    pub version_sort_key: Vec<u8>,
    pub purl: StringId,
    pub released_at_unix: Option<i64>,
    pub flags: VersionFlags,
    pub license_expr: Option<StringId>,
    pub artifact_hash: Option<HashDigest>,
    pub metadata_ref: Option<BlobId>,
}
```

The `version_sort_key` is ecosystem-specific. Prism must not assume that all ecosystems use SemVer. Each ecosystem plugin provides a version scheme that can parse, normalize, compare, and generate sortable keys.

`version_sort_key` is required to induce a **total order**: no two distinct versions of one package may compare equal, and the plugin must define a deterministic tie-break for otherwise-incomparable versions (PEP 440 local versions, Maven `SNAPSHOT` vs release, npm prerelease/build metadata, Go pseudo-versions vs tags). Ordinal candidate ranges (below) are valid **only relative to a specific `SnapshotId`**: a package's sorted version list is immutable within a snapshot, and newly fetched versions create a *new* snapshot rather than renumbering an existing list. This guarantees that a stored `CandidateSet`'s ordinals never silently shift under it. Yanked versions remain in the list (so ordinals are stable) but carry a `VersionFlags::YANKED` bit and are excluded from candidate ranges by default.

### Requirement

A requirement represents a declared dependency constraint from one package version to a target package family. It is not the same thing as a resolved edge.

```rust
pub struct Requirement {
    pub id: RequirementId,
    pub src_version_id: VersionId,
    pub declared_target: StringId,        // raw/uninterpolated coordinate as written
    pub dst_package_id: Option<PackageId>,// resolved AFTER effective-POM build / patch / replace
    pub target_alias: Option<StringId>,
    pub raw_requirement: StringId,
    pub constraint_ref: BlobId,
    pub managed_version_ref: Option<BlobId>, // version supplied by depMgmt/BOM/workspace, if any
    pub kind: DependencyKind,
    pub scope: Option<StringId>,
    pub condition_kind: ConditionKind,    // Environment vs Activation (see resolution)
    pub condition_ref: Option<BlobId>,
    pub features_or_extras_ref: Option<BlobId>,
    pub metadata_ref: Option<BlobId>,
}
```

A requirement can carry:

- a version constraint AST (which may defer to a `managed_version_ref` supplied by Maven `dependencyManagement`/BOM, a Cargo/npm workspace, or a Maven parent);
- a declared target coordinate (`declared_target`) that may be an uninterpolated `${property}` or a name subject to `patch`/`replace`/source redirection; `dst_package_id` is filled in only after effective-POM building / workspace inheritance / redirect application, so it is `Option`. `REQS_BY_DST_PACKAGE` indexes only requirements with a resolved target;
- dependency kind, such as runtime, dev, build, peer, optional, test, plugin, or workspace;
- a `condition_kind` discriminating **environment conditions** (platform, target triple, `Requires-Python`, Maven profile, npm os/cpu — evaluated against `Environment`) from **activation conditions** (PEP 508 `extra ==`, Cargo optional-dependency feature gates — evaluated against the resolved `ActivationContext`);
- feature, extra, profile, or classifier metadata;
- provenance pointing to the original manifest, lockfile, registry metadata, or artifact metadata.

### CandidateSet

A candidate set is the set of package versions that may satisfy a requirement under a source policy, snapshot, and environment. Candidate sets should be represented as ordinal ranges over a package's sorted version list whenever possible.

```rust
pub struct CandidateSet {
    pub requirement_id: RequirementId,   // self-identifying binding ...
    pub environment_id: EnvironmentId,   // ... so a cached set is unambiguous
    pub snapshot_id: SnapshotId,         // ordinals are valid only for this snapshot
    pub package_id: PackageId,
    pub ranges: Vec<std::ops::Range<u32>>,
    pub exclusions: Vec<(u32, ExclusionReason)>, // yanked / requires-python / no-wheel / ...
    pub reason: CandidateReason,
}
```

Ordinals index the package's `snapshot_id`-pinned sorted version list (see `version_sort_key`). Exclusions carry a per-version reason (`Yanked`, `RequiresPythonMismatch`, `NoCompatibleWheel`, `DynamicMetadataUnknown`, …) so the UI can explain *why* a version is not a candidate. Candidate intersection and `compatible()` are only meaningful between two `CandidateSet`s with the **same `package_id`** (see Graph Algorithms).

This avoids materializing thousands of `(requirement, version)` rows and makes intersection, conflict detection, and constraint tightness analysis efficient.

### ResolverRun

A resolver run records a specific analysis context.

```rust
pub struct ResolverRun {
    pub id: RunId,
    pub ecosystem: EcosystemId,
    pub root_set_hash: HashDigest,
    pub resolver_name: StringId,
    pub resolver_version: Option<StringId>,
    pub environment_id: EnvironmentId,
    pub snapshot_id: SnapshotId,
    pub input_manifest_ref: Option<BlobId>,
    pub input_lockfile_ref: Option<BlobId>,
    pub status: RunStatus,
    pub fidelity: GraphFidelity,
    pub created_at_unix: i64,
    pub metadata_ref: Option<BlobId>,
}
```

Prism does not have one global dependency graph. It has many graph snapshots, each tied to a run.

### Environment

An environment is the resolution-relevant target description that condition evaluation and candidate filtering consume. It is **content-addressed**: `EnvironmentId` is the truncated SHA-256 of the canonicalized struct, so equal environments dedup and the `EnvironmentId` in `ResolverRun`, the environment hash in the fetch key, and the `&Environment` passed to `candidates()`/`condition_matches()` all refer to the same canonical form.

```rust
pub struct Environment {
    pub id: EnvironmentId,                  // = hash(canonical fields)
    pub language_version: Option<StringId>, // e.g. Python 3.12, Go 1.22, JDK 21
    pub implementation: Option<StringId>,   // cpython/pypy, etc.
    pub platform: Option<StringId>,         // os/arch/target triple
    pub markers: Vec<(StringId, StringId)>, // PEP 508 marker bindings, cfg() values
    pub toggles: Vec<StringId>,             // analysis-mode feature toggles
}
```

`Environment` holds only true **environment facts** (platform, language version, markers). Feature/extra/optional *activation* is resolution state, not environment state, and is modeled separately as `ActivationContext` (see Resolution and Graph Construction).

### RegistrySnapshot

A registry snapshot is a first-class, content-addressed immutable object that pins registry metadata at a point in time, making `deterministic-snapshot` runs reproducible.

```rust
pub struct RegistrySnapshot {
    pub id: SnapshotId,
    pub source: StringId,                   // registry/source id
    pub captured_at_unix: i64,
    pub digest: HashDigest,                 // over the (source,pkg,version)->blob-hash set
    pub bindings_ref: BlobId,               // frozen {(pkg,version) -> raw-blob-hash} set
}
```

`deterministic-snapshot` runs read only from a snapshot's pinned set and never hit the network; an offline-mode cache miss is an explicit error or an explicit fidelity downgrade, never a silent partial result. One `RegistrySnapshot` (registry metadata frozen in time) can feed many `ResolverRun`s; `SnapshotId` and `RunId` are distinct identities.

### GraphNode and GraphEdge

Graph nodes are occurrences in a resolved graph, not merely package versions. This is important because ecosystems such as npm can contain the same package version at multiple logical positions.

```rust
pub struct GraphNode {
    pub node_id: GraphNodeId,
    pub version_id: VersionId,
    pub occurrence_key: StringId,
    pub depth: u32,                        // min over discovery paths (BFS)
    pub enabled_features_ref: Option<BlobId>, // resolved/unified features+extras (Cargo/PyPI)
    pub flags: NodeFlags,
    pub attributes_ref: Option<BlobId>,
}

pub struct GraphEdge {
    pub edge_id: GraphEdgeId,
    pub from: GraphNodeId,
    pub to: GraphNodeId,
    pub ordinal: u32,                        // position within the source node's dep list
    pub requirement_id: Option<RequirementId>,
    pub kind: DependencyKind,
    pub flags: EdgeFlags,                    // resolution outcome (below)
    pub attributes_ref: Option<BlobId>,
}
```

`ordinal` preserves declaration order within a source node (CSR sorts by source, which otherwise loses it) for deterministic UI and npm path reproduction. `EdgeFlags` records the **resolution outcome** so a dropped dependency is an edge with a flag, not an absent edge — letting Prism answer "which optional deps were dropped":

```text
EdgeFlags:
  RESOLVED            normal resolved edge
  OPTIONAL_OMITTED    optional dependency that was not installed
  PEER_UNMET          peer requirement with no satisfying provider
  OVERRIDDEN          selection forced by an override/patch/replace (see attributes_ref)
  BACK_EDGE           closes a cycle to an existing occurrence (see graph construction)
```

The occurrence key is defined by the ecosystem plugin; see *Identity and ID Model → Occurrence identity* for the structured `OccurrenceContext` it canonicalizes and the per-ecosystem dedup rules.

Examples:

```text
npm:     lockfile package path or node_modules path
Cargo:   source + name + version + feature context when needed
PyPI:    distribution + version + environment + extras context
Maven:   mediated artifact coordinate and dependency path context
Go:      module path + selected version
```

# Storage Architecture

Prism uses a purpose-built local store. The store is optimized for append-heavy ingestion, immutable graph snapshots, reproducible parsing, and repeated graph traversal.

## Workspace Layout

A Prism workspace is a directory:

```text
.prism/
  MANIFEST          machine-managed; never hand-edited
  LOCK              advisory write lock (auto-released on process death)
  config.toml       user-editable, non-secret project config (auth secrets live OUTSIDE .prism/)

  blobs/
    sha256/
      ab/
        abcdef....zst

  segments/
    000001.pseg
    000002.pseg

  indexes/
    000001.pidx
    000002.pidx

  graphs/
    run_00000001.pcsr
    run_00000002.pcsr

  candidates/
    snapshot_0001.pcand

  tda/
    filtration_0001.ptda
    persistence_0001.ptda

  tmp/
```

## Design Principles

1. Store raw evidence separately from normalized facts.
2. Use append-only files for object records.
3. Use immutable sorted index segments for lookup.
4. Store resolved graph snapshots in CSR format.
5. Store large metadata and raw documents as content-addressed blobs.
6. Avoid in-place mutation for crash safety.
7. Use atomic manifest replacement to publish new files.
8. Support compaction but do not require it for correctness.

## Object Segments

Object records are stored in append-only `.pseg` files.

```text
Prism Segment File (.pseg)

[SegmentHeader]
[Record]
[Record]
[Record]
...
[Footer]
```

A record has a typed key and value:

```text
RecordHeader:
  magic:       u32
  record_type: u16
  flags:       u16
  key_len:     u32
  value_len:   u32
  logical_ts:  u64
  crc32:       u32

key_bytes
value_bytes
```

Record types include:

```text
StringAtom
Package
PackageVersion
Requirement
Environment
RegistrySnapshot
ResolverRun
GraphFeature
FiltrationRun
PersistencePair
GeneratorSupport
Tombstone
```

Not every entity is a segment record. The authoritative entity → storage mapping is:

```text
Entity            Segment record   Index(es)                         Physical home
StringAtom        yes              (intern table)                    .pseg
Package           yes              PKG_BY_NAME, PKG_BY_ID            .pseg
PackageVersion    yes              VER_BY_ID, VER_BY_PURL, VERS_BY_PACKAGE  .pseg
Requirement       yes              REQ_BY_ID, REQS_BY_SRC_VER, REQS_BY_DST_PACKAGE  .pseg
Environment       yes              ENV_BY_ID                         .pseg
RegistrySnapshot  yes              SNAPSHOT_BY_ID                    .pseg
ResolverRun       yes              RUN_BY_ID, GRAPH_BY_RUN, TDA_BY_RUN  .pseg
CandidateSet      no               CAND_BY_KEY                       .pcand
GraphNode/Edge    no               (addressed by GraphNodeId/EdgeId) .pcsr (CSR arrays)
GraphFeature      yes              FEATURE_BY_RUN                    .pseg
FiltrationRun     yes              TDA_BY_RUN                        .ptda header + .pseg
PersistencePair   no               PAIR_BY_ID                        .ptda (arrays)
GeneratorSupport  no               (via PairId)                      .ptda (arrays)
```

`GraphNode`/`GraphEdge` are the logical shape of CSR rows, not standalone persisted records; `PersistencePair`/`GeneratorSupport` live as arrays inside the `.ptda` file and are addressed by `PairId`.

A record reference is a physical location, stable only within a **store epoch**:

```rust
pub struct RecordRef {
    pub segment_id: u32,
    pub offset: u64,
    pub len: u32,
}
```

A *store epoch* is an integer in the `MANIFEST` that increments on every compaction. Compaction rewrites object and index segments, reassigning `segment_id`/`offset`, so **all `RecordRef`s are invalidated at an epoch boundary**. Therefore a hard invariant holds: `RecordRef` may live **only inside index segments** of its epoch; it must NEVER be embedded inside an object record, a `.pcsr`/`.ptda` file, or any externally persisted structure. Durable cross-references between records use logical IDs (`PackageId`, `VersionId`, `RequirementId`, …) only; immutable derived files reference logical IDs and `BlobId`s. (`cell_source_ref`/`ObjectRef` and `node_attr_ref` are therefore logical IDs or `BlobId`s, never `RecordRef`s.) Compaction rewrites every `RecordRef`-bearing index in the **same atomic `MANIFEST` publish** as the rewritten object segments.

Updates and deletes are expressed as new records (a tombstone for a delete). "Newest wins" is defined by `logical_ts`, a **per-store strictly-monotonic counter** whose high-water mark is recovered from the durably published `MANIFEST` so it never regresses after a crash. Both the read-merge path (memtable → newest → older) and compaction use `logical_ts` — not segment file order — as the winner function; ties (same key, same `logical_ts`) cannot occur because allocation is strictly increasing. A tombstone with the highest `logical_ts` for a key wins, hiding all older records until compaction drops them.

## Typed Index Segments

Prism indexes are immutable sorted `.pidx` files plus an in-memory memtable during writes.

Read order:

```text
memtable
  -> newest index segment
  -> older index segments
```

Index keys are typed binary keys. Examples:

```text
PKG_BY_NAME
  ecosystem + namespace + canonical_name -> PackageId

PKG_BY_ID
  PackageId -> Package record ref

VER_BY_ID
  VersionId -> PackageVersion record ref

VER_BY_PURL
  purl -> VersionId

VERS_BY_PACKAGE
  PackageId -> VersionListRef

REQ_BY_ID
  RequirementId -> Requirement record ref

REQS_BY_SRC_VER
  VersionId -> RequirementListRef

REQS_BY_DST_PACKAGE
  PackageId -> RequirementListRef

RUN_BY_ID
  RunId -> ResolverRun record ref

GRAPH_BY_RUN
  RunId -> graph snapshot path

TDA_BY_RUN
  RunId -> filtration list

ENV_BY_ID
  EnvironmentId -> Environment record ref

SNAPSHOT_BY_ID
  SnapshotId -> RegistrySnapshot record ref

CAND_BY_KEY
  (RequirementId + EnvironmentId + SnapshotId) -> CandidateSet location

PAIR_BY_ID
  PairId -> (FiltrationId + in-file pair ordinal)

FEATURE_BY_RUN
  RunId -> GraphFeature list
```

Typed keys use prefix bytes and big-endian numeric encoding so lexical order matches numeric order for IDs.

**List-valued keys** (`VERS_BY_PACKAGE → VersionListRef`, `REQS_BY_SRC_VER`/`REQS_BY_DST_PACKAGE → RequirementListRef`) are stored as **fragments unioned across segments at read time**, not as a single rewritten list: each ingest batch appends a fragment record for the key, and a read merges the fragments found across memtable → newest → older index segments, deduplicating by logical ID and keeping the entry with the highest `logical_ts`. This keeps ingestion append-only; compaction collapses fragments into one. Consequently `versions_by_package` is `O(total versions + fragments scanned)`, and the complexity target is stated against the post-compaction single-fragment case.

## Blob Store

The blob store is content-addressed by SHA-256. It stores:

- raw registry responses;
- manifests;
- lockfiles;
- SBOM files;
- parsed metadata payloads;
- constraint ASTs;
- condition ASTs;
- large JSON metadata;
- provenance records.

Path layout:

```text
blobs/sha256/ab/abcdef....zst
```

The normalized object model stores only `BlobId` references to large or rarely queried payloads.

**Secrets never enter a blob.** A mandatory redaction stage in the fetcher runs *before* content-hashing and blob-write: it strips `Authorization` headers, URL userinfo, known token query parameters, and `Set-Cookie`, and normalizes URLs to a credential-free canonical form. The stored `BlobId` is the hash of the **redacted** bytes (reproducible and safe), so the content-address invariant and the no-secrets rule are not in tension.

**Integrity on read.** A blob is verified by re-hashing on first load and checking it against its filename hash; a mismatch is a typed `ChecksumMismatch` error, not a silent read. (CRC32/footer checksums elsewhere are corruption-detection only, not tamper-proofing; the on-disk workspace is a trusted, single-user boundary — see Security Model.)

**Blob garbage collection.** Blobs are not listed in `MANIFEST`, so the file-orphan rule does not apply to them. GC is an explicit **mark-and-sweep** run only by the single writer under `LOCK`: scan every `MANIFEST`-referenced segment for live `BlobId`s, then delete blobs not in the live set whose mtime predates the current publish generation (a grace window that protects blobs written by an in-flight publish). A `.ptda`/`.pcsr` file's blobs are reachable through its run's records; removing a run releases its blobs at the next sweep.

## CSR Graph Snapshots

Each resolver run produces an immutable graph snapshot file:

```text
graphs/run_00000001.pcsr
```

The graph is stored as compressed sparse row arrays:

```text
GraphHeader:
  magic
  format_version
  run_id
  node_count
  edge_count
  flags
  checksum

Node arrays:
  node_version_id:      [VersionId; N]
  node_depth:           [u32; N]
  node_flags:           [u32; N]
  node_attr_ref:        [BlobId or offset; N]

Forward CSR:
  out_offsets:          [u32; N + 1]
  out_targets:          [GraphNodeId; M]
  out_edge_ids:         [GraphEdgeId; M]

Reverse CSR:
  in_offsets:           [u32; N + 1]
  in_sources:           [GraphNodeId; M]
  in_edge_ids:          [GraphEdgeId; M]

Edge arrays:
  edge_ordinal:         [u32; M]
  edge_requirement_id:  [RequirementId; M]
  edge_kind:            [u16; M]
  edge_scope_id:        [StringId; M]
  edge_flags:           [u32; M]
  edge_attr_ref:        [BlobId or offset; M]
```

Offsets are `u32` because any offset is bounded by `M ≤ MAX_EDGES = u32::MAX − 1`, matching the `u32` `node_count`/`edge_count` and the `u32` `GraphNodeId`/`GraphEdgeId` (Identity model). All CSR/TDA arrays are stored **little-endian** with natural alignment so that, when memory-mapping is enabled, `&[u32]`/`&[u64]` slices are zero-copy and sound; readers validate length and alignment on open. **Decision (Open Question 2): v0.1 loads snapshots into owned arrays** (simpler, trivially crash-safe, `Send + Sync`); memory-mapping is a v0.2 optimization that the little-endian/aligned layout already permits without a format change. A `GraphSnapshot` is bound to one immutable epoch's file set; compaction/GC must not delete files referenced by a live snapshot (see Store Invariants).

Direct dependency traversal is a slice lookup:

```rust
fn out_neighbors(g: &GraphSnapshot, node: GraphNodeId) -> &[GraphNodeId] {
    let start = g.out_offsets[node as usize] as usize;
    let end = g.out_offsets[node as usize + 1] as usize;
    &g.out_targets[start..end]
}
```

Reverse impact traversal uses the reverse CSR arrays:

```rust
fn in_neighbors(g: &GraphSnapshot, node: GraphNodeId) -> &[GraphNodeId] {
    let start = g.in_offsets[node as usize] as usize;
    let end = g.in_offsets[node as usize + 1] as usize;
    &g.in_sources[start..end]
}
```

## Candidate Version Index

Candidate versions are not stored as all possible requirement-version pairs by default. Prism stores package version lists sorted by ecosystem version rules and computes candidate ranges.

```text
PackageVersionIndex:
  package_id
  version_ids sorted by version_sort_key
  sort_key_offsets
  sort_key_bytes
  flags
  released_at
```

A requirement such as:

```text
B >=2.0,<3.0
```

is translated into ordinal ranges over the sorted versions of package B:

```text
versions of B:
  0: 1.0.0
  1: 1.5.0
  2: 2.0.0
  3: 2.1.0
  4: 2.5.0
  5: 3.0.0

candidate range:
  [2, 5)
```

This representation supports efficient:

- candidate enumeration;
- range intersection;
- conflict detection;
- constraint tightness scoring;
- version-distance filtration.

## TDA Store

TDA results are stored in array-oriented `.ptda` files, not as arbitrary JSON.

```text
FiltrationHeader:
  filtration_id      u64
  run_id             u64
  filtration_type    u16   (depth | staleness | ...)
  direction          u8    (0 = sublevel/increasing)
  max_dimension      u8
  cell_count         u64   (C)
  pair_count         u64   (P)

Cell arrays (stored in filtration order: the total order (birth_value, dim, cell_id),
             faces strictly before cofaces — this IS the reduction order):
  cell_dim:           [u8;  C]
  cell_birth_value:   [f64; C]
  cell_vertex_start:  [u64; C + 1]
  cell_vertices:      [GraphNodeId; total_vertices]   // 0-cells reconstruct their own boundary
  cell_boundary_start:[u64; C + 1]                    // CSR of codim-1 face incidences ...
  cell_boundary_faces:[CellId; total_faces]           // ... the boundary matrix columns
  cell_boundary_signs:[i8;    total_faces]            // orientation (Z/2: all 1) 
  cell_source_ref:    [ObjectRef; C]                  // logical IDs / BlobId, never RecordRef

Persistence arrays:
  pair_id:            [PairId; P]   // globally unique; indexed by PAIR_BY_ID
  pair_dim:           [u8;  P]
  birth_cell:         [CellId; P]
  death_cell:         [CellId or NULL; P]
  birth_value:        [f64; P]
  death_value:        [f64 or INF; P]

Generator support (one row group per pair_id):
  pair_id:            PairId
  object_type:        u8    (Node | Edge | Version | Package | Requirement)
  object_id:          u64   // logical ID, qualified by the filtration's run_id
  contribution:       i8    // Z/2 membership coefficient in the chosen representative
```

The `cell_boundary_*` CSR is the boundary matrix the reduction consumes; without it only 1-cells (whose boundary is their two endpoints) would be reconstructable. Cells are stored already in reduction order, so the persistence algorithm reads them sequentially.

The most important part is generator support. It connects a persistence feature back to packages, versions, requirements, nodes, and edges (all by logical ID, qualified by the filtration's `run_id`).

## Crash Safety

Prism uses a single-writer, many-reader model. The **write lock** is an OS advisory lock (`flock`/`fcntl`) held on an open fd on `.prism/LOCK`, so it is released automatically on process death — there is no stale-lock problem and no PID/heartbeat scheme is needed. Readers do **not** take the write lock; they snapshot the `MANIFEST` epoch at open and pin (hold open / keep loaded) that epoch's files for their lifetime. The single-writer guarantee is unenforceable on networked/cloud-synced filesystems, which are explicitly out of scope; `prism` warns when `.prism/` appears to be on such a filesystem.

The **unit of durability is a batch**: the memtable is built in RAM together with its `.pseg` records and the `.pidx` segment that indexes them, and all of it is published in one `MANIFEST` swap. The memtable therefore never represents durable-but-unindexed state, which is why **no write-ahead log is required**.

Write protocol:

```text
1. Acquire the advisory write lock on .prism/LOCK.
2. Write new .pseg/.pidx/.pcsr/.ptda/.pcand files (epoch-stamped names) under tmp/.
3. fsync each new file, then fsync the tmp/ directory.
4. Rename each file into segments/ | indexes/ | graphs/ | tda/ | candidates/,
   then fsync that destination directory (rename durability requires a dir fsync).
5. Write MANIFEST.tmp (epoch counter, logical_ts high-water, segment-id high-water,
   per-file length + whole-file digest for every referenced file).
6. fsync MANIFEST.tmp.
7. Atomically rename MANIFEST.tmp to MANIFEST, then fsync the .prism/ directory.
8. Release the lock.
```

Recovery protocol:

```text
1. Read MANIFEST (the authoritative file set, epoch, and counter high-water marks).
2. Load only files referenced by MANIFEST; trust each file only up to the byte length
   recorded in MANIFEST (this defines the valid extent and discards any torn append tail).
3. Verify each file's whole-file digest; a mismatch marks that file — and any run that
   depends on it — recoverable-inconsistent (the run is flagged incomplete), not a late hard error.
4. Restore the logical_ts and segment_id counters from MANIFEST's high-water marks.
5. Clean tmp/ — performed ONLY by the writer under the lock, so it cannot race a live publish.
6. Treat MANIFEST-unreferenced complete files in segments/.../tda/ as orphans eligible for cleanup.
```

**Store invariants** (validated at open):

```text
- segment_ids are globally unique, strictly monotonic, NEVER reused; the next id comes
  from MANIFEST's high-water mark, so it cannot regress after a crash (compaction allocates
  fresh ids, which is why old RecordRefs are invalid in the new epoch).
- Every RecordRef in a published index targets a record in a same-epoch published segment
  (referential integrity); recovery validates this.
- Every graph/candidate/tda path referenced by a published index exists in the MANIFEST
  file set and matches its run_id and digest.
- A record's value_len/key_len are bounds-checked against the segment's valid extent before
  any allocation; an out-of-bounds length or crc32 mismatch is treated as end-of-valid-data.
- Whole-file digests are recorded in MANIFEST; per-record crc32 is a cheap inner check.
```

## Compaction

Compaction is optional for correctness and can be manually triggered.

```text
prism compact
```

Compaction algorithm:

```text
1. Scan index segments from old to new.
2. Keep only the newest record for each key (winner = highest logical_ts).
3. Drop tombstoned keys and collapse list-key fragments into one fragment.
4. Rewrite compacted object and index segments with fresh segment_ids; increment the epoch.
5. Durably publish the new MANIFEST (incl. directory fsync) BEFORE unlinking any old file.
6. Unlink old files only after durable publication, and only when no live reader still
   pins the prior epoch (the writer must not delete files belonging to a pinned epoch).
7. Run blob mark-and-sweep against the new live BlobId set (see Blob Store).
```

A reader that opened the prior epoch keeps using its pinned files until it re-opens at the new epoch; because `RecordRef`s are epoch-local, a reader never mixes refs across epochs.

# Fetcher, Parser, and Resolver Architecture

Prism supports multiple package ecosystems through plugins. A plugin is not only a parser. It is a bundle of ecosystem-specific capabilities.

```text
npm plugin
  name normalizer
  version parser
  requirement parser
  registry fetcher
  registry metadata parser
  manifest parser
  lockfile parser
  condition evaluator
  resolver adapter

Cargo plugin
  name normalizer
  version parser
  requirement parser
  sparse index fetcher
  index entry parser
  Cargo.toml parser
  Cargo.lock parser
  cfg/feature evaluator
  cargo metadata adapter
```

## Input Modes

Prism supports three input modes.

### Project Locked Graph Mode

Input examples:

```text
package-lock.json
Cargo.lock
poetry.lock
uv.lock
pom.xml + dependency tree
go.mod + go.sum
```

Goal:

```text
recover the real resolved dependency graph as accurately as possible
```

This mode prefers lockfiles or native package manager outputs.

### Manifest Requirement Graph Mode

Input examples:

```text
package.json
Cargo.toml
pyproject.toml
pom.xml
go.mod
*.csproj
```

Goal:

```text
extract declared requirements, scopes, features, profiles, extras, markers, and conditions
```

This mode should not prematurely choose concrete versions.

### Candidate Version Space Mode

Input examples:

```text
npm:react@^18
cargo:serde@>=1,<2
pypi:requests~=2.31
maven:org.slf4j:slf4j-api:[1.0,2.0)
go:golang.org/x/net@latest
```

Goal:

```text
enumerate candidate ranges, detect constraint intersections, sample resolution policies, and build topology-ready version spaces
```

## Plugin Trait

The plugin API exposes capability-specific traits.

```rust
pub trait EcosystemPlugin: Send + Sync {
    fn id(&self) -> EcosystemId;
    fn capabilities(&self) -> EcosystemCapabilities;

    fn name_normalizer(&self) -> &dyn NameNormalizer;
    fn version_scheme(&self) -> &dyn VersionScheme;
    fn requirement_parser(&self) -> &dyn RequirementParser;
    fn condition_evaluator(&self) -> &dyn ConditionEvaluator;

    fn project_detector(&self) -> Option<&dyn ProjectDetector>;
    fn manifest_parser(&self) -> Option<&dyn ManifestParser>;
    fn lockfile_parser(&self) -> Option<&dyn LockfileParser>;

    fn registry_fetcher(&self) -> Option<&dyn RegistryFetcher>;
    fn metadata_parser(&self) -> Option<&dyn RegistryMetadataParser>;

    fn candidate_provider(&self) -> Option<&dyn CandidateProvider>;
    fn resolver_adapter(&self) -> Option<&dyn ResolverAdapter>;
}
```

Capability flags:

```rust
bitflags::bitflags! {
    pub struct EcosystemCapabilities: u64 {
        const PARSE_MANIFEST       = 1 << 0;
        const PARSE_LOCKFILE       = 1 << 1;
        const FETCH_REGISTRY       = 1 << 2;
        const FETCH_ARTIFACT_META  = 1 << 3;
        const ENUM_CANDIDATES      = 1 << 4;
        const NATIVE_RESOLVE       = 1 << 5;
        const SYNTHETIC_RESOLVE    = 1 << 6;
        const SUPPORTS_WORKSPACE   = 1 << 7;
        const SUPPORTS_FEATURES    = 1 << 8;
        const SUPPORTS_PLATFORM    = 1 << 9;
        const SUPPORTS_YANKED      = 1 << 10;
    }
}
```

## Fetcher Responsibilities

A fetcher retrieves raw facts. It should not write graph objects and should not make resolution decisions.

```rust
pub struct FetchRequest {
    pub ecosystem: EcosystemId,
    pub source: RegistrySource,
    pub target: FetchTarget,
    pub auth: Option<AuthRef>,
    pub snapshot: Option<SnapshotId>,
    pub cache_policy: CachePolicy,
}

pub enum FetchTarget {
    PackageIndex { package: PackageName },
    VersionMetadata { package: PackageName, version: VersionReqOrExact },
    ArtifactMetadata { artifact_url: String, expected_hash: Option<HashDigest> },
    Artifact { artifact_url: String, expected_hash: Option<HashDigest> },
    LocalFile { path: std::path::PathBuf },
}

pub struct RawDocument {
    pub source_key: String,
    pub media_type: Option<String>,
    pub content_hash: HashDigest,
    pub bytes: Vec<u8>,
    pub fetched_at_unix: i64,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub provenance: Provenance,
}
```

Fetcher responsibilities:

- content-addressed caching;
- conditional requests when supported;
- authentication and secret redaction;
- rate limiting;
- bounded concurrency;
- retry with backoff;
- redirect policy;
- checksum verification;
- maximum response size enforcement;
- offline mode;
- provenance capture.

## Parser Responsibilities

A parser converts raw documents into canonical events. It should be deterministic and side-effect free.

```rust
pub trait RegistryMetadataParser: Send + Sync {
    fn parse_package_index(
        &self,
        raw: &RawDocument,
    ) -> Result<CanonicalBatch, ParseError>;

    fn parse_version_metadata(
        &self,
        raw: &RawDocument,
    ) -> Result<CanonicalBatch, ParseError>;

    fn parse_artifact_metadata(
        &self,
        raw: &RawDocument,
    ) -> Result<CanonicalBatch, ParseError>;
}
```

Canonical events are written to the store by a separate writer:

```rust
pub enum CanonicalEvent {
    PackageSeen(PackageFact),
    VersionSeen(VersionFact),
    RequirementSeen(RequirementFact),
    DistFileSeen(DistributionFileFact),
    YankedStatusSeen(YankedFact),
    DeprecationSeen(DeprecationFact),
    ChecksumSeen(ChecksumFact),
    Warning(ParseWarning),
}
```

## ResolverAdapter Responsibilities

Resolver adapters produce resolved graphs. They come in two forms:

1. Native adapters: parse lockfiles or call ecosystem tools.
2. Synthetic adapters: implement Prism's research-oriented resolver policies.

`resolve` is `async` but the trait is reached via `&dyn ResolverAdapter`, so it is written as an explicit boxed `Send` future (equivalently `#[async_trait]`) to stay object-safe and runtime-agnostic; resolve futures must be `Send` (the scheduler may run them on a multi-threaded executor):

```rust
pub trait ResolverAdapter: Send + Sync {
    fn fidelity(&self) -> GraphFidelity;

    fn resolve<'a>(
        &'a self,
        request: ResolveRequest,
        ctx: &'a ResolveContext,
    ) -> Pin<Box<dyn Future<Output = Result<ResolvedGraph, ResolveError>> + Send + 'a>>;
}
```

Resolution policies include:

```text
locked
latest
lowest
highest-non-prerelease
time-travel-at(timestamp)
security-preferred
random-sample
max-diversity
```

### GraphFidelity (single source of truth)

There is exactly one fidelity type, `GraphFidelity`, used by `ResolverAdapter::fidelity()`, the persisted `ResolverRun.fidelity` field, and the UI. It is a **base tier plus a bitflag set of dropped-semantics caveats**, which the earlier flat enums could not express (a run can be, e.g., model-resolved *and* missing peer placement *and* approximating markers):

```rust
pub struct GraphFidelity {
    pub tier: FidelityTier,
    pub caveats: FidelityCaveats, // bitflags
}

pub enum FidelityTier {
    LockfileExact,        // read verbatim from a lockfile
    NativeToolResolved,   // produced by a native resolver (cargo metadata, go list, ...)
    ModelResolved,        // Prism synthetic resolver, full fixpoint
    CandidateSpaceOnly,   // ranges only, no concrete selection
}

bitflags::bitflags! {
    pub struct FidelityCaveats: u32 {
        const NO_PEER_PLACEMENT     = 1 << 0;
        const NO_FEATURE_UNIFICATION= 1 << 1;
        const NO_PROFILE_EXPANSION  = 1 << 2;
        const NO_MEDIATION          = 1 << 3;
        const NO_EXTRA_ACTIVATION   = 1 << 4;
        const MARKERS_APPROXIMATED  = 1 << 5;
        const DYNAMIC_METADATA_SKIPPED = 1 << 6;
        const BUDGET_TRUNCATED      = 1 << 7;
    }
}
```

The UI surfaces the tier as a coarse label and the caveats as explicit "what was dropped" badges, so a run never overclaims correctness.

# Ecosystem Plugin Design

## npm Plugin

Primary objects:

```text
package.json
package-lock.json
npm-shrinkwrap.json
registry package metadata
```

The npm plugin should parse:

```text
dependencies
devDependencies
optionalDependencies
peerDependencies
peerDependenciesMeta
bundleDependencies
workspaces
engines
os
cpu
overrides
file/link/git/workspace dependencies
```

Occurrence semantics:

```text
lockfile package path or node_modules path
```

Recommended first-version behavior:

- For real projects, prefer `package-lock.json` or `npm-shrinkwrap.json`.
- For registry expansion, parse registry metadata and semver ranges.
- For research mode, implement latest-satisfying and lowest-satisfying policies.
- Mark peer dependency handling fidelity explicitly if not fully implemented.

Modeling: `peerDependencies` are not ordinary edges — they are constraints the *consumer's* tree must satisfy and influence placement; model them with a distinct `DependencyKind::Peer` and an edge that is flagged `PEER_UNMET` when no provider exists (record the npm6-conflict vs npm7-auto-install policy on the run). `overrides` are applied in a phase that rewrites the selected version for a transitive target and flags the edge `OVERRIDDEN` with provenance to the overrides entry. A failed `optionalDependency` is an edge flagged `OPTIONAL_OMITTED`, distinct from a hard conflict. Hoisting/dedup is the occurrence-identity function (one node per dedup path), so the same version at multiple paths is multiple occurrences sharing one `version_id`.

Main risks:

```text
peer dependency placement
hoisting
workspace links
overrides
optional dependency failure
same package version appearing at multiple paths
```

## Cargo Plugin

Primary objects:

```text
Cargo.toml
Cargo.lock
Cargo sparse index entries
cargo metadata output
```

The Cargo plugin should parse:

```text
dependencies
build-dependencies
dev-dependencies
features
optional dependencies
target-specific dependencies
renamed dependencies
default-features
workspace dependencies
patch / replace / path / git sources
rust-version
yanked status
checksums
```

Occurrence semantics:

```text
source + name + version, optionally extended by feature context for analysis modes
```

Recommended first-version behavior:

- Use `cargo metadata` when available for native resolved graphs.
- Use sparse index entries for registry metadata.
- Preserve feature and target conditions even when producing a filtered resolved graph.

Modeling: feature unification is whole-graph — the enabled feature set of a crate is the union across all dependents — and enabling a feature can turn an optional dependency into a real edge (`serde/derive` → `serde_derive`). This is exactly why the synthetic resolver is a fixpoint: activate features → add the now-active conditional requirements → re-resolve. The unified set is stored in `GraphNode.enabled_features_ref`; the occurrence-identity function yields **one node per `(source, name, version)` after unification** (feature context is recorded, it does not split nodes). Workspace inheritance (`dep.workspace = true`, `[workspace.dependencies]`) is a parse-time stage that produces effective member manifests before `Requirement`s are emitted, with provenance to both the member and the workspace root.

Main risks:

```text
feature unification
target-specific dependencies
workspace inheritance
patch and replacement sources
yanked versions
```

## PyPI / Python Plugin

Primary objects:

```text
pyproject.toml
requirements.txt
constraints.txt
poetry.lock
uv.lock
pip inspect JSON
Simple API metadata
wheel METADATA
sdist metadata when necessary
```

The Python plugin should parse:

```text
Requires-Dist
Requires-Python
Provides-Extra
environment markers
extras
direct URL requirements
wheel tags
yanked files
```

Occurrence semantics:

```text
distribution + version + environment + extras context
```

Recommended first-version behavior:

- Prefer lockfiles or `pip inspect` style output for real environments.
- Prefer package metadata without executing setup scripts.
- Treat sdist dynamic metadata as lower fidelity.
- Include Python version, implementation, platform, and extras in the candidate context.

Modeling: extras are **activation** state, not environment state — `requests[socks]` activates `PySocks`, which is otherwise absent — so `extra ==` markers route to activation evaluation (`condition_kind = Activation`) and feed the same fixpoint as Cargo features; the activated extras are stored in `GraphNode.enabled_features_ref`. Candidate filtering consults **per-distribution-file** facts, not just the requirement range: a version is excluded with a typed reason when `Requires-Python` mismatches the interpreter (`RequiresPythonMismatch`), no wheel tag matches the platform (`NoCompatibleWheel`), the file is yanked (`Yanked`), or only an sdist with unread dynamic metadata exists (`DynamicMetadataUnknown`, which also sets the `DYNAMIC_METADATA_SKIPPED` fidelity caveat).

Main risks:

```text
dynamic metadata
multiple distribution files per release
environment markers
extras changing dependency activation
Requires-Python candidate filtering
```

## Maven Plugin

Primary objects:

```text
pom.xml
parent POMs
BOM POMs
maven-metadata.xml
Maven dependency tree output
```

The Maven plugin should parse:

```text
groupId
artifactId
version
packaging
parent
properties
dependencyManagement
dependencies
profiles
scope
optional
exclusions
classifier
type
BOM imports
SNAPSHOT metadata
```

Occurrence semantics:

```text
mediated artifact coordinate plus dependency path context
```

Recommended first-version behavior:

- Use Maven dependency tree output for native resolved graphs when possible.
- Implement a POM model builder incrementally.
- Do not map raw XML dependencies directly to requirements before property interpolation and dependency management are applied.

Modeling: Maven resolution is an explicit effective-POM pipeline — `inherit (parent) → interpolate properties → apply dependencyManagement → import BOMs → activate profiles` — producing fully-determined requirements (`dst_package_id` and `managed_version_ref` filled in), followed by a `nearest-definition mediation + scope-propagation` pass that produces edges. `dependencyManagement`/BOM entries are a managed-version layer the requirement constraint defers to; profiles altering the effective POM that are not expanded set the `NO_PROFILE_EXPANSION` caveat, and mediation not applied sets `NO_MEDIATION`. Scope propagation (a compile dep of a compile dep stays compile; test does not propagate) is plugin logic expressed as a scope matrix.

Main risks:

```text
parent inheritance
property interpolation
dependencyManagement
BOM imports
scope propagation
profiles
nearest-definition mediation
SNAPSHOT resolution
```

## Go Plugin

Primary objects:

```text
go.mod
go.sum
go.work
Go module proxy metadata
go list output
go mod graph output
```

The Go plugin should parse:

```text
module
require
replace
exclude
retract
pseudo-versions
major-version suffixes
workspace files
```

Occurrence semantics:

```text
module path + selected version
```

Recommended first-version behavior:

- Use `go list -m -json all` or `go mod graph` for native resolved graphs.
- Implement Minimal Version Selection for research mode.
- Keep module dependency graph separate from package import graph.

Modeling: a Go run declares which graph it produced via a node-kind discriminator (module-level vs package-import-level) so the two are not conflated in one snapshot. MVS is the ecosystem selection operator: requirements are version *floors* and the selected version is the **max of all minimums** across the whole graph (not a range intersection), with `exclude`/`replace`/`retract` applied as global candidate-space mutations *before* MVS (recorded with provenance), not as per-requirement filters. The Go `version_scheme` parses and orders pseudo-versions (`v0.0.0-<ts>-<hash>`) relative to tagged versions and maps them to a commit; the `/vN` major suffix is part of module identity (Identity model).

Main risks:

```text
replace and exclude scope
retract semantics
module path major version suffix
private modules
proxy fallback behavior
```

## NuGet Plugin as a Later Addition

NuGet should be considered after npm, Cargo, PyPI, Maven, and Go are stable.

Primary objects:

```text
*.csproj
packages.lock.json
nuspec
NuGet registration blobs
```

Main risks:

```text
target framework groups
PackageReference restore semantics
central package management
transitive framework-specific dependencies
```

# Fetch Scheduler

Recursive dependency expansion should use a fetch scheduler, not direct recursive HTTP calls.

```rust
pub struct FetchScheduler {
    queue: BinaryHeap<FetchTask>,
    inflight: HashMap<FetchKey, JoinHandle<FetchResult>>,
    seen: HashSet<FetchKey>,
    budget: FetchBudget,
}
```

A fetch key includes:

```text
ecosystem
source id
package name
version or range
metadata kind
environment hash
snapshot id
```

Budgets prevent accidental registry explosions:

```rust
pub struct FetchBudget {
    pub max_packages: usize,
    pub max_versions_per_package: usize,
    pub max_depth: usize,
    pub max_artifact_bytes: u64,
    pub max_concurrent_requests: usize,
    pub timeout_ms: u64,
}
```

The scheduler should support:

- offline mode;
- stale cache mode;
- online refresh mode;
- deterministic snapshot mode;
- per-registry rate limits;
- per-run fetch budgets;
- cancellation.

# Resolution and Graph Construction

There are two construction paths. The **native/lockfile-exact path** reads an already-resolved graph from a lockfile or native tool and maps it directly to occurrences and edges (no version choosing). The **synthetic path** runs Prism's research resolver. Per-requirement local selection cannot model any real ecosystem (npm hoisting/dedup, Cargo feature unification, Maven nearest-wins mediation, Go MVS, pip backtracking are all *global*), so the synthetic resolver is a **two-phase fixpoint**, not a greedy BFS:

```text
Root input
  -> normalize package names
  -> select root versions or requirements
  -> create ResolverRun (records GraphFidelity)
  -> PHASE 1  collect the requirement closure; per package family accumulate the set of all
              requirements and (for Cargo/PyPI) the activated feature/extra set
  -> PHASE 2  run the ecosystem-specific selection operator (npm intersect+hoist /
              Cargo unify / Maven mediate / Go max-of-minimums / pip backtrack) to choose one
              version (and feature set) per family; activating features/extras can add new
              conditional requirements, so repeat Phase 1+2 to a fixpoint
  -> materialize occurrence nodes (ecosystem occurrence-identity function) and resolved edges,
     flagging dropped optional/unmet-peer/overridden edges rather than omitting them
  -> finalize CSR (sort edges by source -> forward CSR; by target -> reverse CSR;
     assign GraphEdgeId, edge ordinal, and node depth)
  -> write CSR graph snapshot
  -> publish manifest
```

Gating is split into two kinds. **Environment conditions** (`sys_platform`, target `cfg()`, `Requires-Python`, Maven profile activation) are a pure function of `Environment`. **Activation conditions** (PEP 508 `extra ==`, Cargo optional-dependency feature gates) are *resolution* facts evaluated against the running `ActivationContext` (the unified feature/extra closure), not the environment. The builder passes both.

Construction uses transient builder types, distinct from the persisted `GraphNode`/`GraphEdge`: a `BuildNode` carries an `expanded` flag and the `OccurrenceContext`; `node_id`, `edge_id`, `ordinal`, and `depth` are assigned during CSR finalization. Materialization pseudocode (Phase-2 output → graph), showing the cycle guard, depth, and CSR finalization the earlier draft omitted:

```rust
fn materialize_graph(run_id: RunId, sel: &Selection, env: &Environment) -> RunId {
    let mut nodes: Vec<BuildNode> = Vec::new();     // transient: has `expanded`, occurrence ctx
    let mut edges: Vec<BuildEdge> = Vec::new();
    let mut intern: HashMap<OccurrenceKey, GraphNodeId> = HashMap::new();
    let mut queue: VecDeque<(GraphNodeId, Vec<PackageId>)> = VecDeque::new(); // node + root path

    for root in &sel.roots {
        let n = intern_node(&mut nodes, &mut intern, root.version_id, OccurrenceContext::root(), 0);
        queue.push_back((n, vec![pkg_of(root.version_id)]));
    }

    while let Some((src, path)) = queue.pop_front() {
        if nodes[src as usize].depth >= env_budget.max_depth { continue; }      // depth bound
        for req in sel.out_requirements(src, env) {       // already gated by env + activation
            let (selected_ver, outcome) = sel.resolved_edge(src, &req); // Phase-2 result + flag
            let dst_pkg = pkg_of(selected_ver);

            // cycle guard: if this package already appears on the root->src path, close a
            // BACK_EDGE to the existing occurrence instead of minting a new node forever.
            if let Some(&existing) = path.iter().rev().find_map(|_| intern_for(&intern, dst_pkg)) {
                edges.push(BuildEdge::back_edge(src, existing, &req, outcome));
                continue;
            }

            let ctx = sel.child_context(src, &req, selected_ver); // ecosystem occurrence identity
            let dst = intern_node(&mut nodes, &mut intern, selected_ver, ctx,
                                  nodes[src as usize].depth + 1);  // depth = min over BFS paths
            edges.push(BuildEdge::resolved(src, dst, &req, outcome));
            if !nodes[dst as usize].expanded {
                let mut p = path.clone(); p.push(dst_pkg);
                queue.push_back((dst, p));
            }
        }
        nodes[src as usize].expanded = true;
    }

    // CSR finalization: stable-sort edges by `from` -> out_offsets/out_targets/out_edge_ids,
    // then a copy by `to` -> in_offsets/in_sources/in_edge_ids; assign edge_id and ordinal here.
    graph_store.write_csr(run_id, finalize_nodes(nodes), finalize_csr(edges));
    run_id
}
```

The cycle guard makes termination depend on the *package family* reappearing on the current path (which always recurs on a cycle), independent of how path/feature-sensitive the occurrence key is, and `FetchBudget.max_depth` bounds expansion regardless. Budget truncation sets a `truncated` flag on the run and downgrades its fidelity (see below). The resolver policy and ecosystem plugin jointly decide occurrence identity and the selection operator.

# Graph Algorithms

Prism's graph algorithms run over immutable CSR snapshots.

## Forward Reachability

```rust
fn forward_reachable(
    graph: &GraphSnapshot,
    root: GraphNodeId,
    max_depth: u32,
    filter: EdgeFilter,
) -> Vec<GraphNodeId> {
    let mut seen = BitSet::new(graph.node_count());
    let mut out = Vec::new();
    let mut q = VecDeque::new();

    seen.insert(root);
    q.push_back((root, 0));

    while let Some((u, depth)) = q.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for edge in graph.out_edges(u) {
            if !filter.matches(edge) {
                continue;
            }

            let v = edge.to;
            if seen.insert(v) {
                out.push(v);
                q.push_back((v, depth + 1));
            }
        }
    }

    out
}
```

Complexity:

```text
O(reached nodes + scanned edges)
```

By convention `forward_reachable` **excludes the root** from the result (it returns strictly-reachable descendants); the root is inserted into `seen` but never pushed to `out`. `TraversalOptions` exposes an `include_root` flag for callers that want it.

## Reverse Impact Analysis

Reverse traversal mirrors forward traversal over the reverse CSR arrays (`in_offsets`/`in_sources`), with the same `max_depth` bound and the same root-excluded convention:

```rust
fn reverse_reachable(g: &GraphSnapshot, node: GraphNodeId, opts: TraversalOptions) -> Vec<GraphNodeId> {
    // identical to forward_reachable but walks g.in_edges(u); yields ancestors of `node`.
    // "affected roots" = the returned set intersected with the run's root-node set.
}
```

This supports queries such as:

```text
Which roots or packages are affected by this vulnerable version?  (ancestors ∩ roots)
Which dependents would be impacted by upgrading or removing this node?  (all ancestors)
```

## Strongly Connected Components

Dependency graphs should not be assumed to be DAGs. Prism should compute SCCs first and then topologically sort the condensation graph.

```text
Graph
  -> Tarjan or Kosaraju SCC
  -> Condensation DAG
  -> Kahn topological sort
```

Output:

```text
SCC-level topological order
```

Cycles inside an SCC should be preserved and shown explicitly. If Tarjan is used, it emits SCCs in reverse topological order of the condensation as a byproduct, so a separate Kahn pass is unnecessary when only the order is needed; build the explicit condensation DAG only when the inter-SCC graph is itself a required output (e.g. for visualization).

**Multigraph semantics.** Construction can emit multiple edges between the same `(from, to)` occurrence pair (e.g. a runtime and a peer requirement to the same selected version). The CSR is a multigraph: neighbor slices may contain duplicate targets. Set-based algorithms (triangle enumeration, sorted-adjacency intersection) MUST dedup adjacency first; `forward_reachable`'s `seen` set already collapses duplicates. Duplicate-version clustering counts distinct `version_id`s, not edges, so it is unaffected.

## Duplicate Version Clusters

Duplicate-version detection groups graph nodes by package family and counts distinct versions.

```text
Package family -> set of version IDs -> occurrences and paths
```

Metrics:

```text
version count
occurrence count
minimum depth
maximum depth
paths from roots
risk weight
upgrade spread
```

## Conflict Pressure

Conflict pressure is computed over candidate sets. Two requirements conflict when, **targeting the same package family**, their candidate-set intersection is empty. Cross-package pairs are not comparable (ordinal `[2,5)` of package X is unrelated to `[2,5)` of package Y), and the intersection must honor each set's `exclusions`:

```rust
fn compatible(a: &CandidateSet, b: &CandidateSet) -> Option<bool> {
    if a.package_id != b.package_id {
        return None; // not comparable: only same-family requirements can conflict
    }
    // intersection operates over (ranges minus exclusions) for BOTH operands
    Some(!a.intersect(b).is_empty())
}
```

Range intersection that honors exclusions is `O(|ranges_A| + |ranges_B| + |excl_A| + |excl_B|)`. This range-intersection conflict model is correct for ecosystems where multiple versions may coexist and selection is range satisfaction (npm, pip). It is **not** the resolution primitive for Go (MVS: requirements are version *floors*; selection is the max of all minimums, never an upper-bounded intersection) or Maven (nearest-definition mediation over the dependency tree). The selection operator is therefore a plugin responsibility (see Resolution and Graph Construction); intersection-based conflict metrics are reported only for the ecosystems where they are meaningful.

Possible metrics:

```text
candidate set size
candidate set entropy
intersection size
constraint tightness
number of mutually incompatible requirement pairs
minimum upgrade distance to restore compatibility
```

## Path Search

Path enumeration must be bounded.

```text
max_depth required
max_paths required
high fan-out truncation required
```

Prism should never enumerate all paths by default because the number of paths can grow exponentially.

# TDA Architecture

Prism's TDA layer is a derived analysis layer. It should not pollute the canonical dependency model.

## Supported Filtrations in Version 0.1

The first version supports **two** filtrations, both derived from data Prism already holds:

1. Depth filtration.
2. Version age / staleness filtration.

**Risk filtration is deferred to v0.2** because a normalized risk scalar is an unresolved design question (see Design Decisions → Risk scoring); shipping it now would mean filtering on an undefined value. Additional filtrations can be added later:

```text
risk filtration (v0.2, once risk scoring is defined)
centrality filtration
license boundary filtration
candidate-space tightness filtration
upgrade distance filtration
release-time filtration
maintenance-health filtration
```

### Filtration functions must be monotone

A filtration is a function `f` on cells such that `f(face) ≤ f(coface)` for every incidence, so each sublevel set `{cells : f ≤ t}` is itself a subcomplex and the sublevel sets are nested. Vertex (0-cell) values come from the node attribute; **edge and higher-cell values use the lower-star / max rule** `f(cell) = max over its vertices f(v)`, which guarantees monotonicity. The `.ptda` writer asserts `cell_birth_value[cell] ≥ cell_birth_value[face]` for every face and rejects violations.

Each filtration declares a **direction** and an exact scalar:

```text
depth     : sublevel (increasing). f(node) = node.depth. Structure near the roots appears first.
staleness : sublevel (increasing). f(node) = (run.reference_unix - version.released_at_unix),
            in days; the reference timestamp is pinned in the ResolverRun so runs are reproducible.
```

Reduction requires a **total order** on cells consistent with `f`; ties (e.g. all depth-2 nodes share a value) are broken by `(birth_value, dim, cell_id)` with faces strictly before cofaces. Invariants the writer enforces: `death_cell` is NULL exactly when `death_value` is INF (essential class); a dimension-`k` pair has a dim-`k` birth cell and a dim-`k+1` death cell; `birth_value ≤ death_value`.

## Graph to Complex Construction

The dependency graph is directed, but ordinary persistent homology is defined on undirected complexes. **v0.1 computes ordinary PH of the underlying undirected graph as a 1-dimensional complex**, so `H0` = connected components and `H1` = first Betti number (cycle rank) are rigorously correct. Edge direction is **dropped at complex-construction time** and recovered only via generator back-mapping (the snapshot still stores direction, so highlighted cycles render with arrows).

```text
0-cells: graph nodes
1-cells: graph edges (direction dropped for homology; preserved in the snapshot for display)
```

**Backend decision (Open Question 5):** v0.1 ships a small **Rust-native** reduction — union-find for `H0` and standard matrix reduction for `H1` over the 1-complex — plus an export path to external TDA tools for users who want `H2+`. No external library dependency is required for the v0.1 pipeline.

Higher-dimensional cells are a **v0.2 experiment** using the *directed flag complex* (Lütgehetmann et al.): a directed `k`-clique `u0→u1→…→uk` with all forward edges present is a `(k-1)`-cell, with its boundary and `H_k` meaning defined by that construction — *not* the same object as the undirected complex above. Experiments must enforce limits:

```text
max_dimension <= 2 or 3
max_cells
max_degree
max_time
```

Directed triangle enumeration (v0.2) imposes a canonical orientation so each simplex is emitted once and honors the budgets:

```rust
fn enumerate_directed_triangles(g: &GraphSnapshot, lim: &CellLimits) -> Vec<[GraphNodeId; 3]> {
    let mut triangles = Vec::new();
    for u in g.nodes() {
        let out_u = dedup(g.out_neighbors_sorted(u));     // multigraph: dedup adjacency first
        if out_u.len() > lim.max_degree { continue; }
        for &v in &out_u {
            if u == v { continue; }                        // skip self-loops
            let out_v = dedup(g.out_neighbors_sorted(v));
            for &w in intersect_sorted(&out_u, &out_v) {
                // canonical orientation u<v<w emits each unordered triple once, even with 2-cycles
                if u < v && v < w {
                    triangles.push([u, v, w]);
                    if triangles.len() >= lim.max_cells || lim.timed_out() { return triangles; }
                }
            }
        }
    }
    triangles
}
```

## Persistence Mapping

The most important user-facing feature is not the persistence diagram itself. It is the ability to click a persistence feature and recover the dependency substructure that supports it.

```text
Persistence pair
  -> generator support
  -> graph nodes and edges
  -> package versions
  -> package families
  -> requirements and raw provenance
```

This allows Prism to explain a long-lived H1 feature as a concrete set of packages, versions, constraints, and paths rather than as an abstract mathematical artifact.

Representative cycles are **not unique** — a homology class has many representatives, and standard reduction yields one that depends on cell ordering and tie-breaks. Prism therefore defines generator support precisely as *"the representative chain produced by the chosen reduction under the documented total cell order"* (the `(birth_value, dim, cell_id)` order above), and labels it one valid representative, not the unique one; v0.1 does not attempt minimal-representative stabilization. The `contribution` field is the cell's Z/2 membership coefficient in that representative (presence/absence). Generator support can reference `GraphEdgeId` as well as `GraphNodeId`, so a highlighted cycle maps to concrete edges (rendered with their original direction from the snapshot).

Each persistence pair has a **stable, globally-unique `PairId`** (the `PAIR_BY_ID` index maps it to `(FiltrationId, in-file ordinal)`), so `getGeneratorSupport(pairId)`, the CLI `--pair`, and deep links resolve unambiguously. A `.ptda` file is bound to exactly one immutable run snapshot via its `FiltrationHeader.run_id`; it is invalidated if that snapshot is removed, and GC/compaction treat dependent `.ptda` files accordingly.

## Interpretation Rules

Prism should use conservative language:

```text
H0: connected components of the undirected 1-complex; isolated dependency regions, ecosystem islands
H1: the count of independent 1-cycles (first Betti number) of the chosen complex at a given
    filtration value; persistence tracks how this count changes across the filtration, not a single
    number. In the undirected v0.1 complex these are genuine cycles; if v0.2 2-cells are present the
    cycle interpretation is reduced by filled triangles. "Alternative path" is a product gloss, not a
    homological guarantee.
H2+: experimental (directed flag complex), requires domain-specific interpretation
```

These readings hold for the specific complex and theory chosen above (undirected ordinary PH in v0.1); they are not claimed for an arbitrary directed construction.

Prism should not claim:

```text
one topological hole equals one security vulnerability
one cycle equals one bug
one persistent feature directly implies causality
```

# Rust API Design

The Rust API exposes domain operations rather than a general query language.

Every method returns `Result<T>` over one concrete public error type, so callers (and the napi/Tauri/UI mapping) can distinguish failure classes; `find_package` returns `Result<Option<..>>` (absence is not an error) while `get_*` return `Result<T>` (a missing record by a known ID is `NotFound`):

```rust
pub type Result<T> = std::result::Result<T, PrismError>;

#[non_exhaustive]
pub enum PrismError {
    NotFound,
    Corrupt { file: String, detail: String },
    ChecksumMismatch { file: String },
    UnsupportedFormatVersion { found: u32, max: u32 },
    Budget(BudgetError),
    SecurityLimitExceeded(SecurityLimit),
    Plugin(Box<dyn std::error::Error + Send + Sync>),
    Io(std::io::Error),
}
```

Each variant maps to a stable CLI exit code and a TS error code (see TypeScript Integration). All capability sub-traits below (`NameNormalizer`, `VersionScheme`, `RequirementParser`, `ConditionEvaluator`, …) are an **object-safety invariant**: no generic or `Self`-returning or `async` methods, so the `&dyn` returns in `EcosystemPlugin` compile; a `compile_fail`/lint test enforces it, and the returned `&dyn` borrows are valid for the plugin's lifetime.

```rust
pub trait PrismStore {
    fn find_package(
        &self,
        ecosystem: &str,
        namespace: Option<&str>,
        name: &str,
    ) -> Result<Option<PackageId>>;

    fn get_package(&self, id: PackageId) -> Result<Package>;

    fn versions_by_package(
        &self,
        package_id: PackageId,
    ) -> Result<Vec<VersionId>>;

    fn get_version(&self, id: VersionId) -> Result<PackageVersion>;

    fn requirements_by_source_version(
        &self,
        version_id: VersionId,
    ) -> Result<Vec<RequirementId>>;

    fn candidates(
        &self,
        requirement_id: RequirementId,
        env: &Environment,
        snapshot: SnapshotId,
    ) -> Result<CandidateSet>;

    fn open_graph(&self, run_id: RunId) -> Result<GraphSnapshot>;
}
```

Graph interface:

```rust
pub trait DependencyGraph {
    fn node_count(&self) -> usize;
    fn edge_count(&self) -> usize;

    fn node_version(&self, node: GraphNodeId) -> VersionId;

    fn out_edges(&self, node: GraphNodeId) -> EdgeIter<'_>;
    fn in_edges(&self, node: GraphNodeId) -> EdgeIter<'_>;

    fn forward_reachable(&self, root: GraphNodeId, options: TraversalOptions) -> Vec<GraphNodeId>;
    fn reverse_reachable(&self, root: GraphNodeId, options: TraversalOptions) -> Vec<GraphNodeId>;

    // analysis operations backing the UI views and CLI (previously missing from the API)
    fn graph_stats(&self) -> GraphStats;                       // size, depth, #SCCs, #duplicates
    fn scc_of(&self, node: GraphNodeId) -> SccView;            // get_scc
    fn duplicate_cluster(&self, package: PackageId) -> DuplicateClusterView;
    fn conflict_pressure(&self, package: PackageId, env: &Environment, snap: SnapshotId)
        -> ConflictReport;                                     // candidate intersections/conflicts
    fn provenance(&self, obj: ObjectRef) -> ProvenanceView;    // raw manifest/lockfile/registry evidence
}

// Helper types backing bounded traversal (the safety-critical knobs):
pub struct TraversalOptions {
    pub max_depth: u32,
    pub max_nodes: u32,           // bound; sets `truncated` when hit
    pub kinds: EdgeKindMask,      // edge-kind allowlist
    pub include_root: bool,       // default false
}
pub struct EdgeFilter { pub kinds: EdgeKindMask, pub include_dropped: bool }
// EdgeIter yields lightweight edge views (from, to, edge_id, kind, flags, requirement_id).
```

`open_graph` returns a `GraphSnapshot` that **pins its epoch's file set** (it holds a read-epoch guard, so compaction/GC will not delete files under a live snapshot) and is `Send + Sync`; v0.1 backs it with owned arrays (see CSR Graph Snapshots).

# TypeScript Integration

TypeScript should not read Prism storage files directly. It should call stable Rust APIs through Tauri commands, napi-rs bindings, or a local HTTP/IPC service.

**ID model:** all IDs cross to TS as **opaque strings** (Rust IDs are u32/u64; a u64 above 2^53 would lose precision as a JS number). The UI must not parse, compare, or increment them. `PackageId`/`VersionId` are stable across compaction; `GraphNodeId`/`GraphEdgeId` are stable only within a run snapshot and are therefore always carried qualified as `"<runId>:<nodeId>"`. A stale deep link resolves-or-404s. View types are **generated from the Rust types** (ts-rs / typeshare) so the wire shape and the UI example below cannot drift:

```ts
// generated; every field's presence matches the Rust source of truth
type PackageSummary = { id: string; ecosystem: string; namespace?: string; name: string; purl: string; versionCount: number }
type VersionSummary = { id: string; version: string; purl: string; releasedAt?: number; yanked: boolean }
type GraphNodeView  = { id: string; purl: string; package: string; version: string; depth: number; features?: string[] }
type GraphEdgeView  = { source: string; target: string; requirement?: string; kind: string; flags: string[] }
type PersistencePairView = { pairId: string; dim: number; birth: number; death: number | null }
type GeneratorSupportView = {
  nodes: GraphNodeView[]; edges: GraphEdgeView[]
  requirements: { id: string; raw: string }[]; packages: PackageSummary[]
  contributions: { objectType: string; objectId: string; weight: number }[]
}
type GraphNeighborhood = { nodes: GraphNodeView[]; edges: GraphEdgeView[]; truncated: boolean; nextCursor?: string }
type ExpandOptions = { depth: number; limit: number; kinds?: string[]; cursor?: string }

interface PrismApi {
  findPackage(ecosystem: string, name: string, namespace?: string): Promise<PackageSummary | null>
  getVersions(packageId: string): Promise<VersionSummary[]>
  expandForward(runId: string, nodeId: string, opts: ExpandOptions): Promise<GraphNeighborhood>
  expandReverse(runId: string, nodeId: string, opts: ExpandOptions): Promise<GraphNeighborhood>
  getDuplicateCluster(runId: string, packageId: string): Promise<GraphNeighborhood>
  getScc(runId: string, nodeId: string): Promise<GraphNeighborhood>
  getConflictPressure(packageId: string, env: string, snapshot: string): Promise<ConflictReportView>
  getGraphStats(runId: string): Promise<GraphStatsView>
  getProvenance(runId: string, objectRef: string): Promise<ProvenanceView>
  getTdaPairs(runId: string, filtrationId: string): Promise<PersistencePairView[]>
  getGeneratorSupport(pairId: string): Promise<GeneratorSupportView>
}
```

`expandForward`/`expandReverse` take an `ExpandOptions` (matching the UI `get_neighborhood(node, depth, limit)` examples): `limit` bounds returned nodes, and `cursor` resumes. The **cursor** is opaque base64 encoding `runId + snapshot epoch + serialized BFS frontier + visited-bitset offset`; each node appears at most once across all pages of one cursor chain; a cursor whose snapshot epoch no longer exists is rejected with a typed error. `getGeneratorSupport` returns a dedicated `GeneratorSupportView` (not `GraphNeighborhood`) because generator support spans requirements, packages, and contribution weights — not just nodes/edges. Each TS error code maps 1:1 to a `PrismError` variant.

**Transport (v0.1):** Tauri is the normative integration layer for the desktop app (one serialization format, structured IPC). napi-rs (npm library) and a local HTTP/IPC service are future options; any HTTP mode requires a server-side cursor/session store and the same opaque-string ID rule.

For a desktop application, Tauri is the recommended integration layer:

```text
Rust backend:
  storage, parsing, fetching, graph algorithms, TDA

TypeScript frontend:
  graph canvas, search, filters, topology panels, export controls
```

For an npm-distributed library, napi-rs can expose Rust functions to Node.js.

# UI Architecture

The UI should be local-subgraph oriented. It should not attempt to load and layout the entire graph by default.

Primary views:

```text
Project overview
  roots, ecosystem, graph size, fidelity, environment, snapshot

Package search
  package family, versions, requirements, candidate ranges

Resolved graph explorer
  local neighborhood, dependency paths, duplicate versions, SCCs

Reverse impact explorer
  vulnerable/deprecated/outdated package -> affected roots and paths

Constraint space explorer
  requirements, candidate intersections, conflicts, upgrade options

TDA explorer
  filtration configuration, barcode, persistence diagram, generator support

Provenance view
  raw manifest/lockfile/registry evidence for a selected fact
```

Viewport-oriented API examples:

```text
get_neighborhood(node, depth, limit)
get_reverse_impact(node, depth, limit)
get_duplicate_version_cluster(package_id)
get_scc(run_id, node)
get_tda_support(pair_id)
```

Example response:

```json
{
  "nodes": [
    {
      "id": "run_7:42",
      "purl": "pkg:npm/react@18.2.0",
      "package": "react",
      "version": "18.2.0",
      "depth": 2,
      "features": []
    }
  ],
  "edges": [
    {
      "source": "run_7:42",
      "target": "run_7:99",
      "requirement": "^1.0.0",
      "kind": "runtime",
      "flags": ["RESOLVED"]
    }
  ],
  "truncated": true,
  "nextCursor": "opaque-cursor"
}
```

# CLI Design

Suggested CLI commands:

```text
prism init
  initialize a .prism workspace

prism ingest <path>
  ingest manifest, lockfile, SBOM, or project directory

prism fetch <ecosystem> <package>
  fetch package metadata into the local store

prism resolve <input> --ecosystem <id> --policy <policy>
  create a resolver run and graph snapshot

prism graph stats --run <id>
  summarize graph size, depth, SCCs, duplicate versions

prism graph neighborhood --run <id> --node <id> --depth <n>
  output a local dependency neighborhood

prism graph impact --run <id> --package <purl>
  reverse dependency impact analysis

prism candidates <requirement-id>
  show candidate version ranges and exclusions

prism tda run --run <id> --filtration depth --max-dim 1
  build filtration and persistence output

prism tda support --pair <id>
  output graph substructure supporting a persistence pair

prism export graphml --run <id>
prism export json --run <id>
prism export parquet --run <id>
```

Exports are durable artifacts that leave Prism, so they use **PURL (and package/version strings) as the stable primary key**; any internal integer IDs included are documented as snapshot-local and not portable. Each format has a defined schema: GraphML lists its typed `<key>` definitions and flattens chosen `attr_ref` fields; Parquet defines explicit node and edge table schemas; JSON mirrors the generated view types. The run's `GraphFidelity` is included in every export header.

## Versioning and Migration

Three independently-numbered surfaces evolve: (a) each on-disk format (`format_version` per file), (b) the Rust `prism-api` crate, (c) the TS `PrismApi`. Compatibility rule: a newer reader reads an older format read-only and migrates it on the next compaction; a format below the supported floor is rejected with `UnsupportedFormatVersion`. Additive format sections are skippable (Engineering Rule 9); a new format field surfaces in the TS view types only through an explicit, additive (optional) view-type change, so on-disk evolution never silently breaks the UI contract.

# Security Model

Prism must treat external package metadata and artifacts as untrusted input.

Default safety rules:

1. Do not execute package install scripts.
2. Do not execute build scripts.
3. Do not execute `setup.py` or dynamic package code.
4. Do not run a Maven lifecycle by default.
5. Do not follow unbounded redirects.
6. Do not download unbounded artifacts.
7. Do not extract archives without size, count, ratio, link, and path-traversal checks.
8. Do not log authentication tokens.
9. Do not store secrets in raw blobs (enforced by redaction-before-hash; see Blob Store).
10. Do not trust registry metadata without provenance and content hashing.

Every rule has a defined **failure mode**: when a limit is exceeded, the unit of work fails atomically with a typed `SecurityLimitExceeded` error — never a silent truncation or partial extraction. Each is covered by the Security / Abuse test category (Testing Strategy).

### SSRF and fetch hardening

`artifact_url` and Python direct-URL targets come from untrusted registry metadata, so the fetcher applies an SSRF guard as a required control: allowlist schemes (`https`, plus `http` only for explicitly-configured mirrors); reject URL userinfo from registry metadata; resolve DNS and reject RFC1918 / loopback / link-local / ULA / cloud-metadata addresses (e.g. `169.254.169.254`); **re-validate the resolved address on every redirect hop and pin the socket to the validated IP** to defeat DNS rebinding; cap redirect hops and forbid cross-scheme/downgrade redirects.

### Archive inspection

```text
max archive size
max decompressed size
max decompression ratio (streaming)        # zip/tar bomb defense
max nesting depth
max file count
metadata path allowlist
reject absolute paths and entries normalizing outside the root
refuse to create or follow symlinks/hardlinks (extract flat, never honor link entries)
reject non-regular-file entries (devices, FIFOs)
checksum verification when available
```

A detected path-traversal/link/limit violation rejects the **whole archive** (skipping a single bad entry can be used to mask intent), atomically.

### Integrity and trust model

Checksums are classified by trust: **user-pinned lockfile hash > checksum DB / transparency log > registry-provided metadata hash > none**. The source is recorded on each `ChecksumFact`. A registry-provided hash proves transport integrity, not authenticity. Verification: hard-fail on mismatch; configurable fail/warn on absence; never store unverified-as-verified. Reproducibility/offline integrity is grounded in the content-addressed `RegistrySnapshot` (Canonical Domain Model).

### Credential handling

Rule 8 is enforced by a `SafeUrl` newtype: it is the only URL form permitted in `RawDocument.source_key`, `Provenance`, `FetchKey`, logs, error types, and UI responses, and it strips credentials at construction. `Display`/`Debug`/`serde` impls on fetch-path types may only emit `SafeUrl`. `AuthRef` is an **indirect handle** resolved at fetch time from an OS keychain / environment / external secrets file that lives **outside `.prism/`**; secrets never enter any store file, blob, provenance record, or manifest. A test injects `https://user:token@host/...` and asserts no credential pattern appears anywhere in the workspace after a private-registry fetch. (This resolves Open Questions 8 and 9: auth **secrets** live outside the workspace entirely, while user-editable, non-secret project config lives in a separate `.prism/config.toml`, never in the machine-managed `MANIFEST`.)

### Native resolver sandboxing (Open Question 3)

Native adapters (`cargo metadata`, `go list`, `mvn dependency:tree`, `pip inspect`) can execute project- and dependency-controlled code, which the no-code-execution rules otherwise forbid. They are therefore **off by default and require explicit per-run opt-in**. When enabled they run in a restricted subprocess: no network beyond configured registries, restricted filesystem, dropped environment (including registry tokens), and resource limits. The set of subcommands that can execute code is documented per plugin; a sandboxed native run is labeled `NativeToolResolved`.

### Parser hardening

Parsers consume hostile bytes, so determinism is not enough. XML parsing (Maven POMs) runs with DTDs and external entities disabled (no XXE, no billion-laughs entity expansion); property-interpolation recursion depth and output size are bounded. Every parser has a per-parse budget (max input bytes, nodes, depth, time) and fails fast on cyclic metadata. Billion-laughs/XXE and deeply-nested-JSON fixtures are part of the abuse-test suite.

Prism's goal is topology analysis, not package installation.

# Performance Strategy

## Hot Paths

The hot paths are:

```text
package name lookup
package version listing
requirements by source version
candidate range computation
CSR graph traversal
reverse impact traversal
TDA filtration construction
```

## Optimization Choices

Prism uses:

- string interning for repeated names, versions, scopes, and URLs;
- binary sort keys for version comparison;
- range-based candidate sets;
- immutable graph snapshots for memory-mappable traversal;
- forward and reverse CSR arrays;
- bounded traversal and path search;
- local subgraph queries for UI;
- optional compaction for append-only segments.

## Complexity Targets

Representative operations:

```text
find package by name:
  O(log index segment size) per searched index segment

list versions by package:
  O(number of versions)

compute candidates for a simple range:
  O(log V + R), where V is versions and R is produced ranges

forward reachability:
  O(reached nodes + scanned edges)

reverse impact:
  O(reached reverse nodes + scanned reverse edges)

candidate intersection (same package family only):
  O(ranges_A + ranges_B + exclusions_A + exclusions_B)
```

# Testing Strategy

Testing must cover parser correctness, resolver fidelity, storage durability, graph algorithms, and TDA mapping.

## Parser Golden Tests

Each plugin should include fixtures:

```text
minimal project
optional dependencies
platform-specific dependencies
workspace project
multiple versions
renamed dependencies
private source
malformed metadata
yanked/deprecated versions
cyclic metadata
```

Test form:

```text
raw fixture -> CanonicalBatch snapshot
```

## Resolver Differential Tests

Prism should compare synthetic or parsed output against native tools whenever possible:

```text
npm plugin output vs package-lock / npm tree
Cargo plugin output vs cargo metadata
Maven plugin output vs dependency tree
Go plugin output vs go list / go mod graph
Python plugin output vs lockfile / pip inspect / uv output
```

## Store Tests

Storage tests should cover:

```text
record roundtrip
index lookup
manifest publish
crash recovery simulation
orphan cleanup
compaction correctness
checksum failure handling
unknown section skipping
format version rejection
```

## Graph Tests

Graph tests should cover:

```text
CSR construction
forward traversal
reverse traversal
SCC detection
topological sort of condensation graph
path truncation
duplicate-version cluster detection
```

## TDA Tests

TDA tests should cover:

```text
filtration ordering and the (birth_value, dim, cell_id) tie-break
filtration monotonicity assertion (f(face) <= f(coface)) rejection
boundary/face-incidence correctness (reduction consumes the stored boundary)
cell construction limits
persistence pair import/export
generator support mapping (stable PairId, edge references)
pair -> graph substructure reconstruction
known-answer tests: H0 = component count, H1 = cycle rank on hand-built graphs
```

## Security / Abuse Tests

Security tests should cover:

```text
zip/tar bombs (flat and nested) and decompression-ratio bombs
archive path traversal, symlink/hardlink escape, non-regular-file entries
SSRF: artifact_url pointing at loopback/RFC1918/link-local/cloud-metadata, and redirect-to-internal
DNS-rebinding (resolved-IP change across redirect hops)
XXE / billion-laughs in Maven POM parsing; deeply-nested JSON
credential leak scan: inject https://user:token@host and assert no secret in any workspace file/log
checksum mismatch and missing-checksum handling per trust class
native-resolver opt-in gate (disabled by default; sandbox limits honored)
```

# Milestone Plan

## Milestone 0: Model and Store Skeleton

Deliverables:

```text
prism-model
prism-codec
prism-store object segments
typed index segments
blob store
manifest writer and reader
basic CLI init/status
```

Success criteria:

```text
Package, PackageVersion, Requirement, and StringAtom records roundtrip correctly.
Package lookup and version listing work.
The store recovers cleanly after simulated interrupted writes.
```

## Milestone 1: Cargo and npm Ingestion

Deliverables:

```text
Cargo manifest and lockfile parser
Cargo sparse index metadata parser
npm package.json and package-lock parser
npm registry metadata parser
canonical event writer
```

Success criteria:

```text
Prism can ingest a small Cargo project and npm project.
The canonical requirement graph is queryable.
Raw provenance is stored and recoverable.
```

## Milestone 2: CSR Graph Snapshots

Deliverables:

```text
native / lockfile-exact graph construction (no version choosing — depends on no candidates)
.pcsr writer and reader (with CSR finalization: out + reverse CSR, edge ordinal, node depth)
forward and reverse traversal
SCC detection
duplicate-version report
```

Synthetic, policy-driven construction (which calls `candidates()` and the selection operator) is **deferred to Milestone 3**, after candidate ranges and constraint ASTs exist; Milestone 2 covers only the lockfile/native path so it has no forward dependency on candidates.

Success criteria:

```text
Prism can produce a lockfile-exact graph snapshot for a locked npm or Cargo project.
The UI or CLI can query local neighborhoods without loading the full graph into TypeScript.
```

## Milestone 3: Candidate Version Space

Deliverables:

```text
version sort keys (total order, snapshot-pinned)
package version index
constraint ASTs
candidate range computation
candidate intersection (same-family, exclusion-aware)
conflict pressure metrics
synthetic two-phase fixpoint resolver + policy-driven graph construction (moved from M2)
```

Success criteria:

```text
Given a requirement, Prism can explain which versions satisfy it and why.
Prism can detect empty intersections between requirements.
```

## Milestone 4: TDA MVP

Deliverables:

```text
depth filtration
staleness filtration
undirected 1-dimensional cell complex
Rust-native H0/H1 reduction (union-find + matrix reduction)
persistence pair storage with face-incidence (boundary) arrays
generator support mapping (stable PairId)
```

(Risk filtration is deferred to v0.2 with risk scoring; see TDA Architecture and Design Decisions.)

Success criteria:

```text
Prism can produce persistence outputs for a graph snapshot.
A selected persistence pair can be mapped back to a graph substructure.
```

## Milestone 5: TypeScript UI

Deliverables:

```text
Tauri or napi-rs integration
graph explorer
package search
reverse impact view
TDA barcode view
generator support highlighting
```

Success criteria:

```text
The UI can inspect a real project without loading the entire graph at once.
Users can click topology features and see related packages and edges.
```

## Milestone 6: Additional Ecosystems

Suggested order:

```text
PyPI
Maven
Go
NuGet
```

Success criteria:

```text
Each ecosystem plugin passes parser golden tests and at least one native resolver differential test.
```

# Architectural Risks and Mitigations

## Risk: Reimplementing Too Much Database Functionality

Mitigation:

```text
Keep the store domain-specific.
Do not add SQL, general joins, arbitrary indexes, or multi-user transactions in version 0.1.
Expose domain APIs rather than a query language.
```

## Risk: Ecosystem Resolver Semantics Are Too Complex

Mitigation:

```text
Use native resolver adapters for exact project graphs.
Mark synthetic resolver fidelity clearly.
Implement approximate research models only where their limits are visible.
```

## Risk: Candidate Space Explosion

Mitigation:

```text
Use range-based candidate sets.
Avoid enumerating all solutions.
Store sampled solutions and conflict cores instead of all valid graphs.
Apply fetch and resolution budgets.
```

## Risk: Graph Visualization Becomes a Hairball

Mitigation:

```text
Load local neighborhoods, not whole graphs.
Provide package-family folding.
Provide duplicate-version cluster views.
Provide reverse impact and path-focused views.
Use topology features as entry points into graph substructures.
```

## Risk: TDA Output Is Not Interpretable

Mitigation:

```text
Always store generator support.
Always map persistence features back to packages, versions, requirements, and edges.
Use conservative interpretation labels.
Avoid overclaiming causality.
```

## Risk: Unsafe Artifact Handling

Mitigation:

```text
Do not execute package code.
Limit downloads and archive extraction.
Use metadata-first fetch strategies.
Record hashes and provenance.
```

# Design Decisions

## Decision 1: Prism Uses a Custom Store

Rationale:

```text
The system needs specialized access patterns, immutable graph snapshots, provenance, candidate ranges, and TDA arrays.
A general database would add operational weight and may still require custom graph files.
```

Consequence:

```text
Prism must own file formats, crash safety, compaction, and migration discipline.
```

## Decision 2: Graph Snapshots Are Immutable

Rationale:

```text
Resolved dependency graphs are produced by resolver runs and then queried many times.
CSR is efficient for repeated traversal.
```

Consequence:

```text
Updating dependencies creates a new run and graph snapshot rather than mutating an existing graph.
```

## Decision 3: Requirements Are Separate from Resolved Edges

Rationale:

```text
A declaration such as B >=2,<3 is not the same as choosing B@2.5.0.
Multi-version topology and conflict analysis require preserving this distinction.
```

Consequence:

```text
The data model is more complex, but the analysis becomes much more expressive.
```

## Decision 4: Plugins Own Ecosystem Semantics

Rationale:

```text
npm, Cargo, PyPI, Maven, and Go have different naming, version, condition, and resolution rules.
The core system should not be polluted with ecosystem-specific branching.
```

Consequence:

```text
Plugins must be well-tested and must expose fidelity labels.
```

## Decision 5: TDA Is a Derived Layer

Rationale:

```text
TDA outputs depend on filtration choices and should not be treated as canonical dependency facts.
```

Consequence:

```text
Prism stores TDA outputs separately and maps them back to graph evidence through generator support.
```

## Decision 6: PURL Is the External Identity; Integers Are Internal Handles

Rationale:

```text
Exports and deep links must be portable and stable across workspaces and compaction.
Internal monotonic integer IDs are compact and fast but are store-local and epoch-sensitive.
```

Consequence:

```text
PURL is the primary key in every export and external reference; internal IDs never leave as a
primary key. GraphNodeId/GraphEdgeId are snapshot-local and always carried with their RunId.
```

## Decision 7: Risk Scoring Is Deferred and Composite

Rationale:

```text
A risk filtration needs a per-node scalar, but combining security, maintenance, license, and
staleness into one number is a normalization problem with no obviously-correct answer, and
shipping it early would mean filtering on an undefined value.
```

Consequence:

```text
v0.1 ships depth and staleness filtrations only. When risk lands (v0.2), it is a normalized [0,1]
composite that STORES its sub-scores (security, maintenance, license, staleness) separately and
derives the composite by a documented, overridable weighting, so the score is reproducible and
auditable rather than a magic number.
```

# Open Questions

## Resolved (promoted to Design Decisions)

```text
2. Graph snapshots         -> owned arrays in v0.1 (mmap is a v0.2 optimization; little-endian,
                              aligned CSR layout already permits it without a format change).
3. Native resolver in sandbox -> off by default, explicit per-run opt-in, restricted subprocess
                              (no extra network, dropped tokens, resource limits); labeled NativeToolResolved.
4. PURL                    -> adopt PURL as the external/portable identity now; internal integer IDs
                              are store-local handles (see Identity and ID Model).
5. TDA backend             -> Rust-native H0/H1 reduction over the undirected 1-complex in v0.1,
                              plus an export path for H2+.
7. Sampled resolutions     -> full CSR snapshots in v0.1 (immutable, self-contained); base+delta is a
                              v0.2 storage optimization.
8. Private-registry auth   -> AuthRef resolves from an OS keychain / env / external secrets file
                              outside .prism/; secrets never enter any store file (see Security Model).
9. Workspace metadata      -> user-editable, non-secret project config lives in a separate
                              .prism/config.toml; MANIFEST is machine-managed and never hand-edited.
                              Auth secrets live outside the workspace (see #8).
```

## Still open

1. Should Prism expose a small domain-specific query language after the API stabilizes? (Post-1.0; v0.1 ships domain APIs only.)
6. How should risk scores be normalized across security, maintenance, license, and staleness dimensions? Risk filtration is therefore deferred to v0.2 (see Design Decisions → Risk scoring); v0.1 ships depth and staleness filtrations only.

# Recommended Version 0.1 Scope

The recommended first release should include:

```text
Custom local store:
  object segments
  typed index segments
  content-addressed blobs
  manifest publication

Core model:
  package
  package version
  requirement
  candidate range
  resolver run
  graph snapshot

Ecosystems:
  Cargo
  npm

Graph analysis:
  forward traversal
  reverse traversal
  SCC
  duplicate versions
  basic graph stats

TDA:
  depth filtration
  staleness filtration
  Rust-native H0/H1 persistence over the undirected 1-complex (+ export path)
  generator support mapping (stable PairId)

UI/API:
  Rust CLI
  TypeScript graph explorer
  local-neighborhood queries
```

Everything else should be treated as version 0.2 or later:

```text
PyPI, Maven, Go, NuGet
native resolver differential testing at scale
advanced conflict core extraction
higher-dimensional directed flag complexes
collaborative UI
remote workspace service
full SBOM import/export polish
```

# Appendix A: Glossary

Package family
: A package without a specific version, such as `npm:react` or `cargo:serde`.

Package version
: A concrete published version, such as `npm:react@18.2.0`.

Requirement
: A declared dependency constraint from one package version to a package family.

Candidate set
: The versions that satisfy a requirement under a snapshot, environment, and source policy.

Resolved edge
: A concrete dependency edge in a resolver run from one graph node occurrence to another.

Graph node occurrence
: A node in a resolved graph snapshot. It may represent a package version in a specific path, feature context, environment, or mediation context.

Resolver run
: A specific attempt to resolve or reconstruct a dependency graph under a given input, environment, source policy, and resolver policy.

CSR
: Compressed sparse row representation for graph adjacency arrays.

Filtration
: A sequence of topological objects built by adding nodes, edges, or cells according to a scalar value such as depth, risk, age, or centrality.

Persistence pair
: A birth/death pair from persistent homology.

Generator support
: The original graph objects that support or explain a persistence feature.

# Appendix B: Minimal File Format Sketch

## Prism Object Segment

```text
magic:          "PRISMPSEG"
format_version: u32
segment_id:     u32
created_at:     i64
flags:          u32
records:        repeated Record
footer:         SegmentFooter
```

## Prism Index Segment

```text
magic:          "PRISMPIDX"
format_version: u32
index_id:       u32
created_at:     i64
blocks:         sorted key-value blocks
footer:         IndexFooter
```

## Prism CSR Graph Snapshot

```text
magic:          "PRISMCSR"
format_version: u32
run_id:         u64
node_count:     u32
edge_count:     u32
sections:       typed array sections
footer:         GraphFooter
```

## Prism TDA File

```text
magic:          "PRISMTDA"
format_version: u32
filtration_id:  u64
run_id:         u64
sections:       cell arrays (incl. boundary/face-incidence CSR), persistence arrays,
                generator support arrays
footer:         TdaFooter
```

# Appendix C: First Repository Layout

```text
prism/
  crates/
    prism-model/
    prism-codec/
    prism-store/
    prism-textfmt/
    prism-semver/
    prism-plugin-api/
    prism-plugin-cargo/
    prism-plugin-npm/
    prism-plugin-bun/
    prism-plugin-pnpm/
    prism-plugin-uv/
    prism-resolve/
    prism-graph/
    prism-tda/
    prism-api/
    prism-cli/
    prism-server/

  apps/
    prism-desktop/
      src-tauri/
      web/

  packages/
    prism-client/
    prism-ui-components/

  fixtures/
    cargo/
    npm/

  docs/
    architecture.md
    file-formats.md
    plugin-api.md
    storage-engine.md
    tda.md
```

# Appendix D: Suggested Engineering Rules

1. Every parser must have golden fixtures.
2. Every resolver adapter must emit a fidelity label.
3. Every graph snapshot must be immutable.
4. Every raw input must be content-hashed.
5. Every TDA feature shown in UI must support graph back-mapping.
6. Every fetcher must have a budget.
7. Every archive read must have safety limits.
8. Every plugin must keep ecosystem-specific logic out of the core model.
9. Every file format section must be skippable by older readers unless it is marked required.
10. Every write must publish through the manifest protocol.
