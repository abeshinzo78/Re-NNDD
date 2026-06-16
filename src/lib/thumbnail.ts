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

/** 動画 ID 単位の再解決結果キャッシュ。短時間に同じ ID を何度も叩かない。
 *  ただし失敗(null/reject)は **キャッシュしない** — 一時的なネットワーク障害や
 *  Cookie 未保存のまま 1 度引いた結果を恒久キャッシュすると、その動画は復旧後も
 *  二度とバックエンドへ問い合わせなくなってしまう (PR #13 review)。 */
const resolveCache = new Map<string, Promise<string | null>>();

function resolveAuthoritative(videoId: string): Promise<string | null> {
  const cached = resolveCache.get(videoId);
  if (cached) return cached;
  // in-flight の Promise はキャッシュして同時多発の重複呼び出しを 1 本化するが、
  // 解決結果が null(=失敗 or 削除)なら後で再試行できるよう即座に追い出す。
  const pending = invoke<string | null>('resolve_thumbnail_url', { videoId })
    .then((url) => {
      if (!url) resolveCache.delete(videoId);
      return url ?? null;
    })
    .catch(() => {
      resolveCache.delete(videoId);
      return null;
    });
  resolveCache.set(videoId, pending);
  return pending;
}

/** クエリ/フラグメントを除いた比較用の正規化。再解決結果が現 URL と同じなら無駄打ちを避ける。 */
function sameUrl(a: string, b: string): boolean {
  const norm = (s: string) => s.split('#')[0].trim();
  return norm(a) === norm(b);
}

export function thumbFallback(img: HTMLImageElement, params: ThumbFallbackParams = {}) {
  let videoId = params.videoId ?? null;
  let destroyed = false;
  // いま面倒を見ている画像ソース。<img> ノードが別サムネに使い回されて src が
  // 変わったら「別エピソード」として状態を作り直す。videoId が無い/変わらない
  // 貼り替え (シリーズ/マイリスト表紙のような use:thumbFallback 単独呼び出し) でも
  // 取りこぼさないよう、無効化の基準は videoId ではなく **現在の src** にする
  // (PR #13 review)。
  let episodeSrc: string | null = null;
  let triedResolve = false;
  let triedRetry = false;
  // 遅延処理 (再解決の await / リトライの setTimeout) は開始時のトークンを持ち、
  // src を書き込む直前に最新と照合して、古いエピソードの URL を新しい画像へ
  // 書き込むのを防ぐ。
  let token = 0;

  function showPlaceholder() {
    img.dataset.thumbBroken = 'true';
    // 枠が潰れない & 壊れアイコンを出さないよう、テーマ背景の空ボックスにする。
    if (!img.style.background) {
      img.style.background = 'var(--theme-bg, #1b1b1b)';
    }
    // 自前のプレースホルダは「現在ソース」として記録し、error ループを断つ。
    episodeSrc = TRANSPARENT_PX;
    img.src = TRANSPARENT_PX;
  }

  async function onError() {
    if (destroyed) return;
    const current = img.src;
    // 自前で貼ったプレースホルダや空 src は無視 (無限ループ防止)。
    if (!current || current === TRANSPARENT_PX) return;

    // 別ソースへ使い回されていたら状態を作り直し、旧エピソードの遅延処理を
    // トークンで無効化する (videoId が変わらないケースもここで拾える)。
    if (current !== episodeSrc) {
      episodeSrc = current;
      triedResolve = false;
      triedRetry = false;
      token += 1;
      delete img.dataset.thumbBroken;
    }
    if (img.dataset.thumbBroken === 'true') return;

    const myToken = token;
    const broken = current;
    // 遅延処理後に「破棄/別ソースへ差し替え」が起きていれば true(=もう触るな)。
    const superseded = () => destroyed || myToken !== token || img.src !== broken;

    // ① 現行サムネ URL を権威ソースから取り直して貼り替える。
    if (!triedResolve) {
      triedResolve = true;
      if (videoId) {
        const fresh = await resolveAuthoritative(videoId);
        if (superseded()) return;
        if (fresh && !sameUrl(fresh, broken)) {
          episodeSrc = fresh; // 自分の差し替えは新エピソードとして記録
          img.src = fresh;
          return;
        }
      }
    }

    // ② 純粋な一時エラー対策に 1 回だけ貼り直す(同 URL の再フェッチ)。
    if (!triedRetry) {
      triedRetry = true;
      window.setTimeout(() => {
        if (superseded()) return;
        // 一旦 src を外してから戻すと、同 URL でも確実に再フェッチが走る。
        img.removeAttribute('src');
        window.setTimeout(() => {
          // 破棄/別エピソード、または誰かが src を入れ直していたら触らない。
          if (destroyed || myToken !== token || img.getAttribute('src')) return;
          img.src = broken;
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
        // 別動画にバインドし直された。保留中の遅延処理を確実に無効化する
        // (エピソードの作り直し自体は次の error 時に src 比較で行われる)。
        videoId = nextId;
        token += 1;
      }
    },
    destroy() {
      destroyed = true;
      img.removeEventListener('error', onError);
    },
  };
}
