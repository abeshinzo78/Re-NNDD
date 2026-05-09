-- m002: ライブラリの動画行に解像度を持たせる。
-- yt-dlp の info.json から取れる width/height を "1280x720" の形で保存。
-- 既存の行は NULL のまま (空欄表示)。

ALTER TABLE videos ADD COLUMN resolution TEXT;
