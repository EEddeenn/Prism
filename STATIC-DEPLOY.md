# Deploying Prism (static, no backend)

The Prism web UI ships as a fully static site. It opens `.prismbundle`
files in the browser via WebAssembly and persists them to IndexedDB.
There is no server process, no database, no auth, no environment
variables to set.

`prism-server` and the `prism serve` command are gone. The browser does
the decode and the view rendering itself, using the same Rust code that
the CLI calls, compiled to WebAssembly.

---

## 1. Prerequisites

- Node 20+ and npm (or pnpm / yarn — anything that runs `next build`).
- Rust 1.80+ with the WebAssembly target:

  ```sh
  rustup target add wasm32-unknown-unknown
  ```

- `wasm-pack`:

  ```sh
  cargo install wasm-pack
  # or, on macOS:
  brew install wasm-pack
  ```

That's the whole toolchain. No Docker, no system services, no
package-manager registries at runtime.

---

## 2. Build

From the repo root:

```sh
cd web
npm install
npm run build
```

`npm run build` runs:

1. `wasm-pack build ../crates/prism-wasm --target web --release`
   — emits `web/src/wasm/prism_wasm.js` and `web/src/wasm/prism_wasm_bg.wasm`.
2. Copies the `.wasm` to `web/public/wasm/prism_wasm_bg.wasm` so the
   static host can serve it at `/wasm/prism_wasm_bg.wasm`.
3. `next build` with `output: 'export'`.

The result is a single self-contained static directory:

```
web/out/
  index.html
  _next/static/...
  wasm/prism_wasm_bg.wasm     # served as application/wasm
```

`web/out/` is the complete deployable artifact. It does not need a Node
runtime on the host.

---

## 3. Serve locally

```sh
python -m http.server --directory web/out 8080
# open http://localhost:8080/
```

The drop zone accepts a `.prismbundle` produced by
`prism export bundle …`. The browser decodes it via WebAssembly,
persists the bytes in IndexedDB keyed by the bundle's SHA-256, and
updates the URL to `?bundle=<sha>`. Bookmark or share that URL — the
bundle stays in the user's browser; the receiver opens the same URL,
sees the empty state, and re-drops the same `.prismbundle` (or any
other) to populate their local IndexedDB.

---

## 4. Static-host targets

### GitHub Pages

- Push the contents of `web/out/` to a `gh-pages` branch.
- Add an empty `.nojekyll` file at the root so Pages does not skip
  `_next/`:

  ```sh
  touch web/out/.nojekyll
  ```

- If you host under a subpath (e.g. `https://user.github.io/myrepo/`),
  set `basePath: '/myrepo'` in `web/next.config.mjs` and update the
  init call site so the WASM URL is also prefixed
  (`/myrepo/wasm/prism_wasm_bg.wasm`).

### Cloudflare Pages, Netlify, Vercel (static), Render

- Build command: `npm run build`
- Publish directory: `out`
- No environment variables.

### S3 + CloudFront

```sh
aws s3 sync web/out/ s3://your-bucket/ --delete
# Re-upload the .wasm with the correct MIME so CloudFront caches it
# with the right Content-Type:
aws s3 cp web/out/wasm/prism_wasm_bg.wasm \
         s3://your-bucket/wasm/prism_wasm_bg.wasm \
         --content-type application/wasm
```

### Plain nginx

Add to your `http {}` block:

```nginx
types { application/wasm wasm; }
```

Otherwise nginx serves `.wasm` as `application/octet-stream` and the
browser falls back to a slower compile path (or rejects under strict
CSP).

### Caddy

Caddy serves `.wasm` as `application/wasm` by default. No config needed.

---

## 5. MIME types

The single hosting requirement is:

```
*.wasm  →  Content-Type: application/wasm
```

Modern hosts default to this. Where they don't (older nginx, some S3
configs), set it explicitly — streaming compile from
`application/octet-stream` works in most browsers but can fail under
strict CSP, and it's strictly slower.

---

## 6. Cross-origin isolation

Not required. Prism uses single-threaded WebAssembly, no
`SharedArrayBuffer`, no Web Worker. You do not need to set `COOP` or
`COEP` headers.

---

## 7. HTTPS

Strongly recommended. The bundle key is derived in the browser with
`crypto.subtle.digest('SHA-256', …)`, which requires a secure context.
`localhost` is exempt, so a local `python -m http.server` works for
testing, but any deployed host should be served over HTTPS.

---

## 8. Bundle size

The WebAssembly artifact (`prism_wasm_bg.wasm`) is approximately
2–4 MB uncompressed and 700 KB–1.2 MB on the wire with Brotli or gzip
compression. Most static hosts compress automatically. Future
optimisation (`wasm-opt -Oz`) can shave a further 20–30 %.

---

## 9. Storage quotas

Bundles live in the browser's IndexedDB. Per-origin quotas vary by
browser, but are typically 50 %–60 % of the device's free disk space.
A `QuotaExceededError` from `putBundle` surfaces in the UI; the user
can clear stored bundles from the dropzone affordance.

There is no server-side storage to provision or to garbage-collect.

---

## 10. Migration from the old `prism-server` flow

The previous `DEPLOY.md` documented a loopback-only `prism-server`
process plus a Next.js proxy. Both are gone:

- `prism-server` is deleted from the workspace.
- The `prism serve` subcommand is removed.
- The web app no longer has `/api/...` proxy routes; `PRISM_API_URL`
  and `NEXT_PUBLIC_PRISM_API` are no longer read anywhere.

The new flow is:

```sh
# Produce a bundle on the analyst's machine (unchanged):
prism export bundle --workspace . --out my.prismbundle

# Or one-shot ingest + TDA + export (unchanged):
prism analyze ./Cargo.lock ./pnpm-lock.yaml --tda --out my.prismbundle

# Inspect a bundle (unchanged):
prism bundle inspect my.prismbundle
```

Then host `web/out/` on any static host and share the URL. Users drop
`my.prismbundle` onto the page; the browser does the decoding and
view-rendering itself. The `?bundle=<sha>` URL parameter is how
deep-link sharing works — anyone with the link who also has the same
bundle in their IndexedDB lands on the same view.

`prism analyze`, `prism export bundle`, and `prism bundle inspect`
continue to work against on-disk workspaces. They are the producer
side of the workflow; the static web app is the consumer side.
