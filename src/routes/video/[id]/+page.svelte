<script lang="ts">
  import { onDestroy } from 'svelte';
  import { beforeNavigate, goto } from '$app/navigation';
  import { page } from '$app/state';
  import Player from '$lib/player/Player.svelte';
  import CommentList from '$lib/player/CommentList.svelte';
  import QueueBanner from '$lib/QueueBanner.svelte';
  import { fetchVideoComments, issueHlsUrl, preparePlayback, type SearchHit } from '$lib/api';
  import { quickDownload } from '$lib/quickDownload';
  import { formatDate, formatDuration, formatNumber, videoUrl } from '$lib/format';
  import type { PlaybackPayload, PlayerComment } from '$lib/player/types';
  import { fetchRelatedVideos } from '$lib/relatedVideos';
  import { loadSearchState } from '$lib/stores/searchState';
  import SearchHitCard from '$lib/SearchHitCard.svelte';
  import MylistAddButton from '$lib/MylistAddButton.svelte';
  import { filterComments, listNgRules, subscribeNgRules, type NgRule } from '$lib/stores/ngRules';
  import { addHistory } from '$lib/stores/history';
  import { getBool, getStr, loadSettings } from '$lib/stores/settings.svelte';
  import { sanitizeDescriptionHtml } from '$lib/sanitize';
  import { miniPlayer } from '$lib/player/miniPlayerStore.svelte';
  import {
    advanceQueue,
    getQueue,
    hasNextInQueue,
    itemHref,
    setQueueIndexByVideoId,
    subscribeQueue,
  } from '$lib/stores/playbackQueue';

  // この route は **オンライン視聴専用**。ローカル再生は /library/[id] で行う。
  // 別ルートに分けることで、ネット接続が要らないときに偶発的に niconico を
  // 叩いてしまう事故を防ぐ。
  let payload = $state<PlaybackPayload | null>(null);
  let pending = $state(true);
  let error = $state<string | null>(null);
  let currentTime = $state(0);
  let comments = $state<PlayerComment[]>([]);
  let commentsLoading = $state(false);

  let related = $state<SearchHit[]>([]);
  let relatedLoading = $state(false);
  let relatedVisibleCount = $state(0);
  let relatedError = $state<string | null>(null);

  let ngRules = $state<NgRule[]>(listNgRules());
  const ngUnsub = subscribeNgRules(() => (ngRules = listNgRules()));
  onDestroy(() => ngUnsub());

  let visibleComments = $derived(filterComments(ngRules, comments));
  let ngFilteredCount = $derived(comments.length - visibleComments.length);

  function tagSearchHref(tag: string): string {
    return `/search?q=${encodeURIComponent(tag)}&targets=tagsExact`;
  }

  let backHref = $state('/search');
  let backLabel = $state('← 検索に戻る');

  type PlayerRef = {
    seek: (t: number) => void;
    getVideo: () => HTMLVideoElement | null;
    getCurrentTime: () => number;
    isPaused: () => boolean;
  };
  let playerRef = $state<PlayerRef | undefined>();

  let videoId = $derived(page.params.id ?? '');
  let theme = $derived(getStr('appearance.theme'));
  let isClassicTheme = $derived(theme === 'niconico-classic');
  // 説明文を最初から開いた状態で出すか (設定 appearance.expand_description)。
  // 折りたたみ自体は残すので、ユーザは展開後に手動で閉じることもできる。
  let expandDescription = $derived(getBool('appearance.expand_description'));
  let loadingFor: string | null = null;
  let loop = $state(false);
  // ユーザが Player の loop ボタンを明示的に操作したかを記録する。
  // true の間は後段の自動再計算 (キュー変更によるリセットなど) を抑止する。
  let loopUserSet = $state(false);

  // 既定 loop 値を計算する。`always_loop` を尊重しつつ、ユーザの明示的な
  // 連続再生操作 (URL が `?from=queue`) と「キューに後続あり」を満たす時のみ
  // ループを抑制する。生 URL や履歴遷移で偶然キューに含まれているだけの
  // 動画では always_loop を維持する (= bug_019 への対処)。
  function computeDefaultLoop(id: string): boolean {
    if (!getBool('playback.always_loop')) return false;
    const fromQueue = page.url.searchParams.get('from') === 'queue';
    return !(fromQueue && hasNextInQueue(id));
  }

  // 「ユーザが今キュー再生中である」と確証できる時だけ forceAutoplay を返す。
  // `?from=queue` だけでは bookmark / shared URL でも true になってしまい
  // `playback.autoplay=false` 設定をすり抜けてしまう (codex r3283322762)。
  // 永続キューが存在し、その index が今のページに一致する時のみ強制。
  function shouldForceAutoplay(id: string): boolean {
    if (page.url.searchParams.get('from') !== 'queue') return false;
    const q = getQueue();
    if (!q) return false;
    return q.items[q.index]?.videoId === id;
  }

  // キューが ■ 停止 や advance で変化した時、ユーザが手動で loop を
  // 弄っていなければ既定値を再評価する。これで「always_loop=true 設定の人が
  // ページ滞在中にキューを止める → ループが死ぬ」を防ぐ (codex review)。
  const unsubQueueLoop = subscribeQueue(() => {
    if (loopUserSet) return;
    const id = payload?.video.id ?? videoId;
    if (!id) return;
    loop = computeDefaultLoop(id);
  });
  onDestroy(() => unsubQueueLoop());

  let panelWidth = $state(320);
  let dragging = $state(false);
  let dragStartX = 0;
  let dragStartWidth = 0;

  // Choose the back-link target based on referrer info (from query param).
  $effect(() => {
    const from = page.url.searchParams.get('from');
    if (from === 'history') {
      backHref = '/history';
      backLabel = '← 履歴に戻る';
    } else if (from === 'user') {
      const uid = page.url.searchParams.get('uid');
      const kind = page.url.searchParams.get('kind') ?? 'user';
      const name = page.url.searchParams.get('name') ?? '';
      const icon = page.url.searchParams.get('icon') ?? '';
      if (uid) {
        let qs = `kind=${encodeURIComponent(kind)}`;
        if (name) qs += `&name=${encodeURIComponent(name)}`;
        if (icon) qs += `&icon=${encodeURIComponent(icon)}`;
        backHref = `/user/${uid}?${qs}`;
        backLabel = `← ${name || uid} の投稿動画に戻る`;
      } else {
        backHref = '/search';
        backLabel = '← 検索に戻る';
      }
    } else if (from === 'series') {
      const sid = page.url.searchParams.get('seriesId') ?? '';
      const stitle = page.url.searchParams.get('seriesTitle') ?? '';
      if (sid) {
        backHref = `/series/${sid}`;
        backLabel = `← シリーズ「${stitle || sid}」に戻る`;
      } else {
        backHref = '/search';
        backLabel = '← 検索に戻る';
      }
    } else if (from === 'ranking') {
      backHref = '/ranking';
      backLabel = '← ランキングに戻る';
    } else if (from === 'queue') {
      const q = getQueue();
      if (q) {
        if (q.context === 'series') {
          backHref = `/series/${q.contextId}`;
          backLabel = `← シリーズ「${q.label}」に戻る`;
        } else if (q.context === 'mylist') {
          backHref = `/playlists?mylistId=${encodeURIComponent(q.contextId)}`;
          backLabel = `← マイリスト「${q.label}」に戻る`;
        } else if (q.context === 'smart') {
          backHref = `/playlists/smart/${q.contextId}`;
          backLabel = `← スマートプレイリスト「${q.label}」に戻る`;
        } else if (q.context === 'library') {
          backHref = '/library';
          backLabel = '← ライブラリに戻る';
        } else if (q.context === 'user') {
          backHref = `/user/${q.contextId}`;
          backLabel = `← 「${q.label}」の投稿動画に戻る`;
        } else {
          backHref = '/playlists';
          backLabel = '← プレイリストに戻る';
        }
      } else {
        backHref = '/search';
        backLabel = '← 検索に戻る';
      }
    } else {
      const prev = loadSearchState();
      if (prev?.lastQuery) {
        backHref = '/search';
        backLabel = `← 「${prev.lastQuery}」の検索結果に戻る`;
      } else {
        backHref = '/search';
        backLabel = '← 検索に戻る';
      }
    }
  });

  async function load(id: string) {
    if (!id) return;
    loadingFor = id;
    pending = true;
    error = null;
    payload = null;
    comments = [];
    commentsLoading = false;
    related = [];
    relatedError = null;
    try {
      // 設定と再生情報を並列取得
      const [, result] = await Promise.all([loadSettings(), preparePlayback(id)]);
      if (loadingFor !== id) return;
      loop = computeDefaultLoop(id);
      loopUserSet = false;
      payload = result;
      pending = false;

      // Record to history
      addHistory({
        videoId: result.video.id,
        title: result.video.title,
        thumbnailUrl: result.video.thumbnailUrl,
        uploaderName: result.owner?.nickname,
        duration: result.video.duration,
        viewCount: result.video.viewCount,
      });

      // Load comments + related in parallel — video already playable.
      if (result.nvComment) {
        commentsLoading = true;
        void fetchVideoComments(result.nvComment)
          .then((c) => {
            if (loadingFor !== id) return;
            comments = c;
          })
          .catch((e) => {
            console.warn('comment fetch failed', e);
          })
          .finally(() => {
            if (loadingFor === id) commentsLoading = false;
          });
      }

      // Defer related-video fetch so it doesn't compete with the
      // player's initial buffering / segment downloads.
      relatedLoading = true;
      setTimeout(() => {
        void fetchRelatedVideos(id, result.video.title, result.video.tags)
          .then((hits) => {
            if (loadingFor !== id) return;
            related = hits;
            // Reveal cards progressively so thumbnail decode doesn't block video
            relatedVisibleCount = 0;
            const reveal = () => {
              relatedVisibleCount += 3;
              if (relatedVisibleCount < hits.length) {
                if (window.requestIdleCallback) {
                  window.requestIdleCallback(reveal, { timeout: 200 });
                } else {
                  setTimeout(reveal, 200);
                }
              }
            };
            reveal();
          })
          .catch((e) => {
            if (loadingFor !== id) return;
            relatedError = String(e);
          })
          .finally(() => {
            if (loadingFor === id) {
              relatedLoading = false;
            }
          });
      }, 3000);
    } catch (e) {
      if (loadingFor !== id) return;
      error = String(e);
      pending = false;
    }
  }

  $effect(() => {
    void load(videoId);
  });

  // 動画 ID が変わったらキューの index を合わせる (前後遷移ボタンの判定用)。
  // 直接 URL を打って入ってきた場合もここで同期する。
  $effect(() => {
    if (videoId) setQueueIndexByVideoId(videoId);
  });

  function handleSeek(t: number) {
    playerRef?.seek(t);
  }

  function handleEnded() {
    if (!getBool('playback.autoplay_queue')) return;
    const q = getQueue();
    if (!q) return;
    const idx = q.items.findIndex((it) => it.videoId === (payload?.videoId ?? videoId));
    if (idx < 0) return;
    const nxt = q.items[idx + 1];
    if (!nxt) return;
    advanceQueue();
    void goto(itemHref(nxt));
  }

  function getResumePosition(id: string): number {
    const pipPos = miniPlayer.consumeReturnPosition(id);
    if (pipPos > 0) return pipPos;
    if (!getBool('playback.resume_enabled')) return 0;
    try {
      return Number(localStorage.getItem(`resume:${id}`)) || 0;
    } catch {
      return 0;
    }
  }
  function saveResumePosition(id: string, t: number) {
    try {
      localStorage.setItem(`resume:${id}`, String(Math.floor(t)));
    } catch {
      /* */
    }
  }

  function handleTimeUpdate(time: number) {
    currentTime = time;
    if (payload && time > 0) {
      saveResumePosition(payload.video.id, time);
    }
    // PiP 音声引き継ぎ中 (mini が無音ロード中) は、ページ側 currentTime を
    // 共有ストアに書き続ける。mini は引き継ぎ瞬間にこの値へシークすることで
    // 「ロード時間ぶんの音声巻き戻し」を防ぐ。
    if (
      payload &&
      miniPlayer.active &&
      !miniPlayer.audioOwned &&
      miniPlayer.source?.videoId === payload.videoId
    ) {
      miniPlayer.setHandoffTime(time);
    }
  }

  function startDrag(e: MouseEvent) {
    e.preventDefault();
    dragging = true;
    dragStartX = e.clientX;
    dragStartWidth = panelWidth;
  }

  function onMove(e: MouseEvent) {
    if (!dragging) return;
    const delta = dragStartX - e.clientX;
    panelWidth = Math.max(200, Math.min(600, dragStartWidth + delta));
  }

  function stopDrag() {
    dragging = false;
  }

  // DL ボタン用 state
  let dlPending = $state(false);
  let dlMsg = $state<{ kind: 'ok' | 'error'; text: string } | null>(null);
  let dlMsgTimer: ReturnType<typeof setTimeout> | null = null;
  function showDlMsg(kind: 'ok' | 'error', text: string) {
    dlMsg = { kind, text };
    if (dlMsgTimer) clearTimeout(dlMsgTimer);
    dlMsgTimer = setTimeout(() => {
      dlMsg = null;
      dlMsgTimer = null;
    }, 4000);
  }
  function openPipForCurrentVideo(): boolean {
    if (!payload) return false;
    // 同じ動画で既に PiP 起動済み (音声引き継ぎ中も含む) なら何もしない。
    if (miniPlayer.active && miniPlayer.source?.videoId === payload.videoId) return false;
    // 別動画が PiP 稼働中ならこのページの Player はマウントされていない。
    // 再生中でない動画を PiP 化することは無いので、現在の PiP をそのまま維持する。
    if (
      miniPlayer.active &&
      !!miniPlayer.source?.videoId &&
      miniPlayer.source.videoId !== payload.videoId
    ) {
      return false;
    }
    const vid = playerRef?.getVideo();
    // 画質切り替えの再アタッチ中は <video> の currentTime/paused が一時的に
    // 0/paused にリセットされる。Player の論理状態 (getCurrentTime/isPaused) を
    // 使い、PiP 引き継ぎが先頭・一時停止で固定されてしまうのを防ぐ。
    const t = playerRef?.getCurrentTime() ?? currentTime ?? 0;
    // 起動時点でページ側 Player が鳴っているかを掴んでおく。
    // 鳴っている場合のみ、mini は無音ロード→引き継ぎフローを走らせる。
    const wasPlaying = vid != null && playerRef?.isPaused() === false && !vid.ended;
    // `payload` は同一コンポーネントが /video/A → /video/B でパラ遷移した時
    // 後から書き換わる。クロージャに `payload` を直接参照させると、PiP 再生中の
    // 動画 A のトークン再発行が B の URL を取得してしまう。スナップショットで固める。
    const snapVideoId = payload.videoId;
    const snapHlsUrl = payload.hlsUrl;
    const snapTitle = payload.video.title;
    // expandHref はクエリ込みで保存する。?from=history / ?from=user&uid=...
    // のようなコンテキストを保ったまま展開時に元ページへ戻すため。
    const snapHref = page.url.pathname + (page.url.search ?? '');
    if (snapVideoId) {
      try {
        localStorage.setItem(`resume:${snapVideoId}`, String(Math.floor(t)));
      } catch {
        /* ignore */
      }
    }
    miniPlayer.open({
      source: {
        kind: 'online',
        videoId: snapVideoId,
        hlsUrl: snapHlsUrl,
        refreshHlsUrl: () => issueHlsUrl(snapVideoId),
      },
      title: snapTitle,
      comments: visibleComments,
      rawComments: comments,
      resumePosition: t,
      expandHref: snapHref,
      loop,
      wasPlaying,
    });
    return true;
  }

  // PiP (ミニプレイヤー) のトグル。
  // ON: 現在の再生位置を resume に書いて miniPlayer ストアへ流し込み。
  // OFF: 元ページに戻ってきた時点で MiniPlayer 側が自動 handoff する。
  function togglePip() {
    // 音声引き継ぎ中 (audioOwned=false) も同じ動画なら「PiP 化済み」扱いで閉じる。
    if (miniPlayer.active && miniPlayer.source?.videoId === (payload?.videoId ?? '')) {
      miniPlayer.close();
      return;
    }
    openPipForCurrentVideo();
  }

  // 音声引き継ぎが完了するまでは、ページ側 Player を残し続けてプレースホルダに
  // 切り替えない。これで PiP オンの瞬間に音が途切れない。
  let pipActiveForThis = $derived(
    miniPlayer.active &&
      miniPlayer.audioOwned &&
      miniPlayer.source?.videoId === (payload?.videoId ?? ''),
  );

  // グローバル単一アクティブ Player 不変条件: PiP が別動画で稼働中なら、ここでも
  // Player を絶対にマウントしない。audioOwned に関係なく排他する (引き継ぎ中も同様)。
  // これが無いと PiP (動画 A) + ページ Player (動画 B) で二重再生になる。
  let pipActiveForOther = $derived(
    miniPlayer.active &&
      !!miniPlayer.source?.videoId &&
      miniPlayer.source.videoId !== (payload?.videoId ?? ''),
  );
  let pipExpandHref = $derived(miniPlayer.expandHref || '/');
  let pipOtherTitle = $derived(miniPlayer.title || 'ミニプレイヤー');

  // PiP 中はミニ側で取得済みコメの方が新しい可能性があるので、ミニ側にも反映
  $effect(() => {
    if (pipActiveForThis && payload) {
      miniPlayer.updateComments(payload.videoId, visibleComments, comments);
    }
  });

  // 音声引き継ぎ中、ソース側 Player の paused 状態をストアへ反映する。
  // ユーザが引き継ぎ完了前に停止した場合、mini はその意図を引き継ぐ
  // (handleReadyForAudio で pause() してから acquireAudio する)。
  // timeupdate は再生中にしか飛ばないので、独立して 200ms ポーリングする。
  $effect(() => {
    if (!payload) return;
    if (!miniPlayer.active) return;
    if (miniPlayer.audioOwned) return;
    if (miniPlayer.source?.videoId !== payload.videoId) return;
    const id = setInterval(() => {
      const v = playerRef?.getVideo();
      // 再アタッチ中の transient な paused を拾わないよう論理状態 (isPaused) を使う。
      if (v) miniPlayer.setSourcePaused((playerRef?.isPaused() ?? v.paused) || v.ended);
    }, 200);
    return () => clearInterval(id);
  });

  beforeNavigate((nav) => {
    if (!getBool('pip.auto_navigate')) return;
    const toPath = nav.to?.url.pathname;
    const fromPath = nav.from?.url.pathname;
    if (!toPath || toPath === fromPath) return;
    if (/^\/video\//.test(toPath) || /^\/library\//.test(toPath)) return;
    openPipForCurrentVideo();
  });

  async function onDownload(id: string) {
    dlPending = true;
    try {
      const r = await quickDownload(id);
      showDlMsg(r.ok ? 'ok' : 'error', r.message);
    } finally {
      dlPending = false;
    }
  }
</script>

<svelte:window onmousemove={onMove} onmouseup={stopDrag} />

<section class="page" class:classic={isClassicTheme}>
  <div class="head">
    <a class="back" href={backHref}>{backLabel}</a>
    <h2>{payload?.video.title ?? videoId}</h2>
    {#if payload}
      <div class="head-actions">
        <button
          type="button"
          class="dl-btn"
          disabled={dlPending}
          onclick={() => onDownload(payload!.video.id)}
          title="この動画をライブラリに DL"
        >
          {dlPending ? '⏳ DL 起動中…' : '⬇ ライブラリに DL'}
        </button>
        <MylistAddButton
          video={{
            videoId: payload.video.id,
            title: payload.video.title,
            thumbnailUrl: payload.video.thumbnailUrl,
            lengthSeconds: payload.video.duration,
            viewCounter: payload.video.viewCount,
            uploaderName: payload.owner?.nickname,
          }}
        />
      </div>
    {/if}
  </div>
  {#if dlMsg}
    <div class="dl-msg {dlMsg.kind}">{dlMsg.text}</div>
  {/if}

  <QueueBanner videoId={payload?.video.id ?? videoId} />

  {#if pending}
    <div class="player-row">
      <div class="player-col">
        <div class="loading-skeleton">
          <img
            src={`https://tn.smilevideo.jp/smile?i=${videoId.replace(/^[a-z]+/, '')}`}
            alt=""
            class="skeleton-thumb"
            onerror={(e) => {
              (e.target as HTMLElement).style.display = 'none';
            }}
          />
          <div class="skeleton-overlay">
            <div class="skeleton-spinner"></div>
            <div class="skeleton-text">読み込み中…</div>
          </div>
        </div>
      </div>
    </div>
  {:else if error}
    <div class="error">エラー: {error}</div>
    <p class="muted">
      ログインが必要な動画の場合は <a href="/settings">設定</a> で <code>user_session</code> Cookie
      を入れてください。 DL 済みなら <a href={`/library/${videoId}`}>ローカル再生</a> に切り替えてください。
    </p>
  {:else if payload}
    {@const p = payload}
    <div class="player-frame">
      <div class="viewer-toolbar">
        <div class="viewer-toolbar-meta">
          <span class="toolbar-id">{payload.video.id}</span>
          <span class="toolbar-sep">|</span>
          <span>{formatDuration(payload.video.duration)}</span>
          {#if payload.video.registeredAt}
            <span class="toolbar-sep">|</span>
            <span>{formatDate(payload.video.registeredAt)}</span>
          {/if}
        </div>
        <div class="viewer-toolbar-actions">
          <button
            type="button"
            class="toolbar-btn"
            disabled={dlPending}
            onclick={() => onDownload(payload!.video.id)}
          >
            {dlPending ? 'DL 中…' : 'ライブラリに DL'}
          </button>
        </div>
      </div>
      <div class="player-row" class:dragging>
        <div class="player-col">
          {#if pipActiveForThis}
            <div class="pip-placeholder">
              <div class="pip-thumb">
                {#if p.video.thumbnailUrl}
                  <img src={p.video.thumbnailUrl} alt="" />
                {/if}
                <div class="pip-overlay">
                  <div class="pip-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" width="44" height="44">
                      <path d="M3 5h18v14H3V5zm2 2v10h14V7H5zm7 4h6v4h-6v-4z" fill="currentColor" />
                    </svg>
                  </div>
                  <div class="pip-text">ミニプレイヤーで再生中</div>
                  <button type="button" class="pip-resume" onclick={() => miniPlayer.close()}>
                    ここで再生に戻す
                  </button>
                </div>
              </div>
            </div>
          {:else if pipActiveForOther}
            <div class="pip-placeholder">
              <div class="pip-thumb">
                {#if p.video.thumbnailUrl}
                  <img src={p.video.thumbnailUrl} alt="" />
                {/if}
                <div class="pip-overlay">
                  <div class="pip-icon" aria-hidden="true">
                    <svg viewBox="0 0 24 24" width="44" height="44">
                      <path d="M3 5h18v14H3V5zm2 2v10h14V7H5zm7 4h6v4h-6v-4z" fill="currentColor" />
                    </svg>
                  </div>
                  <div class="pip-text">別の動画がミニプレイヤーで再生中</div>
                  <div class="pip-other-title">{pipOtherTitle}</div>
                  <div class="pip-actions">
                    <button
                      type="button"
                      class="pip-resume"
                      onclick={() => miniPlayer.close()}
                      title="PiP を閉じてこのページの動画を再生"
                    >
                      PiP を閉じてここで再生
                    </button>
                    <a
                      class="pip-link"
                      href={pipExpandHref}
                      title="ミニプレイヤーで再生中の動画ページへ"
                    >
                      PiP の動画を開く
                    </a>
                  </div>
                </div>
              </div>
            </div>
          {:else}
            <Player
              bind:this={playerRef}
              hlsUrl={p.hlsUrl}
              comments={visibleComments}
              videoTitle={p.video.title}
              videoId={p.video.id}
              short={p.isShort}
              refreshHlsUrl={() => issueHlsUrl(p.videoId)}
              onTime={handleTimeUpdate}
              onEnded={handleEnded}
              resumePosition={getResumePosition(p.videoId)}
              {loop}
              onLoopChange={(v) => {
                loop = v;
                loopUserSet = true;
              }}
              onTogglePip={togglePip}
              pipActive={false}
              forceAutoplay={shouldForceAutoplay(p.video.id)}
            />
          {/if}
          {#if ngFilteredCount > 0}
            <div class="ng-banner">NG: {ngFilteredCount} 件のコメを除外中</div>
          {/if}
        </div>
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <div
          class="divider"
          role="separator"
          aria-label="コメントパネル幅調整"
          onmousedown={startDrag}
        ></div>
        <div class="comment-panel" style:width="{panelWidth}px" style:min-width="{panelWidth}px">
          {#if isClassicTheme}
            <div class="side-header">
              <span
                >{commentsLoading
                  ? 'コメント読込中'
                  : `${formatNumber(visibleComments.length)} 件`}</span
              >
              <label class="side-toggle">
                <input type="checkbox" checked={loop} onchange={() => (loop = !loop)} />
                <span>連続再生</span>
              </label>
            </div>
          {/if}
          {#if commentsLoading}
            <div class="comment-loading">コメント取得中…</div>
          {/if}
          <CommentList comments={visibleComments} {currentTime} onSeek={handleSeek} />
        </div>
      </div>
    </div>

    <div class="below">
      <div class="meta">
        <div class="row">
          <span>{payload.video.id}</span>
          <span class="dot">·</span>
          <span>{formatDuration(payload.video.duration)}</span>
          {#if payload.video.registeredAt}
            <span class="dot">·</span><span>{formatDate(payload.video.registeredAt)}</span>
          {/if}
          {#if payload.pickedQuality.label}
            <span class="dot">·</span>
            <span class="quality">{payload.pickedQuality.label}</span>
          {/if}
          <span class="dot">·</span>
          <span>コメ {commentsLoading ? '…' : formatNumber(comments.length)}</span>
          <a
            class="external"
            href={videoUrl(payload.video.id)}
            target="_blank"
            rel="noreferrer noopener">ニコニコで開く ↗</a
          >
        </div>
        {#if payload.owner}
          <div class="owner-card">
            {#if payload.owner.iconUrl}
              <a
                href={payload.owner.id
                  ? `/user/${payload.owner.id}?kind=${payload.owner.kind}${payload.owner.nickname ? `&name=${encodeURIComponent(payload.owner.nickname)}` : ''}${payload.owner.iconUrl ? `&icon=${encodeURIComponent(payload.owner.iconUrl)}` : ''}`
                  : undefined}
                class="owner-icon-link"
              >
                <img class="owner-icon" src={payload.owner.iconUrl} alt="" loading="lazy" />
              </a>
            {:else}
              <div class="owner-icon placeholder">
                {payload.owner.kind === 'channel' ? 'CH' : 'U'}
              </div>
            {/if}
            <div class="owner-info">
              <div class="owner-name-row">
                {#if payload.owner.id}
                  <a
                    href={`/user/${payload.owner.id}?kind=${payload.owner.kind}${payload.owner.nickname ? `&name=${encodeURIComponent(payload.owner.nickname)}` : ''}${payload.owner.iconUrl ? `&icon=${encodeURIComponent(payload.owner.iconUrl)}` : ''}`}
                    class="owner-link"
                  >
                    <span class="owner-name">{payload.owner.nickname ?? '不明'}</span>
                  </a>
                {:else}
                  <span class="owner-name">{payload.owner.nickname ?? '不明'}</span>
                {/if}
                <span class="owner-kind-badge"
                  >{payload.owner.kind === 'channel' ? 'チャンネル' : 'ユーザー'}</span
                >
              </div>
              {#if payload.owner.id}
                <a
                  href={`/user/${payload.owner.id}?kind=${payload.owner.kind}${payload.owner.nickname ? `&name=${encodeURIComponent(payload.owner.nickname)}` : ''}${payload.owner.iconUrl ? `&icon=${encodeURIComponent(payload.owner.iconUrl)}` : ''}`}
                  class="owner-videos-link"
                >
                  投稿動画一覧を見る
                </a>
              {/if}
            </div>
          </div>
        {/if}
        {#if payload.series}
          <a class="series-card" href={`/series/${payload.series.id}`}>
            <div class="series-thumb-wrap">
              {#if payload.series.thumbnailUrl}
                <img class="series-thumb" src={payload.series.thumbnailUrl} alt="" loading="lazy" />
              {:else}
                <div class="series-thumb placeholder">
                  <svg viewBox="0 0 24 24" width="28" height="28">
                    <path
                      d="M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm0 14H8V4h12v12zm-8-2l6-4-6-4v8z"
                      fill="currentColor"
                    />
                  </svg>
                </div>
              {/if}
            </div>
            <div class="series-info">
              <div class="series-label">シリーズ</div>
              <div class="series-title">{payload.series.title}</div>
              {#if payload.series.description}
                <div class="series-desc">{payload.series.description}</div>
              {/if}
              {#if payload.series.itemsCount != null}
                <div class="series-count">{payload.series.itemsCount} 本の動画</div>
              {/if}
            </div>
            <span class="series-arrow" aria-hidden="true">›</span>
          </a>
        {/if}
        {#if payload.video.tags && payload.video.tags.length > 0}
          <div class="tags" aria-label="タグ">
            {#each payload.video.tags as tag (tag.name)}
              <a
                class="tag"
                class:locked={tag.isLocked}
                href={tagSearchHref(tag.name)}
                title="このタグで検索"
              >
                {#if tag.isLocked}<span class="lock" aria-hidden="true">🔒</span>{/if}
                {tag.name}
              </a>
            {/each}
          </div>
        {/if}
        {#if payload.video.description}
          <details open={expandDescription}>
            <summary>説明文</summary>
            <!-- niconico API の説明文は外部入力。`{@html}` 前に許可リストで
                 サニタイズして XSS（→ Tauri invoke 経由の任意ファイル削除など）
                 を遮断する。 -->
            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
            <p class="desc">{@html sanitizeDescriptionHtml(payload.video.description)}</p>
          </details>
        {/if}
      </div>

      <aside class="related">
        <h3>関連動画</h3>
        {#if relatedLoading}
          <div class="muted small">関連動画を取得中…</div>
        {:else if relatedError}
          <div class="muted small">取得失敗: {relatedError}</div>
        {:else if related.length === 0}
          <div class="muted small">関連動画は見つかりませんでした。</div>
        {:else}
          <ul class="related-list">
            {#each related.slice(0, relatedVisibleCount) as hit (hit.contentId)}
              <SearchHitCard {hit} compact />
            {/each}
          </ul>
        {/if}
      </aside>
    </div>
  {/if}
</section>

<style>
  .page {
    max-width: 1600px;
  }
  .page.classic {
    max-width: 1760px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .page.classic .head {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.65) inset;
    padding: 12px 14px;
  }
  .head h2 {
    margin: 0;
    font-size: 18px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .head-actions {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .dl-btn {
    background: var(--theme-success-bg);
    color: var(--theme-success-text);
    border: 1px solid var(--theme-success-border);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .dl-btn:hover:not(:disabled) {
    background: var(--theme-success-border);
    color: var(--theme-text);
  }
  .dl-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .page.classic .dl-btn,
  .page.classic .toolbar-btn,
  .page.classic :global(.head-actions .mylist-add-btn),
  .page.classic :global(.head-actions .mylist-create-btn) {
    background: linear-gradient(180deg, #ffffff 0%, #ebe2d4 100%);
    border: 1px solid var(--theme-border-strong);
    color: var(--theme-text);
    border-radius: 3px;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.85) inset;
  }
  .page.classic .dl-btn:hover:not(:disabled),
  .page.classic .toolbar-btn:hover:not(:disabled) {
    background: linear-gradient(180deg, #fff8ef 0%, #e6d6c2 100%);
    color: var(--theme-text);
  }
  .dl-msg {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .dl-msg.ok {
    background: var(--theme-success-bg-2);
    border: 1px solid var(--theme-success-border);
    color: var(--theme-success-text);
  }
  .dl-msg.error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
  }
  .back {
    color: var(--theme-accent-soft);
    text-decoration: none;
    font-size: 13px;
    flex-shrink: 0;
  }
  .back:hover {
    text-decoration: underline;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .small {
    font-size: 12px;
  }
  .error {
    background: var(--theme-danger-bg);
    border: 1px solid var(--theme-danger-border);
    color: var(--theme-danger-text);
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
    white-space: pre-wrap;
  }
  .player-row {
    display: flex;
    align-items: stretch;
  }
  .player-frame {
    display: flex;
    flex-direction: column;
    gap: 0;
  }
  .page.classic .player-frame {
    border: 1px solid var(--theme-border);
    background: var(--theme-surface-2);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.75) inset;
  }
  .viewer-toolbar {
    display: none;
  }
  .page.classic .viewer-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 10px 12px;
    background: linear-gradient(180deg, #fffdf9 0%, #efe6d9 100%);
    border-bottom: 1px solid var(--theme-border);
    color: var(--theme-text-soft);
    font-size: 13px;
  }
  .viewer-toolbar-meta,
  .viewer-toolbar-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .toolbar-id {
    font-weight: 700;
  }
  .toolbar-sep {
    color: var(--theme-text-faint);
  }
  .toolbar-btn {
    padding: 7px 12px;
    cursor: pointer;
    font-size: 12px;
  }
  .player-row.dragging {
    user-select: none;
    cursor: col-resize;
  }
  .player-col {
    flex: 1 1 auto;
    min-width: 0;
    contain: layout style paint;
  }
  .divider {
    width: 5px;
    cursor: col-resize;
    background: var(--theme-surface-3);
    border-left: 1px solid var(--theme-border-strong);
    border-right: 1px solid var(--theme-border-strong);
    flex-shrink: 0;
    transition: background 0.1s;
  }
  .divider:hover {
    background: var(--theme-surface-hover);
  }
  .dragging .divider {
    background: var(--theme-accent);
  }
  .page.classic .divider {
    width: 7px;
    background: linear-gradient(180deg, #f4efe6 0%, #e3d9ca 100%);
    border-left: 1px solid var(--theme-border);
    border-right: 1px solid var(--theme-border);
  }
  .comment-panel {
    flex-shrink: 0;
    overflow: hidden;
    position: relative;
  }
  .page.classic .comment-panel {
    background: var(--theme-surface-2);
    border-left: 1px solid var(--theme-border);
  }
  .side-header {
    display: none;
  }
  .page.classic .side-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--theme-border);
    background: linear-gradient(180deg, #fffdf9 0%, #f2e8db 100%);
    color: var(--theme-text);
    font-size: 13px;
    font-weight: 700;
  }
  .side-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-weight: 400;
    font-size: 12px;
    color: var(--theme-text-soft);
  }
  .comment-loading {
    position: absolute;
    top: 42px;
    left: 12px;
    z-index: 1;
    color: var(--theme-text-muted);
    font-size: 12px;
  }
  .below {
    display: grid;
    grid-template-columns: 1fr 360px;
    gap: 16px;
    margin-top: 12px;
    contain: layout style;
  }
  .page.classic .below {
    grid-template-columns: minmax(0, 1fr) 360px;
    gap: 12px;
  }
  @media (max-width: 1100px) {
    .below {
      grid-template-columns: 1fr;
    }
  }
  .meta {
    color: var(--theme-text-soft);
    font-size: 13px;
    min-width: 0;
    overflow: hidden;
  }
  .page.classic .meta,
  .page.classic .related {
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.75) inset;
    padding: 12px 14px;
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .dot {
    color: var(--theme-text-faint);
  }
  .page.classic .row:first-child {
    margin-top: 0;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--theme-border);
  }
  .quality {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    border-radius: 999px;
    padding: 0 8px;
    font-size: 11px;
  }
  .page.classic .quality {
    border-radius: 3px;
    border: 1px solid var(--theme-accent-border);
    background: linear-gradient(180deg, #fff8ef 0%, #f1e1d2 100%);
    color: var(--theme-accent);
  }
  .external {
    margin-left: auto;
    color: var(--theme-accent-soft);
    text-decoration: none;
  }
  .external:hover {
    text-decoration: underline;
  }
  .owner-card {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 10px;
    padding: 10px 12px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    border-radius: 8px;
  }
  .page.classic .owner-card {
    border-radius: 3px;
    background: linear-gradient(180deg, #fffdf9 0%, #f7f1e8 100%);
  }
  .owner-icon {
    width: 40px;
    height: 40px;
    border-radius: 999px;
    object-fit: cover;
    background: var(--theme-surface-3);
    flex-shrink: 0;
  }
  .owner-icon.placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--theme-text-faint);
    font-weight: 600;
    font-size: 14px;
    border: 1px solid var(--theme-border-strong);
  }
  .owner-icon-link {
    flex-shrink: 0;
    line-height: 0;
  }
  .owner-info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .owner-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .owner-name {
    font-weight: 600;
    font-size: 14px;
    color: var(--theme-text);
  }
  .owner-link {
    color: var(--theme-text);
    text-decoration: none;
  }
  .owner-link:hover {
    text-decoration: underline;
  }
  .owner-kind-badge {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    flex-shrink: 0;
  }
  .page.classic .owner-kind-badge {
    border-radius: 3px;
    border: 1px solid var(--theme-accent-border);
  }
  .owner-videos-link {
    color: var(--theme-accent-soft);
    text-decoration: none;
    font-size: 12px;
  }
  .owner-videos-link:hover {
    text-decoration: underline;
  }
  .series-card {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 10px;
    padding: 10px 12px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-accent-border);
    border-radius: 8px;
    text-decoration: none;
    color: inherit;
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .page.classic .series-card {
    border-radius: 3px;
    background: linear-gradient(180deg, #fffdfa 0%, #f5ede2 100%);
  }
  .series-card:hover {
    background: var(--theme-accent-bg);
    border-color: var(--theme-accent-border);
  }
  .series-thumb-wrap {
    flex-shrink: 0;
    line-height: 0;
  }
  .series-thumb {
    width: 64px;
    height: 36px;
    object-fit: cover;
    border-radius: 4px;
    background: var(--theme-bg);
  }
  .series-thumb.placeholder {
    width: 64px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--theme-accent-bg);
    border: 1px dashed var(--theme-accent-border);
    border-radius: 4px;
    color: var(--theme-accent-soft);
  }
  .series-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .series-label {
    font-size: 10px;
    color: var(--theme-accent-soft);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .series-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--theme-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .series-desc {
    font-size: 11px;
    color: var(--theme-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .series-count {
    font-size: 11px;
    color: var(--theme-text-muted);
  }
  .series-arrow {
    flex-shrink: 0;
    font-size: 20px;
    color: var(--theme-text-faint);
    margin-left: 4px;
  }
  .series-card:hover .series-arrow {
    color: var(--theme-accent-soft);
  }
  details {
    margin-top: 12px;
    color: var(--theme-text-soft);
  }
  details > summary {
    cursor: pointer;
    color: var(--theme-text-soft);
    margin-bottom: 6px;
  }
  .desc {
    white-space: pre-wrap;
    line-height: 1.6;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    padding: 10px 12px;
    border-radius: 6px;
    overflow: hidden;
    min-width: 0;
    word-break: break-word;
  }
  .page.classic .desc {
    border-radius: 3px;
    background: #fffcf7;
  }
  .ng-banner {
    background: var(--theme-danger-bg-2);
    color: var(--theme-danger-text);
    border: 1px solid var(--theme-danger-border);
    padding: 4px 10px;
    border-radius: 6px;
    font-size: 12px;
    margin-top: 6px;
    display: inline-block;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 8px;
    min-width: 0;
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    background: var(--theme-border);
    color: var(--theme-chip-text);
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    text-decoration: none;
    border: 1px solid transparent;
  }
  .page.classic .tag {
    border-radius: 3px;
    background: linear-gradient(180deg, #f6efe4 0%, #e7dccb 100%);
    border-color: var(--theme-border);
    color: var(--theme-text-soft);
  }
  .tag:hover {
    background: var(--theme-border-strong);
    color: var(--theme-text);
    border-color: var(--theme-border-focus);
  }
  .tag.locked {
    background: var(--theme-accent-bg);
    color: var(--theme-accent-soft);
  }
  .tag.locked:hover {
    background: var(--theme-accent-border);
    color: var(--theme-text);
  }
  .lock {
    font-size: 9px;
    opacity: 0.7;
  }
  .related h3 {
    margin: 0 0 8px;
    font-size: 14px;
    color: var(--theme-text-soft);
  }
  .page.classic .related h3 {
    margin-bottom: 10px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--theme-border);
    color: var(--theme-text);
  }
  .related-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* 
   * 全画面表示時に裏側でレイアウト計算（特に関連動画の遅延読み込み画像など）
   * が走って動画がガクつくのを防ぐため、プレーヤー以外を非表示にする
   */
  .pip-placeholder {
    background: var(--theme-bg);
    border-radius: 8px;
    overflow: hidden;
    aspect-ratio: 16 / 9;
    width: 100%;
    position: relative;
  }
  .pip-thumb {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .pip-thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    filter: brightness(0.45) blur(4px);
  }
  .pip-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--theme-on-overlay);
  }
  .pip-icon {
    color: var(--theme-on-overlay);
    opacity: 0.85;
  }
  .pip-text {
    font-size: 14px;
    font-weight: 600;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6);
  }
  .pip-resume {
    margin-top: 4px;
    background: var(--theme-accent);
    color: var(--theme-accent-fg);
    border: none;
    padding: 8px 16px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .pip-resume:hover {
    background: var(--theme-accent-hover);
  }
  .pip-other-title {
    font-size: 12px;
    color: var(--theme-on-overlay);
    opacity: 0.85;
    max-width: 80%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: center;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.6);
  }
  .pip-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin-top: 4px;
  }
  .pip-link {
    color: var(--theme-on-overlay);
    opacity: 0.9;
    font-size: 12px;
    text-decoration: underline;
    padding: 4px 8px;
  }
  .pip-link:hover {
    opacity: 1;
  }

  .loading-skeleton {
    background: var(--theme-bg);
    border-radius: 8px;
    overflow: hidden;
    aspect-ratio: 16 / 9;
    width: 100%;
    position: relative;
  }
  .skeleton-thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    filter: brightness(0.3) blur(2px);
  }
  .skeleton-overlay {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
  }
  .skeleton-spinner {
    width: 36px;
    height: 36px;
    border: 3px solid rgba(255, 255, 255, 0.2);
    border-top-color: #fff;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .skeleton-text {
    color: var(--theme-text);
    font-size: 13px;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6);
  }

  :global(body:has(:fullscreen)) .head,
  :global(body:has(:fullscreen)) .divider,
  :global(body:has(:fullscreen)) .comment-panel,
  :global(body:has(:fullscreen)) .below {
    display: none !important;
  }
</style>
