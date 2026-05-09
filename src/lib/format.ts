export function formatDuration(seconds: number | undefined): string {
  if (seconds == null) return '';
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(sec).padStart(2, '0')}`;
  return `${m}:${String(sec).padStart(2, '0')}`;
}

export function formatNumber(n: number | undefined): string {
  if (n == null) return '';
  return n.toLocaleString('ja-JP');
}

export function formatDate(iso: string | undefined): string {
  if (!iso) return '';
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleString('ja-JP', {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
    });
  } catch {
    return iso;
  }
}

export function videoUrl(contentId: string | undefined): string | undefined {
  if (!contentId) return undefined;
  return `https://www.nicovideo.jp/watch/${contentId}`;
}
