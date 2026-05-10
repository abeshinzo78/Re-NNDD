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

function decodeBase64(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
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
        const resource = await fetchHlsResource(context.url, context.rangeStart, context.rangeEnd);
        if (this.aborted) {
          callbacks.onAbort?.(this.stats, context, null);
          return;
        }

        const bytes = decodeBase64(resource.dataBase64);
        this.stats.loaded = bytes.byteLength;
        this.stats.total = bytes.byteLength;
        this.stats.chunkCount = 1;
        this.stats.loading.first = this.stats.loading.first || performance.now();
        this.stats.loading.end = performance.now();

        const data =
          context.responseType === 'arraybuffer'
            ? (bytes.buffer.slice(
                bytes.byteOffset,
                bytes.byteOffset + bytes.byteLength,
              ) as ArrayBuffer)
            : decodeUtf8(bytes);

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

        callbacks.onSuccess(
          { url: context.url, data, code: resource.status },
          this.stats,
          context,
          {
            contentType: resource.contentType,
          },
        );
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
