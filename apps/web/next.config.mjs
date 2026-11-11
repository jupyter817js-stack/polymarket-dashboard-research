/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  images: {
    remotePatterns: [
      { protocol: "https", hostname: "polymarket-upload.s3.us-east-2.amazonaws.com" },
      { protocol: "https", hostname: "polymarket.com" },
    ],
  },
  async rewrites() {
    return [
      { source: "/api/:path*", destination: `${process.env.RN1_API_BASE || "http://localhost:8000"}/api/:path*` },
    ];
  },
  experimental: {
    optimizePackageImports: ["lucide-react", "framer-motion"],
  },
};
export default nextConfig;
