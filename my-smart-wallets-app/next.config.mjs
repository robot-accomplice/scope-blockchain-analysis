// @coinbase/cdp-sdk (via wagmi's Base Account connector) statically imports
// these optional peer packages for x402 payments, which this app never uses.
// Resolve them to empty modules so the build does not need them installed.
const UNUSED_OPTIONAL_PEERS = ["@x402/core", "@x402/evm", "@x402/svm"];

/** @type {import('next').NextConfig} */
const nextConfig = {
  images: {
    remotePatterns: [
      {
        protocol: "https",
        hostname: "static.alchemyapi.io",
        port: "",
        pathname: "/**",
      },
    ],
  },
  webpack: (config) => {
    for (const name of UNUSED_OPTIONAL_PEERS) {
      config.resolve.alias[name] = false;
    }
    return config;
  },
};

export default nextConfig;
