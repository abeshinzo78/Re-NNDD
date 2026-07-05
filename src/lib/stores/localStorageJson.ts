/**
 * localStorage-backed JSON persistence helpers.
 *
 * `mylists` / `ngRules` / `smartPlaylists` / `playbackQueue` の各 store が
 * 「SSR ガード + `getItem` + `JSON.parse` を try/catch でフォールバック」する
 * read ボディと「SSR ガード + `setItem(JSON.stringify)`」する write ボディを
 * それぞれコピペしていたのをここへ集約する。`listenerRegistry` と同じ方針で、
 * 挙動は元コードと完全一致させている — フォールバック値・パース失敗時の
 * 復帰・quota エラーの伝播の有無・`removeItem` 分岐まで変えていない。
 *
 * localStorage が使えない環境 (SSR / プライベートモード等) では read は
 * `fallback()` を返し、write は「書き込めなかった」ことを表す `false` を
 * 返す。呼び出し側はこの戻り値で `notify()` を出すかどうかを判断する
 * (元コードは localStorage 不在時に notify を呼んでいなかった)。
 */

/** localStorage が参照可能か。SSR / 非許可環境では false。 */
function hasLocalStorage(): boolean {
  return typeof localStorage !== 'undefined';
}

/**
 * `key` を読んで `JSON.parse` する。localStorage が無い / キーが空 /
 * `JSON.parse` が投げた / `validate` が投げた場合は `fallback()` を返す。
 *
 * `validate` はパース済みの値 (`unknown`) を受け取り、検証・整形して最終的な
 * 戻り値を作る。元コードが `Array.isArray(parsed) ? parsed : []` のように
 * パース後に行っていた妥当性チェックをここへ渡す。`validate` 内でさらに
 * `fallback()` 相当の値を返しても良い (元コードと同じく「壊れた値は既定へ倒す」)。
 *
 * `fallback` は関数にしてあるので、既定値の生成が高価なケース
 * (例: `defaultMylists()` が `Date.now()` を呼ぶ) でもパース成功時には
 * 評価されない。
 */
export function readJson<T>(
  key: string,
  fallback: () => T,
  validate: (parsed: unknown) => T = (p) => p as T,
): T {
  if (!hasLocalStorage()) return fallback();
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback();
    return validate(JSON.parse(raw));
  } catch {
    return fallback();
  }
}

/**
 * `value` を `JSON.stringify` して `key` に書く。localStorage が無ければ
 * 何もせず `false` を返す。quota 超過などで `setItem` が投げた場合は
 * **そのまま伝播** する (元の `mylists` / `ngRules` の write と同じ挙動 —
 * これらは setItem を try/catch で包んでいなかった)。書けたら `true`。
 */
export function writeJson(key: string, value: unknown): boolean {
  if (!hasLocalStorage()) return false;
  localStorage.setItem(key, JSON.stringify(value));
  return true;
}

/**
 * `writeJson` の quota 耐性版。`setItem` の例外を握り潰し、`value == null`
 * のときは `removeItem` する (元の `smartPlaylists.write` / `playbackQueue`
 * の `writeQueue` と同じ挙動)。localStorage が無ければ `false`、それ以外は
 * 書き込みの成否に関わらず `true` を返す (元コードも quota エラー後に
 * `notify()` を呼んでいたため)。
 */
export function writeJsonSafe(key: string, value: unknown): boolean {
  if (!hasLocalStorage()) return false;
  try {
    if (value == null) localStorage.removeItem(key);
    else localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* quota / private mode — 元コードと同じく無視 */
  }
  return true;
}
