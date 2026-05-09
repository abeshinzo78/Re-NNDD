# Phase 1.2: ダウンロードキュー CRUD のテストリスト

`src-tauri/src/library/queue.rs` のテスト対象。t_wada 流に Red → Green → Refactor。
DB は `LibraryHandle::open_memory()` でメモリ SQLite を migrate 済みで使う。

- [x] 空の DB で `list_all` は空配列を返す
- [x] 空の DB で `list_pending` は空配列を返す
- [x] `enqueue("sm9", None)` で追加した行が `list_all` に出る、status='pending'、progress=0、retry_count=0
- [x] 同じ video_id を再 enqueue できる（重複許容、別 id の行が増える）
- [x] `enqueue("sm9", Some(unix))` で `scheduled_at` が記録される
- [x] `mark_status(id, "downloading")` で status と started_at が更新される
- [x] `mark_status(id, "done")` で status='done' と finished_at が更新される
- [x] `update_progress(id, 0.5)` で progress が更新される（0..=1 にクランプ）
- [x] `mark_error(id, "msg")` で status='error' と error_message が記録される
- [x] `cancel(id)` で pending/paused 行が削除される
- [x] `cancel(id)` は downloading 中の行も削除する（停止判断は呼び出し側）
- [x] `list_pending` は status='pending' or 'paused' のみ、scheduled_at asc 順
- [x] `clear_finished` で done/error の行が一括削除される（pending は残る）
- [x] 存在しない id への `mark_status` / `update_progress` は 0 行更新で no-op
