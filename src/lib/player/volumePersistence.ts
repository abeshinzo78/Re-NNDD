// プレイヤー音量の永続化。
//
// `playback.default_volume` 設定は「初めて再生する時の初期音量」であり、
// ユーザがコントロールバーで音量を変えた後はその選択を覚えてほしい。
// 何も覚えないと PiP 切替や別動画への遷移、ページ再マウントのたびに
// `default_volume` (既定 1.0) に戻ってしまい鬱陶しい。
//
// 値はセッションを跨いで保持したいので localStorage に保存する。
// 設定 (DB) には書かない: ここに置くのはユーザ設定ではなくランタイム
// state なので、設定画面の「デフォルト音量」は依然として「初回起動時」
// 用の値として独立に機能する。

const VOLUME_KEY = 'player.lastVolume.v1';
const MUTED_KEY = 'player.lastMuted.v1';

export function readSavedVolume(): number | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = localStorage.getItem(VOLUME_KEY);
    if (raw == null) return null;
    const n = Number(raw);
    if (!Number.isFinite(n)) return null;
    return Math.max(0, Math.min(1, n));
  } catch {
    return null;
  }
}

export function saveVolume(v: number): void {
  if (typeof window === 'undefined') return;
  if (!Number.isFinite(v)) return;
  const clamped = Math.max(0, Math.min(1, v));
  try {
    localStorage.setItem(VOLUME_KEY, String(clamped));
  } catch {
    /* ignore quota errors */
  }
}

export function readSavedMuted(): boolean {
  if (typeof window === 'undefined') return false;
  try {
    return localStorage.getItem(MUTED_KEY) === 'true';
  } catch {
    return false;
  }
}

export function saveMuted(m: boolean): void {
  if (typeof window === 'undefined') return;
  try {
    localStorage.setItem(MUTED_KEY, m ? 'true' : 'false');
  } catch {
    /* ignore quota errors */
  }
}
