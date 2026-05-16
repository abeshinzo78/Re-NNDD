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
  tags: VideoTag[];
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

export type SeriesInfo = {
  id: number;
  title: string;
  description?: string;
  thumbnailUrl?: string;
  itemsCount?: number;
  isListed: boolean;
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
};
