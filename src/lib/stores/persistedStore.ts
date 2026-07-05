/**
 * localStorage-backed collection store factory.
 *
 * `mylists` / `ngRules` / `smartPlaylists` / `playbackQueue` の各 store が
 * `createListenerRegistry()` と `readJson` / `writeJson(Safe)` を
 *
 *   const { notify, subscribe } = createListenerRegistry();
 *   function read()  { return readJson(KEY, fallback, validate); }
 *   function write(v) { if (writeJson(KEY, v)) notify(); }
 *
 * という同型のグルーで毎回繋ぎ直していたのをここへ集約する。`listenerRegistry`
 * / `localStorageJson` と同じ方針で、挙動は元コードと完全一致させている:
 *
 *  - `read` は `readJson` をそのまま呼ぶ (fallback / validate の意味は不変)。
 *  - `write` は書き込み成否 (`writeJson` / `writeJsonSafe` の戻り値) を見て
 *    **書けたときだけ** `notify()` する。元コードの `if (writeJson(...)) notify()`
 *    と一致し、localStorage 不在時に notify を出さない挙動も保たれる。
 *  - `safe` を渡すと write が quota 耐性版 (`writeJsonSafe`) になる。
 *    `smartPlaylists` / `playbackQueue` が使っていた `null → removeItem` /
 *    例外握り潰しの挙動をそのまま引き継ぐ。
 *
 * `subscribe` / `notify` は `createListenerRegistry` の戻り値をそのまま公開する。
 */

import { createListenerRegistry } from './listenerRegistry';
import { readJson, writeJson, writeJsonSafe } from './localStorageJson';

export interface PersistedStore<T> {
  /** localStorage から読む。未保存 / 壊れた値は `fallback()` へ倒す。 */
  read: () => T;
  /** localStorage へ書く。書き込めたときだけ subscriber へ通知する。 */
  write: (value: T) => void;
  /** 変更通知の購読。unsubscribe 関数を返す。 */
  subscribe: (fn: () => void) => () => void;
  /** subscriber を手動で発火する (read/write を経由しない更新用)。 */
  notify: () => void;
}

export function createPersistedStore<T>(options: {
  /** localStorage キー。 */
  key: string;
  /** 未保存 / パース失敗時に返す既定値。関数なので成功時は評価されない。 */
  fallback: () => T;
  /** パース済み値の検証・整形。未指定なら値をそのまま `T` として扱う。 */
  validate?: (parsed: unknown) => T;
  /** quota 耐性版 (`writeJsonSafe`) で書くなら true。既定は `writeJson`。 */
  safe?: boolean;
}): PersistedStore<T> {
  const { key, fallback, validate, safe = false } = options;
  const { notify, subscribe } = createListenerRegistry();

  return {
    read: () => readJson(key, fallback, validate),
    write: (value: T): void => {
      if (safe ? writeJsonSafe(key, value) : writeJson(key, value)) notify();
    },
    subscribe,
    notify,
  };
}

export interface PersistedCollection<Item extends { id: string }> extends PersistedStore<Item[]> {
  /** `id` で 1 件引く。見つからなければ `undefined`。 */
  getById: (id: string) => Item | undefined;
}

/**
 * `id: string` を持つ要素の配列を localStorage に持つ store。`createPersistedStore`
 * に「`read().find((x) => x.id === id)`」だけの `getById` を足したもので、
 * `mylists.getMylist` / `smartPlaylists.getSmartPlaylist` がコピペしていた
 * find ボディをここへ寄せる。`fallback` 省略時は空配列。
 */
export function createPersistedCollection<Item extends { id: string }>(options: {
  key: string;
  fallback?: () => Item[];
  validate?: (parsed: unknown) => Item[];
  safe?: boolean;
}): PersistedCollection<Item> {
  const base = createPersistedStore<Item[]>({
    ...options,
    fallback: options.fallback ?? (() => []),
  });
  return {
    ...base,
    getById: (id: string) => base.read().find((x) => x.id === id),
  };
}
