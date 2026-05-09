# Re:NNDD

ニコニコ動画専用クライアント NNDD の精神的後継。Tauri 2 + Rust + Svelte 5 で実装するデスクトップアーカイブクライアント。

## このドキュメントの位置づけ

本ファイルは Claude Code 向けの実装仕様書である。設計意図・制約・優先順位がここに集約されている。実装中に判断に迷ったらまずここを参照する。仕様変更が発生した場合は本ファイルを更新してから実装する。

## プロジェクトの目的

ニコニコ動画を**ローカルでアーカイブ・視聴・管理**するためのデスクトップクライアントを作る。単なるダウンローダではなく、削除耐性のあるライブラリと、コメント別レイヤー方式の専用ビュアーを核とする。NNDD（MineAP 氏オリジナル → SSW-SCIENTIFIC NNDD+DMC → NKS0131/nndd-reboot/kairi003 系）が Adobe AIR の終焉とともにメンテナンス困難になった現状を踏まえ、現代的なスタックで再実装する。

設計の根本動機は次の三点：

1. **削除耐性**：動画が消えてもメタ・コメ・サムネ・説明文がローカルに完全に残る
2. **別レイヤー再生**：コメは焼き込まず、再生時に動的にオーバーレイする。NG・スナップショット切替・透明度調整が常時可能
3. **簡易性**：機能は少なくていい。NNDD 準拠の基本機能 + 現代に必須の最低限 + 簡易 NG。凝らない

## ターゲット環境

- macOS (Apple Silicon / Intel)
- Linux (Debian / Ubuntu / Arch)
- Windows 10/11

開発はメンテナの主要環境（Debian）で動作確認する。

## 技術スタック

| レイヤ | 採用 | 用途 |
|---|---|---|
| アプリフレームワーク | **Tauri 2** | デスクトップシェル、IPC |
| バックエンド | **Rust** (stable) | API、ダウンロード、DB、ファイル管理 |
| UI | **Svelte 5 + SvelteKit (SPA mode)** | フロントエンド |
| データベース | **SQLite** (rusqlite + FTS5) | ライブラリ・コメ・NG・履歴 |
| コメ描画 | **niconicomments** (npm) | 公式互換コメレンダラ |
| 動画再生 | HTML5 `<video>` + 透過 `<canvas>` オーバーレイ | 別レイヤー再生 |
| HLS | Rust 自前実装 (reqwest + aes) | AES-128-CBC 復号対応 |
| 焼き込み出力 | **WebCodecs + mp4-muxer** | エクスポート専用、WebView 内で実行 |
| ロギング | **tracing** | Rust 側構造化ログ |
| エラー | **thiserror** + **anyhow** | ライブラリ層は thiserror、アプリ層は anyhow |
| シリアライズ | **serde** + **serde_json** | API レスポンス・設定ファイル |
| HTTP | **reqwest** (rustls-tls) | ニコニコ API・HLS フェッチ |
| 非同期 | **tokio** | ランタイム |
| テスト | **cargo test** + **vitest** | Rust 単体・結合、Svelte コンポーネント |

採用理由（補足）：

- **Tauri 2**：Electron の 1/10 サイズで起動が速い。NNDD が愛された「軽さ」を継承する
- **Svelte 5**：runes により状態管理が直感的、コンポーネント記述が簡潔
- **niconicomments**：公式互換のコメ描画は自前で再実装すべきでない。NicoCommentDL でも採用済み

## ディレクトリ構成

```
nndd-next/
├── CLAUDE.md                       # このファイル
├── README.md
├── package.json
├── Cargo.toml                      # ワークスペースルート
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── src/
│       ├── main.rs
│       ├── commands.rs             # Tauri invoke ハンドラ集約
│       ├── error.rs                # アプリ全体のエラー型
│       ├── api/                    # ニコニコ API アダプタ層（差し替え可能）
│       │   ├── mod.rs
│       │   ├── auth.rs             # セッション認証
│       │   ├── video.rs            # v3 video API
│       │   ├── comment.rs          # threads (nvComment) API
│       │   ├── search.rs           # 検索 API
│       │   └── types.rs            # API レスポンス型
│       ├── downloader/             # HLS + AES ダウンローダ
│       │   ├── mod.rs
│       │   ├── hls.rs              # m3u8 解析、セグメント並列フェッチ
│       │   ├── aes.rs              # AES-128-CBC 復号
│       │   ├── queue.rs            # ダウンロードキュー、進捗
│       │   └── scheduler.rs        # 予約ダウンロード
│       ├── library/                # ライブラリ層（DB 操作）
│       │   ├── mod.rs
│       │   ├── schema.rs           # マイグレーション
│       │   ├── videos.rs
│       │   ├── comments.rs         # スナップショット管理
│       │   ├── playlists.rs
│       │   ├── ng.rs               # NG ルール CRUD
│       │   ├── history.rs
│       │   └── search.rs           # FTS5 ローカル検索
│       └── exporter/               # コメ焼き込み出力（Phase 1 末）
│           ├── mod.rs
│           └── burn.rs             # WebView 経由で実行（後述）
├── src/                            # Svelte フロント
│   ├── app.html
│   ├── routes/
│   │   ├── +layout.svelte          # サイドバー、グローバル NG プロファイル切替
│   │   ├── +page.svelte            # ライブラリ
│   │   ├── video/[id]/
│   │   │   └── +page.svelte        # プレイヤー
│   │   ├── search/
│   │   ├── playlists/
│   │   ├── downloads/
│   │   ├── ng/
│   │   └── settings/
│   └── lib/
│       ├── player/                 # プレイヤーコア
│       │   ├── Player.svelte
│       │   ├── CommentLayer.svelte # 透過 Canvas + niconicomments
│       │   ├── ControlBar.svelte
│       │   ├── CommentList.svelte
│       │   └── shortcuts.ts
│       ├── ng/                     # NG フィルタ層
│       │   ├── filter.ts
│       │   └── importers.ts        # CSV/JSON インポート
│       ├── api.ts                  # Tauri invoke ラッパー
│       └── stores/                 # Svelte runes ベースの状態
└── tests/
    ├── rust/
    │   ├── api_*.rs                # API モック使った結合テスト
    │   ├── downloader_*.rs
    │   └── library_*.rs
    └── svelte/
        ├── ng-filter.test.ts
        └── player.test.ts
```

## データ保存場所

OS の標準位置に従う。Tauri の `app_data_dir()` を使用する。

```
macOS:   ~/Library/Application Support/nndd-next/
Linux:   ~/.local/share/nndd-next/
Windows: %APPDATA%\nndd-next\
```

物理レイアウト：

```
nndd-next/
├── library.db                      # SQLite 本体
├── library.db-wal                  # WAL モード使用
├── videos/
│   └── {video_id}/                 # sm12345 等
│       ├── video.mp4               # 生動画（コメなし、汎用プレイヤーで再生可）
│       ├── thumbnail.jpg
│       └── description.txt
└── exports/                        # 焼き込み出力先（デフォルト、設定で変更可）
```

コメは全件 SQLite に保存する（FTS5 全文検索のため）。動画ファイルは生 MP4 のみで、コメは焼き込まない。

## SQLite スキーマ

WAL モードで運用する（並行読み取りのため）。マイグレーションは `library/schema.rs` で一元管理する。

```sql
-- 動画メタ
CREATE TABLE videos (
  id                   TEXT PRIMARY KEY,         -- sm12345 / so67890
  title                TEXT NOT NULL,
  description          TEXT,
  uploader_id          TEXT,                     -- channel/ch12345 もしくは user_id
  uploader_name        TEXT,
  uploader_type        TEXT,                     -- 'user' / 'channel'
  category             TEXT,
  duration_sec         INTEGER NOT NULL,
  posted_at            INTEGER,                  -- unix epoch
  view_count           INTEGER,
  comment_count        INTEGER,
  mylist_count         INTEGER,
  thumbnail_url        TEXT,
  status               TEXT NOT NULL DEFAULT 'active',  -- 'active' / 'deleted' / 'private'
  status_checked_at    INTEGER,
  downloaded_at        INTEGER,
  video_path           TEXT,                     -- 'videos/sm12345/video.mp4' 相対
  resume_position_sec  REAL DEFAULT 0,
  last_played_at       INTEGER,
  play_count           INTEGER NOT NULL DEFAULT 0,
  raw_meta_json        TEXT                      -- API レスポンス丸ごと（後方互換用）
);

CREATE INDEX idx_videos_status ON videos(status);
CREATE INDEX idx_videos_uploader ON videos(uploader_id);
CREATE INDEX idx_videos_posted_at ON videos(posted_at);

-- タグ（複数）
CREATE TABLE tags (
  video_id   TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  name       TEXT NOT NULL,
  is_locked  INTEGER NOT NULL DEFAULT 0,
  source     TEXT NOT NULL DEFAULT 'official',   -- 'official' / 'local'
  PRIMARY KEY (video_id, name, source)
);

CREATE INDEX idx_tags_name ON tags(name);

-- コメスナップショット（取得日時別）
CREATE TABLE comment_snapshots (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id    TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  taken_at    INTEGER NOT NULL,
  is_initial  INTEGER NOT NULL DEFAULT 0,         -- ライブラリ取り込み時の最初のスナップショット
  comment_count INTEGER NOT NULL DEFAULT 0,
  note        TEXT
);

CREATE INDEX idx_snapshots_video ON comment_snapshots(video_id, taken_at);

-- コメ本体
CREATE TABLE comments (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  snapshot_id   INTEGER NOT NULL REFERENCES comment_snapshots(id) ON DELETE CASCADE,
  no            INTEGER NOT NULL,                -- コメ番号
  vpos_ms       INTEGER NOT NULL,                -- 動画内位置（ミリ秒）
  content       TEXT NOT NULL,
  mail          TEXT,                            -- コマンド（big red 等）
  user_hash     TEXT,                            -- 匿名コメユーザの hash 識別子
  is_owner      INTEGER NOT NULL DEFAULT 0,      -- 投稿者コメか
  posted_at     INTEGER
);

CREATE INDEX idx_comments_snapshot ON comments(snapshot_id);
CREATE INDEX idx_comments_user_hash ON comments(user_hash);
CREATE INDEX idx_comments_vpos ON comments(snapshot_id, vpos_ms);

-- FTS5 全文検索
CREATE VIRTUAL TABLE comments_fts USING fts5(
  content,
  content=comments,
  content_rowid=id,
  tokenize='trigram'                             -- 日本語対応
);

CREATE TRIGGER comments_fts_ai AFTER INSERT ON comments BEGIN
  INSERT INTO comments_fts(rowid, content) VALUES (new.id, new.content);
END;
CREATE TRIGGER comments_fts_ad AFTER DELETE ON comments BEGIN
  INSERT INTO comments_fts(comments_fts, rowid, content) VALUES('delete', old.id, old.content);
END;

-- プレイリスト（独自＋公式マイリストのインポート版）
CREATE TABLE playlists (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  name               TEXT NOT NULL,
  parent_id          INTEGER REFERENCES playlists(id) ON DELETE CASCADE,
  source             TEXT NOT NULL DEFAULT 'local',  -- 'local' / 'official_imported'
  source_official_id TEXT,                       -- 公式マイリスト ID（インポート時のみ）
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

-- 再生履歴
CREATE TABLE play_history (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id      TEXT NOT NULL REFERENCES videos(id) ON DELETE CASCADE,
  played_at     INTEGER NOT NULL,
  duration_played_sec REAL NOT NULL DEFAULT 0,
  position_at_close_sec REAL
);

CREATE INDEX idx_history_video ON play_history(video_id);
CREATE INDEX idx_history_played_at ON play_history(played_at);

-- NG ルール
CREATE TABLE ng_rules (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  target_type     TEXT NOT NULL,                 -- 'video_title' / 'uploader' / 'video_id' / 'tag' / 'category' / 'comment_body' / 'comment_user'
  match_mode      TEXT NOT NULL,                 -- 'exact' / 'partial' / 'regex'
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

-- ダウンロードキュー
CREATE TABLE download_queue (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id      TEXT NOT NULL,
  status        TEXT NOT NULL,                   -- 'pending' / 'downloading' / 'done' / 'error' / 'paused'
  progress      REAL NOT NULL DEFAULT 0,
  error_message TEXT,
  scheduled_at  INTEGER,
  started_at    INTEGER,
  finished_at   INTEGER,
  retry_count   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_dl_status ON download_queue(status);

-- 設定（KV）
CREATE TABLE settings (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL
);

-- マイグレーション管理
CREATE TABLE schema_version (
  version INTEGER PRIMARY KEY,
  applied_at INTEGER NOT NULL
);
```

スキーマ変更時はマイグレーション関数を追加し、`schema_version` で管理する。後方互換のないスキーマ変更は避ける。

## 機能仕様

### ライブラリ

- グリッド表示／リスト表示の切替
- フィルタ：タイトル部分一致、タグ、投稿者、ステータス（active/deleted）
- ソート：取得日時／投稿日時／タイトル／再生回数／最終再生日時
- FTS5 によるコメ全文検索（「動画内のコメに『キタ━━』を含む動画」等）
- 削除済み動画もデフォルト表示（status:deleted バッジ表示）

### ダウンロード

- 動画 (HLS / AES-128-CBC 復号対応)、コメ全件、投稿者コメ、タグ、カテゴリ、サムネ、説明文、メタを **同時取得**
- ダウンロード時点で comment_snapshots に `is_initial=1` のスナップショットを 1 件生成
- 解像度は最大画質を自動選択（設定で上限指定可）
- DL キュー：並列ダウンロード数は設定可（デフォルト 2）
- 予約ダウンロード：指定時刻に開始
- 失敗時のリトライ：最大 3 回、指数バックオフ

### プレイヤー

別レイヤー方式。`<video>` で生 MP4 を再生し、透過 `<canvas>` に niconicomments で描画する。コメは動画ファイルに焼き込まれない。

**継承機能（NNDD 準拠）**

- 再生・一時停止・シーク・音量・ミュート
- 全画面
- コメ表示 ON/OFF（キーボード `C`）
- コメリスト同期（プレイヤー上のコメをクリック → リスト側ハイライト、リスト側クリック → 該当 vpos にジャンプ）
- 投稿者コメ区別表示（リスト上で `[投稿者]` バッジ）
- 続きから再生（`videos.resume_position_sec` を再生終了時に保存）
- 再生履歴記録（`play_history` に追記）

**現代として最低限**

- 可変速：0.5x / 0.75x / 1.0x / 1.25x / 1.5x / 2.0x
- フレーム送り：一時停止中の `,` `.`
- A-B リピート：`I` で IN 点、`O` で OUT 点、`L` でループ ON/OFF
- シークバーホバーでサムネプレビュー（事前に `ffmpegなどで` で生成 or 再生中に動的生成）
- キーボードショートカット（後述）

**コメスナップショット**

- 動画詳細画面に「コメを再取得」ボタン
- 押すと現時点のコメを API から取得し、新規 `comment_snapshots` 行を追加
- プレイヤーのコメ層にドロップダウン「スナップショット切替」（latest / 過去日時別）
- スナップショット削除 UI は MVP では実装しない（手動 DB 操作で対応）

**動画画面のスクショ**
-コメなしのときはコメ無しで　あるときはある画像を取る

**実装しない（明示）**

- PIP / ミニプレイヤー
- 明度・コントラスト調整
- 比較ビュー（2 画面同時再生）
- GIF/WebP エクスポート
- シーンマーキング（章立て）
- アスペクト比補正
- コメ濃度調整スライダ（NG で代替）

これらは Phase 2 以降で必要に応じて追加する。MVP では入れない。

### キーボードショートカット

| キー | 動作 |
|---|---|
| Space | 再生/一時停止 |
| ← / → | 5 秒シーク |
| Shift + ← / → | 1 秒シーク |
| Ctrl + ← / → | 30 秒シーク |
| `,` / `.` | フレーム送り（一時停止中） |
| C | コメ表示切替 |
| F | 全画面切替 |
| M | ミュート切替 |
| 0-9 | 動画位置にジャンプ（10% 刻み） |
| I / O | A-B リピート IN/OUT 点設定 |
| L | A-B ループ ON/OFF |
| N / P | 次 / 前の動画（プレイリスト再生時） |
| ↑ / ↓ | 音量 ±5% |

### NG 機能

**ターゲット（7 種）**

| ターゲット | match_mode |
|---|---|
| 動画タイトル (`video_title`) | partial / regex |
| 投稿者 (`uploader`) | exact (uploader_id) |
| 動画 ID (`video_id`) | exact |
| タグ (`tag`) | exact |
| カテゴリ (`category`) | exact |
| コメ本文 (`comment_body`) | partial / regex |
| コメ投稿者 (`comment_user`) | exact (user_hash) |

**スコープ（3 種、独立フラグ）**

- `scope_ranking`：ランキング表示で非表示
- `scope_search`：検索結果で非表示
- `scope_comment`：コメ表示で非表示（描画から除外、データは保持）

NG 適用は **描画時計算**で行う。データ層には常に全件保持する。これにより NG ルール変更を再ダウンロードなしで即時反映できる。

NG ルール追加時は `enabled=1` でデフォルト有効。プレイヤーに「NG: N 件除外中」表示を入れて透明性を担保する。

`hit_count` と `last_hit_at` は NG が実際にマッチした際にインクリメントする。Phase 2 で「1 年マッチしてないルール」掃除機能の基礎データになる。

**プロファイル**

MVP では単一プロファイル。Phase 2 で複数プロファイル化する際は `ng_rules.profile_id` を追加し、デフォルト 1 で既存ルールを移行する。

**インポート**

CSV/JSON 形式。フォーマット例：

```json
[
  { "target_type": "uploader", "match_mode": "exact", "pattern": "user/12345", "scope_search": true, "note": "地雷姫リストより" },
  { "target_type": "video_title", "match_mode": "regex", "pattern": "【再投稿】", "scope_ranking": true, "scope_search": true }
]
```

CSV はヘッダ必須。

### プレイリスト

- ローカル独自プレイリスト：階層対応（`parent_id` で親子）、自由命名
- 公式マイリストインポート：API から取得 → `source='official_imported'` で取り込み
- インポートは読み取り専用。アプリから公式マイリスト書き込みはしない
- プレイリスト内動画の再生順序は `playlist_items.position` で管理、ドラッグ&ドロップで並び替え

### 検索

- ローカル検索（ライブラリ内）：タイトル LIKE、タグ完全一致、投稿者完全一致、コメ FTS5
- オンライン検索（公式 API）：MVP では基本検索のみ。並び替え（人気・新着）対応

### 履歴

- `play_history` に記録
- 履歴ビュー：時系列、再生回数集計、フィルタ（直近 7 日 / 30 日 / 全期間）

## ニコニコ API アダプタ層

API は仕様変更が頻発する（特に 2024 年以降）。アプリの寿命を伸ばすため、API 呼び出しは `src-tauri/src/api/` 配下に隔離する。他のレイヤから直接 reqwest を呼ばない。

設計原則：

1. **trait による抽象化**：`trait VideoApi { async fn get_video(&self, id: &str) -> Result<VideoMeta>; ... }` を定義し、本番実装とテストモックを分ける
2. **DTO とドメイン型を分離**：`api/types.rs` に API レスポンスそのものを表す DTO、`library/types.rs` に永続化用ドメイン型を置き、変換関数で繋ぐ
3. **エラーは型安全に**：`thiserror` で `ApiError` 列挙、HTTP ステータス別に分岐可能に
4. **レート制限を尊重**：429 を受けたら指数バックオフ、デフォルト並列数を控えめに

実装の参照元：

- `abeshinzo78/NicoCommentDL` の `src/background/api/niconico.js` （JWT 解析、HLS URL 取得、コメ取得ロジック）
- `abeshinzo78/NicoCommentDL` の `src/background/api/hls-fetcher.js` （HLS セグメント取得、AES-128 復号、プリフェッチ）

これらの JS ロジックを Rust に移植する。テストカバレッジを取りながら段階的に移植する。

User-Agent はニコニコのブラウザクライアントを模倣する（メンテナがいい感じに調整）。

## 焼き込みエクスポート

Phase 1 末に実装。NicoCommentDL のコードを吸収する。

実装方式：**Rust 側ではなく、Tauri WebView 内の JavaScript で実行する**。理由：

- WebCodecs (VideoDecoder/VideoEncoder) は Rust 側にネイティブ実装が無い
- Canvas2D で niconicomments を描画→ VideoFrame 化→ VideoEncoder→ mp4-muxer のパイプラインは NicoCommentDL で実証済み
- WebView 内で動かせば既存コードを最大限再利用できる

UI フロー：

1. ライブラリ／プレイヤーから「焼き込みエクスポート」ボタン
2. Rust 側が動画ファイルとコメ JSON を WebView に渡す（Tauri の asset protocol または invoke 経由でメモリに）
3. WebView 内のエクスポートワーカーが NicoCommentDL のロジックでコメ込み MP4 を生成
4. 完成 MP4 を Rust 側に書き戻し、`exports/` に保存
5. 完了通知

エクスポート時は **現在の NG プロファイル**を適用（NG されたコメは焼き込まれない）。

## TDD 指針（t_wada 流）

メンテナの方針として TDD で進める。サイクルは Red → Green → Refactor を厳守する。

**Test List 駆動**

機能着手前に、`docs/test-lists/` 配下にその機能で書くべきテストを箇条書きする。例：

```markdown
# NG ルール CRUD のテストリスト

- [ ] 空の DB で list_ng_rules() は空配列を返す
- [ ] add_ng_rule() で追加した ルールが list で取得できる
- [ ] target_type に不正な値を渡すと ValidationError が返る
- [ ] match_mode='regex' で不正な正規表現を渡すと InvalidRegexError が返る
- [ ] update_ng_rule() で enabled=0 にしたルールは適用層から除外される
- [ ] delete_ng_rule() で削除すると以後 list に出ない
- [ ] hit_count は match() を呼んだ回数だけ増える
```

**仮実装 → 三角測量 → 明白な実装**

- 最初は決め打ちの返り値で Green にする（仮実装）
- 別の入力で Red にする（三角測量）
- 一般化する（明白な実装）

**テスト粒度**

- Rust 側：`#[cfg(test)] mod tests` で同ファイル内に単体テスト、`tests/` で結合テスト
- DB を触るテストは tempfile でテンポラリディレクトリに SQLite を作る
- API 層のテストは `mockito` クレートで HTTP モック
- Svelte 側：vitest + @testing-library/svelte でコンポーネントテスト

**カバレッジ目標**

- ライブラリ層（library/）：90% 以上
- API 層（api/）：80% 以上（HTTP モック前提）
- ダウンローダ層（downloader/）：80% 以上
- UI 層：主要ストア・フィルタロジックのみ。コンポーネントの見た目テストはしない

**コミット粒度**

Red コミットは原則禁止。Red → Green → Refactor を 1 コミットにまとめる。コミットメッセージは Conventional Commits を緩く採用：

```
feat(ng): add comment_user target type
fix(downloader): handle 404 from HLS segment with retry
test(library): add migration rollback test
refactor(api): extract auth header builder
```

## コーディング規約

**Rust**

- `rustfmt` 設定はデフォルト
- `clippy` の警告はゼロにする（`#[allow]` は理由をコメント）
- エラー型：ライブラリ層は `thiserror` で具体型、アプリ層（commands.rs）は `anyhow::Result`
- 命名：snake_case（変数・関数）、PascalCase（型）、SCREAMING_SNAKE_CASE（定数）
- 非同期：`async fn` を基本、`tokio::spawn` は理由がある時だけ
- パニック：本番コードで `unwrap()` `expect()` 禁止。テスト内のみ許可

**TypeScript / Svelte**

- ESLint + Prettier（プロジェクト初期化時に設定）
- Svelte 5 runes (`$state`, `$derived`, `$effect`) を使う
- 型は `interface` ではなく `type` を優先（プロジェクト内で統一）
- ファイル名：kebab-case（`comment-layer.svelte`）
- イベントハンドラ：`on:click` ではなく Svelte 5 の `onclick=` 構文

**SQL**

- スキーマ定義は SQL 直書き（rusqlite に文字列で渡す）
- マイグレーションファイルは番号付き：`m001_initial.sql`, `m002_add_ng_profile.sql`
- インデックスは命名規則：`idx_<table>_<columns>`

## ビルド・テストコマンド

```bash
# 初期セットアップ
npm install
cargo build --manifest-path src-tauri/Cargo.toml

# 開発
npm run tauri dev

# プロダクションビルド
npm run tauri build

# テスト
cargo test --manifest-path src-tauri/Cargo.toml
npm test                                  # vitest

# Rust の特定モジュールだけテスト
cargo test --manifest-path src-tauri/Cargo.toml library::ng

# テスト＋カバレッジ
cargo install cargo-llvm-cov              # 初回のみ
cargo llvm-cov --html

# Lint
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
npm run lint

# フォーマット
cargo fmt --manifest-path src-tauri/Cargo.toml
npm run format
```

## 実装フェーズ（推奨順序）

各フェーズの完了条件は「テストが書かれている」「メンテナが手元で動作確認した」「CLAUDE.md が必要に応じて更新された」の三点。

### Phase 1.0：基盤

1. プロジェクトセットアップ（Tauri 2 + Svelte 5 スケルトン、Cargo workspace、ESLint・Prettier・rustfmt 設定）
2. SQLite スキーマと マイグレーション（`library/schema.rs`）、`library_test.rs` で migration up/down テスト
3. 基本エラー型（`error.rs`）
4. ロガー設定（tracing）

### Phase 1.1：API アダプタ層

1. `api/types.rs`：DTO 型を一通り
2. `api/auth.rs`：セッション認証（クッキー保持）
3. `api/video.rs`：v3 video API（メタ取得、HLS URL 取得）
4. `api/comment.rs`：threads API（コメ取得）
5. `api/search.rs`：基本検索

各モジュールは `mockito` で HTTP モックしたテストを書く。

### Phase 1.2：ダウンローダ

1. `downloader/hls.rs`：m3u8 解析、セグメントリスト取得
2. `downloader/aes.rs`：AES-128-CBC 復号
3. セグメント並列フェッチ（並列数 6）
4. `downloader/queue.rs`：キュー管理、進捗通知（Tauri event 経由で UI へ）
5. ライブラリ取り込み（DL 完了時に `videos`, `tags`, 初期 `comment_snapshots` + `comments` を一括 INSERT）

### Phase 1.3：ライブラリ層

1. `library/videos.rs`：CRUD、フィルタ、ソート
2. `library/comments.rs`：スナップショット管理、コメ一括 INSERT、FTS5 同期
3. `library/playlists.rs`：階層プレイリスト
4. `library/history.rs`
5. `library/search.rs`：FTS5 クエリビルダ

### Phase 1.4：UI 骨格

1. `+layout.svelte`：サイドバー（ライブラリ／検索／プレイリスト／履歴／NG／設定）
2. ライブラリページ：グリッド/リスト切替、フィルタ
3. 動画詳細ページ（プレイヤー前段）
4. ダウンロードページ：キュー表示

### Phase 1.5：プレイヤー

1. `Player.svelte`：`<video>` ラッパー、再生制御
2. `CommentLayer.svelte`：透過 Canvas + niconicomments 統合
3. `ControlBar.svelte`：再生コントロール、可変速、A-B リピート
4. `CommentList.svelte`：コメリスト、双方向同期
5. キーボードショートカット
6. シークプレビュー
7. 続きから再生・履歴記録

### Phase 1.6：NG 機能

1. `library/ng.rs`：CRUD、`hit_count` 更新
2. `lib/ng/filter.ts`：フロント側フィルタ層（コメ描画前、検索結果フィルタ、ランキングフィルタ）
3. NG 設定 UI
4. CSV/JSON インポート（地雷姫リスト対応）
5. プレイヤーに「NG: N 件除外中」表示

### Phase 1.7：プレイリスト・検索 UI

1. プレイリスト管理 UI（作成・階層化・並び替え）
2. 公式マイリストインポート UI
3. ローカル検索 UI（FTS5 クエリ含む）
4. オンライン検索 UI

### Phase 1.8：コメスナップショット

1. 「コメを再取得」ボタン
2. プレイヤーにスナップショット切替ドロップダウン
3. スナップショット一覧 UI（動画詳細画面）

### Phase 1.9：焼き込みエクスポート

NicoCommentDL のコード吸収。

1. WebCodecs パイプライン移植（VideoDecoder/Encoder + Canvas2D 合成）
2. niconicomments 描画統合
3. mp4-muxer 統合
4. NG プロファイル適用
5. エクスポート UI、進捗表示

### Phase 1.10：仕上げ

1. 設定画面（DL 並列数、デフォルト画質、エクスポート先、テーマ）
2. 自動アップデート（Tauri updater）
3. macOS / Linux / Windows でのビルド確認
4. README、配布用バイナリ

## Phase 2 以降（参考）

MVP 完成後に着手する候補。優先度は運用しながら判断する。

- 削除検知ジョブ（バックグラウンド、定期 API ポーリング）
- 動画系譜・派生グラフ（手動関連付け UI、可視化）
- 再投稿・別人投稿の名寄せ（タイトル類似度 / 動画長 / サムネハッシュ）
- NG プロファイル複数化＋切替 UI
- 自動 DL（マイリスト/タグ監視 → 新着自動取込）
- 公式ランキング表示
- 例のアレ音声保管庫連携
- ヒカマー / Cookie☆ wiki 連携（動画 ID → wiki ページ自動リンク）
- ヤジュッター検索連携
- Internet Archive 自動アップロード
- コメ自動学習 NG（NG 履歴から提案）

## 既存資産との関係

- `abeshinzo78/NicoCommentDL`：HLS+AES、niconicomments 統合、WebCodecs パイプラインの**移植元**。Phase 1.9 で焼き込み機能をこのプロジェクトに統合した時点で、NicoCommentDL は新規開発を停止し、ユーザには NNDD-NEXT への移行を案内する。Issue 受付は当面継続
- `abeshinzo78/better-niconico-firefox`：ニコニコ公式サイト上での体験改善ツールとして**独立維持**。NNDD-NEXT とは役割が違うので統合しない
- `abeshinzo78/niconico-clip-dl`：クリップ機能専用ツールとして**独立維持**。NNDD-NEXT 内のローカルクリップ切り出しとは別軸（公式クリップとローカル動画の切り出しは別物）

## ライセンス

MIT。NNDD オリジナル（MineAP 氏 MIT）への謝意を README に明記する。

## 重要な制約とリマインダ

- ニコニコ API は仕様変更が頻発する。**API 層は必ず差し替え可能な設計**にする
- DRM 付きコンテンツは MVP では扱わない
- 焼き込みエクスポートは **WebView 内** で行う（WebCodecs を使うため）
- 動画ファイルとコメは**分離保存**する（焼き込まない）。これは別レイヤー方式の根本前提
- 公式マイリストへの**書き込みは行わない**。読み取り専用インポートのみ
- NG は描画時計算。データ層には全件保持する
- パニックさせない。Rust 側で `unwrap()` `expect()` を本番コードで使わない
- セキュリティ：認証トークン・セッションクッキーは OS の secure storage（Tauri の `tauri-plugin-stronghold` または `keyring` クレート）に保存する。`library.db` には入れない

## このドキュメントの更新

仕様変更や設計判断の追加・変更があった場合は、実装より**先**にこのファイルを更新する。コミットでは「`docs(claude): ...`」プレフィックスで CLAUDE.md 更新だけのコミットを作る。

最後に書くがUIはNNDDリスペクト