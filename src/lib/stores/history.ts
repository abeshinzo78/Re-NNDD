export type HistorySource = 'online' | 'local';

export type HistoryItem = {
  videoId: string;
  title: string;
  thumbnailUrl?: string;
  uploaderName?: string;
  duration?: number;
  viewCount?: number;
  playedAt: number; // timestamp
  /** どの経路で再生したか。'online' = HLS ストリーミング, 'local' = DL 済 */
  source?: HistorySource;
};

const HISTORY_KEY = 'nndd_play_history';

export function getHistory(): HistoryItem[] {
  if (typeof window === 'undefined') return [];
  try {
    const data = localStorage.getItem(HISTORY_KEY);
    if (!data) return [];
    const parsed = JSON.parse(data);
    // localStorage が手動編集されたり、別バージョンが書いた壊れた値が
    // 入っていたりすると `JSON.parse` がオブジェクトを返してくる事がある。
    // 配列でなければ空配列に倒さないと、後続の `.filter` 等で
    // TypeError を吐いて履歴ページが真っ白になる。
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

/** ソース別フィルタ。`source` 省略で全件。 */
export function getHistoryFiltered(source?: HistorySource): HistoryItem[] {
  const all = getHistory();
  if (!source) return all;
  return all.filter((h) => (h.source ?? 'online') === source);
}

export function addHistory(item: Omit<HistoryItem, 'playedAt'>) {
  if (typeof window === 'undefined') return;
  try {
    const current = getHistory();
    // 同じ videoId + source の組合せは上に持っていく。
    // online と local は別エントリで両方残す（ユーザの体験単位で別物なので）。
    const newSource = item.source ?? 'online';
    const filtered = current.filter(
      (h) => !(h.videoId === item.videoId && (h.source ?? 'online') === newSource),
    );
    filtered.unshift({
      ...item,
      source: newSource,
      playedAt: Date.now(),
    });
    // Keep max 500 items
    if (filtered.length > 500) {
      filtered.length = 500;
    }
    localStorage.setItem(HISTORY_KEY, JSON.stringify(filtered));
  } catch (e) {
    console.warn('Failed to save history', e);
  }
}

export function clearHistory() {
  if (typeof window === 'undefined') return;
  localStorage.removeItem(HISTORY_KEY);
}

export function removeHistoryItem(videoId: string, source?: HistorySource) {
  if (typeof window === 'undefined') return;
  try {
    const current = getHistory();
    const s = source ?? 'online';
    const filtered = current.filter(
      (h) => !(h.videoId === videoId && (h.source ?? 'online') === s),
    );
    localStorage.setItem(HISTORY_KEY, JSON.stringify(filtered));
  } catch (e) {
    console.warn('Failed to remove history item', e);
  }
}
