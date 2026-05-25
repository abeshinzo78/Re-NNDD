// プラグインから観測されるプレイヤー状態のスナップショット。
//
// Player.svelte が `updatePlayerState()` を 1 秒おき + イベント時に呼んで
// in-memory state を更新する。プラグインは `ctx.player.getState()` で同期
// 読み取りできる (Promise を介さない)。
//
// なぜ Rust dispatcher を経由しないか:
// - フロントが真値を持っており Rust に複製しても整合コストが増える
// - getState() の同期性が UX 上重要 (ユーザ操作の handler から取りたい)
// - 状態取得は副作用ゼロなので permission チェックの価値が薄い
//   (`player.read` 権限は将来 Rust 経由の取得を加えるとき用に予約)
//
// 状態が古くなる懸念: Player が unmount された後でも前回値が残るが、
// `videoId === null` を unmount フラグとして使えるので問題にしない。

import type { PlayerObservedState } from './types';

const ZERO_STATE: PlayerObservedState = {
  videoId: null,
  currentTime: 0,
  duration: 0,
  paused: true,
  volume: 1,
  muted: false,
  playbackRate: 1,
};

// reactive にする必要はない (プラグインからは同期 getState() で読むだけ)。
// Player.svelte のリアクティブな更新は別経路 (Svelte の $effect / $state) で
// 行われている。
let snapshot: PlayerObservedState = { ...ZERO_STATE };

export function updatePlayerState(next: Partial<PlayerObservedState>): void {
  // 部分更新で誤って null を入れないため、`null` は許容するのは videoId のみ。
  snapshot = { ...snapshot, ...next };
}

export function getPlayerState(): PlayerObservedState {
  // 防御的にコピー (プラグイン側で破壊的に書き換えられても影響しない)。
  return { ...snapshot };
}

/** Player.svelte が unmount するときに呼ぶ。プラグインが古い状態を読まないよう
 *  videoId を null に戻し、再生中フラグを clear する。 */
export function clearPlayerState(): void {
  snapshot = { ...ZERO_STATE };
}

/** テスト用: snapshot 内容を強制セット。 */
export function _setForTests(s: PlayerObservedState): void {
  snapshot = { ...s };
}
