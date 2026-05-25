// プラグインシステムの型定義。
//
// プラグインはフロントの ES module で、ユーザーが $APPDATA/plugins/<id>/
// に置く。host (`host.ts`) が起動時にインストール済み一覧を取得し、有効化
// されているものを `loader.ts` で動的 import する。

export type PluginManifest = {
  id: string;
  name: string;
  version: string;
  entry: string;
  description?: string | null;
  author?: string | null;
  homepage?: string | null;
  minAppVersion?: string | null;
  permissions?: string[];
};

export type PluginInfo = {
  pluginId: string;
  name: string;
  version: string;
  enabled: boolean;
  description?: string | null;
  author?: string | null;
  homepage?: string | null;
  entry: string;
  permissions: string[];
  entryAbsPath: string;
  installedAt: number;
  updatedAt: number;
};

/** プラグインが追加する設定項目。`key` は `plugin:<plugin_id>:` で始める。 */
export type PluginSettingDef = {
  key: string;
  label: string;
  description?: string;
  kind: 'bool' | 'number' | 'select' | 'text';
  /** 既定値。ユーザがまだ値を保存していないときに表示される。 */
  default: unknown;
  options?: { value: string; label: string }[];
  min?: number;
  max?: number;
  step?: number;
};

/** プラグインが追加するサイドバーナビ項目。 */
export type PluginNavEntry = {
  /** ルーティング先。プラグイン専用ページは `/plugin/<id>/<path>` を使う。
   *  動的ルート `src/routes/plugin/[id]/[...path]/+page.svelte` が
   *  `ctx.pages.register(path, render)` で登録された renderer を mount する。 */
  href: string;
  label: string;
};

/** 動画カードのメニューに差し込むアクション。 */
export type PluginItemAction<Hit = unknown> = {
  label: string;
  /** false を返すと描画されない。デフォルトは常に true。 */
  appliesTo?: (hit: Hit) => boolean;
  handler: (hit: Hit) => void | Promise<void>;
};

/** プレイヤーのコントロールバーに差し込むアクション。 */
export type PluginPlayerAction = {
  label: string;
  /** ボタン表示用の絵文字またはテキスト。 */
  icon?: string;
  handler: () => void | Promise<void>;
  /** 任意の単一キーのキーボードショートカット (組込みショートカット優先)。 */
  key?: string;
};

/** プラグインが追加するコマンドパレットエントリ。Ctrl/⌘+K で開いて検索。
 *  組込みコマンドより常に下に表示される (組込み優先)。 */
export type PluginCommand = {
  /** 一意のID (プラグイン内)。表示には使わない。 */
  id: string;
  /** 表示ラベル。例: "メモを書き出す" */
  title: string;
  /** 補足 (例: "現在の動画にメモを書き出す")。 */
  hint?: string;
  /** 検索キーワード (ラベル以外で hit させたい単語)。 */
  keywords?: string[];
  handler: () => void | Promise<void>;
};

/** プラグインページの renderer。`ctx.pages.register(path, render)` で登録。
 *  `path` はプラグイン内パス (`/dashboard` 等; 先頭スラッシュ任意)。SvelteKit
 *  の `/plugin/<plugin_id><path>` で mount される。
 *
 *  `el` は SvelteKit が用意した空の `<div>`。renderer は `el.innerHTML = ...`
 *  もしくは DOM API で要素を生成して詰めること。返す cleanup 関数は、ページ
 *  離脱時 (= SvelteKit のルート遷移) に呼ばれる — タイマー解除や listener
 *  解除に使う。 */
export type PluginPageRenderer = (
  el: HTMLElement,
) => void | (() => void) | Promise<void | (() => void)>;

/** プラグインから観測できる Player の状態。`ctx.player.getState()` で取得。
 *  非再生中 (= Player がマウントされていない) は `videoId: null`。 */
export type PlayerObservedState = {
  videoId: string | null;
  currentTime: number;
  duration: number;
  paused: boolean;
  volume: number;
  muted: boolean;
  playbackRate: number;
};

/** ホストが emit する標準イベントの payload 型マップ。
 *  プラグインは `ctx.events.emit('custom:foo', payload)` で任意のイベントも
 *  emit できる (型はゆるく unknown)。
 *  注: ここに載っているイベントのみ host が実際に emit する。設計途上で
 *  declared だが emit していなかった download:progress / library:* は型
 *  からも削除した (Codex 別件: dead-event 型と実装の乖離)。 */
export type StandardPluginEventMap = {
  'player:play': { videoId: string; currentTime: number };
  'player:pause': { videoId: string; currentTime: number };
  'player:time': { videoId: string; currentTime: number };
  'player:ended': { videoId: string };
  'download:start': { id: number; videoId: string };
  'download:complete': { id: number; videoId: string };
  'download:error': { id: number; videoId: string; message: string };
  /** dispatcher の notify.toast から発火される (`{pluginId, message, kind}`)。 */
  'notify:toast': { pluginId: string; message: string; kind: string };
  /** dispatcher の player.command から発火される。Player.svelte が消費する。
   *  プラグインは購読しなくてよい (host 内部イベント)。 */
  'plugin:player:control': {
    pluginId: string;
    kind: 'play' | 'pause' | 'toggle' | 'seek' | 'setRate' | 'setVolume' | 'toggleMute';
    value: number | null;
  };
};

/** プラグインに渡す context。`activate(ctx)` で受け取る。 */
export type PluginContext = {
  manifest: PluginManifest;
  events: {
    on<K extends keyof StandardPluginEventMap>(
      name: K,
      handler: (payload: StandardPluginEventMap[K]) => void,
    ): () => void;
    on(name: string, handler: (payload: unknown) => void): () => void;
    emit(name: string, payload: unknown): void;
  };
  settings: {
    register(def: PluginSettingDef): void;
    /** plugin:<id>:* キーのみ。それ以外は dispatcher が拒否する。 */
    get(key: string): Promise<unknown>;
    set(key: string, value: string): Promise<void>;
  };
  nav: { addPage(entry: PluginNavEntry): void };
  items: { addAction(action: PluginItemAction): void };
  player: {
    /** ControlBar にボタンを追加。 */
    addAction(action: PluginPlayerAction): void;
    /** 現在の Player 状態を同期取得。Player がマウントされていなければ
     *  `videoId: null` の zero state を返す。 */
    getState(): PlayerObservedState;
    /** 動画を再生 (一時停止中なら再生開始)。 */
    play(): Promise<void>;
    /** 動画を一時停止。 */
    pause(): Promise<void>;
    /** 再生/一時停止を反転。 */
    toggle(): Promise<void>;
    /** 動画の指定秒へシーク。範囲外は Player 側で clamp される。 */
    seek(toSec: number): Promise<void>;
    /** 再生速度。0.25 〜 4.0 程度を想定 (実装は <video> に委譲)。 */
    setRate(rate: number): Promise<void>;
    /** 音量。0.0 〜 1.0。 */
    setVolume(vol: number): Promise<void>;
    /** ミュート切替。 */
    toggleMute(): Promise<void>;
  };
  /** コマンドパレット (Ctrl/⌘+K) への登録。 */
  commands: {
    register(cmd: PluginCommand): void;
  };
  /** プラグイン専用ページの登録。`/plugin/<id><subpath>` に動的ルートが
   *  対応しているので、`ctx.pages.register('/dashboard', render)` で登録した
   *  あと `ctx.nav.addPage({ href: '/plugin/<id>/dashboard', label: '...' })`
   *  すると、サイドバー → そのページに遷移できる。 */
  pages: {
    register(subpath: string, render: PluginPageRenderer): void;
  };
  /** UI ヘルパ。 */
  ui: {
    /** トーストを表示。dispatcher の notify.toast 経由ではなく、フロント
     *  直接 (権限 `notify` 不要)。プラグイン作者がよく使う一手間を削減。 */
    toast(message: string, kind?: 'info' | 'ok' | 'warn' | 'error'): void;
  };
  invoke(action: string, payload?: unknown): Promise<unknown>;
  log: {
    info: (...args: unknown[]) => void;
    warn: (...args: unknown[]) => void;
    error: (...args: unknown[]) => void;
  };
};

/** プラグインが export することを期待する shape。 `activate` は optional。 */
export type PluginModule = {
  activate?: (ctx: PluginContext) => void | Promise<void>;
  deactivate?: () => void | Promise<void>;
};
