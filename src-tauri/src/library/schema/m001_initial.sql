-- Phase 1.0 initial schema.
-- Mirrors CLAUDE.md §"SQLite スキーマ".

CREATE TABLE videos (
  id                   TEXT PRIMARY KEY,
  title                TEXT NOT NULL,
  description          TEXT,
  uploader_id          TEXT,
  uploader_name        TEXT,
  uploader_type        TEXT,
  category             TEXT,
  duration_sec         INTEGER NOT NULL,
  posted_at            INTEGER,
  view_count           INTEGER,
  comment_count        INTEGER,
  mylist_count         INTEGER,
  thumbnail_url        TEXT,
  status               TEXT NOT NULL DEFAULT 'active',
  status_checked_at    INTEGER,
  downloaded_at        INTEGER,
  video_path           TEXT,
  resume_position_sec  REAL DEFAULT 0,
  last_played_at       INTEGER,
  play_count           INTEGER NOT NULL DEFAULT 0,
  raw_meta_json        TEXT
);

CREATE INDEX idx_videos_status ON videos(status);
CREATE INDEX idx_videos_uploader ON videos(uploader_id);
CREATE INDEX idx_videos_posted_at ON videos(posted_at);

CREATE TABLE tags (
  video_id   TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  is_locked  INTEGER NOT NULL DEFAULT 0,
  source     TEXT NOT NULL DEFAULT 'official',
  PRIMARY KEY (video_id, name, source)
);

CREATE INDEX idx_tags_name ON tags(name);

CREATE TABLE comment_snapshots (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id      TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  taken_at      INTEGER NOT NULL,
  is_initial    INTEGER NOT NULL DEFAULT 0,
  comment_count INTEGER NOT NULL DEFAULT 0,
  note          TEXT
);

CREATE INDEX idx_snapshots_video ON comment_snapshots(video_id, taken_at);

CREATE TABLE comments (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  snapshot_id   INTEGER NOT NULL REFERENCES comment_snapshots(id) ON DELETE CASCADE,
  no            INTEGER NOT NULL,
  vpos_ms       INTEGER NOT NULL,
  content       TEXT NOT NULL,
  mail          TEXT,
  user_hash     TEXT,
  is_owner      INTEGER NOT NULL DEFAULT 0,
  posted_at     INTEGER
);

CREATE INDEX idx_comments_snapshot ON comments(snapshot_id);
CREATE INDEX idx_comments_user_hash ON comments(user_hash);
CREATE INDEX idx_comments_vpos ON comments(snapshot_id, vpos_ms);

CREATE VIRTUAL TABLE comments_fts USING fts5(
  content,
  content='comments',
  content_rowid='id',
  tokenize='trigram'
);

CREATE TRIGGER comments_fts_ai AFTER INSERT ON comments BEGIN
  INSERT INTO comments_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER comments_fts_ad AFTER DELETE ON comments BEGIN
  INSERT INTO comments_fts(comments_fts, rowid, content) VALUES('delete', old.id, old.content);
END;

CREATE TRIGGER comments_fts_au AFTER UPDATE ON comments BEGIN
  INSERT INTO comments_fts(comments_fts, rowid, content) VALUES('delete', old.id, old.content);
  INSERT INTO comments_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TABLE playlists (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  name               TEXT NOT NULL,
  parent_id          INTEGER REFERENCES playlists(id) ON DELETE CASCADE,
  source             TEXT NOT NULL DEFAULT 'local',
  source_official_id TEXT,
  imported_at        INTEGER,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL
);

CREATE TABLE playlist_items (
  playlist_id  INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
  video_id     TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  position     INTEGER NOT NULL,
  added_at     INTEGER NOT NULL,
  note         TEXT,
  PRIMARY KEY (playlist_id, video_id)
);

CREATE INDEX idx_playlist_items_position ON playlist_items(playlist_id, position);

CREATE TABLE play_history (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id              TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  played_at             INTEGER NOT NULL,
  duration_played_sec   REAL NOT NULL DEFAULT 0,
  position_at_close_sec REAL
);

CREATE INDEX idx_history_video ON play_history(video_id);
CREATE INDEX idx_history_played_at ON play_history(played_at);

CREATE TABLE ng_rules (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  target_type     TEXT NOT NULL,
  match_mode      TEXT NOT NULL,
  pattern         TEXT NOT NULL,
  scope_ranking   INTEGER NOT NULL DEFAULT 0,
  scope_search    INTEGER NOT NULL DEFAULT 0,
  scope_comment   INTEGER NOT NULL DEFAULT 0,
  enabled         INTEGER NOT NULL DEFAULT 1,
  note            TEXT,
  created_at      INTEGER NOT NULL,
  hit_count       INTEGER NOT NULL DEFAULT 0,
  last_hit_at     INTEGER
);

CREATE INDEX idx_ng_target_enabled ON ng_rules(target_type, enabled);

CREATE TABLE download_queue (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id      TEXT NOT NULL,
  status        TEXT NOT NULL,
  progress      REAL NOT NULL DEFAULT 0,
  error_message TEXT,
  scheduled_at  INTEGER,
  started_at    INTEGER,
  finished_at   INTEGER,
  retry_count   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_dl_status ON download_queue(status);

CREATE TABLE settings (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL
);
