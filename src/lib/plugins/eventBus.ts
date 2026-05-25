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
// - 同じ (owner, name, handler) を 2 度 on しても 1 度しか登録しない
//   (Set に新しい entry object を入れる素朴な実装だと dedup されないので、
//   挿入前に既存 entry を線形検索する; Codex review #6)

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
 *  `owner` は plugin id を渡す (一括解除用)。
 *  同じ `(owner, handler)` の組での再登録は no-op (既存 entry を返す)。
 *  返り値の off クロージャは bucket 参照を **キャプチャしない** — 削除時に
 *  必ず最新の bucket を name で再 lookup する。これにより、間に
 *  offAllByOwner + 別 owner の on で bucket が作り直された後でも、古い
 *  off が新 bucket を誤って消すこと (Codex #12) を防ぐ。 */
export function on(owner: string, name: string, handler: AnyHandler): () => void {
  const b = bucketOf(name);
  // Set でも entry object が毎回新規だと重複登録になるので、(owner, handler)
  // の同値性で線形検索して dedup する。バケット当たりの listener 数は通常
  // 高々数件想定なので線形でよい。
  for (const existing of b) {
    if (existing.owner === owner && existing.handler === handler) {
      return makeOff(name, existing);
    }
  }
  const entry: Entry = { owner, handler };
  b.add(entry);
  return makeOff(name, entry);
}

function makeOff(name: string, entry: Entry): () => void {
  return () => {
    const current = buckets.get(name);
    if (!current) return;
    current.delete(entry);
    if (current.size === 0) buckets.delete(name);
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
