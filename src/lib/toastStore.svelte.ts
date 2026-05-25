// アプリ全体のトースト通知ストア。
//
// プラグイン機構の `notify.toast` (= Rust dispatcher 経由) と
// `ctx.ui.toast()` (= フロント直接) の両経路から呼ばれ、UI コンポーネント
// `Toast.svelte` がこのストアを購読して画面に表示する。
//
// 既存の SearchHitCard.svelte 内ローカルトースト等とは独立。一度に複数件
// 出せる stack 形式とし、各トーストは `durationMs` 後に自動消去する。

import { SvelteMap } from 'svelte/reactivity';

export type ToastKind = 'info' | 'ok' | 'warn' | 'error';

export type Toast = {
  id: number;
  message: string;
  kind: ToastKind;
  /** plugin が発行したトーストかどうか (UI で「プラグイン: <id>」と表示する用)。
   *  null なら host 自身が出したトースト。 */
  pluginId: string | null;
  createdAt: number;
};

// SvelteMap で reactive に追跡。Toast.svelte が `[...toasts.values()]` で
// 配列に変換して each で描画する。同時表示の最大数は実用上 5 件で十分。
const toasts = new SvelteMap<number, Toast>();
const MAX_TOASTS = 5;
const DEFAULT_DURATION_MS = 3500;

let nextId = 1;

/** トーストを 1 件表示。`pluginId` を渡すと「プラグイン: <id>」付きで描画される。
 *  デフォルト kind は `info`。 */
export function showToast(
  message: string,
  kind: ToastKind = 'info',
  options?: { pluginId?: string | null; durationMs?: number },
): number {
  const id = nextId++;
  const toast: Toast = {
    id,
    message,
    kind,
    pluginId: options?.pluginId ?? null,
    createdAt: Date.now(),
  };
  // 上限超過なら最古を捨てる。Map は insertion order を保つので最初の key が最古。
  if (toasts.size >= MAX_TOASTS) {
    const oldest = toasts.keys().next().value;
    if (oldest != null) toasts.delete(oldest);
  }
  toasts.set(id, toast);
  const dur = options?.durationMs ?? DEFAULT_DURATION_MS;
  if (dur > 0 && typeof setTimeout !== 'undefined') {
    setTimeout(() => {
      toasts.delete(id);
    }, dur);
  }
  return id;
}

export function dismissToast(id: number): void {
  toasts.delete(id);
}

/** Toast.svelte が購読する getter。$derived で使う。 */
export function listToasts(): Toast[] {
  return [...toasts.values()];
}

/** テスト用: 全クリア。 */
export function _resetForTests(): void {
  toasts.clear();
  nextId = 1;
}

/** テスト用: 現在のトースト件数。 */
export function _count(): number {
  return toasts.size;
}
