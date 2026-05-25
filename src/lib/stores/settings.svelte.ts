// アプリ設定。Rust 側 `settings` テーブルに永続化。
// 値は KV (string-string)、型は JS 側でパース。
//
// 既定値はこのファイルで一元管理。DB に未登録 = 既定値、と扱う。
//
// 起動時に `loadSettings()` を 1 回呼んで in-memory に load しておけば、
// 各画面は同期的に値を読める。書き換えは `setSetting(key, value)` で
// in-memory 即時更新 + DB 永続化を非同期で行う。

import { SvelteSet } from 'svelte/reactivity';
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
    key: 'playback.autoplay_queue',
    label: 'プレイリスト/シリーズの連続再生',
    description: '連続再生キューが入っている時、現在の動画が終わると次の動画へ自動で遷移する',
    section: 'playback',
    default: true,
    kind: 'bool',
    order: 36,
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
    description: 'アプリ全体の配色と質感',
    section: 'appearance',
    default: 'dark',
    kind: 'select',
    options: [
      { value: 'dark', label: 'ダーク' },
      { value: 'niconico-classic', label: 'ニコニコクラシック' },
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
  // プラグイン機構のマスターキルスイッチ。OFF にすると bootstrapPluginHost()
  // が即 return し、有効化されているプラグインも一切ロードされない (=
  // プラグイン機構導入前と完全に同一の挙動になる)。再起動不要 (次回起動から効く)。
  {
    key: 'plugins.enabled',
    label: 'プラグイン機構を有効にする',
    description: 'OFF にするとインストール済みプラグインも一切ロードされない (再起動で反映)',
    section: 'advanced',
    default: true,
    kind: 'bool',
    order: 90,
  },
] as const satisfies readonly SettingDef<unknown>[];

export type SettingKey = (typeof SETTING_DEFS)[number]['key'];

// ----- in-memory state (Svelte 5 runes) -----

const cache = $state<Record<string, string>>({});
let loaded = false;
let loadPromise: Promise<void> | null = null;

// app.html の sync 初期化が data-theme を読む localStorage キー。
// `setSetting` (DB write 成功時) と `resetSetting` (DB delete 成功時)、
// および `loadSettings` (DB load 完了時) からのみ更新する。
// layout.svelte の $effect からは書かない (= DB との乖離を防ぐ)。
const LS_THEME_KEY = 'appearance.theme';

function syncThemeToLocalStorage(value: string | null): void {
  if (typeof globalThis === 'undefined') return;
  const ls = (globalThis as unknown as { localStorage?: Storage }).localStorage;
  if (!ls) return;
  try {
    if (value == null) ls.removeItem(LS_THEME_KEY);
    else ls.setItem(LS_THEME_KEY, value);
  } catch {
    /* localStorage 不可環境 (シークレットモード等) は無視 */
  }
}

// 起動直後にローカルストレージから theme だけ先行シードしておく。
// loadSettings() は Tauri invoke で非同期、それを待つあいだに $derived の
// theme が def.default ('dark') を返してしまい、せっかく app.html の sync
// script が data-theme をクラシックに揃えても layout の $effect が一過性に
// 'dark' で上書きする (classic → dark → classic の FOUC)。
// 永続化は DB が正だが、表示の整合のためにここでもキャッシュを温める。
// `loadSettings()` 完了時に DB 値で上書きされ、DB に無ければ後段で削除する
// (= DB が空 = default を採用、を尊重)。
const seededKeys = new SvelteSet<string>();
if (typeof globalThis !== 'undefined') {
  const ls = (globalThis as unknown as { localStorage?: Storage }).localStorage;
  if (ls) {
    try {
      const t = ls.getItem(LS_THEME_KEY);
      if (t != null) {
        cache[LS_THEME_KEY] = t;
        seededKeys.add(LS_THEME_KEY);
      }
    } catch {
      /* localStorage 不可環境は無視 */
    }
  }
}

export function loadSettings(): Promise<void> {
  if (loadPromise) return loadPromise;
  loadPromise = (async () => {
    try {
      const all = await getSettings();
      for (const k of Object.keys(all)) {
        cache[k] = all[k];
        // DB が canonical を返したので seed フラグはクリア。
        seededKeys.delete(k);
      }
      // DB に無い (= default を使う semantics) のに localStorage seed で
      // cache に残っている値は削除する。新規プロファイル / DB リセット /
      // 別 DB のインポート後で localStorage だけ古い値が残っているケース
      // で、seed が永続的に残って DB と乖離する問題を防ぐ
      // (codex review r3293708193)。
      for (const k of seededKeys) {
        delete cache[k];
      }
      seededKeys.clear();
      // DB の真値を localStorage にも反映 (次回起動の app.html sync 用)。
      // DB に theme が無ければ localStorage も消す (= default 'dark' で
      // bootstrap される)。
      syncThemeToLocalStorage(all[LS_THEME_KEY] ?? null);
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
  // DB 書き込みが成功してから localStorage に反映する。await 前に
  // ミラーすると、DB エラーで失敗した値を次回起動時に app.html sync が
  // 拾って、DB と乖離した状態で起動してしまう (codex review r3293708194)。
  if (key === LS_THEME_KEY) syncThemeToLocalStorage(raw);
}

/** 既定値に戻す (DB 行削除)。 */
export async function resetSetting(key: SettingKey): Promise<void> {
  delete cache[key];
  await deleteSettingRaw(key);
  if (key === LS_THEME_KEY) syncThemeToLocalStorage(null);
}

/** Svelte 内で reactivity に使えるラッパ。`$derived(get(key))` で値を読める。 */
export function get(key: SettingKey): unknown {
  // cache 自体が $state なのでアクセスすればトラックされる
  return parseValue(defOf(key)!, cache[key]);
}

// ----- プラグイン設定用の Raw アクセサ -----
//
// プラグインが `ctx.settings.register({key: 'plugin:<id>:foo', ...})` で
// 登録した設定項目を、設定画面の組込み UI から編集できるようにするための
// 低レベル API。`SettingKey` 制約をバイパスするので呼び出し元の責務は
// 「キーが安全な名前空間 (plugin:<id>:*) であること」の確認。
//
// `SETTING_DEFS` 由来のキー (例: 'playback.autoplay') は通常 `getBool` 系
// 経由でアクセスするべきだが、ここを通すのも禁止していない (互換性のため)。

/** 生キーで値を取得。未登録なら undefined。値は文字列のまま。 */
export function getRawSetting(key: string): string | undefined {
  // cache に直アクセス。reactive 追跡したいなら `get` を見ること。
  // SETTING_DEFS 制約に縛られない汎用 getter。
  return cache[key];
}

/** 生キーで値を保存。in-memory + DB 両方に書く。 */
export async function setRawSetting(key: string, value: string): Promise<void> {
  cache[key] = value;
  await setSettingRaw(key, value);
  // theme は SETTING_DEFS 由来のキーなので localStorage 同期もここで拾う。
  if (key === LS_THEME_KEY) syncThemeToLocalStorage(value);
}

/** 生キーで値を削除 (default に戻す)。 */
export async function resetRawSetting(key: string): Promise<void> {
  delete cache[key];
  await deleteSettingRaw(key);
  if (key === LS_THEME_KEY) syncThemeToLocalStorage(null);
}

/** 現在 cache されている設定キーの一覧 (= DB に書き込みのあったキー)。
 *  プラグイン設定の編集 UI が「未設定なら default を表示」を判定するときに使う。 */
export function listRawSettingKeys(): string[] {
  return Object.keys(cache);
}
