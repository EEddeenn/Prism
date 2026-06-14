/* Thin IndexedDB wrapper for persisting decoded .prismbundle byte payloads.
 *
 * Keyed by the full 64-character SHA-256 hex digest of the bundle bytes — the
 * same value the WASM `load_bundle` reports as `bundle_sha256` / `id`, and the
 * one the URL carries as `?bundle=<sha>`.
 *
 * Uint8Array is structured-cloneable, so values round-trip without conversion.
 */
import { createStore, get, set, del, keys } from 'idb-keyval';

const store = createStore('prism', 'bundles');

export const getBundle = (sha: string) => get<Uint8Array>(sha, store);
export const putBundle = (sha: string, bytes: Uint8Array) => set(sha, bytes, store);
export const deleteBundle = (sha: string) => del(sha, store);
export const listBundles = () => keys(store) as Promise<string[]>;
