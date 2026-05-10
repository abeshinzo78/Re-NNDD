/**
 * Local mylist store — persisted in localStorage.
 *
 * MVP: backend DB is not yet wired up, so mylists live entirely in the
 * browser. The shape mirrors what the SQLite `playlists` / `playlist_items`
 * tables will hold, so a future migration to Tauri-backed storage stays
 * mechanical.
 *
 * The built-in mylist with id `"saved"` doubles as the bookmark / "保存"
 * surface until the HLS downloader (Phase 1.2) lands.
 */

export type MylistVideo = {
  videoId: string;
  title: string;
  thumbnailUrl?: string;
  lengthSeconds?: number;
  viewCounter?: number;
  uploaderName?: string;
  addedAt: number;
};

export type Mylist = {
  id: string;
  name: string;
  builtin?: boolean;
  createdAt: number;
  updatedAt: number;
  items: MylistVideo[];
};

const KEY = 'nndd:mylists';
const SAVED_ID = 'saved';

function defaultMylists(): Mylist[] {
  const now = Date.now();
  return [
    {
      id: SAVED_ID,
      name: 'マイリスト',
      builtin: true,
      createdAt: now,
      updatedAt: now,
      items: [],
    },
  ];
}

function read(): Mylist[] {
  if (typeof localStorage === 'undefined') return defaultMylists();
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return defaultMylists();
    const parsed = JSON.parse(raw) as Mylist[];
    if (!Array.isArray(parsed) || parsed.length === 0) return defaultMylists();
    if (!parsed.some((m) => m.id === SAVED_ID)) {
      parsed.unshift(defaultMylists()[0]);
    }
    // 旧名「保存済み」を「マイリスト」へマイグレート
    let migrated = false;
    for (const m of parsed) {
      if (m.id === SAVED_ID && m.builtin && m.name === '保存済み') {
        m.name = 'マイリスト';
        migrated = true;
      }
    }
    if (migrated) {
      try {
        localStorage.setItem(KEY, JSON.stringify(parsed));
      } catch {
        /* */
      }
    }
    return parsed;
  } catch {
    return defaultMylists();
  }
}

function write(list: Mylist[]): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(KEY, JSON.stringify(list));
  notify();
}

const listeners = new Set<() => void>();
function notify() {
  for (const fn of listeners) fn();
}
export function subscribeMylists(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function listMylists(): Mylist[] {
  return read();
}

export function getMylist(id: string): Mylist | undefined {
  return read().find((m) => m.id === id);
}

export function createMylist(name: string): Mylist {
  const list = read();
  const now = Date.now();
  const m: Mylist = {
    id: `ml_${now}_${Math.random().toString(36).slice(2, 8)}`,
    name: name.trim() || '無題のマイリスト',
    createdAt: now,
    updatedAt: now,
    items: [],
  };
  list.push(m);
  write(list);
  return m;
}

export function renameMylist(id: string, name: string): void {
  const list = read();
  const m = list.find((x) => x.id === id);
  if (!m) return;
  m.name = name.trim() || m.name;
  m.updatedAt = Date.now();
  write(list);
}

export function deleteMylist(id: string): void {
  const list = read().filter((m) => !(m.id === id && !m.builtin));
  write(list);
}

export function addToMylist(id: string, video: Omit<MylistVideo, 'addedAt'>): boolean {
  const list = read();
  const m = list.find((x) => x.id === id);
  if (!m) return false;
  if (m.items.some((v) => v.videoId === video.videoId)) return false;
  m.items.unshift({ ...video, addedAt: Date.now() });
  m.updatedAt = Date.now();
  write(list);
  return true;
}

export function removeFromMylist(id: string, videoId: string): void {
  const list = read();
  const m = list.find((x) => x.id === id);
  if (!m) return;
  const before = m.items.length;
  m.items = m.items.filter((v) => v.videoId !== videoId);
  if (m.items.length !== before) {
    m.updatedAt = Date.now();
    write(list);
  }
}

export function isInMylist(id: string, videoId: string): boolean {
  return getMylist(id)?.items.some((v) => v.videoId === videoId) ?? false;
}

export function mylistsContaining(videoId: string): string[] {
  return read()
    .filter((m) => m.items.some((v) => v.videoId === videoId))
    .map((m) => m.id);
}

export const SAVED_MYLIST_ID = SAVED_ID;
