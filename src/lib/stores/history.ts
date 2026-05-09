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
    return data ? JSON.parse(data) : [];
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
