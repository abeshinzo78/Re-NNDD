<script lang="ts">
  import { onDestroy } from 'svelte';
  import { beforeNavigate } from '$app/navigation';
  import { page } from '$app/state';
  import Player from '$lib/player/Player.svelte';
  import CommentList from '$lib/player/CommentList.svelte';
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
  import { getBool, loadSettings } from '$lib/stores/settings.svelte';
  import { sanitizeDescriptionHtml } from '$lib/sanitize';
  import { miniPlayer } from '$lib/player/miniPlayerStore.svelte';

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

  type PlayerRef = { seek: (t: number) => void; getVideo: () => HTMLVideoElement | null };
  let playerRef = $state<PlayerRef | undefined>();

  let videoId = $derived(page.params.id ?? '');
  let loadingFor: string | null = null;
  let loop = $state(false);

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
      loop = getBool('playback.always_loop');
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

  function handleSeek(t: number) {
    playerRef?.seek(t);
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
    const vid = playerRef?.getVideo();
    const t = vid?.currentTime ?? currentTime ?? 0;
    // 起動時点でページ側 Player が鳴っているかを掴んでおく。
    // 鳴っている場合のみ、mini は無音ロード→引き継ぎフローを走らせる。
    const wasPlaying = vid != null && !vid.paused && !vid.ended;
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

  // PiP 中はミニ側で取得済みコメの方が新しい可能性があるので、ミニ側にも反映
  $effect(() => {
    if (pipActiveForThis && payload) {
      miniPlayer.updateComments(payload.videoId, visibleComments);
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
      if (v) miniPlayer.setSourcePaused(v.paused || v.ended);
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

<section class="page">
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
        {:else}
          <Player
            bind:this={playerRef}
            hlsUrl={p.hlsUrl}
            comments={visibleComments}
            videoTitle={p.video.title}
            videoId={p.video.id}
            refreshHlsUrl={() => issueHlsUrl(p.videoId)}
            onTime={handleTimeUpdate}
            resumePosition={getResumePosition(p.videoId)}
            {loop}
            onLoopChange={(v) => (loop = v)}
            onTogglePip={togglePip}
            pipActive={false}
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
        {#if commentsLoading}
          <div class="comment-loading">コメント取得中…</div>
        {/if}
        <CommentList comments={visibleComments} {currentTime} onSeek={handleSeek} />
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
          <details>
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
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 12px;
    flex-wrap: wrap;
  }
  .head h2 {
    margin: 0;
    font-size: 18px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-heading);
  }
  .head-actions {
    flex-shrink: 0;
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .dl-btn {
    background: var(--success-icon-bg);
    color: var(--success-icon-text);
    border: 1px solid var(--success-icon-border);
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 13px;
  }
  .dl-btn:hover:not(:disabled) {
    background: var(--success-icon-border);
    color: var(--text-heading);
  }
  .dl-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .dl-msg {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12px;
    margin-bottom: 8px;
  }
  .dl-msg.ok {
    background: var(--success-bg);
    border: 1px solid var(--success-border);
    color: var(--success-text);
  }
  .dl-msg.error {
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    color: var(--error-text);
  }
  .back {
    color: var(--link);
    text-decoration: none;
    font-size: 13px;
    flex-shrink: 0;
  }
  .back:hover {
    text-decoration: underline;
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: 12px;
  }
  .error {
    background: var(--error-bg);
    border: 1px solid var(--error-border);
    color: var(--error-text);
    padding: 10px 12px;
    border-radius: 6px;
    font-size: 13px;
    white-space: pre-wrap;
  }
  .player-row {
    display: flex;
    align-items: stretch;
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
    background: var(--surface-3);
    border-left: 1px solid var(--border-2);
    border-right: 1px solid var(--border-2);
    flex-shrink: 0;
    transition: background 0.1s;
  }
  .divider:hover {
    background: var(--menu-border);
  }
  .dragging .divider {
    background: var(--accent);
  }
  .comment-panel {
    flex-shrink: 0;
    overflow: hidden;
    position: relative;
  }
  .comment-loading {
    position: absolute;
    top: 42px;
    left: 12px;
    z-index: 1;
    color: var(--text-muted);
    font-size: 12px;
  }
  .below {
    display: grid;
    grid-template-columns: 1fr 360px;
    gap: 16px;
    margin-top: 12px;
    contain: layout style;
  }
  @media (max-width: 1100px) {
    .below {
      grid-template-columns: 1fr;
    }
  }
  .meta {
    color: var(--text-2);
    font-size: 13px;
    min-width: 0;
    overflow: hidden;
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .dot {
    color: var(--text-faint);
  }
  .quality {
    background: var(--badge-blue-bg);
    color: var(--badge-blue-text);
    border-radius: 999px;
    padding: 0 8px;
    font-size: 11px;
  }
  .external {
    margin-left: auto;
    color: var(--link);
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
    background: var(--owner-card-bg);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .owner-icon {
    width: 40px;
    height: 40px;
    border-radius: 999px;
    object-fit: cover;
    background: var(--surface-3);
    flex-shrink: 0;
  }
  .owner-icon.placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-dim);
    font-weight: 600;
    font-size: 14px;
    border: 1px solid var(--border-2);
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
    color: var(--text);
  }
  .owner-link {
    color: var(--text);
    text-decoration: none;
  }
  .owner-link:hover {
    text-decoration: underline;
  }
  .owner-kind-badge {
    background: var(--badge-blue-bg);
    color: var(--badge-blue-text);
    padding: 1px 8px;
    border-radius: 999px;
    font-size: 10px;
    flex-shrink: 0;
  }
  .owner-videos-link {
    color: var(--link);
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
    background: var(--owner-card-bg);
    border: 1px solid var(--badge-blue-border);
    border-radius: 8px;
    text-decoration: none;
    color: inherit;
    transition:
      background 0.15s,
      border-color 0.15s;
  }
  .series-card:hover {
    background: var(--badge-blue-bg-soft);
    border-color: var(--accent-soft-border);
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
    background: var(--code-bg);
  }
  .series-thumb.placeholder {
    width: 64px;
    height: 36px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--badge-blue-bg-soft);
    border: 1px dashed var(--badge-blue-border);
    border-radius: 4px;
    color: var(--link);
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
    color: var(--link);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    font-weight: 600;
  }
  .series-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .series-desc {
    font-size: 11px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .series-count {
    font-size: 11px;
    color: var(--text-dim);
  }
  .series-arrow {
    flex-shrink: 0;
    font-size: 20px;
    color: var(--text-faint);
    margin-left: 4px;
  }
  .series-card:hover .series-arrow {
    color: var(--link);
  }
  details {
    margin-top: 12px;
    color: var(--text-2);
  }
  details > summary {
    cursor: pointer;
    color: var(--text-3);
    margin-bottom: 6px;
  }
  .desc {
    white-space: pre-wrap;
    line-height: 1.6;
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 10px 12px;
    border-radius: 6px;
    overflow: hidden;
    min-width: 0;
    word-break: break-word;
  }
  .ng-banner {
    background: var(--error-bg);
    color: var(--error-text);
    border: 1px solid var(--error-border);
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
    background: var(--tag-bg);
    color: var(--tag-text);
    padding: 3px 10px;
    border-radius: 999px;
    font-size: 12px;
    text-decoration: none;
    border: 1px solid transparent;
  }
  .tag:hover {
    background: var(--tag-hover-bg);
    color: var(--tag-hover-text);
    border-color: var(--tag-hover-border);
  }
  .tag.locked {
    background: var(--tag-locked-bg);
    color: var(--tag-locked-text);
  }
  .tag.locked:hover {
    background: var(--tag-locked-hover-bg);
    color: var(--tag-hover-text);
  }
  .lock {
    font-size: 9px;
    opacity: 0.7;
  }
  .related h3 {
    margin: 0 0 8px;
    font-size: 14px;
    color: var(--text-2);
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
    background: #000;
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
    color: #fff;
  }
  .pip-icon {
    color: #fff;
    opacity: 0.85;
  }
  .pip-text {
    font-size: 14px;
    font-weight: 600;
    text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6);
  }
  .pip-resume {
    margin-top: 4px;
    background: #2563eb;
    color: #fff;
    border: none;
    padding: 8px 16px;
    border-radius: 8px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .pip-resume:hover {
    background: #3b78f0;
  }

  .loading-skeleton {
    background: #000;
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
    color: #ddd;
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
