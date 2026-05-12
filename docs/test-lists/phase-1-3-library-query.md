# Phase 1.3 — ライブラリ検索/整列/集計のテストリスト

`src-tauri/src/library/query.rs` で実装。

## 単体テスト（`query.rs` 内 `#[cfg(test)] mod tests`）

### 基本取得

- [x] 空ライブラリに `query_videos(default)` → total_count=0, items 空
- [x] データありの `query_videos(default)` → 全件返却（downloaded_at DESC）

### ソート

- [x] `sort_by=title, asc` → タイトル辞書順
- [x] `sort_by=duration_sec, desc` → 長い順

### ページネーション

- [x] `offset=0, limit=2` → 2件のみ、total_count は全体数
- [x] `offset=2, limit=2` → 続きの2件
- [x] `limit=99999` → 500 にキャップ

### フィルタ

- [x] `tags=["VOCALOID"]` → VOCALOID タグを持つ動画のみ
- [x] `tags=["VOCALOID","初音ミク"]` → AND 条件で両方タグを持つ動画のみ
- [x] `uploader_id="u2"` → 該当投稿者の動画のみ
- [x] `min_duration=200, max_duration=300` → 時間範囲内のみ
- [x] `resolution="1280x720"` → 該当解像度のみ
- [x] フィルタ複数組み合わせ → AND 条件

### テキスト検索

- [x] `q="ミク"` → タイトル LIKE でヒット（≥3文字で FTS5 も使用）
- [x] `q="ゲーム"` → タグ名 LIKE でヒット
- [x] `q="弾幕で"` → コメント FTS5 でヒット（≥3文字）
- [x] 短いクエリ（<3文字）→ LIKE のみ、FTS スキップ
- [x] マッチなし → total_count=0

### タグ付与

- [x] 結果の各 item に tags が正しく設定される

### バリデーション

- [x] 不正 sort_by → エラー
- [x] 不正 sort_order → エラー

### 集計 (`get_stats`)

- [x] シード済み → total_videos=5, total_duration_sec 正しい
- [x] unique_uploaders=3
- [x] top_tags が頻度順
- [x] resolution_distribution に "1280x720" が count=2
- [x] 空ライブラリ → 全て 0 / 空

### 補助 API

- [x] `list_all_tags` → DISTINCT 昇順
- [x] `list_resolutions` → DISTINCT 昇順、NULL 除外

## 範囲外（Phase 1.4 以降）

- フロントエンド UI（フィルタパネル・ソートドロップダウン・ページネーション）
- 再生回数 / 最終再生日時の更新 API
- マイリスト（playlist）の検索
- NG ルールの適用除外
