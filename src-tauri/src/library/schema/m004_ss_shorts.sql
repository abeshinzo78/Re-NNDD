-- ss で始まる既存動画をショートに設定
UPDATE videos SET is_short = 1 WHERE id LIKE 'ss%';
