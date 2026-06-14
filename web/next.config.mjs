/** @type {import('next').NextConfig} */
const nextConfig = {
  // Pure static export — no Node runtime on the host, no server code.
  output: 'export',
  // Static export disables next/image's optimizer.
  images: { unoptimized: true },
  reactStrictMode: true,
  // three.js ships untranspiled ESM in places; let Next transpile it.
  transpilePackages: ['three'],
  // Types are enforced by `tsc`/the build; don't let lint nits block a build.
  eslint: { ignoreDuringBuilds: true },
  // Required for ESM-style WebAssembly imports under Next 15 / Webpack 5.
  // (Still off by default in Next 15.)
  //
  // WARNING — this `webpack` hook is IGNORED by Turbopack. Running
  // `next dev --turbo` or a future Next that defaults `build` to Turbopack
  // will silently break `await import('../wasm/prism_wasm')`. The
  // `dev`/`build` scripts in package.json deliberately omit `--turbo`;
  // do not add it without a Turbopack-equivalent wasm-loader replacement.
  webpack(config) {
    config.experiments = { ...config.experiments, asyncWebAssembly: true };
    return config;
  },
};

export default nextConfig;
