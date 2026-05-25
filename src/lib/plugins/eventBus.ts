// プラグイン向けイベントバス。
//
// 既存の `createListenerRegistry` (src/lib/stores/listenerRegistry.ts) は
// payload を持たない notify/subscribe しか提供しないため、ここではイベント名
// + payload を扱う薄い独自実装を用意する (listenerRegistry と同型のセット運用)。
//
// 中心的な不変条件:
// - 1 ハンドラが throw しても他のハンドラ呼び出しは続行する
// - `offAllByOwner(token)` で同じ owner (= plugin id) のハンドラを一括解除
//   できる (プラグインの uninstall / disable 時に使う)
// - 同じ name に同じハンドラを 2 度 on しても 1 度しか登録しない (Set 動作)

type AnyHandler = (payload: unknown) => void;

type Entry = {
  owner: string;
  handler: AnyHandler;
};

const buckets = new Map<string, Set<Entry>>();

function bucketOf(name: string): Set<Entry> {
  let b = buckets.get(name);
  if (!b) {
    b = new Set();
    buckets.set(name, b);
  }
  return b;
}

/** イベントを発火。listener 0 件なら no-op。 */
export function emit(name: string, payload: unknown): void {
  const b = buckets.get(name);
  if (!b) return;
  // 反復中に on/off が呼ばれる可能性に備えてスナップショットを取る
  const snapshot = [...b];
  for (const entry of snapshot) {
    try {
      entry.handler(payload);
    } catch (e) {
      // 1 つの handler の例外で他の handler を巻き込まない
      // (owner を入れて plugin id を特定可能にする)
      console.error(`[plugin event] handler for ${name} (owner=${entry.owner}) threw:`, e);
    }
  }
}

/** ハンドラを登録。返値の関数を呼ぶと off できる。
 *  `owner` は plugin id を渡す (一括解除用)。 */
export function on(owner: string, name: string, handler: AnyHandler): () => void {
  const b = bucketOf(name);
  const entry: Entry = { owner, handler };
  b.add(entry);
  return () => {
    b.delete(entry);
    if (b.size === 0) buckets.delete(name);
  };
}

/** 指定 owner の全 handler を解除。 */
export function offAllByOwner(owner: string): void {
  for (const [name, b] of buckets) {
    for (const e of [...b]) {
      if (e.owner === owner) b.delete(e);
    }
    if (b.size === 0) buckets.delete(name);
  }
}

/** テスト用: 全 handler をクリア。 */
export function _resetForTests(): void {
  buckets.clear();
}

/** テスト用: 登録されている handler の総数。 */
export function _handlerCount(): number {
  let n = 0;
  for (const b of buckets.values()) n += b.size;
  return n;
}
