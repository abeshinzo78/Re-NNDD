# Re:NNDD

<!-- Build / CI -->
[![CI](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/ci.yml)
[![CodeQL](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/codeql.yml)
[![Security audit](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/security.yml)
[![Release](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/release.yml/badge.svg)](https://github.com/abeshinzo78/Re-NNDD/actions/workflows/release.yml)

<!-- Tech stack -->
[![Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app/)
[![Rust stable / edition 2021](https://img.shields.io/badge/Rust-stable%20%2F%20edition%202021-DEA584?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Svelte 5](https://img.shields.io/badge/Svelte-5-FF3E00?logo=svelte&logoColor=white)](https://svelte.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Vite](https://img.shields.io/badge/Vite-8-646CFF?logo=vite&logoColor=white)](https://vitejs.dev/)
[![SQLite](https://img.shields.io/badge/SQLite-bundled-003B57?logo=sqlite&logoColor=white)](https://www.sqlite.org/)
[![Node.js 20+](https://img.shields.io/badge/Node.js-20%2B-339933?logo=nodedotjs&logoColor=white)](https://nodejs.org/)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey)](#必要環境)

<!-- Quality -->
[![License: MIT](https://img.shields.io/github/license/abeshinzo78/Re-NNDD?color=blue)](./LICENSE)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success?logo=rust&logoColor=white)](./Cargo.toml)
[![clippy: deny warnings](https://img.shields.io/badge/clippy-deny%20warnings-orange?logo=rust&logoColor=white)](https://github.com/rust-lang/rust-clippy)
[![rustfmt](https://img.shields.io/badge/code%20style-rustfmt-000000?logo=rust&logoColor=white)](./rustfmt.toml)
[![ESLint](https://img.shields.io/badge/lint-ESLint-4B32C3?logo=eslint&logoColor=white)](./eslint.config.js)
[![Prettier](https://img.shields.io/badge/code%20style-Prettier-F7B93E?logo=prettier&logoColor=black)](./.prettierrc)
[![typos](https://img.shields.io/badge/spellcheck-typos-0F8C40)](https://github.com/crate-ci/typos)
[![cargo-deny](https://img.shields.io/badge/audit-cargo--deny-orange?logo=rust&logoColor=white)](./deny.toml)
[![Dependabot](https://img.shields.io/badge/deps-Dependabot-025E8C?logo=dependabot&logoColor=white)](./.github/dependabot.yml)
[![Tests: Vitest](https://img.shields.io/badge/tests-Vitest-6E9F18?logo=vitest&logoColor=white)](https://vitest.dev/)

<!-- Repo info -->
[![Last commit](https://img.shields.io/github/last-commit/abeshinzo78/Re-NNDD)](https://github.com/abeshinzo78/Re-NNDD/commits/main)
[![Issues](https://img.shields.io/github/issues/abeshinzo78/Re-NNDD)](https://github.com/abeshinzo78/Re-NNDD/issues)
[![Pull requests](https://img.shields.io/github/issues-pr/abeshinzo78/Re-NNDD)](https://github.com/abeshinzo78/Re-NNDD/pulls)
[![Stars](https://img.shields.io/github/stars/abeshinzo78/Re-NNDD?style=flat)](https://github.com/abeshinzo78/Re-NNDD/stargazers)
[![Contributors](https://img.shields.io/github/contributors/abeshinzo78/Re-NNDD)](https://github.com/abeshinzo78/Re-NNDD/graphs/contributors)
[![Repo size](https://img.shields.io/github/repo-size/abeshinzo78/Re-NNDD)](https://github.com/abeshinzo78/Re-NNDD)
[![Top language](https://img.shields.io/github/languages/top/abeshinzo78/Re-NNDD)](https://github.com/abeshinzo78/Re-NNDD)

<!-- External -->
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/abeshinzo78/Re-NNDD)

ニコニコ動画専用クライアント NNDD の精神的後継を目指したい。  
Tauri 2 + Rust + Svelte 5 で実装するデスクトップアーカイブクライアントです。
現在開發途中です。

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
- Phase 1.3: ライブラリ層の拡張（検索/整列/集計など）←現在進行中

詳細は [`docs/test-lists/`](./docs/test-lists/) の各テストリストを参照してください。

## 設計概要

- Desktop first: Tauri 2（Rust バックエンド + Svelte フロント）で Mac、Windows、Linux対応を目指す。
- API 境界: niconico 連携は `src-tauri/src/api/` 配下に集約し、UI からは Tauri command 経由で呼ぶ
- 永続化: SQLite（`library.db`）を単一ソースとし、動画メタ/タグ/コメント/キュー/設定を管理
- ダウンロード: 実運用の安定性を優先し、`yt-dlp` + `ffmpeg` をサイドカーとして利用
- 再生: ローカル保存動画は内蔵 HTTP サーバーの Range 配信で再生互換性を確保
- Rust コア重視: 重要ロジックを Rust 側に集約し、WebView 依存由来の制約や Tauri 固有の弱点の影響を最小化する
- 品質方針: テストリスト駆動（Red/Green/Refactor）でフェーズごとに実装を積み上げる

## 今後の予定

- Phase 1.4: UI 骨格の整理と画面間導線の安定化
- Phase 1.5: プレイヤー機能の強化（操作性・安定性）
- Phase 1.6: NG 機能の充足
- Phase 1.7: プレイリスト・検索 UI の拡充
- Phase 1.8: コメントスナップショット運用
- Phase 1.9: コメント焼き込みエクスポート
- Phase 2.0: 仕上げ（安定化・ドキュメント・運用調整）

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
