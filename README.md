# Re:NNDD

ニコニコ動画専用クライアント NNDD の精神的後継。Tauri 2 + Rust + Svelte 5 で実装するデスクトップアーカイブクライアント。

設計仕様の正典は [`CLAUDE.md`](./CLAUDE.md) を参照。

## ステータス

Phase 1.0（基盤）+ Phase 1.1 のスナップショット検索 API を実装。スナップショット検索 UI 経由で niconico の検索 API を叩いて結果を表示できる。詳細は [`docs/test-lists/`](./docs/test-lists/) のテストリストを参照。

## 必要環境

- Rust stable（rustup で最新）
- Node.js ≥ 20
- Linux: `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `librsvg2-dev`, `libjavascriptcoregtk-4.1-dev`, `libayatana-appindicator3-dev`, `build-essential`, `pkg-config`

## 開発

```bash
npm install
npm run tauri:dev          # Vite (localhost:1420) + Tauri ウィンドウを同時起動
npm run dev                # Vite だけブラウザで確認したいとき
```

## テスト

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npm run check              # svelte-check で型検査
npm test                   # vitest（テストが追加されたら有効）
```

## 謝意

NNDD オリジナルの著者 MineAP 氏に深く敬意を表します。本プロジェクトは MineAP 氏の MIT ライセンス NNDD を起点に、現代的なスタックで再実装するものです。

## ライセンス

MIT
