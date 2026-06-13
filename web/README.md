# Prism Web — dependency-topology explorer

A Next.js + TypeScript + Tailwind CSS + **React Three Fiber** UI over the
`prism-serve` JSON API. It replaces the old hermetic vanilla-JS UI with a modern
2D **and** interactive **3D** topology experience.

## Architecture

```
browser ──>  Next.js app (this)  ──proxy──>  prism-serve (Rust JSON API)  ──>  .prism store
            React / R3F / Tailwind          crates/prism-server
```

- The Rust `prism-serve` is now a slim, read-only, CORS-enabled JSON API
  (`crates/prism-server`). Start it against a workspace:
  `prism-serve --workspace /path/to/ws --port 7831`.
- This app's server-side proxy (`src/app/api/[...path]/route.ts`) forwards
  `/api/*` to `PRISM_API_URL` (default `http://127.0.0.1:7831`), so the browser
  stays same-origin. Set `NEXT_PUBLIC_PRISM_API` to hit `prism-serve` directly
  instead.

## Run

```sh
# 1. start the Rust API (in the Prism repo)
cargo run --release -p prism-cli -- serve --workspace /path/to/workspace --port 7831
#    or the dedicated binary: prism-serve --workspace /path/to/workspace --port 7831

# 2. start the web app
cd web
cp .env.example .env.local          # optional: point PRISM_API_URL elsewhere
npm install
npm run dev                         # http://localhost:3000
```

`npm run build` for a production build; `npm run typecheck` for types only.

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
  app/            Next.js app router (page, layout, globals, API proxy)
  components/     Topbar, Graph2D, Graph3D, Controls3D, SidePanel
    scene/        R3F scene pieces (Nodes, Edges, LayerPlanes, Fibers, TerrainMesh, …)
    panel/        side-panel tabs (Overview, Node, Duplicates, Cycles, TDA)
  lib/            types, api client, palette, graph metrics, 2D/3D layout
  store/          the shared Zustand store
```
