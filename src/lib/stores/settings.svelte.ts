// アプリ設定。Rust 側 `settings` テーブルに永続化。
// 値は KV (string-string)、型は JS 側でパース。
//
// 既定値はこのファイルで一元管理。DB に未登録 = 既定値、と扱う。
//
// 起動時に `loadSettings()` を 1 回呼んで in-memory に load しておけば、
// 各画面は同期的に値を読める。書き換えは `setSetting(key, value)` で
// in-memory 即時更新 + DB 永続化を非同期で行う。

import { deleteSettingRaw, getSettings, setSettingRaw } from '$lib/api';

// ----- 設定キー定義 -----

export type SettingDef<T> = {
  key: string;
  /** UI ラベル */
  label: string;
  /** 説明文 (optional) */
  description?: string;
  /** UI セクション */
  section: 'playback' | 'comment' | 'download' | 'library' | 'appearance' | 'advanced';
  /** 既定値 */
  default: T;
  /** UI コントロールの型 */
  kind: 'bool' | 'number' | 'select' | 'text';
  /** select の選択肢 (kind='select' のとき) */
  options?: { value: string; label: string }[];
  /** number の min/max/step */
  min?: number;
  max?: number;
  step?: number;
  /** UI 並び順 */
  order: number;
};

// 設定一覧 (順序が UI 表示順)
export const SETTING_DEFS = [
  // === 再生 ===
  {
    key: 'playback.resume_enabled',
    label: '続きから再生',
    description: '前回の再生位置から自動的に再生を再開する',
    section: 'playback',
    default: true,
    kind: 'bool',
    order: 10,
  },
  {
    key: 'playback.always_loop',
    label: '常にリピート再生',
    description: '動画を最後まで見たら自動的に最初から再生する',
    section: 'playback',
    default: false,
    kind: 'bool',
    order: 20,
  },
  {
    key: 'playback.autoplay',
    label: '自動再生',
    description: '動画を開いたら即座に再生を開始する',
    section: 'playback',
    default: true,
    kind: 'bool',
    order: 30,
  },
  {
    key: 'pip.auto_navigate',
    label: 'ページ移動時に自動的に PiP にする',
    description: '再生ページから別のページに移動する時に自動でミニプレイヤーを開始する',
    section: 'playback',
    default: false,
    kind: 'bool',
    order: 35,
  },
  {
    key: 'playback.default_rate',
    label: 'デフォルト再生速度',
    description: '動画を開いた直後の再生倍率',
    section: 'playback',
    default: 1.0,
    kind: 'select',
    options: [
      { value: '0.5', label: '0.5x' },
      { value: '0.75', label: '0.75x' },
      { value: '1.0', label: '1.0x' },
      { value: '1.25', label: '1.25x' },
      { value: '1.5', label: '1.5x' },
      { value: '2.0', label: '2.0x' },
    ],
    order: 40,
  },
  {
    key: 'playback.default_volume',
    label: 'デフォルト音量',
    description: '0〜1 の範囲',
    section: 'playback',
    default: 1.0,
    kind: 'number',
    min: 0,
    max: 1,
    step: 0.05,
    order: 50,
  },

  // === コメント ===
  {
    key: 'comment.default_enabled',
    label: 'コメ初期表示',
    description: '動画を開いた時にコメントを表示するか',
    section: 'comment',
    default: true,
    kind: 'bool',
    order: 10,
  },
  {
    key: 'comment.default_opacity',
    label: 'コメ透明度',
    description: '0 (透明) 〜 1 (不透明)',
    section: 'comment',
    default: 1.0,
    kind: 'number',
    min: 0.1,
    max: 1.0,
    step: 0.05,
    order: 20,
  },

  // === ダウンロード ===
  {
    key: 'download.parallelism',
    label: '並列ダウンロード数',
    description: '同時に DL する動画の最大本数',
    section: 'download',
    default: 2,
    kind: 'number',
    min: 1,
    max: 10,
    step: 1,
    order: 10,
  },
  {
    key: 'download.default_quality',
    label: 'デフォルト画質',
    description: 'auto は yt-dlp 任せ (最高画質)',
    section: 'download',
    default: 'auto',
    kind: 'select',
    options: [
      { value: 'auto', label: '自動 (最高)' },
      { value: '1080p', label: '1080p' },
      { value: '720p', label: '720p' },
      { value: '480p', label: '480p' },
      { value: '360p', label: '360p' },
    ],
    order: 20,
  },

  // === ライブラリ ===
  {
    key: 'library.default_view',
    label: '表示モード',
    section: 'library',
    default: 'grid',
    kind: 'select',
    options: [
      { value: 'grid', label: 'グリッド' },
      { value: 'list', label: 'リスト' },
    ],
    order: 10,
  },
  {
    key: 'library.default_sort',
    label: 'デフォルトソート',
    section: 'library',
    default: 'downloaded_at_desc',
    kind: 'select',
    options: [
      { value: 'downloaded_at_desc', label: 'DL 新しい順' },
      { value: 'posted_at_desc', label: '投稿日 新しい順' },
      { value: 'title_asc', label: 'タイトル昇順' },
      { value: 'play_count_desc', label: '再生回数順' },
    ],
    order: 20,
  },

  // === 外観 ===
  {
    key: 'appearance.theme',
    label: 'テーマ',
    description: '切り替えは即座に反映されます',
    section: 'appearance',
    default: 'niconico',
    kind: 'select',
    options: [
      { value: 'niconico', label: 'ニコニコ風 (オリジナル)' },
      { value: 'dark', label: 'ダーク' },
      { value: 'light', label: 'ライト' },
    ],
    order: 10,
  },

  // === 高度な設定 ===
  {
    key: 'advanced.log_level',
    label: 'ログレベル',
    description: 'NNDD_LOG 環境変数より優先度低 (再起動で反映)',
    section: 'advanced',
    default: 'info',
    kind: 'select',
    options: [
      { value: 'error', label: 'error' },
      { value: 'warn', label: 'warn' },
      { value: 'info', label: 'info' },
      { value: 'debug', label: 'debug' },
      { value: 'trace', label: 'trace' },
    ],
    order: 10,
  },
] as const satisfies readonly SettingDef<unknown>[];

export type SettingKey = (typeof SETTING_DEFS)[number]['key'];

// ----- in-memory state (Svelte 5 runes) -----

const cache = $state<Record<string, string>>({});
let loaded = false;
let loadPromise: Promise<void> | null = null;

export function loadSettings(): Promise<void> {
  if (loadPromise) return loadPromise;
  loadPromise = (async () => {
    try {
      const all = await getSettings();
      for (const k of Object.keys(all)) cache[k] = all[k];
    } finally {
      loaded = true;
    }
  })();
  return loadPromise;
}

export function isLoaded(): boolean {
  return loaded;
}

function defOf(key: string): SettingDef<unknown> | undefined {
  return SETTING_DEFS.find((d) => d.key === key);
}

function parseValue(def: SettingDef<unknown>, raw: string | undefined): unknown {
  if (raw == null) return def.default;
  switch (def.kind) {
    case 'bool':
      return raw === 'true';
    case 'number': {
      const n = Number(raw);
      return Number.isFinite(n) ? n : def.default;
    }
    default:
      return raw;
  }
}

/** 値を取得 (in-memory cache から)。未ロードでも default を返す。 */
export function getSetting<T = unknown>(key: SettingKey): T {
  const def = defOf(key);
  if (!def) {
    throw new Error(`unknown setting key: ${key}`);
  }
  return parseValue(def, cache[key]) as T;
}

export function getBool(key: SettingKey): boolean {
  return getSetting<boolean>(key);
}
export function getNum(key: SettingKey): number {
  return getSetting<number>(key);
}
export function getStr(key: SettingKey): string {
  return getSetting<string>(key);
}

/** 値を保存。in-memory + DB に書く。 */
export async function setSetting(key: SettingKey, value: unknown): Promise<void> {
  const raw = String(value);
  cache[key] = raw;
  await setSettingRaw(key, raw);
}

/** 既定値に戻す (DB 行削除)。 */
export async function resetSetting(key: SettingKey): Promise<void> {
  delete cache[key];
  await deleteSettingRaw(key);
}

/** Svelte 内で reactivity に使えるラッパ。`$derived(get(key))` で値を読める。 */
export function get(key: SettingKey): unknown {
  // cache 自体が $state なのでアクセスすればトラックされる
  return parseValue(defOf(key)!, cache[key]);
}
