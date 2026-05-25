-- plugins: ユーザーインストール済みプラグインのメタデータ。
--
-- 本体ファイル (manifest.json / index.js / assets) は
-- $APPDATA/plugins/<plugin_id>/ に展開する。この表はその索引と
-- 有効/無効状態のみを保持する (manifest_json は冗長コピーだが、
-- DB だけ見れば一覧 UI が描画できるよう同梱する)。
--
-- enabled は 0/1。デフォルトは 0 — ZIP インストール直後は
-- 必ずユーザーが明示的に有効化する操作を要求する。

CREATE TABLE plugins (
  plugin_id     TEXT PRIMARY KEY,
  enabled       INTEGER NOT NULL DEFAULT 0,
  version       TEXT NOT NULL,
  manifest_json TEXT NOT NULL,
  installed_at  INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE INDEX idx_plugins_enabled ON plugins(enabled);
