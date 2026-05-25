# Re:NNDD プラグイン開発ガイド

> **⚠️ 重要な信頼モデルの警告**
>
> プラグインは Re:NNDD のメインレンダラ realm 内で **アプリ本体と同じ権限**
> で動作します。サンドボックス (iframe / Web Worker) はありません。
> 信頼できない提供元のプラグインを絶対にインストールしないでください。
>
> プラグイン機構を完全に停止したい場合は、設定 → 高度な設定 →
> 「プラグイン機構を有効にする」を OFF にしてアプリを再起動してください。
> プラグインが一切ロードされない (= プラグイン機構導入前と完全に同一の挙動)
> 状態になります。

---

## 配置と構造

各プラグインは以下のディレクトリ構造を持ちます:

```
$APPDATA/plugins/<plugin_id>/
  manifest.json     # 必須
  index.js          # 必須 (manifest の "entry" で指定)
  assets/...        # 任意
```

`$APPDATA` の解決先は OS により異なります (Linux: `~/.local/share/in.yajuvideo.nndd-next/`、
macOS: `~/Library/Application Support/in.yajuvideo.nndd-next/`、
Windows: `%APPDATA%\in.yajuvideo.nndd-next\`)。
設定画面の「アプリ情報 → データ保存場所」で確認できます。

## インストール方法

1. **ZIP インポート (推奨)**: 設定 → プラグイン → 「ZIP からインストール」で
   `manifest.json` を root に含む zip を選択。
2. **手動配置**: 上記のディレクトリ構造を手で作成して再起動。

インストール直後は **無効状態** です。設定画面で各プラグインの有効化トグル
をオンにしてください。

> アプリは起動時に `<plugins_root>` 直下の `<id>.tmp-*` / `<id>.bak-*` 残骸
> (= 前回 install/uninstall 中にクラッシュした場合のゴミディレクトリ) を
> ベストエフォートで掃除します。プラグイン本体ディレクトリは消しません。

## manifest.json スキーマ

```json
{
  "id": "com.example.my-plugin",
  "name": "My Plugin",
  "version": "0.1.0",
  "entry": "index.js",
  "description": "短い説明 (500 文字以下)",
  "author": "Your Name",
  "homepage": "https://example.com",
  "minAppVersion": "0.1.0",
  "permissions": ["net.fetch", "library.read", "settings.read", "settings.write", "notify"]
}
```

### 必須フィールド

| フィールド | 制約                                                                        |
| ---------- | --------------------------------------------------------------------------- |
| `id`       | `^[a-z0-9][a-z0-9._-]{2,63}$` 形式 (英小字数字 + `.` `_` `-` のみ、3–64 字) |
| `name`     | 1–80 字                                                                     |
| `version`  | semver (例 `0.1.0`)                                                         |
| `entry`    | `.js` または `.mjs` ファイル、`..` / `/` / `\` / `:` 不可                   |

### 任意フィールド

| フィールド      | 内容                                                   |
| --------------- | ------------------------------------------------------ |
| `description`   | 500 字以下                                             |
| `author`        | 任意文字列                                             |
| `homepage`      | `http://` または `https://`                            |
| `minAppVersion` | semver。アプリ本体のバージョン未満ならインストール拒否 |
| `permissions`   | 下表の権限名の配列。未知の名前はインストール拒否       |

### 権限 (permission) 一覧

プラグインが `ctx.invoke(action, ...)` で呼べる action は、`manifest.permissions`
に対応する権限が含まれている場合のみ受け付けられます (Rust 側 dispatcher
で enforce)。

| 権限名           | 呼べる action                                                       | 内容                                                                                                                           |
| ---------------- | ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `net.fetch`      | `net.fetch`                                                         | https のみの HTTP fetch (GET/POST/PUT/PATCH/DELETE/HEAD)                                                                       |
| `library.read`   | `library.list` / `library.get` / `library.search` / `library.stats` | ローカルライブラリの動画情報を取得                                                                                             |
| `settings.read`  | `settings.get`                                                      | `plugin:<id>:*` キーの値を取得 (他キーは拒否)                                                                                  |
| `settings.write` | `settings.set`                                                      | `plugin:<id>:*` キーに保存 (他キーは拒否)                                                                                      |
| `notify`         | `notify.toast`                                                      | Rust 経由でトースト通知を発行 (アプリのトースト UI が表示)                                                                     |
| `player.control` | `player.command`                                                    | 再生 / 一時停止 / シーク / 速度 / 音量 / ミュートの操作                                                                        |
| `commands`       | (action 無し)                                                       | `ctx.commands.register(cmd)` でコマンドパレット (Ctrl/⌘+K) に項目追加。実行は host UI 内 (permission チェックは register 段階) |

> `ctx.player.getState()` は **同期取得 + 権限不要**。フロント (Player.svelte)
> が持つ状態スナップショットを読むだけで Rust を経由しません。
> 再生/一時停止/シーク等の **操作** は `player.control` 権限が必要です。

### action の制限事項

- `net.fetch`:
  - https のみ (http は拒否)。
  - 自動リダイレクト無効 (3xx は自分で再 fetch する設計; SSRF バイパス防止)。
  - レスポンス body 上限 **10 MiB** (超過すると stream 打ち切り)。
  - timeout 30s / connect_timeout 10s。
  - DNS 解決後の IP が private / loopback / link-local / CGNAT の場合は拒否
    (DNS rebinding 防止)。
  - 受け付けるメソッド: `GET`, `POST`, `PUT`, `PATCH`, `DELETE`, `HEAD`。
    `OPTIONS` / `CONNECT` / `TRACE` は SSRF/CSRF 観点で意図的に拒否。
  - 受け付けるリクエストヘッダ: `Accept`, `Accept-Language`, `Accept-Encoding`,
    `Cache-Control`, `Content-Type`, `If-Match`, `If-None-Match`,
    `If-Modified-Since`, `If-Unmodified-Since`, `User-Agent`, `Range`, `Referer`。
    認証系 (`Authorization`, `Cookie`) / framing 系 (`Host`, `Content-Length`,
    `X-Forwarded-*`) は拒否。
- `library.list` / `library.search`:
  - `limit` の上限は **200**。それ以上指定しても clamp される。
  - `offset` は 0..=u32::MAX に clamp。
- `library.stats`:
  - 軽量集計のみを返す (`totalVideos`, `totalDurationSec`, `totalComments`,
    `uniqueUploaders`, `uniqueTags`)。Top-N タグや解像度分布は含まない。
- `settings.get` / `settings.set`:
  - キーは `plugin:<plugin_id>:` で始まること必須 (dot-prefix 攻撃防止のため
    区切りは `:`)。
- `notify.toast`:
  - 引数: `{ message: string, kind?: 'info'|'ok'|'warn'|'error' }`。アプリ本体の
    トースト UI に表示される。

## Plugin API (`activate(ctx)`)

`index.js` は ES module として `export async function activate(ctx)` を提供します
(任意で `export function deactivate()` も)。`ctx` の shape は以下のとおり:

```ts
ctx.manifest             // 自分の manifest.json (read-only)

ctx.events.on(name, fn)  // name のイベントを購読。返値の関数で off
ctx.events.emit(name, p) // 任意名のイベントを emit (ホストが標準で listen はしない)

// ----- 設定項目の登録と読み書き (要 settings.read / settings.write) -----
ctx.settings.register({key, label, kind, default, ...})
  // 設定画面の「プラグイン」セクションに項目を登録 (key は plugin:<id>:* 必須)。
  // 登録された項目はアプリの設定画面で **直接編集可能** (bool/number/select/text)。
ctx.settings.get(key)    // 値を取得 (Promise<string|null>)
ctx.settings.set(key, v) // 値を保存 (Promise<void>) ※ v は文字列に変換される

// ----- UI 拡張 -----
ctx.nav.addPage({href, label})
  // サイドバーにナビゲーション項目を追加。
  // プラグイン専用ページは `/plugin/<id>/<subpath>` を使う (下記参照)。
ctx.items.addAction({label, appliesTo?, handler})
  // 動画カードの ⋯ メニューに項目を追加。
  // 対象ルート: 検索 / 動画詳細 (関連動画) / ランキング / ローカルライブラリ /
  //   視聴履歴 / ユーザー動画一覧。
  // appliesTo() が throw した項目は表示されない (防御)。
ctx.player.addAction({label, icon?, handler, key?})
  // プレイヤーコントロールバーにボタンを追加。
  // key にキー (例: 'g') を入れると組込みショートカット未使用キーに対して有効。

// ----- プレイヤー制御 -----
ctx.player.getState()           // 現在の {videoId, currentTime, duration, paused, volume, muted, playbackRate}
                                // (同期。Player が mount されていなければ videoId=null)
ctx.player.play()               // 再生開始 (Promise<void>)。要 player.control 権限。
ctx.player.pause()              // 一時停止
ctx.player.toggle()             // 再生/一時停止反転
ctx.player.seek(toSec)          // 指定秒へシーク
ctx.player.setRate(rate)        // 再生速度 (0.25..=4.0 推奨)
ctx.player.setVolume(vol)       // 音量 (0..=1)
ctx.player.toggleMute()         // ミュート反転

// ----- コマンドパレット (Ctrl/⌘+K) -----
ctx.commands.register({id, title, hint?, keywords?, handler})
  // コマンドパレットに項目を追加。
  // id は `app.*` で始まらないこと (組込み名前空間。違反は warn して無視)。

// ----- プラグイン専用ページ -----
ctx.pages.register(subpath, render)
  // `/plugin/<id>/<subpath>` の URL に対応する DOM レンダラを登録。
  // render(el): el は SvelteKit が用意した空 <div>。要素を生成して appendChild する。
  //   返り値 (() => void) は cleanup (ページ離脱時に呼ばれる)。
  // 同 subpath を 2 度 register したら後勝ち。
  // subpath は `/dashboard` / `dashboard` のどちらでも OK。

// ----- UI ヘルパ -----
ctx.ui.toast(message, kind?)
  // ホスト UI 直接呼び出しのトースト (permission 不要)。
  // kind は 'info' | 'ok' | 'warn' | 'error'。デフォルト 'info'。

await ctx.invoke('net.fetch', {url: 'https://...'})
  // permission に応じた Rust API を呼ぶ。Promise<unknown>

ctx.log.info(...)                // console.info に [plugin:<id>] プレフィックス付きで出力
ctx.log.warn(...) / ctx.log.error(...)
```

## ホストが emit する標準イベント

`ctx.events.on(name, handler)` で購読できる組込みイベント:

| name                | payload                     | 発火タイミング                                                         |
| ------------------- | --------------------------- | ---------------------------------------------------------------------- |
| `player:play`       | `{videoId, currentTime}`    | プレイヤーが実フレーム送出を開始したとき                               |
| `player:pause`      | `{videoId, currentTime}`    | プレイヤーが一時停止 (ended ではない)                                  |
| `player:time`       | `{videoId, currentTime}`    | 200ms throttle で再生時刻更新                                          |
| `player:ended`      | `{videoId}`                 | 動画が自然終了 (loop 中は発火しない)                                   |
| `notify:toast`      | `{pluginId, message, kind}` | プラグインが `ctx.invoke('notify.toast', ...)` で発行 (host UI で表示) |
| `download:start`    | `{id, videoId}`             | DL キューが downloading 状態になった                                   |
| `download:complete` | `{id, videoId}`             | DL 成功で done に遷移した                                              |
| `download:error`    | `{id, videoId, message}`    | DL 失敗 (キャンセル含む)                                               |

これら以外の名前は host からは emit されません。
プラグイン同士の通信は `ctx.events.emit(name, payload)` で任意名を使って
ください (将来サンドボックスを入れたときに名前空間を整理する予定)。

> `plugin:player:control` は host 内部イベント (dispatcher → Player.svelte)
> のため、プラグインは購読・emit する必要がありません。

## サンプル

リポジトリの [`examples/plugins/`](../examples/plugins/) に動作するサンプルが 2 つあります:

- **`hello-world/`**: 最小サンプル。トースト + コマンドパレット項目 + 動画
  カードメニュー + プレイヤーボタン + 専用ページの 5 つを 1 つずつ示します。
- **`play-stats/`**: 再生時間を動画ごとに集計し、ダッシュボードページで
  上位 N 件を表示する実用サンプル。`ctx.settings.{register, get, set}` と
  `ctx.events.on('player:time', ...)` の使い方を示します。

ローカルで試す手順 (Linux 例):

```bash
# 1. アプリを起動して plugins ディレクトリを作成させる
$ npm run tauri:dev   # 起動後すぐ閉じてよい

# 2. サンプルを ZIP に固める
$ cd examples/plugins/hello-world
$ zip -r /tmp/hello-world.zip .

# 3. 設定 → プラグイン → 「ZIP からインストール」で /tmp/hello-world.zip を選択
#    → 一覧に出てきたらトグルを ON
```

`manifest.json`:

```json
{
  "id": "com.example.sample",
  "name": "サンプル",
  "version": "0.1.0",
  "entry": "index.js"
}
```

最小 `index.js` (権限不要):

```js
export async function activate(ctx) {
  ctx.log.info('hello from', ctx.manifest.name);
  ctx.ui.toast('起動しました', 'ok');
  ctx.events.on('player:play', (ev) => {
    ctx.log.info('played', ev.videoId, 'at', ev.currentTime);
  });
}
export function deactivate() {}
```

## ファイルレイアウト (ホスト側 ソース)

開発・調査のために Re:NNDD 本体側のプラグインホストコードの場所を残します:

| 場所                                                                 | 役割                                                                |
| -------------------------------------------------------------------- | ------------------------------------------------------------------- |
| `src/lib/plugins/host.ts`                                            | ブートストラップ、Rust→JS イベント橋渡し                            |
| `src/lib/plugins/loader.ts`                                          | 動的 import、`activate`/`deactivate` 呼び出し、context 構築         |
| `src/lib/plugins/eventBus.ts`                                        | プラグイン間 / ホスト → プラグインのイベント配信                    |
| `src/lib/plugins/registry.ts`                                        | nav / settings / items / player / commands / pages の reactive 保持 |
| `src/lib/plugins/api.ts`                                             | Rust command の TS ラッパ                                           |
| `src/lib/plugins/playerState.svelte.ts`                              | プラグインから観測される Player 状態スナップショット                |
| `src/lib/toastStore.svelte.ts` / `src/lib/Toast.svelte`              | アプリ共通トースト UI (notify:toast / ctx.ui.toast の表示先)        |
| `src/lib/commandPalette.svelte.ts` / `src/lib/CommandPalette.svelte` | コマンドパレット (Ctrl/⌘+K) の状態と UI                             |
| `src/lib/VideoActionMenu.svelte`                                     | 各ルートの動画カードに差す ⋯ アクションメニュー (プラグイン用)      |
| `src/routes/plugin/[id]/[...path]/+page.svelte`                      | プラグイン専用ページの動的ルート                                    |
| `src-tauri/src/plugins/manifest.rs`                                  | manifest スキーマ + バリデーション                                  |
| `src-tauri/src/plugins/installer.rs`                                 | ZIP インストーラ + 起動時ゴミ掃除                                   |
| `src-tauri/src/plugins/registry.rs`                                  | SQLite plugins テーブル                                             |
| `src-tauri/src/plugins/runtime.rs`                                   | プロセス内キャッシュ (parking_lot::RwLock)                          |
| `src-tauri/src/plugins/dispatcher.rs`                                | permission チェック + action 振り分け                               |
| `src-tauri/src/plugins/commands.rs`                                  | `plugin_*` Tauri commands                                           |

## キルスイッチ

設定 → 高度な設定 → **「プラグイン機構を有効にする」** (`plugins.enabled`)。

OFF にすると `bootstrapPluginHost()` は最初の 1 行で return し:

- `nndd:plugin:event` Tauri リスナーを張らない
- インストール済みプラグインを 1 つも load しない
- registry / eventBus が空のまま
- Toast / CommandPalette の UI コンポーネントは存在するが、プラグイン
  寄与が 0 件なので **DOM は完全に空** になる (プラグイン機構導入前と同じ)

つまりプラグイン機構導入前と **完全に同じ起動シーケンス** になります。
回帰の疑いがあればまずこれを OFF にして再現性を確認してください。
