import { fetchRecommendedVideos, type RelatedVideoItem, type SearchHit } from '$lib/api';

function toSearchHit(item: RelatedVideoItem): SearchHit {
  return {
    contentId: item.contentId,
    title: item.title,
    viewCounter: item.viewCounter,
    commentCounter: item.commentCounter,
    mylistCounter: item.mylistCounter,
    lengthSeconds: item.lengthSeconds,
    thumbnailUrl: item.thumbnailUrl,
    startTime: item.startTime,
    userId: item.userId,
    channelId: item.channelId,
  };
}

export async function fetchRelatedVideos(
  videoId: string,
  _title?: string,
  _tags?: unknown,
  limit = 12,
): Promise<SearchHit[]> {
  const items = await fetchRecommendedVideos(videoId, limit);
  return items
    .filter((item) => item.contentId && item.contentId !== videoId)
    .map(toSearchHit);
}
