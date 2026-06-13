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

Rust core (a Cargo workspace under `crates/`), with a TypeScript UI planned on top:

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
| `prism-graph` | Immutable CSR graph snapshots, traversal, SCC, duplicate-version analysis, `.pcsr` format. |
| `prism-resolve` | Synthetic two-phase fixpoint resolver engine (cycle guard, budgets, feature activation). |
| `prism-tda` | Persistent homology over the dependency graph: filtrations, H0/H1, generator support, `.ptda` format. |
| `prism-api` | High-level facade tying the above together (ingest, resolve, graph queries, TDA). |
| `prism-cli` | The `prism` command-line interface. |

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
prism status                       # → runs / packages / graphs summary
prism graph stats --run <id>       # nodes, edges, SCCs, cycles, duplicate versions
prism graph impact --run <id> --node <id>   # reverse-dependency impact
prism tda run --run <id> --filtration depth # persistent homology (H0/H1) barcode
prism export json --run <id>       # emit the resolved graph as JSON
```

Example: ingesting a Cargo.lock for `sample-app → {serde → serde_derive, rand → rand_core}`
yields a 5-node / 4-edge graph; `graph impact` on `serde_derive` reports `serde` and
`sample-app` as dependents; `tda run` reports one connected component (H0 = 1).

## Status

The v0.1 core is implemented bottom-up following the milestone plan in the design doc and
the **end-to-end vertical slice runs**: a Cargo/npm lockfile ingests into an immutable CSR
graph snapshot, which is queried for stats / neighborhoods / reverse impact / SCCs /
duplicate versions, and fed through the TDA persistence pipeline (depth filtration → H0/H1 →
generator support). The storage engine (segments, typed indexes, blob store, atomic publish +
recovery), the ecosystem/semver/plugin layer, the graph algorithms, the synthetic resolver,
and the `prism-api`/`prism-cli` surfaces are in place and tested (400+ tests, clippy clean).

Known follow-ups: ingest currently writes more index segments than necessary (interning is
not yet batched), live registry fetching is out of scope for the offline v0.1, and the
TypeScript UI is not yet built.

## License

MIT OR Apache-2.0.
