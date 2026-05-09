# Phase 1.1 — Snapshot Search API のテストリスト

`src-tauri/src/api/search.rs` と `src-tauri/tests/api_search.rs`。`mockito` で HTTP をスタブする。

## 単体テスト（`search.rs` 内）

- [x] `q` が空 → `ApiError::InvalidQuery`
- [x] `targets` が空 → `ApiError::InvalidQuery`
- [x] `limit` > 100 → `ApiError::InvalidQuery`
- [x] `offset` > 100_000 → `ApiError::InvalidQuery`
- [x] `_context` 文字数 > 40 → `ApiError::InvalidQuery`
- [x] `build_url` が `q` / `targets` / `_offset` / `_limit` / `_context` を必ず付ける
- [x] `filters[viewCounter][gte]=1000` がブラケット形式で URL に乗る（パーセントエンコード可）
- [x] `targets` / `fields` がカンマ区切りで連結される
- [x] `_sort` は `-fieldName` / `+fieldName` 形式
- [x] デフォルト User-Agent が `Re:NNDD/` で始まる

## 統合テスト（`tests/api_search.rs`、`mockito` 利用）

- [x] 200 + 妥当 JSON → `SearchResponse` にパース、`data[0].contentId == "sm9"`
- [x] 必須クエリパラメータ（`q`/`targets`/`_limit`/`_offset`）がリクエストに含まれる
- [x] 400 → `ApiError::QueryParseError`
- [x] 503 → `ApiError::ServerError { status: 503, .. }`
- [x] 429 → `ApiError::RateLimited`
- [x] 事前バリデーション失敗時はサーバへ到達しない（接続不可 URL でも `InvalidQuery` を返す）

## 手動確認用（任意、本番 API へ実通信）

非商用利用に限り、CLI から疎通確認できる：

```bash
curl -G \
  --data-urlencode "q=ゆっくり" \
  --data-urlencode "targets=title" \
  --data-urlencode "fields=contentId,title,viewCounter" \
  --data-urlencode "_limit=3" \
  --data-urlencode "_context=Re:NNDD/0.1.0" \
  -A "Re:NNDD/0.1.0 (manual)" \
  "https://snapshot.search.nicovideo.jp/api/v2/snapshot/video/contents/search"
```

## 範囲外（次フェーズ）

- 認証クッキーや JWT 取得（Phase 1.1 残り：`auth.rs`）
- 動画 v3 API（`video.rs`）/ コメント threads API（`comment.rs`）
- リトライポリシー（Phase 1.2 ダウンローダのスケジューラで実装）
