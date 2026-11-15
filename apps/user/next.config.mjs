import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(fileURLToPath(import.meta.url));
const prod = process.env.NODE_ENV === "production";

/** @type {import('next').NextConfig} */
const nextConfig = {
  ...(prod ? { output: "standalone" } : {}),
  eslint: { ignoreDuringBuilds: true },
  webpack: (config) => {
    config.resolve.alias["@pom/format"] = path.join(root, "../../packages/format/src/numeric-input.ts");
    return config;
  },
};

export default nextConfig;
