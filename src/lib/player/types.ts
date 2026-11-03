export type VideoTag = {
  name: string;
  isLocked: boolean;
  isCategory: boolean;
};

export type WatchVideoMeta = {
  id: string;
  title: string;
  description: string;
  duration: number;
  thumbnailUrl?: string;
  registeredAt?: string;
  isDeleted: boolean;
  viewCount?: number;
  commentCount?: number;
  mylistCount?: number;
  likeCount?: number;
  tags: VideoTag[];
  contentType?: string;
};

export type WatchOwner = {
  kind: string;
  id?: string;
  nickname?: string;
  iconUrl?: string;
};

export type PickedQuality = {
  videoTrack: string;
  audioTrack: string;
  label?: string;
};

export type NvCommentSetup = {
  server: string;
  threadKey: string;
  params: unknown;
};

export type PlayerComment = {
  id: string;
  no: number;
  vposMs: number;
  content: string;
  mail: string;
  commands: string[];
  userId?: string;
  postedAt?: string;
  fork: string;
  isOwner: boolean;
  nicoruCount?: number;
  score?: number;
};

/** シリーズ内の前後の動画。 */
export type SeriesVideoRef = {
  id: string;
  title: string;
  thumbnailUrl?: string;
};

export type SeriesInfo = {
  id: number;
  title: string;
  description?: string;
  thumbnailUrl?: string;
  itemsCount?: number;
  isListed: boolean;
  prevVideo?: SeriesVideoRef;
  nextVideo?: SeriesVideoRef;
  firstVideo?: SeriesVideoRef;
};

export type WatchGenre = {
  key: string;
  label: string;
};

export type PlaybackPayload = {
  video: WatchVideoMeta;
  owner?: WatchOwner;
  series?: SeriesInfo;
  hlsUrl: string;
  pickedQuality: PickedQuality;
  allQualities: PickedQuality[];
  nvComment: NvCommentSetup | null;
  accessRightKey: string;
  videoId: string;
  isShort: boolean;
  /** watch 取得時の actionTrackId。再生開始時の視聴記録に使う。 */
  actionTrackId?: string;
  /** ゲストとして取得したデータか。 */
  isGuest?: boolean;
  /** セッション Cookie はあるのにゲスト扱いだった (= 期限切れの疑い)。 */
  sessionExpired?: boolean;
  genre?: WatchGenre;
};
