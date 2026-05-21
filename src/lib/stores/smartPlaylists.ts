/**
 * Smart playlist store — saved filter conditions over the niconico
 * snapshot search.
 *
 * A smart playlist is a *recipe*, not a static list of videos. Each time
 * the user opens one we re-run the niconico Snapshot Search API with the
 * saved filter, so the result reflects the latest catalog state on
 * niconico (新着動画やランキング変動が即時反映される)。
 *
 * Persisted in localStorage. The filter shape mirrors what the UI editor
 * captures; conversion to the Snapshot Search query (with required q /
 * targets / filters / sort) happens in `filterToSearchQuery`.
 *
 * NOTE: 旧実装はローカルライブラリのみを対象に `queryLibraryVideos` を
 * 叩いていたが、ユーザの期待はオンラインから取得することなので
 * Snapshot Search ベースに切り替えた。`filterToQueryParams` はテスト/
 * 後方互換のため残してあるが、本体ロジックでは使われていない。
 */

import type {
  FilterClause,
  LibraryQueryParams,
  SearchField,
  SearchQuery,
  SearchTarget,
  SortSpec,
} from '$lib/api';

const KEY = 'nndd:smart-playlists';

export type SmartPlaylistFilter = {
  /** 自由文検索 (タイトル / タグ / コメント FTS にマッチ)。 */
  q?: string;
  /** AND: 全タグ必須。 */
  tags?: string[];
  /** OR: いずれかのタグを含む。 */
  tagsAny?: string[];
  uploaderId?: string;
  minDuration?: number;
  maxDuration?: number;
  resolution?: string;
  sortBy?: string;
  sortOrder?: 'asc' | 'desc';
  /** 表示件数上限。 */
  limit?: number;
};

export type SmartPlaylist = {
  id: string;
  name: string;
  /** 任意の備考。 */
  description?: string;
  createdAt: number;
  updatedAt: number;
  filter: SmartPlaylistFilter;
};

const listeners = new Set<() => void>();

function notify(): void {
  for (const fn of listeners) fn();
}

export function subscribeSmartPlaylists(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function read(): SmartPlaylist[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as SmartPlaylist[];
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((p) => p && typeof p.id === 'string' && typeof p.name === 'string');
  } catch {
    return [];
  }
}

function write(list: SmartPlaylist[]): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(KEY, JSON.stringify(list));
  } catch {
    /* quota — ignore */
  }
  notify();
}

export function listSmartPlaylists(): SmartPlaylist[] {
  return read();
}

export function getSmartPlaylist(id: string): SmartPlaylist | undefined {
  return read().find((p) => p.id === id);
}

function newId(): string {
  return `sp_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`;
}

export function createSmartPlaylist(
  name: string,
  filter: SmartPlaylistFilter,
  description?: string,
): SmartPlaylist {
  const list = read();
  const now = Date.now();
  const p: SmartPlaylist = {
    id: newId(),
    name: name.trim() || '無題のスマートプレイリスト',
    description: description?.trim() || undefined,
    createdAt: now,
    updatedAt: now,
    filter: normalizeFilter(filter),
  };
  list.unshift(p);
  write(list);
  return p;
}

export function updateSmartPlaylist(
  id: string,
  patch: Partial<Pick<SmartPlaylist, 'name' | 'description' | 'filter'>>,
): SmartPlaylist | undefined {
  const list = read();
  const p = list.find((x) => x.id === id);
  if (!p) return undefined;
  if (patch.name != null) p.name = patch.name.trim() || p.name;
  if (patch.description !== undefined) p.description = patch.description.trim() || undefined;
  if (patch.filter != null) p.filter = normalizeFilter(patch.filter);
  p.updatedAt = Date.now();
  write(list);
  return p;
}

export function deleteSmartPlaylist(id: string): boolean {
  const list = read();
  const before = list.length;
  const next = list.filter((p) => p.id !== id);
  if (next.length === before) return false;
  write(next);
  return true;
}

/** Strip empty / invalid values so the resulting filter is round-trippable. */
export function normalizeFilter(f: SmartPlaylistFilter): SmartPlaylistFilter {
  const out: SmartPlaylistFilter = {};
  if (f.q && f.q.trim()) out.q = f.q.trim();
  if (Array.isArray(f.tags)) {
    const cleaned = Array.from(new Set(f.tags.map((t) => t.trim()).filter((t) => t.length > 0)));
    if (cleaned.length > 0) out.tags = cleaned;
  }
  if (Array.isArray(f.tagsAny)) {
    const cleaned = Array.from(new Set(f.tagsAny.map((t) => t.trim()).filter((t) => t.length > 0)));
    if (cleaned.length > 0) out.tagsAny = cleaned;
  }
  if (f.uploaderId && f.uploaderId.trim()) out.uploaderId = f.uploaderId.trim();
  if (Number.isFinite(f.minDuration) && (f.minDuration as number) > 0)
    out.minDuration = Math.floor(f.minDuration as number);
  if (Number.isFinite(f.maxDuration) && (f.maxDuration as number) > 0)
    out.maxDuration = Math.floor(f.maxDuration as number);
  if (f.resolution && f.resolution.trim()) out.resolution = f.resolution.trim();
  if (f.sortBy && f.sortBy.trim()) out.sortBy = f.sortBy.trim();
  if (f.sortOrder === 'asc' || f.sortOrder === 'desc') out.sortOrder = f.sortOrder;
  if (Number.isFinite(f.limit) && (f.limit as number) > 0)
    out.limit = Math.floor(f.limit as number);
  return out;
}

/** Convert a smart filter to the parameters expected by `queryLibraryVideos`.
 *  注: 本体は `filterToSearchQuery` (オンライン取得) を使う。これは
 *  既存テストと、将来「ローカル限定モード」を再導入したくなった時の
 *  ために残してある純粋関数。 */
export function filterToQueryParams(f: SmartPlaylistFilter): LibraryQueryParams {
  const params: LibraryQueryParams = {};
  if (f.q) params.q = f.q;
  if (f.tags && f.tags.length > 0) params.tags = f.tags;
  if (f.tagsAny && f.tagsAny.length > 0) params.tagsAny = f.tagsAny;
  if (f.uploaderId) params.uploaderId = f.uploaderId;
  if (f.minDuration != null) params.minDuration = f.minDuration;
  if (f.maxDuration != null) params.maxDuration = f.maxDuration;
  if (f.resolution) params.resolution = f.resolution;
  if (f.sortBy) params.sortBy = f.sortBy;
  if (f.sortOrder) params.sortOrder = f.sortOrder;
  if (f.limit != null) params.limit = f.limit;
  return params;
}

/** Smart playlist の sortBy ラベルを Snapshot Search の field 名へ。
 *  ローカル専用 (`downloaded_at` / `play_count` / `last_played_at` / `random` /
 *  `title`) は online には無いので `null` を返し、UI 側で sort 未指定として扱う。 */
function mapSortFieldToSearch(sortBy?: string): SearchField | null {
  switch (sortBy) {
    case 'posted_at':
      return 'startTime';
    case 'view_count':
      return 'viewCounter';
    case 'comment_count':
      return 'commentCounter';
    case 'mylist_count':
      return 'mylistCounter';
    case 'duration_sec':
      return 'lengthSeconds';
    default:
      return null;
  }
}

/** Snapshot Search が許す最大件数 (1 リクエスト)。サーバ側で固定値。 */
const SEARCH_MAX_LIMIT = 100;

/** Smart playlist filter を Snapshot Search の SearchQuery へ変換する。
 *
 *  Snapshot Search は `q` 必須 / `targets` 必須なので、ユーザ入力に
 *  応じて妥当なデフォルトを組み立てる:
 *   - キーワードがあれば: targets=[title,tags] でフリーワード検索
 *   - キーワードが無くタグ AND がある: 1 つ目のタグを q に、tagsExact で
 *     完全一致。残りのタグは filters[tagsExact][0] として AND。
 *   - キーワードが無くタグ OR のみある: タグを " OR " で連結し
 *     targets=[tagsExact] で投げる (snapshot search の q 構文に OR 演算子あり)
 *   - 何も無い: 空文字を返す呼び出し側が「条件無しの smart playlist は
 *     online 検索不可」とエラーにする
 *
 *  resolution はオンライン版に対応する場が無いため無視する。
 */
export function filterToSearchQuery(f: SmartPlaylistFilter): SearchQuery {
  const q = (f.q ?? '').trim();
  const andTags = (f.tags ?? []).filter((t) => t.length > 0);
  const orTags = (f.tagsAny ?? []).filter((t) => t.length > 0);

  let searchQ = '';
  let targets: SearchTarget[] = ['title'];
  const filters: FilterClause[] = [];

  if (q) {
    searchQ = q;
    targets = ['title', 'tags'];
    // キーワード + AND タグは、タグを `tagsExact eq` フィルタで足す。
    for (const t of andTags) {
      filters.push({ field: 'tagsExact', op: 'eq', value: t });
    }
  } else if (andTags.length > 0) {
    // タグ AND のみ。先頭タグを q (targets=tagsExact)、残りは filter で AND。
    searchQ = andTags[0];
    targets = ['tagsExact'];
    for (const t of andTags.slice(1)) {
      filters.push({ field: 'tagsExact', op: 'eq', value: t });
    }
  } else if (orTags.length > 0) {
    // タグ OR のみ。 q の OR 構文を使う。
    searchQ = orTags.join(' OR ');
    targets = ['tagsExact'];
  }
  // 何も無い場合は searchQ='' のまま返す → 呼び出し側がエラー表示。

  if (f.uploaderId) {
    filters.push({ field: 'userId', op: 'eq', value: f.uploaderId });
  }
  if (f.minDuration != null) {
    filters.push({ field: 'lengthSeconds', op: 'gte', value: String(f.minDuration) });
  }
  if (f.maxDuration != null) {
    filters.push({ field: 'lengthSeconds', op: 'lte', value: String(f.maxDuration) });
  }

  const fields: SearchField[] = [
    'contentId',
    'title',
    'viewCounter',
    'commentCounter',
    'mylistCounter',
    'lengthSeconds',
    'thumbnailUrl',
    'startTime',
    'tags',
    'userId',
    'channelId',
  ];

  const sortField = mapSortFieldToSearch(f.sortBy);
  const sort: SortSpec | undefined = sortField
    ? { field: sortField, direction: f.sortOrder ?? 'desc' }
    : undefined;

  const requested = f.limit;
  const limit =
    requested != null && Number.isFinite(requested) && requested > 0
      ? Math.min(SEARCH_MAX_LIMIT, Math.floor(requested))
      : SEARCH_MAX_LIMIT;

  const query: SearchQuery = {
    q: searchQ,
    targets,
    fields,
    filters,
    limit,
    offset: 0,
  };
  if (sort) query.sort = sort;
  return query;
}

/** Render a short human-readable summary of the filter, e.g. for cards. */
export function summarizeFilter(f: SmartPlaylistFilter): string {
  const parts: string[] = [];
  if (f.q) parts.push(`"${f.q}"`);
  if (f.tags?.length) parts.push(`タグAND: ${f.tags.join(', ')}`);
  if (f.tagsAny?.length) parts.push(`タグOR: ${f.tagsAny.join(', ')}`);
  if (f.uploaderId) parts.push(`投稿者:${f.uploaderId}`);
  if (f.minDuration != null) parts.push(`${f.minDuration}s〜`);
  if (f.maxDuration != null) parts.push(`〜${f.maxDuration}s`);
  if (f.resolution) parts.push(f.resolution);
  if (f.sortBy) parts.push(`順:${f.sortBy} ${f.sortOrder ?? 'desc'}`);
  if (f.limit) parts.push(`上限 ${f.limit}`);
  return parts.length === 0 ? '条件なし (全件)' : parts.join(' / ');
}
