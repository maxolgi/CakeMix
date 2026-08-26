// Shared WebSRT gateway discovery — cert-hash.js resolution for both the
// receive (./store.ts) and publish (./publish.ts) paths. Canonical
// implementation lives here so the two stores cannot drift (publish's
// former private copy only did same-origin fetches and could not target
// remote gateways).

/** Fetch + parse a gateway's cert-hash.js. Same origin when `origin` is
 *  omitted; a remote origin goes through OUR server's /api/cert-hash proxy
 *  (embedding.md "Delivering the cert hash cross-origin" — the browser
 *  cannot fetch another origin's cert-hash.js directly, CORS). Shape
 *  (written by websrt-gateway/src/main.rs):
 *      window.CERT_HASH = "<64 hex chars>";   …or null (mkcert/PKI mode)
 *      window.WT_PORT = 4433; */
export interface CertHashInfo {
  certHashHex: string | null;
  wtPort: number;
}

export async function resolveCertHash(origin?: string): Promise<CertHashInfo> {
  if (!origin) {
    const resp = await fetch("/cert-hash.js", { cache: "no-store" });
    if (!resp.ok) {
      throw new Error(`No cert-hash.js (HTTP ${resp.status}) — is the gateway running?`);
    }
    const text = await resp.text();
    return parseCertHashJs(text);
  }
  const resp = await fetch(`/api/cert-hash?url=${encodeURIComponent(origin)}`, { cache: "no-store" });
  const j = await resp.json().catch(() => null);
  if (!j) throw new Error(`proxy response not JSON (HTTP ${resp.status})`);
  if (j.error) throw new Error(String(j.error));
  return {
    certHashHex: typeof j.hash === "string" && j.hash.length === 64 ? j.hash : null,
    wtPort: Number(j.wtPort) || 4433,
  };
}

export function parseCertHashJs(text: string): CertHashInfo {
  const hash = text.match(/window\.CERT_HASH\s*=\s*(?:"([^"]*)"|null)/);
  const port = text.match(/window\.WT_PORT\s*=\s*(\d+)/);
  if (!hash || !port) {
    throw new Error("cert-hash.js is not parseable");
  }
  return { certHashHex: hash[1] ?? null, wtPort: parseInt(port[1], 10) };
}

/** Hex → 32 bytes, tolerant of ':' / whitespace separators.
 *  Copied from vendor/WebSRT/web/src/shared/viewer.ts hexToBytes. */
export function hexToBytes(hex: string): Uint8Array {
  const clean = hex.replace(/[:\s]/g, "");
  if (clean.length !== 64) {
    throw new Error(`expected 32-byte (64 hex char) hash, got ${clean.length} hex chars`);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    out[i] = parseInt(clean.substring(i * 2, i * 2 + 2), 16);
  }
  return out;
}
