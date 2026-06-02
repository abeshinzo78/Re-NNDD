import { fetchHlsResource } from '$lib/api';
import type {
  Loader,
  LoaderCallbacks,
  LoaderConfiguration,
  LoaderContext,
  LoaderStats,
} from 'hls.js';

function emptyStats(): LoaderStats {
  return {
    aborted: false,
    loaded: 0,
    retry: 0,
    total: 0,
    chunkCount: 0,
    bwEstimate: 0,
    loading: { start: 0, first: 0, end: 0 },
    parsing: { start: 0, end: 0 },
    buffering: { start: 0, first: 0, end: 0 },
  };
}

function decodeUtf8(bytes: Uint8Array): string {
  return new TextDecoder('utf-8').decode(bytes);
}

function classifyUrl(url: string, size: number): string {
  if (size === 16) return 'aes-key';
  if (url.includes('/init') || url.includes('init.cmfv')) return 'init-segment';
  if (url.includes('.cmfv') || url.includes('/seg')) return 'media-segment';
  if (url.includes('.m3u8')) return 'playlist';
  return 'other';
}

function hexHead(bytes: Uint8Array, n: number): string {
  return [...bytes.slice(0, n)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

export class TauriHlsLoader implements Loader<LoaderContext> {
  context: LoaderContext | null = null;
  stats: LoaderStats = emptyStats();
  private aborted = false;

  destroy() {
    this.abort();
  }

  abort() {
    this.aborted = true;
    this.stats.aborted = true;
  }

  load(
    context: LoaderContext,
    _config: LoaderConfiguration,
    callbacks: LoaderCallbacks<LoaderContext>,
  ) {
    this.context = context;
    this.aborted = false;
    this.stats = emptyStats();
    this.stats.loading.start = performance.now();

    void (async () => {
      try {
        const buffer = await fetchHlsResource(context.url, context.rangeStart, context.rangeEnd);
        if (this.aborted) {
          callbacks.onAbort?.(this.stats, context, null);
          return;
        }

        // `buffer` is the exact response body (a tauri::ipc::Response on the
        // Rust side), so the Uint8Array view covers it 1:1 — no slicing needed.
        const bytes = new Uint8Array(buffer);
        this.stats.loaded = bytes.byteLength;
        this.stats.total = bytes.byteLength;
        this.stats.chunkCount = 1;
        this.stats.loading.first = this.stats.loading.first || performance.now();
        this.stats.loading.end = performance.now();

        const data = context.responseType === 'arraybuffer' ? buffer : decodeUtf8(bytes);

        const kind = classifyUrl(context.url, bytes.byteLength);

        console.debug(
          `[TauriHlsLoader] OK kind=${kind} bytes=${bytes.byteLength} ` +
            `firstHex=${hexHead(bytes, 16)} respType=${context.responseType} ` +
            `url=${context.url.slice(-80)}`,
        );

        // AES key must be exactly 16 bytes in an ArrayBuffer — verify.
        if (kind === 'aes-key' && bytes.byteLength !== 16) {
          console.warn(
            `[TauriHlsLoader] unexpected AES key size: ${bytes.byteLength} (expected 16)`,
          );
        }

        // Raw-byte responses carry no HTTP status/headers, so report a
        // synthetic 200 on success (failures throw and are handled below) and
        // pass no networkDetails. hls.js consumes `data` directly and infers
        // the payload kind from responseType, not from a Content-Type header.
        callbacks.onSuccess({ url: context.url, data, code: 200 }, this.stats, context, null);
      } catch (e) {
        this.stats.loading.end = performance.now();
        if (this.aborted) {
          callbacks.onAbort?.(this.stats, context, null);
          return;
        }
        callbacks.onError(
          { code: 0, text: e instanceof Error ? e.message : String(e) },
          context,
          null,
          this.stats,
        );
      }
    })();
  }
}
