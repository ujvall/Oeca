// oeca: Nearby's Content-Security-Policy and security headers (OECA.md, Security headers).
// One policy for the page's <meta> and for the host's headers. Inline scripts
// are allowed only by the hash of their exact text; nothing else is inline.
import { createHash } from "node:crypto";

const INLINE_SCRIPT = /<script(?![^>]*\bsrc=)(?![^>]*type="application\/json")[^>]*>([\s\S]*?)<\/script>/g;

/** The hashes of a page's executable inline scripts (the import map and Nuxt's config). */
export function inlineScriptHashes(html: string): string[] {
  const out = new Set<string>();
  for (const m of html.matchAll(INLINE_SCRIPT)) {
    out.add(`'sha256-${createHash("sha256").update(m[1] ?? "", "utf8").digest("base64")}'`);
  }
  return [...out].sort();
}

/** The signaling server, and nothing else outside this origin. */
export function signalingOrigin(url: string): string {
  const u = new URL(url);
  if (u.protocol !== "wss:" && u.protocol !== "ws:") throw new Error(`signaling must be ws(s): ${url}`);
  if (u.protocol === "ws:" && !["127.0.0.1", "localhost", "[::1]"].includes(u.hostname)) {
    throw new Error(`unencrypted signaling only on loopback: ${url}`);
  }
  return `${u.protocol}//${u.host}`;
}

/**
 * WebRTC itself (ICE, STUN, DTLS) is not governed by CSP; the page asks for
 * no STUN beyond its configuration, and the data channel is end-to-end.
 */
export function policy(scriptHashes: string[], signalingUrl: string, framing = false): string {
  return [
    "default-src 'none'",
    `script-src 'self' ${scriptHashes.join(" ")}`.trim(),
    "style-src 'self'",
    "img-src 'self'",
    // oeca: the bundled typeface (public/fonts), from this origin only
    "font-src 'self'",
    `connect-src 'self' ${signalingOrigin(signalingUrl)}`,
    "worker-src 'self'",
    "manifest-src 'self'",
    "base-uri 'none'",
    "form-action 'none'",
    "object-src 'none'",
    // Only a header can say this; a <meta> policy may not
    ...(framing ? ["frame-ancestors 'none'"] : []),
  ].join("; ");
}

/** The page, with its policy as the first thing in <head>. */
export function withPolicy(html: string, signalingUrl: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${policy(inlineScriptHashes(html), signalingUrl)}">`;
  if (!html.includes("<head>")) throw new Error("no <head>");
  return html.replace("<head>", `<head>${meta}`);
}

/** For hosts that read a `_headers` file (Netlify, Cloudflare Pages); others: OECA.md. */
/** Every header the host sends with every path (the policy with frame-ancestors). */
export function securityHeaders(scriptHashes: string[], signalingUrl: string): [string, string][] {
  return [
    ["Content-Security-Policy", policy(scriptHashes, signalingUrl, true)],
    ["X-Content-Type-Options", "nosniff"],
    ["Referrer-Policy", "no-referrer"],
    ["Cross-Origin-Opener-Policy", "same-origin"],
    ["Permissions-Policy", "camera=(), microphone=(), geolocation=(), payment=(), usb=(), serial=(), bluetooth=()"],
    // Every load asks the host (cheaply, by ETag): a fixed version reaches every
    // visitor on their next load, and nothing outdated is served from a cache
    ["Cache-Control", "no-cache"],
  ];
}

/** `_headers`, for hosts that read one (Netlify, Cloudflare Pages). */
export function headersFile(scriptHashes: string[], signalingUrl: string): string {
  return ["/*", ...securityHeaders(scriptHashes, signalingUrl).map(([k, v]) => `  ${k}: ${v}`), ""].join("\n");
}

/** `vercel.json`, for Vercel: the same headers, a static site with nothing to build. */
export function vercelConfig(scriptHashes: string[], signalingUrl: string): string {
  const headers = securityHeaders(scriptHashes, signalingUrl).map(([key, value]) => ({ key, value }));
  return `${JSON.stringify({ cleanUrls: false, trailingSlash: false, headers: [{ source: "/(.*)", headers }] }, null, 2)}\n`;
}
