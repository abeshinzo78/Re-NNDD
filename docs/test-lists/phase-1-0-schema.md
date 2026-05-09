# Phase 1.0 — SQLite スキーマ・マイグレーションのテストリスト

`src-tauri/src/library/schema.rs` と `src-tauri/tests/library_schema.rs` で実装する。

## 単体テスト（`schema.rs` 内 `#[cfg(test)] mod tests`）

- [x] 空 DB に `run_migrations` を実行 → `schema_version` の最新 version が `MIGRATIONS` 末尾と一致
- [x] 二度実行しても `schema_version` の行数が増えない（idempotent）
- [x] CLAUDE.md の主要テーブル + `schema_version` + `comments_fts` がすべて存在する

## 統合テスト（`tests/library_schema.rs`、tempfile 使用）

- [x] WAL モードと外部キー制約が有効
- [x] `comments` への INSERT で FTS5 (`comments_fts`) に自動登録され `MATCH 'キタ'` でヒットする
- [x] `videos` 削除で `tags` / `comment_snapshots` / `play_history` がカスケード削除される
- [x] プロセス再起動（同一 DB ファイルで Connection を作り直し）後も idempotent

## 範囲外（Phase 1.3 以降）

- スナップショット切替の実時間挙動
- FTS5 のスコアリング
- マイグレーションの down／ロールバック（CLAUDE.md は append-only 方針）
