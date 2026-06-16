import { invoke } from '@tauri-apps/api/core';

/**
 * ニコニコのサムネ画像が「たまに表示されない」問題への共通耐性付与アクション。
 *
 * 失敗要因は概ね次の 3 つ:
 *  1. CDN の一時的エラー / `loading="lazy"` の取りこぼしで単発で読み込み失敗する。
 *  2. 投稿者がサムネを差し替えると URL がハッシュ付き (`{id}.{hash}`) に変わり、
 *     API レスポンスや履歴・ライブラリに保存済みの旧 URL が 404 になる。
 *  3. `listingUrl` 等の署名付き URL の鍵が失効する。
 *
 * いずれも `<img>` に `onerror` が無いと壊れアイコンのまま放置される。そこで本
 * アクションは error を捕まえて多段フォールバックする:
 *  ① `getthumbinfo`(権威ソース)から現行 URL を取り直して貼り替える (2,3 を解消)
 *  ② それでも駄目なら 1 回だけ素の貼り直しを行う (純粋な一時エラー=1 を解消)
 *  ③ 万策尽きたら壊れアイコンの代わりにプレースホルダ化する
 *
 * 使い方: `<img src={url} use:thumbFallback={{ videoId: hit.contentId }} />`
 * `videoId` を渡せた時だけ①の再解決が効く(ローカルサムネやシリーズ表紙など、
 * 動画 ID に紐付かない画像は渡さなくてよい — その場合は②③のみ動く)。
 */

export type ThumbFallbackParams = {
  /** 動画 ID。権威的な再解決に使う。無ければ再解決はスキップ。 */
  videoId?: string | null;
};

/** 1x1 透明 GIF。壊れアイコンを消しつつ枠(背景)だけ残すために使う。 */
const TRANSPARENT_PX =
  'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';

/** 動画 ID 単位の再解決結果キャッシュ。短時間に同じ ID を何度も叩かない。 */
const resolveCache = new Map<string, Promise<string | null>>();

function resolveAuthoritative(videoId: string): Promise<string | null> {
  let pending = resolveCache.get(videoId);
  if (!pending) {
    pending = invoke<string | null>('resolve_thumbnail_url', { videoId }).catch(() => null);
    resolveCache.set(videoId, pending);
  }
  return pending;
}

/** クエリ/フラグメントを除いた比較用の正規化。再解決結果が現 URL と同じなら無駄打ちを避ける。 */
function sameUrl(a: string, b: string): boolean {
  const norm = (s: string) => s.split('#')[0].trim();
  return norm(a) === norm(b);
}

export function thumbFallback(img: HTMLImageElement, params: ThumbFallbackParams = {}) {
  let videoId = params.videoId ?? null;
  let triedResolve = false;
  let triedRetry = false;
  let destroyed = false;

  function showPlaceholder() {
    img.dataset.thumbBroken = 'true';
    // 枠が潰れない & 壊れアイコンを出さないよう、テーマ背景の空ボックスにする。
    if (!img.style.background) {
      img.style.background = 'var(--theme-bg, #1b1b1b)';
    }
    img.src = TRANSPARENT_PX;
  }

  async function onError() {
    if (destroyed || img.dataset.thumbBroken) return;
    const broken = img.src;

    // ① 現行サムネ URL を権威ソースから取り直して貼り替える。
    if (!triedResolve) {
      triedResolve = true;
      if (videoId) {
        const fresh = await resolveAuthoritative(videoId);
        if (destroyed || img.dataset.thumbBroken) return;
        if (fresh && !sameUrl(fresh, broken)) {
          img.src = fresh;
          return;
        }
      }
    }

    // ② 純粋な一時エラー対策に 1 回だけ貼り直す(同 URL の再フェッチ)。
    if (!triedRetry) {
      triedRetry = true;
      const url = broken;
      window.setTimeout(() => {
        if (destroyed || img.dataset.thumbBroken) return;
        // 一旦 src を外してから戻すと、同 URL でも確実に再フェッチが走る。
        img.removeAttribute('src');
        window.setTimeout(() => {
          if (!destroyed && !img.dataset.thumbBroken && !img.getAttribute('src')) {
            img.src = url;
          }
        }, 30);
      }, 300);
      return;
    }

    // ③ 万策尽きた。
    showPlaceholder();
  }

  img.addEventListener('error', onError);

  return {
    update(next: ThumbFallbackParams) {
      const nextId = next?.videoId ?? null;
      if (nextId !== videoId) {
        // バインド先の動画が変わったらフォールバック状態をリセットする
        // (リスト仮想化等で同じ <img> ノードが再利用されるケース)。
        videoId = nextId;
        triedResolve = false;
        triedRetry = false;
        delete img.dataset.thumbBroken;
      }
    },
    destroy() {
      destroyed = true;
      img.removeEventListener('error', onError);
    },
  };
}
