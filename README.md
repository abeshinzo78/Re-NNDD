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
[![Ask Zread](https://img.shields.io/badge/Ask_Zread-_.svg?style=flat&color=00b0aa&labelColor=000000&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB3aWR0aD0iMTYiIGhlaWdodD0iMTYiIHZpZXdCb3g9IjAgMCAxNiAxNiIgZmlsbD0ibm9uZSIgeG1sbnM9Imh0dHA6Ly93d3cudzMub3JnLzIwMDAvc3ZnIj4KPHBhdGggZD0iTTQuOTYxNTYgMS42MDAxSDIuMjQxNTZDMS44ODgxIDEuNjAwMSAxLjYwMTU2IDEuODg2NjQgMS42MDE1NiAyLjI0MDFWNC45NjAxQzEuNjAxNTYgNS4zMTM1NiAxLjg4ODEgNS42MDAxIDIuMjQxNTYgNS42MDAxSDQuOTYxNTZDNS4zMTUwMiA1LjYwMDEgNS42MDE1NiA1LjMxMzU2IDUuNjAxNTYgNC45NjAxVjIuMjQwMUM1LjYwMTU2IDEuODg2NjQgNS4zMTUwMiAxLjYwMDEgNC45NjE1NiAxLjYwMDFaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik00Ljk2MTU2IDEwLjM5OTlIMi4yNDE1NkMxLjg4ODEgMTAuMzk5OSAxLjYwMTU2IDEwLjY4NjQgMS42MDE1NiAxMS4wMzk5VjEzLjc1OTlDMS42MDE1NiAxNC4xMTM0IDEuODg4MSAxNC4zOTk5IDIuMjQxNTYgMTQuMzk5OUg0Ljk2MTU2QzUuMzE1MDIgMTQuMzk5OSA1LjYwMTU2IDE0LjExMzQgNS42MDE1NiAxMy43NTk5VjExLjAzOTlDNS42MDE1NiAxMC42ODY0IDUuMzE1MDIgMTAuMzk5OSA0Ljk2MTU2IDEwLjM5OTlaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik0xMy43NTg0IDEuNjAwMUgxMS4wMzg0QzEwLjY4NSAxLjYwMDEgMTAuMzk4NCAxLjg4NjY0IDEwLjM5ODQgMi4yNDAxVjQuOTYwMUMxMC4zOTg0IDUuMzEzNTYgMTAuNjg1IDUuNjAwMSAxMS4wMzg0IDUuNjAwMUgxMy43NTg0QzE0LjExMTkgNS42MDAxIDE0LjM5ODQgNS4zMTM1NiAxNC4zOTg0IDQuOTYwMVYyLjI0MDFDMTQuMzk4NCAxLjg4NjY0IDE0LjExMTkgMS42MDAxIDEzLjc1ODQgMS42MDAxWiIgZmlsbD0iI2ZmZiIvPgo8cGF0aCBkPSJNNCAxMkwxMiA0TDQgMTJaIiBmaWxsPSIjZmZmIi8%2BCjxwYXRoIGQ9Ik00IDEyTDEyIDQiIHN0cm9rZT0iI2ZmZiIgc3Ryb2tlLXdpZHRoPSIxLjUiIHN0cm9rZS1saW5lY2FwPSJyb3VuZCIvPgo8L3N2Zz4K&logoColor=ffffff)](https://zread.ai/abeshinzo78/Re-NNDD)

ニコニコ動画専用クライアント NNDD の精神的後継を目指したい。  
Tauri 2 + Rust + Svelte 5 で実装するデスクトップアーカイブクライアントです。
Discord鯖はこちらです　https://discord.gg/cDhnfZ6HMa
スクリーンショット付きの紹介ページはこちらです https://abeshinzo78.github.io/Re-NNDD/
現在開發途中です。

## 現在できること

**検索・閲覧**

- 動画検索（スナップショット検索 API / nvapi の切り替えに対応）
- ランキングの閲覧（ジャンル・期間別、ショートランキング対応）
- ユーザーの投稿動画・マイリスト・シリーズ一覧、チャンネルの投稿動画一覧の取得
- 関連動画の表示
- 再生履歴の記録・一覧・削除

**アカウント**

- ログイン（メール/パスワード + 2 段階認証、`user_session` Cookie 直入力）

**再生**

- オンライン動画の HLS 再生（画質切り替え対応）とコメント threads API 取得・再生連携
- ローカル保存動画の再生（内蔵 HTTP Range 配信）
- ミニプレイヤー、キーボードショートカット、再生キューによる連続再生

**ダウンロード**

- ダウンロードキューの管理（追加・一覧・開始・キャンセル・完了削除）
- `yt-dlp` + `ffmpeg` を使った動画保存とライブラリ取り込み（メタデータ/サムネイルを mp4 へ埋め込み）
- 検索結果・ランキング・関連動画・動画詳細ページからのクイックダウンロード

**ライブラリ**

- 検索/整列/集計、タグ・解像度・投稿者での絞り込み、保存コメントの横断検索
- プレイリストと、検索条件を保存して自動でオンライン取得するスマートプレイリスト
- コメントスナップショットの複数管理（一覧・切り替え・削除・メモ・再取得）
- コメントを映像へ焼き込んだ MP4 のエクスポート（`@xpadev-net/niconicomments` で本物どおりに描画し ffmpeg で合成）
- ライブラリ動画の削除、ストレージ掃除

**カスタマイズ・その他**

- NG ルール管理（ランキング/検索/コメントを対象に、完全一致・部分一致・正規表現で指定）
- プラグイン機構（ZIP からのインストール、権限宣言、有効/無効切り替え。詳細は [`docs/plugins.md`](./docs/plugins.md)）
- コマンドパレット、設定保存、環境情報表示

## 進捗

Phase 1（1.0〜1.9）はすべて実装済みです。現在は v0.2.x 系として安定化・仕上げ（Phase 2.0）を進めています。

- Phase 1.0: SQLite スキーマ/マイグレーション（実装済み）
- Phase 1.1: Snapshot Search API（実装済み）
- Phase 1.2: ダウンロードキュー CRUD + HLS パーサ（実装済み）
- Phase 1.3: ライブラリ層の拡張（検索/整列/集計など）（実装済み）
- Phase 1.4: UI 骨格の整理と画面間導線の安定化（実装済み）
- Phase 1.5: プレイヤー機能の強化（操作性・安定性）（実装済み）
- Phase 1.6: NG 機能の充足（実装済み）
- Phase 1.7: プレイリスト・検索 UI の拡充（実装済み）
- Phase 1.8: コメントスナップショット運用（実装済み）
- Phase 1.9: コメント焼き込みエクスポート（実装済み）

フェーズ計画外で追加した主なもの:

- プラグイン機構（フロントエンド拡張。サンプルは [`examples/plugins/`](./examples/plugins/)）
- ランキングページ、関連動画、再生履歴、コマンドパレット
- `tauri-driver` で実アプリを起動する E2E スモークテスト
- バージョンバンプで GitHub Release を発行する自動リリース CI

詳細は [`docs/test-lists/`](./docs/test-lists/) の各テストリストを参照してください。

## 設計概要

- Desktop first: Tauri 2（Rust バックエンド + Svelte フロント）で Mac、Windows、Linux対応を目指す。
- API 境界: niconico 連携は `src-tauri/src/api/` 配下に集約し、UI からは Tauri command 経由で呼ぶ
- 永続化: SQLite（`library.db`）を単一ソースとし、動画メタ/タグ/コメント/キュー/設定を管理
- ダウンロード: 実運用の安定性を優先し、`yt-dlp` + `ffmpeg` をサイドカーとして利用
- 再生: ローカル保存動画は内蔵 HTTP サーバーの Range 配信で再生互換性を確保
- プラグイン: フロントエンド拡張はマニフェスト + 権限宣言つきのプラグイン機構として提供（インストール直後は無効、設定から個別に有効化）
- Rust コア重視: 重要ロジックを Rust 側に集約し、WebView 依存由来の制約や Tauri 固有の弱点の影響を最小化する
- 品質方針: テストリスト駆動（Red/Green/Refactor）でフェーズごとに実装を積み上げる

## 今後の予定

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

実アプリを起動する E2E スモークテスト（要: デバッグビルド、`tauri-driver`。Linux はさらに `webkit2gtk-driver` と `xvfb` が必要）:

```bash
sudo apt-get install webkit2gtk-driver xvfb   # Linux (Debian/Ubuntu 系) のみ
npx tauri build --debug --no-bundle
cargo install tauri-driver --locked           # 初回のみ
xvfb-run -a npm run test:e2e
```

## 謝意

NNDD オリジナルの著者 MineAP 氏に深く敬意を表します。  
本プロジェクトは MineAP 氏の MIT ライセンス NNDD を起点に、現代的なスタックで再実装するものです。

## ライセンス

MIT
