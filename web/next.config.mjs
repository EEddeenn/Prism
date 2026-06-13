/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // three.js ships untranspiled ESM in places; let Next transpile it.
  transpilePackages: ['three'],
  // Types are enforced by `tsc`/the build; don't let lint nits block a build.
  eslint: { ignoreDuringBuilds: true },
};

export default nextConfig;
