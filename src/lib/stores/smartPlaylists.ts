/**
 * Smart playlist store — saved filter conditions over the local library.
 *
 * A smart playlist is a *recipe*, not a static list of videos. Each time
 * the user opens one we re-run `queryLibraryVideos` with the saved filter,
 * so the result reflects whatever is currently in the library (newly DL'd
 * videos appear automatically, deleted ones disappear).
 *
 * Persisted in localStorage. Mirrors the shape of `LibraryQueryParams`
 * (in `$lib/api`) so opening a smart playlist is a 1:1 invocation.
 */

import type { LibraryQueryParams } from '$lib/api';

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

/** Convert a smart filter to the parameters expected by `queryLibraryVideos`. */
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
