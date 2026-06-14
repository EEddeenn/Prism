/* Singleton, single-flight WASM loader for the prism-wasm bridge.
 *
 * `wasm()` returns a cached promise of the fully-initialised module. The
 * underlying `init('/wasm/prism_wasm_bg.wasm')` call runs at most once per
 * SUCCESSFUL page load — even under React Strict Mode's double-mount,
 * because the promise is stored at module scope and reused.
 *
 * If init FAILS (offline at boot, transient 404 on the .wasm asset, a CDN
 * blip), the cached slot is cleared inside the catch so the next caller
 * gets a fresh attempt instead of inheriting a poisoned promise that
 * would only clear on a full reload.
 *
 * The .wasm artifact is fetched from `/wasm/prism_wasm_bg.wasm`, which the
 * build pipeline copies into `web/public/wasm/` from the wasm-pack output.
 */
type WasmMod = typeof import('../wasm/prism_wasm');

let initPromise: Promise<WasmMod> | null = null;

export function wasm(): Promise<WasmMod> {
  if (!initPromise) {
    initPromise = (async () => {
      try {
        const m = await import('../wasm/prism_wasm');
        await m.default('/wasm/prism_wasm_bg.wasm');
        return m;
      } catch (e) {
        // Allow retry: clear the cached failure so the next `wasm()` call
        // does a fresh import + init instead of re-resolving the rejection.
        initPromise = null;
        throw e;
      }
    })();
  }
  return initPromise;
}
