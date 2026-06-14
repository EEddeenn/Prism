# Prism

A **local-first dependency topology analysis system**. Prism ingests a project, lockfile,
manifest, or explicit package list; recursively expands dependencies while preserving
multi-version relationships; builds immutable resolved-graph snapshots; derives graph and
risk features; and constructs topology-ready representations for TDA (topological data
analysis) workflows.

Prism is **not** a package manager, a general database, or a graph database. It is a
domain-specific dependency-topology engine with a purpose-built local store:

```
append-only object segments
+ typed immutable indexes
+ content-addressed raw blobs
+ immutable CSR graph snapshots
+ range-based candidate version indexes
+ array-based TDA result files
```

The full design rationale lives in [`Prism_Architecture_Design.md`](./Prism_Architecture_Design.md).

## Workspace layout

Rust core (a Cargo workspace under `crates/`), with a hermetic local web UI on top:

| Crate | Responsibility |
|-------|----------------|
| `prism-model` | Canonical domain model: typed IDs, bitflags, entities, `PrismError`. |
| `prism-codec` | Binary primitives: little-endian I/O, LEB128 varints, CRC-32, SHA-256, record framing. |
| `prism-store` | Purpose-built store: object segments, typed indexes, blob store, manifest, atomic publish + crash recovery. |
| `prism-textfmt` | Hermetic hand-rolled JSON + (subset) TOML parsers for offline manifest/lockfile ingestion. |
| `prism-semver` | SemVer 2.0 version/constraint engine: ordering, sort keys, candidate ranges. |
| `prism-plugin-api` | Object-safe ecosystem plugin traits + canonical fact events. |
| `prism-plugin-cargo` | Cargo plugin: `Cargo.toml` / `Cargo.lock` parsing and semantics. |
| `prism-plugin-npm` | npm plugin: `package.json` / `package-lock.json` parsing and semantics. |
| `prism-plugin-bun` | Bun plugin: `bun.lock` (jsonc-like) ingestion reshaped into npm semantics. |
| `prism-plugin-pnpm` | pnpm plugin: native `pnpm-lock.yaml` (v5–v9) ingestion via the flat `snapshots:` graph. |
| `prism-plugin-uv` | uv plugin: native `uv.lock` (PyPI) ingestion with marker conditions. |
| `prism-graph` | Immutable CSR graph snapshots, traversal, SCC, duplicate-version analysis, `.pcsr` format. |
| `prism-resolve` | Synthetic two-phase fixpoint resolver engine (cycle guard, budgets, feature activation). |
| `prism-tda` | Persistent homology over the dependency graph: filtrations, H0/H1, generator support, `.ptda` format. |
| `prism-api` | High-level facade tying the above together (ingest, resolve, candidates/conflicts, graph queries, TDA). |
| `prism-cli` | The `prism` command-line interface. |
| `prism-server` | `prism-serve`: a pure-std (localhost-only) HTTP server + vanilla-JS/SVG graph explorer. |

### Design principles

- **Hermetic core** — the Rust core has *zero external crate dependencies*; primitives
  (hashing, parsing, bitflags) are implemented in-tree and verified against standard vectors.
- **Immutable snapshots** — resolved graphs and TDA results are immutable, content-checksummed
  files; updating dependencies creates a new run, never mutates an existing graph.
- **Crash safety** — a single-writer store with an atomic manifest-swap publish protocol
  (stage in `tmp/` → fsync → rename → fsync directory → swap `MANIFEST`), and recovery that
  trusts files only up to their manifest-recorded length and SHA-256 digest.
- **Provenance & fidelity** — raw inputs are content-addressed; every resolved graph records a
  `GraphFidelity` (tier + "what was dropped" caveats) so the UI never overclaims correctness.
- **No code execution** — Prism analyzes topology; it never runs install/build scripts or
  arbitrary package code.

## Build & test

```sh
cargo build
cargo test          # unit + integration tests across all crates
cargo clippy --all-targets
```

The workspace targets stable Rust (edition 2021) and builds offline with no network access.

## Quickstart

```sh
cargo build -p prism-cli           # builds the `prism` binary at target/debug/prism

prism init .                       # create a .prism/ workspace
prism ingest Cargo.lock            # parse a lockfile into a resolved graph snapshot
                                   # (also: package-lock.json, bun.lock, pnpm-lock.yaml, uv.lock)
prism status                       # → runs / packages / graphs summary
prism regions                      # every run as a comparable dependency region
prism graph stats --run <id>       # nodes, edges, SCCs, cycles, duplicate versions
prism graph impact --run <id> --node <id>   # reverse-dependency impact
prism graph chokepoints --run <id>          # structural gateway packages
prism graph why --run <id> --node <id>      # why a dependency is present
prism graph diamonds --run <id>             # diamond-reuse foundations
prism graph fidelity --run <id>             # per-metric fidelity notes
prism graph diff --base <a> --head <b>      # interpretive cross-run diff
prism graph budget --head <id> --file budget.json   # CI topology budget gate
prism candidates --package pkg:cargo/serde --constraint "^1.0"  # candidate version ranges
prism conflicts --package pkg:cargo/serde   # same-package conflict pressure
prism resolve Cargo.toml --policy highest   # synthetic resolve a manifest (lowest|highest|locked)
prism tda run --run <id> --filtration depth # persistent homology (H0/H1) barcode
prism tda compare --run <id>                # topology under multiple filtration lenses
prism tda h2 --run <id>                     # experimental H2 (void) homology
prism export json --run <id>       # emit the resolved graph as JSON
prism export graphml --run <id>    # emit the resolved graph as GraphML (XML)
```

See `prism help` for the full list (status, regions, graph {stats, neighborhood, impact,
duplicates, scope, chokepoints, why, cycles, dup-causes, risk, fragmentation, fidelity,
subsystems, diamonds, minimal, shared, overlay, distance, diff, budget}, tda {run, support,
compare, h2}, candidates, conflicts, resolve, export {json, graphml}).

Example: ingesting a Cargo.lock for `sample-app → {serde → serde_derive, rand → rand_core}`
yields a 5-node / 4-edge graph (depths 0→2); `graph impact` on `serde_derive` reports `serde`
and `sample-app` as dependents; `tda run` reports one connected component (H0 = 1).

### Web UI

```sh
cargo build -p prism-server        # builds the `prism-serve` binary
prism-serve --workspace . --port 7777   # serves http://127.0.0.1:7777 (localhost only)
```

A self-contained graph explorer (vanilla JS + inline SVG, no build step, no external
resources):

- **Pan/zoom canvas** with a deterministic 2D layout — connected components laid out in
  barycenter-ordered BFS layers, each wide layer *wrapped into a grid* (so a shallow
  900-node npm tree spreads into a block, not a single long column) and the components
  shelf-packed — plus a fit-to-view control and a colour legend; labels appear when zoomed
  in or selected.
- **Click a node** to trace what it depends on (forward) and what depends on it (reverse
  impact), with clickable dependency/dependent lists.
- **Tabbed side panel** surfacing every query: run provenance (ecosystem + `GraphFidelity`
  tier and caveats), graph stats, duplicate-version families (click to highlight all
  occurrences), strongly-connected components / cycles, and the TDA barcode.
- **Search** to find and centre a package by name.

The server opens the workspace **read-only** (no write lock), so it runs alongside the CLI
and other viewers, a crash never strands a lock, and it binds `127.0.0.1` exclusively.

## Status

The v0.1 system is implemented bottom-up following the milestone plan in the design doc, and
the **end-to-end vertical slices run** for both Cargo and npm: a lockfile ingests into an
immutable CSR graph snapshot, which is queried for stats / neighborhoods / reverse impact /
SCCs / duplicate versions / candidate ranges / conflict pressure, synthetically re-resolved
from a manifest under a policy, fed through the TDA persistence pipeline (depth filtration →
H0/H1 → generator support), and browsed live in the `prism-serve` web UI. The storage engine
(segments, typed indexes, blob store, atomic publish + recovery), the ecosystem/semver/plugin
layer, the graph algorithms, the synthetic resolver, and the `prism-api` / `prism-cli` /
`prism-server` surfaces are in place and tested (480+ tests, clippy clean). Ingest is batched
(one segment + index pair per lockfile) and idempotent (re-ingesting a manifest does not
duplicate requirements or edges).

Known follow-ups: live registry fetching is out of scope for the offline v0.1 (resolution
selects from versions already on hand), and `export` emits JSON and GraphML (both PURL-keyed,
with the run's `GraphFidelity` in the header); Parquet export is recognized but deferred — a
spec-compliant, dependency-free columnar writer is a later milestone.

## License

MIT OR Apache-2.0.
