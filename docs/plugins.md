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

| 権限名           | 呼べる action  | 内容                                                                                                                                                                                                  |
| ---------------- | -------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `net.fetch`      | `net.fetch`    | https のみの HTTP fetch (`{url, method?, headers?, body?}` → `{status, headers, bodyBase64}`)                                                                                                         |
| `library.read`   | `library.list` | ローカルライブラリの動画一覧を取得 (`{limit?, offset?}` → ページング結果)                                                                                                                             |
| `settings.read`  | `settings.get` | `plugin:<id>:*` キーの値を取得 (他キーは拒否)                                                                                                                                                         |
| `settings.write` | `settings.set` | `plugin:<id>:*` キーに保存 (他キーは拒否)                                                                                                                                                             |
| `notify`         | `notify.toast` | フロント側プラグインイベントバスに `notify:toast` を emit する (`{message, kind?}` → `{pluginId, message, kind}`)。トーストを実 UI に出すかは購読側 (他プラグイン / 将来のアプリ標準 toast UI) の責務 |

## Plugin API (`activate(ctx)`)

`index.js` は ES module として `export async function activate(ctx)` を提供します
(任意で `export function deactivate()` も)。`ctx` の shape は以下のとおり:

```ts
ctx.manifest             // 自分の manifest.json (read-only)

ctx.events.on(name, fn)  // name のイベントを購読。返値の関数で off
ctx.events.emit(name, p) // 任意名のイベントを emit (ホストが標準で listen はしない)

ctx.settings.register({key, label, kind, default, ...})
  // 設定画面の「プラグイン」セクションに項目を登録 (key は plugin:<id>:* 必須)
ctx.settings.get(key)    // 値を取得 (Promise)
ctx.settings.set(key, v) // 値を保存 (Promise) ※ v は文字列に変換される

ctx.nav.addPage({href, label})
  // サイドバーにナビゲーション項目を追加。href の例: /plugin/<id>/main
ctx.items.addAction({label, appliesTo?, handler})
  // 動画カードの ⋯ メニューに項目を追加。handler は (hit) => void
ctx.player.addAction({label, icon?, handler, key?})
  // プレイヤーコントロールバーにボタンを追加。
  // key にキー (例: 'g') を入れると組込みショートカット未使用キーに対して有効。

await ctx.invoke('net.fetch', {url: 'https://...'})
  // permission に応じた Rust API を呼ぶ。Promise<unknown>

ctx.log.info(...)        // console.info に [plugin:<id>] プレフィックス付きで出力
ctx.log.warn(...) / ctx.log.error(...)
```

## ホストが emit する標準イベント

`ctx.events.on(name, handler)` で購読できる組込みイベント:

| name                | payload                     | 発火タイミング                                                  |
| ------------------- | --------------------------- | --------------------------------------------------------------- |
| `player:play`       | `{videoId, currentTime}`    | プレイヤーが実フレーム送出を開始したとき                        |
| `player:pause`      | `{videoId, currentTime}`    | プレイヤーが一時停止 (ended ではない)                           |
| `player:time`       | `{videoId, currentTime}`    | 200ms throttle で再生時刻更新                                   |
| `player:ended`      | `{videoId}`                 | 動画が自然終了 (loop 中は発火しない)                            |
| `notify:toast`      | `{pluginId, message, kind}` | 別のプラグイン (または自分自身) が `ctx.invoke('notify.toast')` |
| `download:start`    | `{id, videoId}`             | DL キューが downloading 状態になった                            |
| `download:complete` | `{id, videoId}`             | DL 成功で done に遷移した                                       |
| `download:error`    | `{id, videoId, message}`    | DL 失敗 (キャンセル含む)                                        |

これら以外の名前は host からは emit されません。
プラグイン同士の通信は `ctx.events.emit(name, payload)` で任意名を使って
ください (将来サンドボックスを入れたときに名前空間を整理する予定)。

## 最小サンプル (`index.js`)

```js
// 動画再生開始のたびに console に記録するだけのプラグイン。
export async function activate(ctx) {
  ctx.log.info('hello from', ctx.manifest.name);
  ctx.events.on('player:play', (ev) => {
    ctx.log.info('played', ev.videoId, 'at', ev.currentTime);
  });
  ctx.nav.addPage({ href: '/plugin/sample/main', label: 'サンプル' });
}

export function deactivate() {
  // 何もしない (events.on の返値で off できるが、host が offAllByOwner するので
  // 任意)。
}
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

この 2 ファイルを `sample.zip` に固めて、設定画面の「ZIP からインストール」で
読み込み、有効化トグルをオンにすれば動作します。
_(`/plugin/sample/main` 自体のページは付属していないので、サイドバーリンクは
404 になります。プラグインで実 UI を描画したい場合は対応する SvelteKit
ルートを別途用意するか、将来の追加 API を待ってください。)_

## ファイルレイアウト (ホスト側 ソース)

開発・調査のために Re:NNDD 本体側のプラグインホストコードの場所を残します:

| 場所                                  | 役割                                                        |
| ------------------------------------- | ----------------------------------------------------------- |
| `src/lib/plugins/host.ts`             | ブートストラップ、Rust→JS イベント橋渡し                    |
| `src/lib/plugins/loader.ts`           | 動的 import、`activate`/`deactivate` 呼び出し、context 構築 |
| `src/lib/plugins/eventBus.ts`         | プラグイン間 / ホスト → プラグインのイベント配信            |
| `src/lib/plugins/registry.ts`         | プラグイン寄与 (nav/settings/items/player) の reactive 保持 |
| `src/lib/plugins/api.ts`              | Rust command の TS ラッパ                                   |
| `src-tauri/src/plugins/manifest.rs`   | manifest スキーマ + バリデーション                          |
| `src-tauri/src/plugins/installer.rs`  | ZIP インストーラ                                            |
| `src-tauri/src/plugins/registry.rs`   | SQLite plugins テーブル                                     |
| `src-tauri/src/plugins/runtime.rs`    | プロセス内キャッシュ (parking_lot::RwLock)                  |
| `src-tauri/src/plugins/dispatcher.rs` | permission チェック + action 振り分け                       |
| `src-tauri/src/plugins/commands.rs`   | `plugin_*` Tauri commands                                   |

## キルスイッチ

設定 → 高度な設定 → **「プラグイン機構を有効にする」** (`plugins.enabled`)。

OFF にすると `bootstrapPluginHost()` は最初の 1 行で return し:

- `nndd:plugin:event` Tauri リスナーを張らない
- インストール済みプラグインを 1 つも load しない
- registry / eventBus が空のまま

つまりプラグイン機構導入前と **完全に同じ起動シーケンス** になります。
回帰の疑いがあればまずこれを OFF にして再現性を確認してください。
