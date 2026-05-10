// Forwards WebView console.* output to the Rust `web_log` Tauri command
// so it lands in /tmp/tauri-dev.log alongside Rust-side tracing. Useful
// when WebKit DevTools is unavailable (e.g. webkit2gtk inspector disabled).
//
// The original console behavior is preserved so DevTools (when reachable)
// still works as expected.

import { invoke } from '@tauri-apps/api/core';
import { disableSubtleCryptoOnce } from './player/disableSubtleCrypto';

type Level = 'log' | 'debug' | 'info' | 'warn' | 'error';

function format(args: unknown[]): string {
  return args
    .map((value) => {
      if (typeof value === 'string') return value;
      if (value instanceof Error) return `${value.name}: ${value.message}`;
      try {
        return JSON.stringify(value);
      } catch {
        return String(value);
      }
    })
    .join(' ');
}

let installed = false;

export function installConsoleBridge() {
  if (installed) return;
  installed = true;

  // Hide webkit2gtk's broken SubtleCrypto AES-CBC at app boot — before any
  // hls.js Decrypter is constructed. `enableSoftwareAES: true` alone does
  // not help here: WebCrypto returns wrong plaintext WITHOUT throwing, so
  // hls.js never knows to use the JS AES fallback.
  disableSubtleCryptoOnce();

  const levels: Level[] = ['log', 'debug', 'info', 'warn', 'error'];
  for (const level of levels) {
    const original = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      original(...args);
      // Fire and forget; ignore failures so logging never breaks the app.
      void invoke('web_log', { level, message: format(args) }).catch(() => undefined);
    };
  }

  // Surface uncaught errors / promise rejections too — these are the most
  // common things lost when DevTools is unavailable.
  window.addEventListener('error', (event) => {
    void invoke('web_log', {
      level: 'error',
      message: `[uncaught] ${event.message} @ ${event.filename}:${event.lineno}:${event.colno}`,
    }).catch(() => undefined);
  });
  window.addEventListener('unhandledrejection', (event) => {
    const reason =
      event.reason instanceof Error
        ? (event.reason.stack ?? event.reason.message)
        : String(event.reason);
    void invoke('web_log', {
      level: 'error',
      message: `[unhandledrejection] ${reason}`,
    }).catch(() => undefined);
  });
}
