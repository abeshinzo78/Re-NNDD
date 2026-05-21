/**
 * Playback queue store — drives "オートプレイ" (autoplay) across the
 * `/video/[id]` and `/library/[id]` pages.
 *
 * When a queue is active, the video page navigates to the next item once
 * the current video ends (and the user hasn't enabled per-video loop).
 *
 * Persisted in localStorage so a full reload / new tab keeps the queue
 * intact — this matches how series / mylist pages spawn the queue and the
 * user expects "play all" to survive incidental refreshes.
 */

const KEY = 'nndd:playback-queue';

export type QueueSource = 'online' | 'local';

export type QueueContext = 'series' | 'mylist' | 'smart' | 'library' | 'user' | 'manual';

export type PlaybackQueueItem = {
  videoId: string;
  title?: string;
  thumbnailUrl?: string;
  /** どの route で再生するかの既定値。 'online' = /video/[id], 'local' = /library/[id]. */
  source: QueueSource;
  lengthSeconds?: number;
};

export type PlaybackQueue = {
  context: QueueContext;
  /** Series ID / Mylist ID / Smart playlist ID / etc. UI 表示には使わない。 */
  contextId: string;
  /** UI 表示用のラベル (例: シリーズタイトル / マイリスト名 / スマートプレイリスト名)。 */
  label: string;
  items: PlaybackQueueItem[];
  /** Current index. -1 means "not yet started". */
  index: number;
  /** 作成タイムスタンプ (ms)。 */
  createdAt: number;
};

const listeners = new Set<() => void>();

function notify(): void {
  for (const fn of listeners) fn();
}

export function subscribeQueue(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function getQueue(): PlaybackQueue | null {
  if (typeof localStorage === 'undefined') return null;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PlaybackQueue;
    if (!parsed || !Array.isArray(parsed.items)) return null;
    return parsed;
  } catch {
    return null;
  }
}

function writeQueue(q: PlaybackQueue | null): void {
  if (typeof localStorage === 'undefined') return;
  try {
    if (q == null) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, JSON.stringify(q));
  } catch {
    /* quota / private mode — silently ignore */
  }
  notify();
}

/** Replace the queue and return it. The first item is the "current" one. */
export function setQueue(
  context: QueueContext,
  contextId: string,
  label: string,
  items: PlaybackQueueItem[],
  startIndex = 0,
): PlaybackQueue | null {
  const cleaned = items.filter((it) => !!it.videoId);
  if (cleaned.length === 0) {
    clearQueue();
    return null;
  }
  const idx = Math.max(0, Math.min(cleaned.length - 1, startIndex));
  const q: PlaybackQueue = {
    context,
    contextId,
    label,
    items: cleaned,
    index: idx,
    createdAt: Date.now(),
  };
  writeQueue(q);
  return q;
}

export function clearQueue(): void {
  writeQueue(null);
}

/** Update the queue index so it points at `videoId`. No-op if not in queue. */
export function setQueueIndexByVideoId(videoId: string): PlaybackQueue | null {
  const q = getQueue();
  if (!q) return null;
  const idx = q.items.findIndex((it) => it.videoId === videoId);
  if (idx < 0) return q;
  if (idx === q.index) return q;
  q.index = idx;
  writeQueue(q);
  return q;
}

export function currentItem(q: PlaybackQueue | null = getQueue()): PlaybackQueueItem | null {
  if (!q) return null;
  return q.items[q.index] ?? null;
}

export function nextItem(q: PlaybackQueue | null = getQueue()): PlaybackQueueItem | null {
  if (!q) return null;
  return q.items[q.index + 1] ?? null;
}

export function prevItem(q: PlaybackQueue | null = getQueue()): PlaybackQueueItem | null {
  if (!q) return null;
  if (q.index <= 0) return null;
  return q.items[q.index - 1] ?? null;
}

/** Advance the queue. Returns the new current item, or null if at the end. */
export function advanceQueue(): PlaybackQueueItem | null {
  const q = getQueue();
  if (!q) return null;
  if (q.index + 1 >= q.items.length) return null;
  q.index += 1;
  writeQueue(q);
  return q.items[q.index] ?? null;
}

export function rewindQueue(): PlaybackQueueItem | null {
  const q = getQueue();
  if (!q) return null;
  if (q.index <= 0) return null;
  q.index -= 1;
  writeQueue(q);
  return q.items[q.index] ?? null;
}

/** Compute the route for an item respecting its source preference. */
export function itemHref(item: PlaybackQueueItem, withQueueContext = true): string {
  const base = item.source === 'local' ? `/library/${item.videoId}` : `/video/${item.videoId}`;
  return withQueueContext ? `${base}?from=queue` : base;
}

/** Whether `videoId` is in the active queue and has a successor.
 *  Used to override `playback.always_loop` while a queue is advancing — the
 *  user's explicit 連続再生 action should take precedence over the global
 *  loop preference for queued items. */
export function hasNextInQueue(videoId: string): boolean {
  const q = getQueue();
  if (!q) return false;
  const idx = q.items.findIndex((it) => it.videoId === videoId);
  if (idx < 0) return false;
  return idx + 1 < q.items.length;
}
