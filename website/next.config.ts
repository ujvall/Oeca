import type { NextConfig } from "next";

// Static export: the site is plain HTML, CSS, JS and images (Vercel or any static host).
const nextConfig: NextConfig = {
  output: "export",
  // /changelog/index.html: direct visits to /changelog resolve on any static host
  trailingSlash: true,
  images: { unoptimized: true },
};

export default nextConfig;
