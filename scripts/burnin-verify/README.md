# コメント焼き込みエクスポート 実機検証ハーネス

コメント焼き込み機能が **本物の niconico / niconicomments-convert と同じ出力**に
なっていることを、実データで検証するための Node ハーネスです。アプリ本体では
なく検証専用ツールなので、ESLint / Prettier / `npm run build` の対象外にしています
(`eslint.config.js` / `.prettierignore` で除外)。

## 何を検証するか

本番 (Tauri WebView) と **同じ共有コア** (`src/lib/burnin/core.ts` +
`src/lib/burnin/comments.ts`) を使って、`@xpadev-net/niconicomments` で 1 フレーム
ずつ描画し、**Rust 側 (`src-tauri/src/downloader/burnin.rs`) と寸分違わぬ ffmpeg
フィルタ**で元動画へオーバーレイします。つまり「座標・サイズ・色・流れ」の決定を
niconicomments 本体へ完全委譲していること=旧来の独自 ASS 実装のズレが解消されて
いることを、実際の MP4 出力で確認できます。

ハーネスは Re:NNDD の Rust 実装を忠実に写経しています:

- `api/video.rs` … watch ページ取得 → `server-response` meta 抽出 → `html_unescape`
- `api/comment.rs` … `POST {server}/v1/threads` (nvComment v1)
- `downloader/burnin.rs` … `overlay_filter()` / `spawn_session()` の ffmpeg 引数
- `lib/burnin/core.ts` + `comments.ts` … 共有フレームループ / v1 変換 (本番と同一)

## 前提

- `npm install` 済み (devDependencies の `@napi-rs/canvas`, `esbuild` が必要)。
- サイドカーバイナリ取得済み: `bash scripts/fetch-binaries.sh`
  (`src-tauri/binaries/ffmpeg-<triple>` と `yt-dlp-<triple>`)。
- niconico の `user_session` Cookie 値 (環境変数 `NICO_USER_SESSION`)。
  **ソースには絶対に直書きしないこと** (秘匿情報)。
- ネットワーク経路に自己署名 CA がある環境では `NODE_TLS_REJECT_UNAUTHORIZED=0`。

## 実行

リポジトリルートから:

```bash
# 1) バンドル (native addon の @napi-rs/canvas は external)
node_modules/.bin/esbuild scripts/burnin-verify/verify.ts \
  --bundle --platform=node --format=cjs \
  --outfile=scripts/burnin-verify/verify.cjs \
  --external:@napi-rs/canvas --tsconfig-raw='{}'

# 2) 実行 (sm9 を 30 秒ぶん焼き込み)
NICO_USER_SESSION="user_session_..." \
NODE_TLS_REJECT_UNAUTHORIZED=0 \
NODE_PATH=node_modules \
node scripts/burnin-verify/verify.cjs
```

## 出力

実行ごとに `mkdtemp` で作られる一意の作業ディレクトリ (パスは標準出力の
`[setup] work dir:` 行と終了時の summary JSON に出ます) に:

- `input.mp4` … yt-dlp で取得した元動画
- `output.mp4` … コメントを焼き込んだ結果 (1280x720 / 30fps / bt709)
- `frame_{2,5,10,20,28}.png` … 目視確認用のサンプルフレーム

サンプルフレームを開けば、流れるコメント (naka)・上下固定 (ue/shita)・色・サイズ・
弾幕の重なり方が本物の niconico と一致していることを確認できます。

なお既定ではローカルスナップショット相当 (`posted_at` を Unix 秒文字列に変換) で
描画し、本番と同じ入力で `toIsoPostedAt()` の正規化を実地検証します
(`SIMULATE_LOCAL=0` で無効化してオンラインの ISO `postedAt` のまま描画)。
