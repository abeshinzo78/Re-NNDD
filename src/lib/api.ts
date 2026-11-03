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

/** 検索に使うバックエンド API。
 * - `snapshot`: 公開スナップショット検索 API v2 (認証不要・日次更新)。
 * - `nvapi`: niconico Web クライアントと同じ内部検索 API。ログイン中は
 *   保存済みセッション Cookie で認証付き検索になる。 */
export type SearchEngine = 'snapshot' | 'nvapi';

export async function getAppVersion(): Promise<string> {
  return invoke<string>('get_app_version');
}

export async function searchVideosOnline(
  query: SearchQuery,
  engine: SearchEngine = 'snapshot',
): Promise<SearchResponse> {
  return invoke<SearchResponse>('search_videos_online', { query, engine });
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

export type RelatedVideoItem = {
  contentId?: string;
  title?: string;
  viewCounter?: number;
  commentCounter?: number;
  mylistCounter?: number;
  lengthSeconds?: number;
  thumbnailUrl?: string;
  startTime?: string;
  userId?: number;
  channelId?: number;
};

export async function fetchRecommendedVideos(
  videoId: string,
  limit?: number,
): Promise<RelatedVideoItem[]> {
  return invoke<RelatedVideoItem[]>('fetch_related_videos', { videoId, limit });
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
  | { kind: 'invalid_credentials' }
  /** niconico 側のログイン方式が変わり、この経路ではログインできない。 */
  | { kind: 'unsupported'; message: string };

export async function loginPassword(email: string, password: string): Promise<LoginResult> {
  return invoke<LoginResult>('login_password', { email, password });
}

export async function loginMfa(mfaSession: string, oneTimePassword: string): Promise<LoginResult> {
  return invoke<LoginResult>('login_mfa', {
    mfaSession,
    oneTimePassword,
  });
}

export type AccountInfo = {
  id: string;
  nickname: string;
  isPremium: boolean;
  iconUrl?: string;
};

/** 保存中のセッションの状態 (`nvapi /v1/users/me` で確かめた結果)。 */
export type AccountState =
  | { state: 'logged_out' }
  | { state: 'valid'; account: AccountInfo }
  /** Cookie はあるが niconico に拒否された (期限切れ・ログアウト済み)。 */
  | { state: 'expired' }
  /** 通信エラーなどで判定できなかった。 */
  | { state: 'unknown'; message: string };

export async function getAccountState(): Promise<AccountState> {
  return invoke<AccountState>('get_account_state');
}

export type LoginWindowResult =
  | { kind: 'success'; account?: AccountInfo }
  | { kind: 'canceled' }
  | { kind: 'timed_out' };

/**
 * niconico のログインページを別ウィンドウで開き、ログインが済むまで待つ。
 * 2026 年のログイン方式変更 (Turnstile 付き SPA) 以降はこれが正規のログイン手段。
 * 二段階認証や外部アカウントでのログインもそのウィンドウで行える。
 */
export async function openLoginWindow(): Promise<LoginWindowResult> {
  return invoke<LoginWindowResult>('open_login_window');
}

/**
 * 再生開始を niconico に記録する (ログイン中は視聴履歴、ゲストでも再生数)。
 * v4 の視聴データ取得は副作用が無いので、記録したい時だけ明示的に呼ぶ。
 */
export async function recordWatchView(
  videoId: string,
  actionTrackId: string,
  isGuest?: boolean,
): Promise<void> {
  await invoke('record_watch_view', { videoId, actionTrackId, isGuest: isGuest ?? null });
}

export type VideoTagInfo = {
  name: string;
  isLocked: boolean;
};

/** 動画のタグ (ロック状態付き)。ランキング NG のタグ判定用。 */
export async function fetchVideoTags(videoId: string): Promise<VideoTagInfo[]> {
  return invoke<VideoTagInfo[]>('fetch_video_tags', { videoId });
}

/**
 * Fetch a signed Domand HLS resource (playlist / init / media segment / AES
 * key) through the Rust side. The body comes back as a raw `ArrayBuffer`
 * (a `tauri::ipc::Response` on the Rust side) rather than a base64-in-JSON
 * envelope, so multi-MB segments are not inflated ~33% and re-encoded through
 * the IPC JSON layer on every fetch.
 */
export async function fetchHlsResource(
  url: string,
  rangeStart?: number,
  rangeEnd?: number,
): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>('fetch_hls_resource', { url, rangeStart, rangeEnd });
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
  seriesTitle?: string;
  seriesDescription?: string;
  seriesThumbnailUrl?: string;
  /** 投稿者 (チャンネル) の表示名が一覧から分かった時だけ入る。 */
  ownerName?: string;
};

export async function extractOnlineFrame(hlsUrl: string, seekSec: number): Promise<string | null> {
  return invoke<string | null>('extract_online_frame', { hlsUrl, seekSec });
}

export async function extractVideoFrame(videoId: string, seekSec: number): Promise<string | null> {
  return invoke<string | null>('extract_video_frame', { videoId, seekSec });
}

export async function fetchSeriesVideos(
  seriesId: string,
  page: number,
  pageSize: number,
): Promise<UserVideosResponse> {
  return invoke<UserVideosResponse>('fetch_series_videos', {
    seriesId,
    page,
    pageSize,
  });
}

export async function fetchMylistVideos(
  mylistId: string,
  page: number,
  pageSize: number,
): Promise<UserVideosResponse> {
  return invoke<UserVideosResponse>('fetch_mylist_videos', {
    mylistId,
    page,
    pageSize,
  });
}

export type UserMylistSummary = {
  id: string;
  name: string;
  description?: string;
  thumbnailUrl?: string;
  itemsCount?: number;
  isPublic: boolean;
};

/** `{ items, totalCount }` 形式のページネーション付きレスポンス。 */
export type Paged<T> = {
  items: T[];
  totalCount: number;
};

export type UserMylistsResponse = Paged<UserMylistSummary>;

export async function fetchUserMylists(ownerId: string): Promise<UserMylistsResponse> {
  return invoke<UserMylistsResponse>('fetch_user_mylists', { ownerId });
}

export type UserSeriesSummary = {
  id: string;
  title: string;
  description?: string;
  thumbnailUrl?: string;
  itemsCount?: number;
};

export type UserSeriesListResponse = Paged<UserSeriesSummary>;

export async function fetchUserSeriesList(ownerId: string): Promise<UserSeriesListResponse> {
  return invoke<UserSeriesListResponse>('fetch_user_series_list', { ownerId });
}

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

/** `LocalPlayerCommentDto` (Rust 由来、null 明示) を player 内部用の
 *  `PlayerComment` (`?:` 省略) に変換する。`null → undefined` の差を吸収するだけ。 */
export function dtoToPlayerComment(c: LocalPlayerCommentDto): PlayerComment {
  return {
    id: c.id,
    no: c.no,
    vposMs: c.vposMs,
    content: c.content,
    mail: c.mail,
    commands: c.commands,
    userId: c.userId ?? undefined,
    postedAt: c.postedAt ?? undefined,
    fork: c.fork,
    isOwner: c.isOwner,
    nicoruCount: c.nicoruCount ?? undefined,
    score: c.score ?? undefined,
  };
}

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
  isShort: boolean;
};

export async function listLibraryVideos(): Promise<LibraryVideoItem[]> {
  return invoke<LibraryVideoItem[]>('list_library_videos');
}

// =================== ライブラリ検索・整列・集計 ===================

export type LibraryQueryParams = {
  q?: string;
  tags?: string[];
  tagsAny?: string[];
  uploaderId?: string;
  /** `"user"` / `"channel"`。ID の名前空間が別なので、種別まで分かって
   *  いる時は一緒に送ると同じ数値 ID の別投稿者が混ざらない。 */
  uploaderType?: string;
  minDuration?: number;
  maxDuration?: number;
  resolution?: string;
  isShort?: boolean;
  sortBy?: string;
  sortOrder?: 'asc' | 'desc';
  /** `sortBy: 'random'` の並びをページ間で固定する種。 */
  randomSeed?: number;
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
  isShort: boolean;
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

export type CommentSearchHit = {
  videoId: string;
  videoTitle: string;
  commentNo: number;
  vposMs: number;
  content: string;
  userHash: string | null;
  postedAt: number | null;
};

export type CommentSearchResult = {
  items: CommentSearchHit[];
  totalCount: number;
  offset: number;
  limit: number;
};

export async function searchLibraryComments(
  query: string,
  offset?: number,
  limit?: number,
): Promise<CommentSearchResult> {
  return invoke<CommentSearchResult>('search_library_comments', {
    query,
    offset: offset ?? 0,
    limit: limit ?? 50,
  });
}

export type UploaderInfo = {
  uploaderId: string;
  uploaderName: string | null;
  videoCount: number;
  totalDurationSec: number;
  /** `"user"` / `"channel"`。オンライン検索で userId と channelId の
   *  どちらで絞るかがこれで決まる。不明な取り込み済み動画は null。 */
  uploaderType: string | null;
};

export async function listLibraryUploaders(limit?: number): Promise<UploaderInfo[]> {
  return invoke<UploaderInfo[]>('list_library_uploaders', { limit: limit ?? 50 });
}

export async function prepareLocalPlayback(
  videoId: string,
  snapshotId?: number | null,
): Promise<LocalPlaybackPayload | null> {
  return invoke<LocalPlaybackPayload | null>('prepare_local_playback', {
    videoId,
    snapshotId: snapshotId ?? null,
  });
}

// =================== コメントスナップショット運用 ===================

export type CommentSnapshotRow = {
  id: number;
  videoId: string;
  takenAt: number;
  isInitial: boolean;
  commentCount: number;
  note: string | null;
};

export async function listCommentSnapshots(videoId: string): Promise<CommentSnapshotRow[]> {
  return invoke<CommentSnapshotRow[]>('list_comment_snapshots', { videoId });
}

export async function loadSnapshotComments(snapshotId: number): Promise<LocalPlayerCommentDto[]> {
  return invoke<LocalPlayerCommentDto[]>('load_snapshot_comments', { snapshotId });
}

export async function deleteCommentSnapshot(snapshotId: number): Promise<boolean> {
  return invoke<boolean>('delete_comment_snapshot', { snapshotId });
}

export async function updateSnapshotNote(
  snapshotId: number,
  note: string | null,
): Promise<boolean> {
  return invoke<boolean>('update_snapshot_note', { snapshotId, note });
}

export async function refetchVideoComments(videoId: string): Promise<number> {
  return invoke<number>('refetch_video_comments', { videoId });
}

export async function remuxLocalVideo(videoId: string): Promise<string> {
  return invoke<string>('remux_local_video', { videoId });
}

// =================== コメント焼き込みエクスポート ===================
//
// 実装は `src/lib/burnin/` へ移動した。WebView 上で `@xpadev-net/niconicomments`
// が 1 フレームずつ描画し、PNG を Rust 経由で ffmpeg へ流し込んで元動画へ
// オーバーレイする (niconicomments-convert と同じ方式)。座標・サイズの算出は
// niconicomments 本体に完全委譲しており、旧来の独自 ASS 生成は廃止した。
export { runBurnInExport, type BurnInExportOptions, type BurnInResult } from './burnin/browser';

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

// =================== プレイリスト ===================

export type Playlist = {
  id: number;
  name: string;
  parentId: number | null;
  source: string;
  sourceOfficialId: string | null;
  importedAt: number | null;
  createdAt: number;
  updatedAt: number;
  itemCount: number;
};

export type PlaylistItem = {
  playlistId: number;
  videoId: string;
  position: number;
  addedAt: number;
  note: string | null;
  title: string | null;
  thumbnailUrl: string | null;
  durationSec: number | null;
};

export async function listPlaylists(): Promise<Playlist[]> {
  return invoke<Playlist[]>('list_playlists');
}

export async function createPlaylist(name: string, parentId?: number | null): Promise<Playlist> {
  return invoke<Playlist>('create_playlist', { name, parentId: parentId ?? null });
}

export async function updatePlaylist(
  id: number,
  name: string,
  parentId?: number | null,
): Promise<Playlist> {
  return invoke<Playlist>('update_playlist', { id, name, parentId: parentId ?? null });
}

export async function deletePlaylist(id: number): Promise<boolean> {
  return invoke<boolean>('delete_playlist', { id });
}

export async function listPlaylistItems(playlistId: number): Promise<PlaylistItem[]> {
  return invoke<PlaylistItem[]>('list_playlist_items', { playlistId });
}

export async function addPlaylistItem(
  playlistId: number,
  videoId: string,
  position?: number | null,
  note?: string | null,
): Promise<PlaylistItem> {
  return invoke<PlaylistItem>('add_playlist_item', {
    playlistId,
    videoId,
    position: position ?? null,
    note: note ?? null,
  });
}

export async function removePlaylistItem(playlistId: number, videoId: string): Promise<boolean> {
  return invoke<boolean>('remove_playlist_item', { playlistId, videoId });
}

// =================== 再生履歴 ===================

export type PlayHistoryItem = {
  id: number;
  videoId: string;
  playedAt: number;
  durationPlayedSec: number;
  positionAtCloseSec: number | null;
  title: string | null;
  thumbnailUrl: string | null;
  durationSec: number | null;
  isShort: boolean;
};

export async function recordPlayback(
  videoId: string,
  durationPlayedSec: number,
  positionAtCloseSec?: number | null,
): Promise<PlayHistoryItem> {
  return invoke<PlayHistoryItem>('record_playback', {
    videoId,
    durationPlayedSec,
    positionAtCloseSec: positionAtCloseSec ?? null,
  });
}

export async function listPlayHistory(
  offset?: number,
  limit?: number,
  isShort?: boolean | null,
): Promise<PlayHistoryItem[]> {
  return invoke<PlayHistoryItem[]>('list_play_history', {
    offset: offset ?? 0,
    limit: limit ?? 50,
    isShort: isShort ?? null,
  });
}

export async function deletePlayHistoryItem(id: number): Promise<boolean> {
  return invoke<boolean>('delete_play_history_item', { id });
}

/** yt-dlp / ffmpeg をどこから見つけたか。Rust 側 `BinarySource::as_str` と対。 */
export type BinarySource = 'managed' | 'bundled' | 'sidecar' | 'system_path' | 'not_found';

export type AppInfo = {
  version: string;
  identifier: string;
  dataDir: string;
  videosDir: string;
  dbPath: string;
  localServerPort: number;
  ytdlpAvailable: boolean;
  ytdlpVersion: string | null;
  ytdlpSource: BinarySource;
  ytdlpPath: string;
  ffmpegAvailable: boolean;
  ffmpegVersion: string | null;
  ffmpegSource: BinarySource;
  ffmpegPath: string;
  libraryVideoCount: number;
  libraryVideosSizeBytes: number;
};

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('get_app_info');
}

// =================== yt-dlp アップデート追従 ===================

/** 追従するリリースチャンネル。 */
export type YtdlpChannel = 'stable' | 'nightly';

export type YtdlpUpdateCheck = {
  channel: YtdlpChannel;
  /** 今 yt-dlp として使われている実行ファイル。 */
  currentPath: string;
  /** その版 (`--version` 実測)。取れなければ null。 */
  currentVersion: string | null;
  currentSource: BinarySource;
  /** リモートの最新版。オフライン等で取れなければ null。 */
  latestVersion: string | null;
  releaseUrl: string | null;
  updateAvailable: boolean;
  /** アプリが管理している版を入れているか。 */
  managedInstalled: boolean;
  managedPath: string | null;
  /** 同梱 / PATH 版の方が新しいか (= 管理下の物を消した方が良い)。 */
  fallbackIsNewer: boolean;
  fallbackVersion: string | null;
  lastCheckedAt: number | null;
  /** 取得経路 (`api` / `redirect`)。診断用。 */
  via: string | null;
  /** 最新版を取れなかった理由。 */
  error: string | null;
};

/** インストールの進行フェーズ。Rust 側 `UpdateProgress.phase` と対。 */
export type YtdlpUpdatePhase =
  | 'idle'
  | 'checking'
  | 'downloading'
  | 'verifying'
  | 'installing'
  | 'done'
  | 'error';

export type YtdlpUpdateProgress = {
  phase: YtdlpUpdatePhase;
  downloadedBytes: number;
  totalBytes: number | null;
  version: string | null;
  message: string | null;
};

export type YtdlpInstallResult = {
  version: string;
  path: string;
  bytes: number;
  /** リリース同梱の SHA2-256SUMS と照合できたか。 */
  sha256Verified: boolean;
  channel: YtdlpChannel;
};

/** 現在の yt-dlp とリモート最新版を調べる。`channel` 省略時は設定値を使う。 */
export async function ytdlpCheckUpdate(channel?: YtdlpChannel): Promise<YtdlpUpdateCheck> {
  return invoke<YtdlpUpdateCheck>('ytdlp_check_update', { channel: channel ?? null });
}

/** 最新版を落として `<app_data_dir>/bin` に入れる。 */
export async function ytdlpInstallUpdate(channel?: YtdlpChannel): Promise<YtdlpInstallResult> {
  return invoke<YtdlpInstallResult>('ytdlp_install_update', { channel: channel ?? null });
}

/** インストール中の進捗。ポーリングして読む。 */
export async function ytdlpUpdateStatus(): Promise<YtdlpUpdateProgress> {
  return invoke<YtdlpUpdateProgress>('ytdlp_update_status');
}

/** 管理下の yt-dlp を消して同梱 / PATH 版に戻す。消す物が無ければ false。 */
export async function ytdlpRemoveManaged(): Promise<boolean> {
  return invoke<boolean>('ytdlp_remove_managed');
}
