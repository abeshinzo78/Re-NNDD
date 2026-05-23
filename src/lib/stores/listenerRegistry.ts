/**
 * Shared in-memory observer helper.
 *
 * `mylists` / `ngRules` / `playbackQueue` / `smartPlaylists` の各 store が
 * それぞれ `Set<() => void>` + `notify()` + `subscribeXxx(fn)` の同型ボディを
 * コピペしていたのをここに集約する。挙動は元コードと一致させており、
 * subscribe は unsubscribe 関数を返し、notify はその時点の登録順で同期に呼ぶ。
 */
export function createListenerRegistry(): {
  notify: () => void;
  subscribe: (fn: () => void) => () => void;
} {
  const listeners = new Set<() => void>();
  return {
    notify(): void {
      for (const fn of listeners) fn();
    },
    subscribe(fn: () => void): () => void {
      listeners.add(fn);
      return () => {
        listeners.delete(fn);
      };
    },
  };
}
