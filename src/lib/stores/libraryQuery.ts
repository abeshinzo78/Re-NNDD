/**
 * Library search UI state — the front-end half of the Phase 1.3 query layer.
 *
 * `src-tauri/src/library/query.rs` already supports sorting, pagination,
 * tag / uploader / resolution / duration filters and a comment FTS search.
 * This module owns the *UI-side* representation of those knobs and the pure
 * conversion into {@link LibraryQueryParams}, so the page component stays a
 * thin rendering layer and the logic can be unit-tested without a DOM.
 *
 * 状態は sessionStorage に保存し、動画詳細から戻ってきた時に検索条件と
 * ページ位置を復元する (検索ページの `searchState` と同じ方針)。
 */

import type { LibraryQueryParams } from '$lib/api';
import type { SmartPlaylistFilter } from './smartPlaylists';

/** 動画検索 / 保存コメントの横断検索。 */
export type LibrarySearchMode = 'videos' | 'comments';

/** `query.rs` の `ALLOWED_SORT` のうち UI に出す並び順。
 *  `is_short` は並び替えとして意味が薄いので除外している。 */
export const LIBRARY_SORT_KEYS = [
  'downloaded_at',
  'posted_at',
  'title',
  'view_count',
  'mylist_count',
  'comment_count',
  'duration_sec',
  'play_count',
  'last_played_at',
  'random',
] as const;

export type LibrarySortKey = (typeof LIBRARY_SORT_KEYS)[number];

export type LibrarySortOption = {
  value: LibrarySortKey;
  label: string;
};

export const LIBRARY_SORT_OPTIONS: readonly LibrarySortOption[] = [
  { value: 'downloaded_at', label: 'DL 日時' },
  { value: 'posted_at', label: '投稿日' },
  { value: 'title', label: 'タイトル' },
  { value: 'view_count', label: '再生数' },
  { value: 'mylist_count', label: 'マイリスト数' },
  { value: 'comment_count', label: 'コメント数' },
  { value: 'duration_sec', label: '再生時間' },
  { value: 'play_count', label: 'ライブラリ再生回数' },
  { value: 'last_played_at', label: '最終再生' },
  { value: 'random', label: 'ランダム' },
];

export function isLibrarySortKey(v: unknown): v is LibrarySortKey {
  return typeof v === 'string' && (LIBRARY_SORT_KEYS as readonly string[]).includes(v);
}

/** 1 ページあたりの件数の候補。`query.rs` 側の上限 500 を超えない範囲。 */
export const LIBRARY_PAGE_SIZES = [24, 48, 96, 200] as const;

/** コメント FTS (trigram) が引けるのは 3 文字以上。`search_library_comments`
 *  コマンドも 3 文字未満をエラーで弾くので、UI 側でも同じ閾値で抑止する。 */
export const COMMENT_SEARCH_MIN_CHARS = 3;

export type LibraryFilterState = {
  mode: LibrarySearchMode;
  /** フリーワード (タイトル / タグ / コメント FTS)。 */
  q: string;
  /** AND 条件のタグ。 */
  tags: string[];
  /** 空文字 = 絞り込みなし。 */
  uploaderId: string;
  /** 空文字 = 絞り込みなし。`"1280x720"` のような完全一致値。 */
  resolution: string;
  /** true の時だけショート動画に限定 (false は「絞り込みなし」)。 */
  isShort: boolean;
  /** 分単位の入力文字列。空文字 = 未指定。 */
  minMinutes: string;
  maxMinutes: string;
  sortBy: LibrarySortKey;
  sortOrder: 'asc' | 'desc';
  pageSize: number;
  /** 0 始まりのページ番号。 */
  page: number;
};

export const DEFAULT_LIBRARY_FILTER: LibraryFilterState = {
  mode: 'videos',
  q: '',
  tags: [],
  uploaderId: '',
  resolution: '',
  isShort: false,
  minMinutes: '',
  maxMinutes: '',
  sortBy: 'downloaded_at',
  sortOrder: 'desc',
  pageSize: 48,
  page: 0,
};

/** 分入力 → 秒。空 / 数値でない / 負値は `undefined` (＝条件なし)。 */
export function minutesToSeconds(input: string): number | undefined {
  const trimmed = input.trim();
  if (!trimmed) return undefined;
  const minutes = Number(trimmed);
  if (!Number.isFinite(minutes) || minutes < 0) return undefined;
  return Math.round(minutes * 60);
}

/** 秒 → 分入力用の文字列。割り切れない時は小数 1 桁まで。 */
export function secondsToMinutes(seconds: number | undefined | null): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return '';
  const minutes = seconds / 60;
  return Number.isInteger(minutes) ? String(minutes) : minutes.toFixed(1);
}

/**
 * フィルタ状態を更新する。`page` 以外が 1 つでも変わったら 0 ページ目へ戻す
 * — 3 ページ目で条件を変えた時に「0 件」に見える事故を防ぐため。
 */
export function patchLibraryFilter(
  state: LibraryFilterState,
  patch: Partial<LibraryFilterState>,
): LibraryFilterState {
  const next = { ...state, ...patch };
  const touchesOnlyPage = Object.keys(patch).every((k) => k === 'page');
  if (!touchesOnlyPage) next.page = patch.page ?? 0;
  return next;
}

/** タグを AND 条件に追加する (重複と空白は無視)。 */
export function addTagFilter(state: LibraryFilterState, tag: string): LibraryFilterState {
  const name = tag.trim();
  if (!name || state.tags.includes(name)) return state;
  return patchLibraryFilter(state, { tags: [...state.tags, name] });
}

export function removeTagFilter(state: LibraryFilterState, tag: string): LibraryFilterState {
  if (!state.tags.includes(tag)) return state;
  return patchLibraryFilter(state, { tags: state.tags.filter((t) => t !== tag) });
}

/** フリーワードと並び順・ページ設定を保ったまま、絞り込みだけ解除する。 */
export function clearLibraryFilters(state: LibraryFilterState): LibraryFilterState {
  return patchLibraryFilter(state, {
    tags: [],
    uploaderId: '',
    resolution: '',
    isShort: false,
    minMinutes: '',
    maxMinutes: '',
  });
}

/** 「絞り込み中」バッジ用。フリーワード・並び順・ページは数えない。 */
export function activeFilterCount(state: LibraryFilterState): number {
  let n = state.tags.length;
  if (state.uploaderId) n++;
  if (state.resolution) n++;
  if (state.isShort) n++;
  if (minutesToSeconds(state.minMinutes) != null) n++;
  if (minutesToSeconds(state.maxMinutes) != null) n++;
  return n;
}

/** `query_library_videos` に渡すパラメータへ変換する。
 *  未指定の項目は **キーごと落とす** — Rust 側は `Option` なので
 *  `undefined` を送っても同じだが、IPC のペイロードを小さく保つ。 */
export function buildLibraryQuery(state: LibraryFilterState): LibraryQueryParams {
  const params: LibraryQueryParams = {
    sortBy: state.sortBy,
    sortOrder: state.sortOrder,
    limit: state.pageSize,
    offset: state.page * state.pageSize,
  };
  const q = state.q.trim();
  if (q) params.q = q;
  if (state.tags.length > 0) params.tags = [...state.tags];
  if (state.uploaderId) params.uploaderId = state.uploaderId;
  if (state.resolution) params.resolution = state.resolution;
  if (state.isShort) params.isShort = true;
  const min = minutesToSeconds(state.minMinutes);
  if (min != null) params.minDuration = min;
  const max = minutesToSeconds(state.maxMinutes);
  if (max != null) params.maxDuration = max;
  return params;
}

/** スマートプレイリスト (オンライン検索) 用のフィルタへ変換する。
 *  解像度 / ショート / ローカル専用の並び順はオンライン検索に無いので
 *  ここでは渡さない (`normalizeFilter` 側でも落ちる)。 */
export function toSmartPlaylistFilter(state: LibraryFilterState): SmartPlaylistFilter {
  const filter: SmartPlaylistFilter = {};
  const q = state.q.trim();
  if (q) filter.q = q;
  if (state.tags.length > 0) filter.tags = [...state.tags];
  if (state.uploaderId) filter.uploaderId = state.uploaderId;
  const min = minutesToSeconds(state.minMinutes);
  if (min != null) filter.minDuration = min;
  const max = minutesToSeconds(state.maxMinutes);
  if (max != null) filter.maxDuration = max;
  filter.sortBy = state.sortBy;
  filter.sortOrder = state.sortOrder;
  return filter;
}

/** コメント検索を投げてよいか (3 文字未満はバックエンドがエラーにする)。 */
export function canSearchComments(q: string): boolean {
  return Array.from(q.trim()).length >= COMMENT_SEARCH_MIN_CHARS;
}

/** コメント検索ヒット → ライブラリ動画ページの URL。
 *  `?t=` はそのコメントが流れる位置 (秒) で、遷移先はそこから再生する。 */
export function commentHitHref(videoId: string, vposMs: number): string {
  const sec = Number.isFinite(vposMs) ? Math.max(0, Math.floor(vposMs / 1000)) : 0;
  return `/library/${encodeURIComponent(videoId)}?from=library&t=${sec}`;
}

/** `?t=` を秒へ。未指定 / 不正値 / 負値は `null` (＝頭出ししない)。 */
export function parseStartTime(raw: string | null): number | null {
  if (raw == null || raw.trim() === '') return null;
  const t = Number(raw);
  if (!Number.isFinite(t) || t < 0) return null;
  return Math.floor(t);
}

/** 「120 件中 49–96 件」表記。0 件なら「0 件」。 */
export function resultRangeLabel(totalCount: number, offset: number, shown: number): string {
  if (totalCount <= 0 || shown <= 0) return '0 件';
  const from = offset + 1;
  const to = offset + shown;
  return `${totalCount.toLocaleString('ja-JP')} 件中 ${from.toLocaleString('ja-JP')}–${to.toLocaleString('ja-JP')} 件`;
}

/** 総ページ数 (最低 1)。 */
export function pageCount(totalCount: number, pageSize: number): number {
  if (pageSize <= 0) return 1;
  return Math.max(1, Math.ceil(Math.max(0, totalCount) / pageSize));
}

/** 総件数が減った時に、範囲外のページへ取り残されないよう丸める。 */
export function clampPage(state: LibraryFilterState, totalCount: number): LibraryFilterState {
  const last = pageCount(totalCount, state.pageSize) - 1;
  if (state.page <= last) return state;
  return { ...state, page: Math.max(0, last) };
}

/** 検索条件の短い説明。スマートプレイリストの既定名などに使う。 */
export function summarizeLibraryFilter(state: LibraryFilterState): string {
  const parts: string[] = [];
  const q = state.q.trim();
  if (q) parts.push(`"${q}"`);
  if (state.tags.length > 0) parts.push(`タグ: ${state.tags.join(' + ')}`);
  if (state.uploaderId) parts.push(`投稿者: ${state.uploaderId}`);
  if (state.resolution) parts.push(state.resolution);
  if (state.isShort) parts.push('ショート');
  const min = minutesToSeconds(state.minMinutes);
  const max = minutesToSeconds(state.maxMinutes);
  if (min != null && max != null) parts.push(`${state.minMinutes}〜${state.maxMinutes} 分`);
  else if (min != null) parts.push(`${state.minMinutes} 分以上`);
  else if (max != null) parts.push(`${state.maxMinutes} 分以下`);
  return parts.length === 0 ? '条件なし' : parts.join(' / ');
}

// ---------------------------------------------------------------------------
// 永続化 (sessionStorage)
// ---------------------------------------------------------------------------

const KEY = 'nndd:libraryQuery';

function toStringArray(v: unknown): string[] {
  if (!Array.isArray(v)) return [];
  const cleaned = v.filter((x): x is string => typeof x === 'string' && x.trim().length > 0);
  return Array.from(new Set(cleaned.map((x) => x.trim())));
}

function toNumericInput(v: unknown): string {
  if (typeof v === 'number' && Number.isFinite(v) && v >= 0) return String(v);
  if (typeof v !== 'string') return '';
  return minutesToSeconds(v) == null ? '' : v.trim();
}

/** 壊れた / 古い保存値でも必ず妥当な状態にする。 */
export function normalizeLibraryFilter(raw: unknown): LibraryFilterState {
  if (!raw || typeof raw !== 'object') return { ...DEFAULT_LIBRARY_FILTER };
  const r = raw as Record<string, unknown>;
  const pageSize = LIBRARY_PAGE_SIZES.includes(r.pageSize as (typeof LIBRARY_PAGE_SIZES)[number])
    ? (r.pageSize as number)
    : DEFAULT_LIBRARY_FILTER.pageSize;
  const page =
    typeof r.page === 'number' && Number.isFinite(r.page) && r.page >= 0 ? Math.floor(r.page) : 0;
  return {
    mode: r.mode === 'comments' ? 'comments' : 'videos',
    q: typeof r.q === 'string' ? r.q : '',
    tags: toStringArray(r.tags),
    uploaderId: typeof r.uploaderId === 'string' ? r.uploaderId.trim() : '',
    resolution: typeof r.resolution === 'string' ? r.resolution.trim() : '',
    isShort: r.isShort === true,
    minMinutes: toNumericInput(r.minMinutes),
    maxMinutes: toNumericInput(r.maxMinutes),
    sortBy: isLibrarySortKey(r.sortBy) ? r.sortBy : DEFAULT_LIBRARY_FILTER.sortBy,
    sortOrder: r.sortOrder === 'asc' ? 'asc' : 'desc',
    pageSize,
    page,
  };
}

export function loadLibraryFilter(): LibraryFilterState {
  if (typeof sessionStorage === 'undefined') return { ...DEFAULT_LIBRARY_FILTER };
  try {
    const raw = sessionStorage.getItem(KEY);
    if (!raw) return { ...DEFAULT_LIBRARY_FILTER };
    return normalizeLibraryFilter(JSON.parse(raw));
  } catch {
    return { ...DEFAULT_LIBRARY_FILTER };
  }
}

export function saveLibraryFilter(state: LibraryFilterState): void {
  if (typeof sessionStorage === 'undefined') return;
  try {
    sessionStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    // quota / serialization error — 検索条件は捨てても致命的ではない
  }
}

export function clearLibraryFilter(): void {
  if (typeof sessionStorage === 'undefined') return;
  sessionStorage.removeItem(KEY);
}
