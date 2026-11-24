import { decodeBase64, encodeBase64 } from "~/utils/base64";

type CryptoParams = {
  identifier: string;
  generate: AlgorithmIdentifier;
  import: AlgorithmIdentifier;
  sign: AlgorithmIdentifier;
};

const cryptoParams = {
  ed25519: {
    identifier: "ed25519",
    generate: { name: "Ed25519" },
    import: { name: "Ed25519" },
    sign: { name: "Ed25519" },
  },
  rsaPss: {
    identifier: "rsa-pss",
    generate: {
      name: "RSA-PSS",
      modulusLength: 2048,
      publicExponent: new Uint8Array([1, 0, 1]),
      hash: "SHA-256",
    },
    import: {
      name: "RSA-PSS",
      hash: "SHA-256",
    },
    sign: {
      name: "RSA-PSS",
      saltLength: 32,
    },
  },
};

// Older Chrome versions do not support Ed25519 in the WebCrypto API.
// https://developer.mozilla.org/en-US/docs/Web/API/SubtleCrypto/generateKey#browser_compatibility
let selectedParams: CryptoParams = cryptoParams.rsaPss;

export function isWebCryptoSupported(): boolean {
  return (
    window.crypto &&
    window.crypto.subtle &&
    "generateKey" in window.crypto.subtle
  );
}

export async function upgradeToEd25519IfSupported(): Promise<void> {
  try {
    await window.crypto.subtle.generateKey(
      cryptoParams.ed25519.generate,
      false,
      ["sign", "verify"],
    );
    selectedParams = cryptoParams.ed25519;
  } catch (e) {
    console.warn("Ed25519 not supported.");
  }
}

/**
 * Generate a key pair (publicKey + privateKey).
 * oeca: the private key cannot be exported (only the public one ever is).
 */
export async function generateKeyPair(): Promise<CryptoKeyPair> {
  return (await window.crypto.subtle.generateKey(
    selectedParams.generate,
    false,
    ["sign", "verify"],
  )) as CryptoKeyPair;
}

export async function cryptoKeyToPem(cryptoKey: CryptoKey): Promise<string> {
  const exported = await window.crypto.subtle.exportKey("spki", cryptoKey);
  const exportedAsBase64 = btoa(
    String.fromCharCode(...new Uint8Array(exported)),
  );

  // Format as PEM
  const pemHeader = "-----BEGIN PUBLIC KEY-----\n";
  const pemFooter = "\n-----END PUBLIC KEY-----";
  const pemBody = exportedAsBase64.match(/.{1,64}/g)!.join("\n"); // Wrap lines at 64 chars

  return pemHeader + pemBody + pemFooter;
}

export async function publicKeyFromDer(der: Uint8Array<ArrayBuffer>): Promise<CryptoKey> {
  return await window.crypto.subtle.importKey(
    "spki",
    der,
    selectedParams.import,
    true,
    ["verify"],
  );
}

export async function publicKeyFromPem(pem: string): Promise<CryptoKey> {
  // Remove PEM header and footer
  const pemBody = pem
    .split("\n")
    .filter((line) => !line.includes("-----"))
    .join("");

  // Decode the base64 body
  const der = new Uint8Array(
    atob(pemBody)
      .split("")
      .map((c) => c.charCodeAt(0)),
  );

  return publicKeyFromDer(der);
}

/**
 * Generates a client token with the following structure:
 *
 *  HASH_METHOD.HASH.SALT_BASE64.SIGN_METHOD.SIGN
 *
 *  1) HASH_METHOD (e.g. "sha256")
 *  2) HASH (base64-url-no-padding of sha256(publicKeyDER + SALT))
 *  3) SALT_BASE64 (base64-url-no-padding of 8 bytes = timestamp [u64])
 *  4) SIGN_METHOD (e.g. "ed25519")
 *  5) SIGN (base64-url-no-padding of Ed25519 signature over HASH)
 */
export async function generateClientTokenFromCurrentTimestamp(
  key: CryptoKeyPair,
): Promise<string> {
  const salt = unixTimestampU64();
  return await generateClientTokenFromNonce(key, salt);
}

export async function generateClientTokenFromNonce(
  key: CryptoKeyPair,
  nonce: Uint8Array,
): Promise<string> {
  const publicKeyDER = new Uint8Array(
    await window.crypto.subtle.exportKey("spki", key.publicKey),
  );
  const hashInput = concatBytes(publicKeyDER, nonce);
  const digest = await window.crypto.subtle.digest("SHA-256", hashInput);
  const signature = await window.crypto.subtle.sign(
    selectedParams.sign,
    key.privateKey,
    digest,
  );

  const hashMethod = "sha256";
  const hashBase64 = encodeBase64(new Uint8Array(digest));
  const saltBase64 = encodeBase64(nonce);
  const signMethod = selectedParams.identifier;
  const signatureBase64 = encodeBase64(new Uint8Array(signature));

  return [hashMethod, hashBase64, saltBase64, signMethod, signatureBase64].join(
    ".",
  );
}

export async function verifyToken(
  publicKey: CryptoKey,
  token: string,
): Promise<boolean> {
  const parts = token.split(".");
  if (parts.length !== 5) {
    return false;
  }

  const [hashMethod, hashB64, saltB64, signMethod, signB64] = parts;
  // oeca: an empty part fails below anyway; said here so the types know
  if (!hashB64 || !saltB64 || !signB64) {
    return false;
  }

  if (hashMethod !== "sha256") {
    return false;
  }

  if (signMethod !== selectedParams.identifier) {
    return false;
  }

  // Decode the salt (8 bytes, big-endian u64, in SECONDS)
  const saltBuffer = decodeBase64(saltB64);
  if (saltBuffer.byteLength !== 8) {
    return false;
  }
  const saltSeconds = Number(decodeU64(saltBuffer));

  const nowInSeconds = Math.floor(Date.now() / 1000);
  if (nowInSeconds - saltSeconds > 60 * 60) {
    // Fingerprint is older than 1h, reject
    return false;
  }

  const publicKeyDER = await crypto.subtle.exportKey("spki", publicKey);
  const hashInput = concatBytes(new Uint8Array(publicKeyDER), saltBuffer);

  // Recompute the SHA-256 hash
  const digest = await crypto.subtle.digest("SHA-256", hashInput);
  const recomputedHashB64 = encodeBase64(new Uint8Array(digest));

  if (recomputedHashB64 !== hashB64) {
    return false;
  }

  const signatureBuffer = decodeBase64(signB64);
  return await crypto.subtle.verify(
    selectedParams.sign,
    publicKey,
    signatureBuffer,
    digest,
  );
}

function unixTimestampU64(): Uint8Array {
  const nowInSeconds = Math.floor(Date.now() / 1000);
  return encodeU64(BigInt(nowInSeconds));
}

function encodeU64(value: bigint): Uint8Array {
  const saltBuffer = new ArrayBuffer(8);
  const saltView = new DataView(saltBuffer);
  saltView.setBigUint64(0, value, true);
  return new Uint8Array(saltBuffer);
}

function decodeU64(buffer: Uint8Array<ArrayBuffer>): bigint {
  const saltView = new DataView(typedArrayToBuffer(buffer));
  return saltView.getBigUint64(0, true);
}

function concatBytes(...arrays: Uint8Array[]): Uint8Array<ArrayBuffer> {
  const totalLength = arrays.reduce((acc, arr) => acc + arr.length, 0);
  const result = new Uint8Array(totalLength);
  let offset = 0;
  for (const arr of arrays) {
    result.set(arr, offset);
    offset += arr.length;
  }
  return result;
}

function typedArrayToBuffer(array: Uint8Array<ArrayBuffer>): ArrayBuffer {
  return array.buffer.slice(
    array.byteOffset,
    array.byteLength + array.byteOffset,
  );
}
