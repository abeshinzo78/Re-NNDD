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

/** プラグインが追加する設定項目。`key` は `plugin.<plugin_id>.` で始める。 */
export type PluginSettingDef = {
  key: string;
  label: string;
  description?: string;
  kind: 'bool' | 'number' | 'select' | 'text';
  default: unknown;
  options?: { value: string; label: string }[];
  min?: number;
  max?: number;
  step?: number;
};

/** プラグインが追加するサイドバーナビ項目。 */
export type PluginNavEntry = {
  /** ルーティング先。`/plugin/<id>/` で始めるのが慣例。 */
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
    /** plugin.<id>.* キーのみ。それ以外は dispatcher が拒否する。 */
    get(key: string): Promise<unknown>;
    set(key: string, value: string): Promise<void>;
  };
  nav: { addPage(entry: PluginNavEntry): void };
  items: { addAction(action: PluginItemAction): void };
  player: { addAction(action: PluginPlayerAction): void };
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
