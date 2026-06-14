# Prism Web — dependency-topology explorer

A Next.js + TypeScript + Tailwind CSS + **React Three Fiber** UI rendered as a
**fully static site**. There is no backend at runtime: a WASM module
(`crates/prism-wasm`) decodes `.prismbundle` files in the browser and answers
every view query against an in-memory workspace.

## Architecture

```
browser  ──>  static HTML/JS  ──load.wasm──>  prism-wasm (in-browser dispatcher)
                                                   │
                              IndexedDB ──┐        │
                                          ▼        ▼
                                  bundle bytes ──> Prism::open_in_memory
                                                       │
                                                       ▼
                                              view JSON (regions/TDA/etc.)
```

1. User drops a `.prismbundle` on the landing page.
2. The dropzone hashes the bytes (`crypto.subtle.digest('SHA-256', …)`),
   verifies the envelope via `wasm.load_bundle(...)`, then persists the bytes
   into IndexedDB keyed by the full 64-char SHA-256 hex.
3. The URL becomes `?bundle=<sha>` and the page reloads.
4. `WasmBundleLoader` reads `?bundle=`, fetches bytes from IndexedDB, and calls
   `wasm.load_bundle(...)` again to install the singleton workspace.
5. All view fetches go through `wasm.call_view(name, args_json)` — the same
   names the deleted `prism-server` HTTP routes once carried (see
   `crates/prism-api/src/dispatch.rs`).

## Run (dev / build)

The Rust WASM artifact is built into `web/src/wasm/` and copied to
`web/public/wasm/prism_wasm_bg.wasm` so Next.js serves it as a static asset.

```sh
cd web
npm install
npm run dev       # http://localhost:3000  (prebuild auto-runs wasm-pack)
npm run build     # static export → web/out/  (host-anywhere bundle)
npm run typecheck # tsc --noEmit
```

`npm run dev` and `npm run build` both invoke `wasm-pack` via the `predev` /
`prebuild` script. You can also run it explicitly:

```sh
npm run build:wasm
```

Prerequisites (one-time): `rustup target add wasm32-unknown-unknown` and a
`wasm-pack` binary on `$PATH` (`cargo install wasm-pack` or
`brew install wasm-pack`).

## Producing a bundle

```sh
prism analyze inputs/Cargo.lock --tda --out demo.prismbundle
# or, against an existing .prism/ workspace:
prism export bundle --out demo.prismbundle
prism bundle inspect demo.prismbundle   # verify + dump TOC
```

See `STATIC-DEPLOY.md` (repo root) for hosting the `web/out/` directory on
GitHub Pages, Cloudflare Pages, Netlify, S3 + CloudFront, nginx, or Caddy.

## Views

- **2D** — the deterministic layered graph (component-tiled, barycenter-ordered).
- **3D** — four modes:
  - **Layered** — dependency depth as stacked horizontal planes (roots on top).
  - **Fiber** — each package family is a vertical version stack; duplicates rise tall.
  - **Terrain** — a heightfield whose elevation is a chosen signal (centrality,
    duplicate pressure, fan-in/out, depth, composite risk).
  - **TDA** — pick a persistence feature (H₀ component / H₁ loop) in the TDA tab
    and its generator support lights up on the depth substrate.

Selection is shared across both views: clicking a node, a duplicate family, a
cycle member, or a TDA feature highlights it everywhere.

## Layout

```
src/
  app/             Next.js app router (page, layout, globals — no /api routes)
  components/      Topbar, Graph2D, Graph3D, Controls3D, SidePanel,
                   BundleDropzone, WasmBundleLoader
    scene/         R3F scene pieces (Nodes, Edges, LayerPlanes, Fibers, TerrainMesh, …)
    panel/         side-panel tabs (Overview, Node, Duplicates, Cycles, TDA)
  lib/             api client (wasm dispatcher), bundleStore (IndexedDB),
                   wasm loader, types, palette, layout
  store/           the shared Zustand store
  wasm/            wasm-pack output (gitignored — regenerated on every build)
```

## Why static + WASM (not a server)

The Rust hermetic core already parses lockfiles, builds the graph, computes
TDA, and renders view JSON correctly and panic-free on untrusted input. WASM
reuses every byte of that — no parallel TypeScript reimplementation, no
backend to host. Bundles are produced once by the CLI, shared as a single
file, and replayed offline by anyone with a browser.
