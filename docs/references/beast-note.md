# 参考資料：野獣ノート (@Kongyokongyo) ニコニコ動画 リバースエンジニアリングメモ

出典：<https://beast-note.yajuvideo.in/text_contents/61a6ffbf-4e85-42fb-be4a-a0d69df0f57a>

CLAUDE.md の Phase 1.1（`auth.rs` / `video.rs` / `comment.rs`）と Phase 1.2（ダウンローダ）を実装するときに参照する。Phase 1.0 では使わない。

## 押さえるポイント

### アーキテクチャ
- フロント：React + Remix
- 配信：HLS via CloudFront CDN
- 認証：Cookie セッション（`nicosid` / `user_session`） + JWT トークン
- API：RESTful JSON マイクロサービス群

### 動画配信（Domand）
- 内部基盤名は **Domand**
- HLS プレイリストはマスタ + メディアの 2 層構造
- セグメントは **AES-128-CBC 暗号化**
- アクセスは **CloudFront 署名付き URL**
- アクセス権 API でプレイリスト URL とトークンを払い出す

### コメ（threads）
- 投稿者コメ / メイン / かんたん（owner / main / easy）の 3 fork
- コマンド：`184`（匿名化）、`big` / `small`、色指定など
- リクエストには **JWT 形式の `threadKey`** が必要

### 認証
- `nicosid`（永続）、`user_session`（ログイン）の 2 種 Cookie
- `localStorage` を補助に使う実装あり
- JWT のペイロードに権限情報

## 実装上の注意（Re:NNDD として）


- HLS フェッチャは並列 6（NicoCommentDL の実績値）
- AES-128-CBC 復号は `aes` クレート + 手書き CBC、もしくは `aes-gcm` 系は不可（CBC 専用）
- CloudFront 署名 URL は再利用不可期限あり、フェッチ前に都度払い出す
- threads API は仕様変更が多いので必ずアダプタ層に隔離する（CLAUDE.md §「ニコニコ API アダプタ層」と整合）
- 利用は私的アーカイブ目的に限る。CLAUDE.md ライセンス節と齟齬なし
## 参考実装

- [`abeshinzo78/NicoCommentDL`](https://github.com/abeshinzo78/NicoCommentDL)
  - `src/background/api/niconico.js`：JWT 解析・HLS URL 取得・コメ取得
  - `src/background/api/hls-fetcher.js`：セグメント並列取得・AES-128 復号・プリフェッチ
- Phase 1.1〜1.2 で上記 JS ロジックを Rust に移植する（CLAUDE.md「重要な制約とリマインダ」）
