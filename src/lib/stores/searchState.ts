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

/** Score used for client-side popularity ordering. */
export function popularityScore(hit: SearchHit): number {
  const v = hit.viewCounter ?? 0;
  const m = hit.mylistCounter ?? 0;
  const c = hit.commentCounter ?? 0;
  return v + m * 10 + c * 5;
}

export function sortByPopularity(hits: SearchHit[]): SearchHit[] {
  return [...hits].sort((a, b) => popularityScore(b) - popularityScore(a));
}
