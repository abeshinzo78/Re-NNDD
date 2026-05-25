// 単一プラグインのロード/アンロード。
//
// プラグインは `$APPDATA/plugins/<id>/<entry>` に置かれた ES module。
// Tauri 2 の `convertFileSrc` で asset:// URL に変換し、`import(/* @vite-ignore */ url)`
// で動的読み込みする (アセットプロトコルは tauri.conf.json で $APPDATA/**
// にスコープ済み)。
//
// ロード失敗 / activate 中の throw は **全部 catch して console.error にとどめる**
// — 1 プラグインの失敗で他プラグインやホストが死なないことを最優先する。

import { convertFileSrc } from '@tauri-apps/api/core';
import * as bus from './eventBus';
import * as registry from './registry';
import { pluginInvoke } from './api';
import { getPlayerState } from './playerState.svelte';
import { showToast } from '$lib/toastStore.svelte';
import type {
  PluginCommand,
  PluginContext,
  PluginInfo,
  PluginItemAction,
  PluginManifest,
  PluginModule,
  PluginNavEntry,
  PluginPageRenderer,
  PluginPlayerAction,
  PluginSettingDef,
} from './types';

/** 一度ロードしたプラグインの deactivate を呼び出すためのフック保持。 */
const loadedModules = new Map<string, PluginModule>();

/** plugin id ごとのロード状態 (UI の "失敗" 表示用に保持)。 */
export type LoadState = 'loaded' | 'failed';
const loadStates = new Map<string, { state: LoadState; error?: string }>();

/** 進行中の activate を id ごとに 1 つだけ追跡し、unload が活性中の
 *  activate を await できるようにする (Codex #14)。 */
const activationsInFlight = new Map<string, Promise<void>>();

/** プラグインが `ctx.events.emit` で偽装できない host 予約イベント名。
 *  これらは Rust 側 dispatcher の `emit_event` 経由 (もしくは host 内コンポーネント
 *  からの `pluginBus.emit` 経由) でのみ発火される。
 *
 *  ここに載っていないと、`player.control` permission を持たないプラグインが
 *  `ctx.events.emit('plugin:player:control', {kind:'play'})` で Player を
 *  操作できる permission バイパスになる (Codex review r3299045281)。
 *
 *  test からの参照用に export しているが、プラグイン側 API ではない。 */
export const RESERVED_HOST_EVENT_NAMES: ReadonlySet<string> = new Set([
  'plugin:player:control',
  // 以下は Rust dispatcher が emit する標準イベント。プラグインが偽装すると
  // 他プラグインや host UI を欺ける (notify トーストの spoof、ダウンロード
  // 完了/失敗のフェイク通知など) ため、こちらも host emit のみに限定する。
  'notify:toast',
  'download:start',
  'download:complete',
  'download:error',
  // player 状態イベントは現状フロント (Player.svelte) が emit しているが、
  // プラグインが偽装しても他プラグインを誤動作させうるため同様に予約。
  'player:play',
  'player:pause',
  'player:time',
  'player:ended',
]);

export function getLoadState(pluginId: string): { state: LoadState; error?: string } | undefined {
  return loadStates.get(pluginId);
}

function buildContext(info: PluginInfo): PluginContext {
  const manifest: PluginManifest = {
    id: info.pluginId,
    name: info.name,
    version: info.version,
    entry: info.entry,
    description: info.description ?? null,
    author: info.author ?? null,
    homepage: info.homepage ?? null,
    permissions: info.permissions,
  };
  const pid = info.pluginId;
  const logTag = `[plugin:${pid}]`;
  // settings key prefix。`:` 区切りで dot-prefix 攻撃を防ぐ (dispatcher と一致)。
  const settingsPrefix = `plugin:${pid}:`;
  // player.command の薄いラッパ。kind の妥当性は Rust 側でも再検査される。
  const playerCmd = (kind: string, value?: number) =>
    pluginInvoke(pid, 'player.command', { kind, value: value ?? null }) as Promise<unknown>;
  return {
    manifest,
    events: {
      on(name: string, handler: (payload: never) => void) {
        return bus.on(pid, name, handler as (p: unknown) => void);
      },
      emit(name: string, payload: unknown) {
        // 予約イベントの偽装防止: Player などホスト側コンポーネントが
        // permission ゲート済みと信じて処理する内部イベントを、プラグインが
        // `ctx.events.emit` で勝手に発火できると、permission モデルが破られる。
        // 該当イベントは host (Rust dispatcher → bridge) からの emit のみ
        // 許可する (Codex review r3299045281)。
        if (RESERVED_HOST_EVENT_NAMES.has(name)) {
          console.warn(
            logTag,
            `events.emit rejected: ${name} は host 予約イベント (Rust dispatcher のみが発火可能)`,
          );
          return;
        }
        bus.emit(name, payload);
      },
    },
    settings: {
      register(def: PluginSettingDef) {
        // key prefix の防御 (Rust 側でも enforce されるが UX のため事前に弾く)
        if (!def.key.startsWith(settingsPrefix)) {
          console.warn(logTag, 'settings.register rejected: key must start with', settingsPrefix);
          return;
        }
        registry.addSetting(pid, def);
      },
      get(key: string) {
        return pluginInvoke(pid, 'settings.get', { key });
      },
      set(key: string, value: string) {
        return pluginInvoke(pid, 'settings.set', { key, value }) as Promise<void>;
      },
    },
    nav: {
      addPage(entry: PluginNavEntry) {
        registry.addNav(pid, entry);
      },
    },
    items: {
      addAction(action: PluginItemAction) {
        registry.addItemAction(pid, action);
      },
    },
    player: {
      addAction(action: PluginPlayerAction) {
        registry.addPlayerAction(pid, action);
      },
      // 状態取得は Rust を介さない (フロント module-state を読むだけ)。
      // permission チェックは `player.command` (= 実操作) にのみ課す設計
      // (state read は副作用ゼロ)。詳細は docs/plugins.md。
      getState() {
        return getPlayerState();
      },
      play() {
        return playerCmd('play') as Promise<void>;
      },
      pause() {
        return playerCmd('pause') as Promise<void>;
      },
      toggle() {
        return playerCmd('toggle') as Promise<void>;
      },
      seek(toSec: number) {
        return playerCmd('seek', toSec) as Promise<void>;
      },
      setRate(rate: number) {
        return playerCmd('setRate', rate) as Promise<void>;
      },
      setVolume(vol: number) {
        return playerCmd('setVolume', vol) as Promise<void>;
      },
      toggleMute() {
        return playerCmd('toggleMute') as Promise<void>;
      },
    },
    commands: {
      register(cmd: PluginCommand) {
        // `commands` permission を持たないプラグインは register できない。
        // permission モデルの一貫性のため (manifest で `commands` を宣言した
        // プラグインのみがコマンドパレットに項目を追加可能)。
        // Codex review r3298977870 の指摘に対応。
        if (!info.permissions.includes('commands')) {
          console.warn(
            logTag,
            'commands.register rejected: manifest.permissions に "commands" がありません',
          );
          return;
        }
        // 組込みコマンドの名前空間 `app.*` への侵害を弾く (UX 上の混乱防止)。
        // プラグインの commands は何でも入れていいが、`app.` 始まりだけ予約。
        if (cmd.id.startsWith('app.')) {
          console.warn(logTag, 'commands.register rejected: id `app.*` is reserved');
          return;
        }
        registry.addCommand(pid, cmd);
      },
    },
    pages: {
      register(subpath: string, render: PluginPageRenderer) {
        registry.addPage(pid, subpath, render);
      },
    },
    ui: {
      // ホスト直のトースト (permission `notify` 不要)。プラグイン作者の
      // 「最低限のフィードバック手段」を低摩擦に提供する。
      toast(message: string, kind = 'info') {
        const k: 'info' | 'ok' | 'warn' | 'error' =
          kind === 'ok' || kind === 'warn' || kind === 'error' ? kind : 'info';
        showToast(message, k, { pluginId: pid });
      },
    },
    invoke(action: string, payload?: unknown) {
      return pluginInvoke(pid, action, payload ?? null);
    },
    log: {
      info: (...args: unknown[]) => console.info(logTag, ...args),
      warn: (...args: unknown[]) => console.warn(logTag, ...args),
      error: (...args: unknown[]) => console.error(logTag, ...args),
    },
  };
}

/** プラグインを動的 import して `activate()` を呼ぶ。失敗しても throw しない。
 *  activate が途中で throw した場合は、それまでに register された全寄与を
 *  ロールバックしてから failed 状態を記録する (Codex review r3297535055)。 */
export async function loadPlugin(info: PluginInfo): Promise<void> {
  const pid = info.pluginId;
  // 進行中の load を保留 promise として登録 → unloadPlugin が await できる
  // ようにし、activate 完了後に unload が走るような race (Codex #14) を防ぐ。
  const task = (async () => {
    try {
      // import URL に updatedAt をキャッシュバスト付きクエリで付与し、同じ
      // entry path への再インストール後に旧モジュールがキャッシュから返る
      // 問題 (Codex #9) を防ぐ。asset:// は通常 query を許容する。
      const assetUrl = convertFileSrc(info.entryAbsPath);
      const sep = assetUrl.includes('?') ? '&' : '?';
      const cacheBust = `${assetUrl}${sep}v=${info.updatedAt ?? Date.now()}`;
      // Vite の解析を回避 (実行時に決まる URL を import するため)
      const mod = (await import(/* @vite-ignore */ cacheBust)) as PluginModule;
      // unload が間に走っていた場合は ここで abort する
      // (activationsInFlight から消えていたら unload に巻き取られた合図)。
      if (!activationsInFlight.has(pid)) {
        // ここから先の副作用を出さない (registry も触らない)。
        return;
      }
      loadedModules.set(pid, mod);
      if (typeof mod.activate === 'function') {
        await mod.activate(buildContext(info));
      }
      // 再チェック: activate await 中に unloadPlugin が走った可能性
      if (!activationsInFlight.has(pid)) {
        return;
      }
      loadStates.set(pid, { state: 'loaded' });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // activate 途中で throw すると、既に addNav/addAction/on などで登録した
      // 寄与だけが残って UI に "幽霊" のように現れる可能性がある。失敗時は
      // 必ず全寄与を取り消す。
      registry.removeAllByPlugin(pid);
      bus.offAllByOwner(pid);
      loadedModules.delete(pid);
      loadStates.set(pid, { state: 'failed', error: msg });
      console.error(`[plugin] failed to load ${pid}:`, e);
    }
  })();
  activationsInFlight.set(pid, task);
  try {
    await task;
  } finally {
    // 自分自身を消す (他者が既に書き換えていたら何もしない)
    if (activationsInFlight.get(pid) === task) {
      activationsInFlight.delete(pid);
    }
  }
}

/** プラグインを停止する。registry/bus からの解除 + (あれば) deactivate 呼び出し。 */
export async function unloadPlugin(pluginId: string): Promise<void> {
  // 進行中の activate を先に止める。activationsInFlight から消した時点で
  // loadPlugin 側がチェックして副作用を出さずに return する。
  const inflight = activationsInFlight.get(pluginId);
  activationsInFlight.delete(pluginId);
  if (inflight) {
    // activate 完了 (or throw) を待ってから unload を進める。await を投げない。
    try {
      await inflight;
    } catch {
      /* loadPlugin 側で catch 済み */
    }
  }
  const mod = loadedModules.get(pluginId);
  loadedModules.delete(pluginId);
  loadStates.delete(pluginId);
  registry.removeAllByPlugin(pluginId);
  bus.offAllByOwner(pluginId);
  if (mod && typeof mod.deactivate === 'function') {
    try {
      await mod.deactivate();
    } catch (e) {
      console.error(`[plugin] deactivate of ${pluginId} threw:`, e);
    }
  }
}

/** テスト用: 内部状態を全クリア。 */
export function _resetForTests(): void {
  loadedModules.clear();
  loadStates.clear();
  activationsInFlight.clear();
}
