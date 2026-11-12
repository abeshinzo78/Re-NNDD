/**
 * 再生位置のニコニコへの保存 (オプション機能 `playback.sync_niconico_position`)。
 *
 * 動画ページとミニプレイヤーのどちらで再生していても同じ規則で送る
 * (Web プレイヤーと同じ意味論):
 * - ログイン中かつ通常の視聴 (`okReason === 'PURELY'`) で、設定が ON の時だけ
 * - 実際に再生が始まった動画だけ (開いただけの動画で、他の端末で保存した位置を
 *   0 で上書きしない)
 * - 見終わったら 0。見終わった後に離れても終端の位置で上書きしない。巻き戻したら解除
 * - 同じ位置は二度送らない
 */

import { saveNiconicoPlaybackPosition } from '$lib/api';
import { getBool } from '$lib/stores/settings.svelte';

/** 送ってよい視聴かどうかの判定に使う、視聴データの一部。 */
export type PositionSyncInfo = {
  isGuest?: boolean;
  sessionExpired?: boolean;
  okReason?: string;
  /** 動画の長さ (秒)。見終わった後の巻き戻しの判定に使う。 */
  durationSec: number;
};

/** `preparePlayback` の結果から判定材料を取り出す。 */
export function positionSyncInfoOf(p: {
  isGuest?: boolean;
  sessionExpired?: boolean;
  okReason?: string;
  video: { duration: number };
}): PositionSyncInfo {
  return {
    isGuest: p.isGuest,
    sessionExpired: p.sessionExpired,
    okReason: p.okReason,
    durationSec: p.video.duration,
  };
}

export function canSyncNiconicoPosition(info: PositionSyncInfo): boolean {
  return (
    getBool('playback.sync_niconico_position') &&
    getBool('playback.record_niconico_view') &&
    !info.isGuest &&
    !info.sessionExpired &&
    info.okReason === 'PURELY'
  );
}

export type PositionSaver = (videoId: string, seconds: number) => Promise<void>;

/** 1 つのプレイヤーが再生している動画について、位置の保存を受け持つ。 */
export class PositionSync {
  private videoId: string | null = null;
  private info: PositionSyncInfo | null = null;
  private armed = false;
  private ended = false;
  private lastSaved: number | null = null;

  constructor(
    private readonly saver: PositionSaver = saveNiconicoPlaybackPosition,
    private readonly eligible: (info: PositionSyncInfo) => boolean = canSyncNiconicoPosition,
  ) {}

  /** 再生する動画が変わった (`null` なら何も送らない状態にする)。 */
  reset(videoId: string | null, info: PositionSyncInfo | null = null): void {
    this.videoId = videoId;
    this.info = info;
    this.armed = false;
    this.ended = false;
    this.lastSaved = null;
  }

  /** 実際に再生が始まった。 */
  started(): void {
    if (this.videoId) this.armed = true;
  }

  /**
   * 別のプレイヤーへ再生を引き継いだ (動画ページ → ミニプレイヤー)。このプレイヤーが
   * 再び再生を始めるまで送らない (引き継ぎ時点の古い位置で上書きしないため)。
   */
  handOff(): void {
    this.armed = false;
  }

  /** 再生位置の更新。見終わった後に巻き戻されたら、また現在位置を送るようにする。 */
  timeUpdate(seconds: number): void {
    if (this.ended && this.info && seconds < this.info.durationSec - 2) this.ended = false;
  }

  /** 一時停止・離脱などで現在位置を送る。 */
  saveAt(seconds: number): void {
    const videoId = this.videoId;
    if (!videoId || !this.info || !this.armed || !this.eligible(this.info)) return;
    const s = this.ended ? 0 : Math.max(0, Math.floor(Number.isFinite(seconds) ? seconds : 0));
    if (this.lastSaved === s) return;
    this.lastSaved = s;
    this.saver(videoId, s).catch((e) => {
      console.warn('save playback position failed', e);
    });
  }

  /** 最後まで再生した (0 秒で保存する)。 */
  finished(): void {
    this.ended = true;
    this.saveAt(0);
  }
}
