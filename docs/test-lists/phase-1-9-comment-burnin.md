# Phase 1.9 — コメント焼き込みエクスポートのテストリスト

ASS 生成のコアは `src-tauri/src/downloader/comment_ass.rs`、ffmpeg 連携は
`src-tauri/src/downloader/ffmpeg.rs`、コマンドは `src-tauri/src/commands.rs`
(`export_video_with_comments`)、UI は `src/routes/library/[id]/+page.svelte`。

## 背景

DL 済み動画 + コメントスナップショットから、コメントを字幕として **映像へ
焼き込んだ MP4** を書き出す。プレイヤーが `@xpadev-net/niconicomments` で
Canvas にリアルタイム描画しているものを、ffmpeg の `ass` フィルタで合成できる
静的 ASS へ落とし込む方式。

設計方針 (README「Rust コア重視」) に従い、レイアウト計算は I/O を持たない純
関数 `generate_ass` に集約し、単体テストで全分岐を固定する。重い再エンコードは
ffmpeg に委譲し、`-progress pipe:1` を解析して `burnin:progress` イベントで
進捗を流す。

## 単体テスト（`comment_ass.rs` 内 `#[cfg(test)] mod tests`）

### 色

- [x] 名前付き色 (通常 + プレミアム 2 系列 + 別名) を 0xRRGGBB へ
- [x] `#RRGGBB` / `#RGB` の hex 色をパース、`#` 無しは拒否
- [x] 0xRRGGBB → ASS の `&HBBGGRR&` (BGR 順) 変換

### コマンド解釈

- [x] 既定は naka / medium / 白
- [x] 位置 (ue/shita/naka)・サイズ (big/small/medium)・色を解釈
- [x] 色は最後勝ち、hex も反映
- [x] 大文字小文字を無視
- [x] `invisible` フラグ

### エスケープ

- [x] `{` `}` `\` をエスケープ (override 注入を防ぐ)
- [x] 改行を `\N` へ
- [x] 行頭スペースを `\h` へ

### 時刻・アルファ

- [x] 秒 → `H:MM:SS.cs` タイムコード
- [x] 不透明度 → アルファバイト (1.0→00 / 0.0→FF / 0.5→80)

### 生成全体 (`generate_ass`)

- [x] ヘッダに PlayResX/Y と Style 行が出る
- [x] コメント 0 件 → Dialogue 行なし
- [x] naka は `\move(...)` で流れる
- [x] ue は `\an8` + `\pos` で上固定・3 秒表示
- [x] shita は画面下半分に配置
- [x] `invisible` / 空白本文はスキップ
- [x] 動画長を超えて出現するコメントはスキップ
- [x] 色コマンドが `\c` タグになる
- [x] 同時刻の流れるコメントは別レーン (別 y)
- [x] 時間差があれば同一レーンを再利用できる
- [x] 入力が vpos 逆順でも時刻昇順で出力
- [x] 不透明度が Style のアルファに焼き込まれる
- [x] フォント名が Style に反映
- [x] 本文の `{ }` はタグではなくエスケープされる

## 単体テスト（`ffmpeg.rs` 内 `#[cfg(test)] mod tests`）

### プローブ (`parse_probe` / `find_dimensions` / `parse_timecode`)

- [x] stderr から解像度と長さを抽出
- [x] 縦長動画も解像度を取れる
- [x] 映像ストリームが無ければ None
- [x] `N/A` のタイムコードは None
- [x] DAR 表記の `16x9` ではなく実解像度を拾う

### 進捗パース (`parse_progress_line`)

- [x] `out_time_us` / `out_time=HH:MM:SS.cs` を秒に
- [x] それ以外の行は None

## 手動 / E2E 検証（要・本物の ffmpeg + libass）

ローカルで使い捨て統合テストを通し、以下を確認済み（CI は ffmpeg を空スタブに
するため恒久テストには含めない）:

- [x] `generate_ass` の出力を libass が構文エラーなく受理する
- [x] `burn_in_comments` の ffmpeg コマンド (filter_complex `[0:v]ass=...[v]` +
      libx264 再エンコード + faststart) が成功する
- [x] 進捗コールバックが ~1.0 まで到達する
- [x] 出力が入力と同じ解像度・長さの再生可能な MP4 になる (probe 往復)

## コマンド層（`export_video_with_comments`）

- [x] video_id を検証し、未 DL ならエラー
- [x] snapshot_id 省略時は最新スナップショットを使用
- [x] コメント 0 件ならエラー（焼き込む対象なし）
- [x] 解像度・長さは DB 優先、欠落時は ffmpeg プローブで補完
- [x] 出力は `exports/` 配下（`cleanup_storage` が消さない場所）
- [x] 一時 ASS は成否に関わらず削除
- [x] ffmpeg 失敗時は stderr 抜粋を返し、壊れた出力を削除
- [x] 進捗を `burnin:progress` (`{ videoId, percent }`) で 1% 刻みに間引いて通知

## UI（ライブラリ動画ページ）

- [x] スナップショットセクションに焼き込みエクスポートのトグル
- [x] フォント倍率・不透明度のスライダ
- [x] 進捗バー（`burnin:progress` 購読）
- [x] 完了後に出力パス表示 + フォルダを開くボタン
- [x] 実行中は二重起動を抑止、離脱時にリスナ解除

## 範囲外（Phase 2.0 以降）

- 保存先をユーザが選ぶ保存ダイアログ（`dialog:allow-save` 追加が必要）
- AA（複数行アスキーアート）の厳密なレイアウト再現
- コメント密度に応じた自動フォント縮小（niconicomments の resized 相当）
- スナップショット間の diff 表示
