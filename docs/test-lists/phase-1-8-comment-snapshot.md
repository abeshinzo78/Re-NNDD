# Phase 1.8 — コメントスナップショット運用のテストリスト

`src-tauri/src/library/snapshots.rs` で実装。コマンドは `src-tauri/src/commands.rs` に追加。

## 背景

現状のスナップショットはダウンロード時に 1 件作られるだけで、ユーザが
複数スナップショットを管理（一覧・切替・削除・メモ・再取得）できない。
本フェーズでは `comment_snapshots` テーブルをフル活用する CRUD と、
既存 DL 動画に対してコメントを再取得する機能を提供する。

## 単体テスト（`snapshots.rs` 内 `#[cfg(test)] mod tests`）

### スナップショット一覧 (`list_snapshots`)

- [ ] スナップショットなし → 空配列
- [ ] 動画に 2 件スナップショット → taken_at DESC 順で取得
- [ ] 別動画のスナップショットは混ざらない

### スナップショット取得 (`get_snapshot`)

- [ ] 存在する ID → Some(CommentSnapshotRow) + note 込み
- [ ] 存在しない ID → None

### コメント取得 (`get_snapshot_comments`)

- [ ] コメントなし → 空配列
- [ ] コメントあり → vpos_ms ASC 順
- [ ] 存在しないスナップショット ID → 空配列

### スナップショット削除 (`delete_snapshot`)

- [ ] 存在する ID → true が返り、CASCADE でコメントも消える
- [ ] 存在しない ID → false が返る
- [ ] 最後の 1 件を削除しても動画本体は残る（videos 行は消えない）

### ノート更新 (`update_snapshot_note`)

- [ ] note を設定 → 更新反映
- [ ] note を None でクリア → NULL に戻る
- [ ] 存在しない ID → false が返る

### スナップショット新規取得 (`take_snapshot`)

- [ ] 初回のスナップショット → is_initial=1、comment_count 正しい
- [ ] 既存スナップショットあり → is_initial=0
- [ ] note 付きで作成 → note が保存される
- [ ] コメントが FTS5 に同期される
- [ ] video_id が videos テーブルに存在しなくてもエラーにならない（外部キー制約に注意）

## コマンド層テスト（`library_snapshot.rs` 統合テスト）

### `list_comment_snapshots`

- [ ] スナップショット一覧を返す

### `get_snapshot_comments`（コマンド）

- [ ] スナップショットのコメントを LocalPlayerComment 形式で返す

### `delete_comment_snapshot`

- [ ] 削除成功、後続の一覧から消える

### `update_snapshot_note`（コマンド）

- [ ] note の設定/クリア

### `refetch_video_comments`

- [ ] （要セッション）API からコメントを再取得し新規スナップショット作成
- [ ] （要セッション）再取得後、is_initial=0 であること

### `prepare_local_playback` snapshot_id 指定

- [ ] snapshot_id 指定なし → 最新スナップショットのコメント（後方互換）
- [ ] snapshot_id 指定あり → 指定スナップショットのコメント
- [ ] 存在しない snapshot_id → コメント空（エラーではなく）

## 範囲外（Phase 1.9 以降）

- コメント焼き込み（burn-in）エクスポート
- スナップショット間の diff 表示
- UI でのスナップショット切替・管理画面（Phase 2.0 以降）
