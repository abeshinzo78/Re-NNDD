/**
 * ランキング NG 用の動画タグ取得ヘルパー。
 *
 * Rust の `fetch_video_tags` コマンドで `(name, isLocked)` のタグ情報を取る
 * (軽量な getthumbinfo XML を使い、読めなければ watch データから取る)。
 * 以前は watch ページ HTML を `@kongyo2/nicotag-api` で読んでいたが、2026 年に
 * 埋め込みが `$watchV4` 形へ変わって読めなくなった (失敗は「タグ未取得」扱いに
 * なるので、タグ系の NG ルールが黙って効かなくなっていた)。
 *
 * 結果はプロセス内 Map と localStorage に TTL 付きでキャッシュする。
 * 取得は呼び出し側が指定する並行度で並列化する (デフォルト 8)。
 */

import { fetchVideoTags } from '$lib/api';
import type { RankingTagInfo } from '$lib/stores/ngRules';

type CacheEntry = { tags: RankingTagInfo[]; fetchedAt: number };

const memCache = new Map<string, CacheEntry>();
const LS_KEY = 'nndd:rankingTagCache';
const TTL_MS = 24 * 60 * 60 * 1000; // 24h

let lsLoaded = false;
function loadLs(): void {
  if (lsLoaded) return;
  lsLoaded = true;
  if (typeof localStorage === 'undefined') return;
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (!raw) return;
    const parsed = JSON.parse(raw) as Record<string, CacheEntry>;
    const now = Date.now();
    for (const [id, entry] of Object.entries(parsed)) {
      if (entry && Array.isArray(entry.tags) && now - entry.fetchedAt < TTL_MS) {
        memCache.set(id, entry);
      }
    }
  } catch {
    // ignore corrupt cache
  }
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;
function persistLs(): void {
  if (typeof localStorage === 'undefined') return;
  if (saveTimer) return;
  saveTimer = setTimeout(() => {
    saveTimer = null;
    try {
      const obj: Record<string, CacheEntry> = {};
      for (const [id, entry] of memCache) obj[id] = entry;
      localStorage.setItem(LS_KEY, JSON.stringify(obj));
    } catch {
      // localStorage 容量超過などは無視
    }
  }, 500);
}

export function getCachedTags(videoId: string): RankingTagInfo[] | undefined {
  loadLs();
  const entry = memCache.get(videoId);
  if (!entry) return undefined;
  if (Date.now() - entry.fetchedAt > TTL_MS) {
    memCache.delete(videoId);
    return undefined;
  }
  return entry.tags;
}

async function fetchOne(videoId: string): Promise<RankingTagInfo[]> {
  const tags = await fetchVideoTags(videoId);
  return tags.map((t) => ({ name: t.name, isLocked: t.isLocked }));
}

export type FetchTagsOptions = {
  concurrency?: number;
  onProgress?: (done: number, total: number) => void;
  signal?: AbortSignal;
};

/**
 * 与えられた動画 ID 群のタグをまとめて取得する。キャッシュ済みは即返す。
 * 戻り値は `id -> tags` の Map。失敗した ID は `undefined` のままになる。
 */
export async function fetchTagsBulk(
  videoIds: ReadonlyArray<string>,
  opts: FetchTagsOptions = {},
): Promise<Map<string, RankingTagInfo[] | undefined>> {
  loadLs();
  const concurrency = Math.max(1, opts.concurrency ?? 8);
  const result = new Map<string, RankingTagInfo[] | undefined>();
  const toFetch: string[] = [];

  for (const id of videoIds) {
    const cached = getCachedTags(id);
    if (cached) {
      result.set(id, cached);
    } else {
      toFetch.push(id);
      result.set(id, undefined);
    }
  }

  const total = videoIds.length;
  let done = total - toFetch.length;
  opts.onProgress?.(done, total);

  let idx = 0;
  async function worker() {
    while (idx < toFetch.length) {
      if (opts.signal?.aborted) return;
      const my = idx++;
      const id = toFetch[my];
      try {
        const tags = await fetchOne(id);
        memCache.set(id, { tags, fetchedAt: Date.now() });
        result.set(id, tags);
      } catch {
        // 失敗時は undefined のまま (= 「タグ未取得」 = タグ系ルールはスキップ)
      }
      done++;
      opts.onProgress?.(done, total);
    }
  }

  const workers = Array.from({ length: Math.min(concurrency, toFetch.length) }, () => worker());
  await Promise.all(workers);
  persistLs();
  return result;
}

/** デバッグ用 — キャッシュ全クリア。 */
export function clearTagCache(): void {
  memCache.clear();
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.removeItem(LS_KEY);
    } catch {
      // ignore
    }
  }
}
