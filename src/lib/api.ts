import { invoke } from '@tauri-apps/api/core';
import type { PlaybackPayload, PlayerComment } from './player/types';

export type SearchTarget = 'title' | 'description' | 'tags' | 'tagsExact';

export type SearchField =
  | 'contentId'
  | 'title'
  | 'description'
  | 'userId'
  | 'channelId'
  | 'viewCounter'
  | 'mylistCounter'
  | 'likeCounter'
  | 'lengthSeconds'
  | 'thumbnailUrl'
  | 'startTime'
  | 'lastResBody'
  | 'commentCounter'
  | 'lastCommentTime'
  | 'categoryTags'
  | 'tags'
  | 'tagsExact'
  | 'genre'
  | 'genreKeyword'
  | 'contentType';

export type FilterOp = 'eq' | 'gt' | 'gte' | 'lt' | 'lte';

export type FilterClause = {
  field: SearchField;
  op: FilterOp;
  value: string;
};

export type SortDirection = 'asc' | 'desc';

export type SortSpec = {
  field: SearchField;
  direction: SortDirection;
};

export type SearchQuery = {
  q: string;
  targets: SearchTarget[];
  fields?: SearchField[];
  filters?: FilterClause[];
  sort?: SortSpec;
  offset?: number;
  limit?: number;
  context?: string;
};

export type SearchHit = {
  contentId?: string;
  title?: string;
  description?: string;
  userId?: number;
  channelId?: number;
  viewCounter?: number;
  mylistCounter?: number;
  likeCounter?: number;
  lengthSeconds?: number;
  thumbnailUrl?: string;
  startTime?: string;
  lastResBody?: string;
  commentCounter?: number;
  lastCommentTime?: string;
  categoryTags?: string;
  tags?: string;
  genre?: string;
  contentType?: string;
};

export type SearchResponse = {
  meta: {
    status: number;
    totalCount?: number;
    id: string;
  };
  data: SearchHit[];
};

export async function getAppVersion(): Promise<string> {
  return invoke<string>('get_app_version');
}

export async function searchVideosOnline(query: SearchQuery): Promise<SearchResponse> {
  return invoke<SearchResponse>('search_videos_online', { query });
}

export async function preparePlayback(videoId: string): Promise<PlaybackPayload> {
  return invoke<PlaybackPayload>('prepare_playback', { videoId });
}

export async function fetchVideoComments(nvComment: {
  server: string;
  threadKey: string;
  params: unknown;
}): Promise<PlayerComment[]> {
  return invoke<PlayerComment[]>('fetch_video_comments', { nvComment });
}

export async function issueHlsUrl(videoId: string): Promise<string> {
  return invoke<string>('issue_hls_url', { videoId });
}

export async function saveSessionCookie(value: string): Promise<void> {
  await invoke('save_session_cookie', { value });
}

export async function clearSessionCookie(): Promise<void> {
  await invoke('clear_session_cookie');
}

export async function sessionCookieStatus(): Promise<boolean> {
  return invoke<boolean>('session_cookie_status');
}

export type LoginResult =
  | { kind: 'success' }
  | { kind: 'mfa'; mfaSession?: string }
  | { kind: 'invalid_credentials' };

export async function loginPassword(email: string, password: string): Promise<LoginResult> {
  return invoke<LoginResult>('login_password', { email, password });
}

export type HlsResource = {
  dataBase64: string;
  contentType?: string;
  status: number;
};

export async function fetchHlsResource(
  url: string,
  rangeStart?: number,
  rangeEnd?: number,
): Promise<HlsResource> {
  return invoke<HlsResource>('fetch_hls_resource', { url, rangeStart, rangeEnd });
}

export type UserVideoItem = {
  contentId: string;
  title: string;
  thumbnailUrl?: string;
  lengthSeconds?: number;
  viewCounter?: number;
  commentCounter?: number;
  mylistCounter?: number;
  startTime?: string;
  userId?: number;
  channelId?: number;
};

export type UserVideosResponse = {
  totalCount: number;
  items: UserVideoItem[];
  debugRaw?: string;
};

export async function fetchUserVideos(
  ownerKind: string,
  ownerId: string,
  page: number,
  pageSize: number,
  sortKey: string,
  sortOrder: string,
): Promise<UserVideosResponse> {
  return invoke<UserVideosResponse>('fetch_user_videos', {
    ownerKind,
    ownerId,
    page,
    pageSize,
    sortKey,
    sortOrder,
  });
}

// =================== ダウンロードキュー ===================

export type DownloadStatus = 'pending' | 'downloading' | 'done' | 'error' | 'paused';

export type DownloadQueueItem = {
  id: number;
  videoId: string;
  status: DownloadStatus;
  progress: number;
  errorMessage: string | null;
  scheduledAt: number | null;
  startedAt: number | null;
  finishedAt: number | null;
  retryCount: number;
};

export async function enqueueDownload(
  videoId: string,
  scheduledAt?: number | null,
): Promise<DownloadQueueItem> {
  return invoke<DownloadQueueItem>('enqueue_download', {
    videoId,
    scheduledAt: scheduledAt ?? null,
  });
}

export async function listDownloads(): Promise<DownloadQueueItem[]> {
  return invoke<DownloadQueueItem[]>('list_downloads');
}

export async function cancelDownload(id: number): Promise<boolean> {
  return invoke<boolean>('cancel_download', { id });
}

export async function clearFinishedDownloads(): Promise<number> {
  return invoke<number>('clear_finished_downloads');
}

export async function startDownload(id: number): Promise<void> {
  await invoke('start_download', { id });
}

// =================== ライブラリ ===================

export type LibraryVideoItem = {
  id: string;
  title: string;
  durationSec: number;
  uploaderId: string | null;
  uploaderName: string | null;
  viewCount: number | null;
  postedAt: number | null;
  downloadedAt: number | null;
  resolution: string | null;
  thumbnailUrl: string | null;
  localThumbnailPath: string | null;
  localVideoPath: string | null;
  tags: string[];
};

export type LocalPlayerCommentDto = {
  id: string;
  no: number;
  vposMs: number;
  content: string;
  mail: string;
  commands: string[];
  userId: string | null;
  postedAt: string | null;
  fork: string;
  isOwner: boolean;
  nicoruCount: number | null;
  score: number | null;
};

export type LibraryTagDto = {
  name: string;
  isLocked: boolean;
};

export type LocalPlaybackPayload = {
  videoId: string;
  title: string;
  description: string | null;
  durationSec: number;
  uploaderId: string | null;
  uploaderName: string | null;
  uploaderType: string | null;
  viewCount: number | null;
  commentCount: number | null;
  mylistCount: number | null;
  postedAt: number | null;
  thumbnailUrl: string | null;
  tags: LibraryTagDto[];
  localVideoPath: string;
  localAudioPath: string | null;
  localThumbnailPath: string | null;
  comments: LocalPlayerCommentDto[];
};

export async function listLibraryVideos(): Promise<LibraryVideoItem[]> {
  return invoke<LibraryVideoItem[]>('list_library_videos');
}

// =================== ライブラリ検索・整列・集計 ===================

export type LibraryQueryParams = {
  q?: string;
  tags?: string[];
  uploaderId?: string;
  minDuration?: number;
  maxDuration?: number;
  resolution?: string;
  sortBy?: string;
  sortOrder?: 'asc' | 'desc';
  offset?: number;
  limit?: number;
};

export type LibraryVideoRow = {
  id: string;
  title: string;
  description: string | null;
  uploaderId: string | null;
  uploaderName: string | null;
  uploaderType: string | null;
  category: string | null;
  durationSec: number;
  postedAt: number | null;
  viewCount: number | null;
  commentCount: number | null;
  mylistCount: number | null;
  thumbnailUrl: string | null;
  videoPath: string | null;
  resolution: string | null;
  downloadedAt: number | null;
  playCount: number;
  lastPlayedAt: number | null;
  tags: string[];
  localThumbnailPath: string | null;
};

export type QueryResult = {
  items: LibraryVideoRow[];
  totalCount: number;
  offset: number;
  limit: number;
};

export type TagCount = {
  name: string;
  count: number;
};

export type ResolutionCount = {
  resolution: string;
  count: number;
};

export type LibraryStats = {
  totalVideos: number;
  totalDurationSec: number;
  totalComments: number;
  uniqueUploaders: number;
  uniqueTags: number;
  topTags: TagCount[];
  resolutionDistribution: ResolutionCount[];
};

export async function queryLibraryVideos(q: LibraryQueryParams): Promise<QueryResult> {
  return invoke<QueryResult>('query_library_videos', { q });
}

export async function getLibraryStats(): Promise<LibraryStats> {
  return invoke<LibraryStats>('get_library_stats');
}

export async function listLibraryTags(): Promise<string[]> {
  return invoke<string[]>('list_library_tags');
}

export async function listLibraryResolutions(): Promise<string[]> {
  return invoke<string[]>('list_library_resolutions');
}

export async function prepareLocalPlayback(videoId: string): Promise<LocalPlaybackPayload | null> {
  return invoke<LocalPlaybackPayload | null>('prepare_local_playback', { videoId });
}

export async function remuxLocalVideo(videoId: string): Promise<string> {
  return invoke<string>('remux_local_video', { videoId });
}

/**
 * app_data_dir 配下のファイルを ArrayBuffer で取得する。
 * <video> が asset:// を食わない WebKitGTK 向けに Blob URL を作るために使う。
 * (現在は内蔵 HTTP サーバ経由を使う方が seek が安定するため、video には推奨しない)
 */
export async function readLocalFile(path: string): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>('read_local_file', { path });
}

/**
 * 内蔵 HTTP サーバ経由のローカル動画 URL。
 * `<video src=...>` に渡すと Range/206 配信されるので後方シークもクリーンに動く。
 */
export async function localVideoUrl(videoId: string): Promise<string> {
  return invoke<string>('local_video_url', { videoId });
}
export async function localAudioUrl(videoId: string): Promise<string> {
  return invoke<string>('local_audio_url', { videoId });
}

/** ライブラリから 1 動画分を完全削除（DB + ディスク両方）。 */
export async function deleteLibraryVideo(videoId: string): Promise<void> {
  await invoke('delete_library_video', { videoId });
}

/** 既存 DL 物から不要 sidecar を削除。返り値は削除した総 byte 数。 */
export async function cleanupStorage(): Promise<number> {
  return invoke<number>('cleanup_storage');
}

// =================== 設定 ===================

export async function getSettings(): Promise<Record<string, string>> {
  return invoke<Record<string, string>>('get_settings');
}

export async function setSettingRaw(key: string, value: string): Promise<void> {
  await invoke('set_setting', { key, value });
}

export async function deleteSettingRaw(key: string): Promise<void> {
  await invoke('delete_setting', { key });
}

export type AppInfo = {
  version: string;
  identifier: string;
  dataDir: string;
  videosDir: string;
  dbPath: string;
  localServerPort: number;
  ytdlpAvailable: boolean;
  ytdlpVersion: string | null;
  ytdlpSource: 'bundled' | 'sidecar' | 'system_path' | 'not_found';
  ytdlpPath: string;
  ffmpegAvailable: boolean;
  ffmpegVersion: string | null;
  ffmpegSource: 'bundled' | 'sidecar' | 'system_path' | 'not_found';
  ffmpegPath: string;
  libraryVideoCount: number;
  libraryVideosSizeBytes: number;
};

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('get_app_info');
}
