# Re:NNDD

ニコニコ動画専用クライアント NNDD の精神的後継。  
Tauri 2 + Rust + Svelte 5 で実装するデスクトップアーカイブクライアントです。

設計仕様の正典は [`CLAUDE.md`](./CLAUDE.md) を参照してください。

## 現在できること

- スナップショット検索 API 経由の動画検索
- ログイン（メール/パスワード、`user_session` Cookie 直入力）
- 動画ページ情報の取得と HLS 再生準備
- コメント threads API 取得と再生連携
- ユーザー/チャンネル投稿動画一覧の取得
- ダウンロードキューの管理（追加・一覧・開始・キャンセル・完了削除）
- `yt-dlp` + `ffmpeg` を使った動画保存とライブラリ取り込み
- ローカル保存動画の再生（内蔵 HTTP Range 配信）
- ライブラリ動画削除、設定保存、ストレージ掃除、環境情報表示

## 進捗

- Phase 1.0: SQLite スキーマ/マイグレーション実装済み
- Phase 1.1: Snapshot Search API 実装済み
- Phase 1.2: ダウンロードキュー CRUD + HLS パーサ実装済み

詳細は [`docs/test-lists/`](./docs/test-lists/) の各テストリストを参照してください。

## 必要環境

- Rust stable（rustup 推奨）
- Node.js 20 以上
- npm

Linux (Debian/Ubuntu 系) 開発依存:

- `libwebkit2gtk-4.1-dev`
- `libsoup-3.0-dev`
- `libjavascriptcoregtk-4.1-dev`
- `libayatana-appindicator3-dev`
- `librsvg2-dev`
- `build-essential`
- `pkg-config`

## セットアップ

```bash
npm install
```

`yt-dlp` / `ffmpeg` の準備（どちらか）:

```bash
# 推奨: 配布向けのスタンドアロンバイナリを取得
bash scripts/fetch-binaries.sh

# 開発機の PATH 上のコマンドを使う場合
bash scripts/fetch-binaries.sh --system
```

## 開発実行

```bash
npm run tauri:dev   # Vite + Tauri を同時起動
npm run dev         # Web 側のみ確認
```

## ビルド

```bash
npm run tauri:build
```

## テスト/検証

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npm run check
npm run lint
npm test
```

## 謝意

NNDD オリジナルの著者 MineAP 氏に深く敬意を表します。  
本プロジェクトは MineAP 氏の MIT ライセンス NNDD を起点に、現代的なスタックで再実装するものです。

## ライセンス

MIT
