import { searchVideosOnline, type SearchHit } from '$lib/api';
import { sortByPopularity } from '$lib/stores/searchState';

function deriveRelatedQuery(title: string): string {
  if (!title) return '';
  const stripped = title
    .replace(/【[^】]*】/g, ' ')
    .replace(/\[[^\]]*\]/g, ' ')
    .replace(/\([^)]*\)/g, ' ')
    .replace(/[「」『』、。!?！？:：・/\\|]/g, ' ')
    .replace(/\bpart\s*\d+/gi, ' ')
    .replace(/\b第\s*\d+\s*[話回弾]/g, ' ')
    .trim();
  const truncated = stripped.slice(0, 25).trim();
  return truncated || title.slice(0, 25);
}

export async function fetchRelatedVideos(
  videoId: string,
  title: string,
  _tags?: unknown,
  limit = 12,
): Promise<SearchHit[]> {
  const q = deriveRelatedQuery(title);
  if (!q) return [];

  const resp = await searchVideosOnline({
    q,
    targets: ['title', 'tags'],
    fields: [
      'contentId', 'title', 'viewCounter', 'commentCounter',
      'mylistCounter', 'lengthSeconds', 'thumbnailUrl', 'startTime',
      'userId', 'channelId',
    ],
    sort: { field: 'viewCounter', direction: 'desc' },
    limit: Math.min(50, limit + 5),
    offset: 0,
  });

  return sortByPopularity(
    resp.data.filter((h) => h.contentId && h.contentId !== videoId),
  ).slice(0, limit);
}
