// 「この動画を DL」ワンクリックヘルパ。
// /video/[id] や検索結果カードから呼ぶ。
//
// フロー:
// 1) enqueue で download_queue に行を作る
// 2) すぐ start_download を呼んで yt-dlp 起動
// 3) 結果 (キュー id) と人間向けメッセージを返す
//
// 失敗してもキャッチして「失敗の理由」を返す（呼び出し側で toast 表示）。

import { enqueueDownload, startDownload } from '$lib/api';

export type QuickDownloadResult =
  | { ok: true; queueId: number; message: string }
  | { ok: false; message: string };

export async function quickDownload(videoId: string): Promise<QuickDownloadResult> {
  const id = videoId.trim();
  if (!/^[A-Za-z0-9]+$/.test(id)) {
    return { ok: false, message: '動画 ID が不正です' };
  }
  try {
    const item = await enqueueDownload(id);
    try {
      await startDownload(item.id);
      return {
        ok: true,
        queueId: item.id,
        message: `${id} の DL を開始しました`,
      };
    } catch (e) {
      // enqueue は成功しているので、ユーザは /downloads から手動再開できる
      return {
        ok: false,
        message: `キュー追加は成功 / 起動失敗: ${e}`,
      };
    }
  } catch (e) {
    return {
      ok: false,
      message: `キュー追加失敗: ${e}`,
    };
  }
}
