// webkit2gtk (Linux Tauri) is known to ship a buggy SubtleCrypto AES-CBC
// implementation: it succeeds the call but returns wrong plaintext, which
// makes hls.js's Decrypter think every fragment is malformed and emit
// `fragParsingError` / `transmuxer-interface flush error`. Forcing hls.js
// to use its bundled pure-JS AES (`enableSoftwareAES`) requires that
// `crypto.subtle` look unavailable at the time `new Hls(...)` runs.
//
// The override has to happen before the first Hls instantiation. Calling
// it more than once is safe; subsequent calls verify the previous patch
// is still in effect and re-apply if needed.

let applied = false;

export function disableSubtleCryptoOnce(): boolean {
  if (typeof globalThis === 'undefined') return false;
  const cryptoObj = (globalThis as { crypto?: Crypto }).crypto;
  if (!cryptoObj) return false;

  // Already overridden in this process.
  if (applied && cryptoObj.subtle == null) return true;

  let success = false;
  try {
    Object.defineProperty(cryptoObj, 'subtle', {
      configurable: true,
      get: () => undefined,
    });
    success = cryptoObj.subtle == null;
  } catch (e) {
    console.warn('[disableSubtleCrypto] defineProperty failed', e);
  }

  if (!success) {
    // Fallback: try a plain assignment (works in some hosts where the
    // prototype getter isn't fully locked down).
    try {
      (cryptoObj as { subtle?: unknown }).subtle = undefined;
      success = cryptoObj.subtle == null;
    } catch (e) {
      console.warn('[disableSubtleCrypto] direct assignment failed', e);
    }
  }

  if (success) {
    applied = true;
    console.info('[disableSubtleCrypto] crypto.subtle hidden — hls.js will use JS AES');
  } else {
    console.error(
      '[disableSubtleCrypto] could not hide crypto.subtle — fragParsingError likely persists',
    );
  }
  return success;
}
