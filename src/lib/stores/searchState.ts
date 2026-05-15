import type { SearchHit, SearchResponse, SearchTarget } from '$lib/api';

export type SortKey =
  | 'popularity'
  | 'viewCounter'
  | 'commentCounter'
  | 'mylistCounter'
  | 'startTime'
  | 'lengthSeconds';

export type SearchState = {
  query: string;
  targets: SearchTarget[];
  sortField: SortKey;
  sortDir: 'asc' | 'desc';
  limit: number;
  response: SearchResponse | null;
  lastQuery: string | null;
  scrollY: number;
};

const KEY = 'nndd:lastSearch';

export function loadSearchState(): SearchState | null {
  if (typeof sessionStorage === 'undefined') return null;
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return null;
    return JSON.parse(raw) as SearchState;
  } catch {
    return null;
  }
}

export function saveSearchState(state: SearchState): void {
  if (typeof sessionStorage === 'undefined') return;
  try {
    sessionStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // quota or serialization error — silently ignore
  }
}

export function clearSearchState(): void {
  if (typeof sessionStorage === 'undefined') return;
  sessionStorage.removeItem(KEY);
}

/** Score used for client-side popularity ordering.
 *
 * NicoNico's official popularity ranking uses a time-decay formula that is not
 * exposed via the Snapshot Search API. We approximate it with a simple decay:
 * newer videos score higher, and mylist/comment ratios are weighted more than
 * raw view count (mirroring the official "you can see that a video with 100
 * mylists and 1000 views is more popular than one with 10000 views and 0 mylists"
 * philosophy).
 */
export function popularityScore(hit: SearchHit): number {
  const v = hit.viewCounter ?? 0;
  const m = hit.mylistCounter ?? 0;
  const c = hit.commentCounter ?? 0;
  const raw = v * 1 + m * 200 + c * 20;

  // Time decay: halve every ~60 days. Snapshot data is already daily-updated
  // so we just need relative ordering, not absolute accuracy.
  const nowSec = Math.floor(Date.now() / 1000);
  let postedSec = 0;
  if (hit.startTime) {
    postedSec = Math.floor(new Date(hit.startTime).getTime() / 1000);
  }
  const ageSec = Math.max(nowSec - postedSec, 0);
  const decay = 1 / (1 + ageSec / (60 * 86400));
  return raw * decay;
}

export function sortByPopularity(hits: SearchHit[]): SearchHit[] {
  return [...hits].sort((a, b) => popularityScore(b) - popularityScore(a));
}
