import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));
const api = process.env.API_INTERNAL_URL || "http://127.0.0.1:4321";
const prod = process.env.NODE_ENV === "production";

/** @type {import('next').NextConfig} */
const nextConfig = {
  ...(prod ? { output: "standalone" } : {}),
  eslint: { ignoreDuringBuilds: true },
  webpack: (config) => {
    config.resolve.alias["@pom/format"] = path.join(root, "../../packages/format/src/numeric-input.ts");
    return config;
  },
  async rewrites() {
    return [
      { source: "/api/:path*", destination: `${api}/api/:path*` },
      { source: "/health", destination: `${api}/health` },
    ];
  },
};

export default nextConfig;
