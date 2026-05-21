<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { formatDuration } from '$lib/format';
  import type { Level } from 'hls.js';

  type Props = {
    video: HTMLVideoElement | null;
    paused: boolean;
    currentTime: number;
    duration: number;
    volume: number;
    muted: boolean;
    playbackRate: number;
    commentsEnabled: boolean;
    commentOpacity: number;
    abLoop: { in: number | null; out: number | null; enabled: boolean };
    hlsLevels: Level[];
    currentLevel: number;
    loop: boolean;
    onTogglePlay: () => void;
    onSeek: (t: number) => void;
    onVolume: (v: number) => void;
    onToggleMute: () => void;
    onRate: (r: number) => void;
    onToggleComments: () => void;
    onCommentOpacity: (o: number) => void;
    onSetAbIn: () => void;
    onSetAbOut: () => void;
    onToggleAb: () => void;
    onClearAb: () => void;
    onScreenshot?: () => void;
    onFullscreen: () => void;
    onToggleLoop: () => void;
    onQuality: (levelIndex: number) => void;
    onTogglePip?: () => void;
    /** PiP ボタンを表示するか */
    showPip?: boolean;
    /** PiP が現在 ON か (ボタンの active 表示) */
    pipActive?: boolean;
  };

  let {
    video,
    paused,
    currentTime,
    duration,
    volume,
    muted,
    playbackRate,
    commentsEnabled,
    commentOpacity,
    abLoop,
    hlsLevels,
    currentLevel,
    loop,
    onTogglePlay,
    onSeek,
    onVolume,
    onToggleMute,
    onRate,
    onToggleComments,
    onCommentOpacity,
    onSetAbIn,
    onSetAbOut,
    onToggleAb,
    onClearAb,
    onScreenshot,
    onFullscreen,
    onToggleLoop,
    onQuality,
    onTogglePip,
    showPip = false,
    pipActive = false,
  }: Props = $props();

  const speeds = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

  // 速度ピッカーは <select> だと全画面プレイヤーの下端で popup が画面外
  // (= 隠れる) になるので、自前で「上方向に開く」ポップアップにする。
  let rateOpen = $state(false);
  function toggleRate() {
    rateOpen = !rateOpen;
  }
  function pickRate(s: number) {
    onRate(s);
    rateOpen = false;
  }
  function rateLabel(r: number): string {
    return r.toFixed(2).replace(/\.?0+$/, '') + 'x';
  }
  function onDocClickRate(e: MouseEvent) {
    if (!rateOpen) return;
    const t = e.target as HTMLElement;
    if (t.closest('.rate-picker')) return;
    rateOpen = false;
  }
  onMount(() => document.addEventListener('mousedown', onDocClickRate));
  onDestroy(() => document.removeEventListener('mousedown', onDocClickRate));

  // スライダーのドラッグ中は input イベントが連発する。後方シークだと
  // decoder が GOP リセットを連射されてガビガビになるので、間引いて投げる。
  // mouseup (change) の最終値は throttle 無視で必ず適用する。
  let lastSeekAt = 0;
  const SEEK_THROTTLE_MS = 120;
  function handleSeekBar(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const now = performance.now();
    if (now - lastSeekAt < SEEK_THROTTLE_MS) return;
    lastSeekAt = now;
    onSeek(Number(input.value));
  }
  function handleSeekCommit(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    lastSeekAt = performance.now();
    onSeek(Number(input.value));
  }

  function handleVolumeBar(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    onVolume(Number(input.value));
  }

  function qualityLabel(level: Level): string {
    if (level.height) return `${level.height}p`;
    if (level.name) return level.name;
    return `${level.bitrate}bps`;
  }

  // Deduplicate levels by height — niconico often has multiple
  // tracks at the same resolution with different audio codecs.
  let uniqueLevels = $derived.by(() => {
    const seen = new SvelteSet<number>();
    const result: { index: number; level: Level }[] = [];
    for (let i = 0; i < hlsLevels.length; i++) {
      const h = hlsLevels[i].height ?? 0;
      if (!seen.has(h)) {
        seen.add(h);
        result.push({ index: i, level: hlsLevels[i] });
      }
    }
    return result;
  });

  // Map the raw currentLevel (which can be a duplicate height) to the
  // corresponding unique-level index so the <select> always highlights
  // the right option.
  let displayLevel = $derived.by(() => {
    if (currentLevel < 0 || hlsLevels.length === 0) return -1;
    const curHeight = hlsLevels[currentLevel]?.height;
    if (curHeight == null) return -1;
    const match = uniqueLevels.find((u) => u.level.height === curHeight);
    return match ? match.index : -1;
  });

  // 再生中、左側の時間表示 (currentTime) は秒ごとに桁が変わる。
  // `.seek` は `max-content 1fr max-content` の grid なので、文字列の
  // 自然幅が変わると col1 幅が変動し、その分 1fr のシークバーが左右に
  // ガタつく。
  // ・桁ごとの幅は親 `.seek` の `tabular-nums` で揃うはずだが、Linux/
  //   WebKitGTK の system fallback (DejaVu Sans 等) は tabular feature
  //   を持たないフォントが多く、実環境では完全には揃わない。
  // ・また、9:59 → 10:00 のように文字数が増える場面では tabular でも
  //   col1 幅が変動する。
  // 両方を一括で潰すため、duration の文字数ぶんを ch 単位で min-width
  // として両側 `.time` に予約する (currentTime ≤ duration なので、これ
  // で col1 幅が再生中に縮みも伸びもしなくなる)。
  let timeMinCh = $derived(formatDuration(duration).length);
</script>

<div class="bar">
  <div class="seek">
    <span class="time" style:min-width="{timeMinCh}ch">{formatDuration(currentTime)}</span>
    <div class="seek-track">
      <input
        type="range"
        min="0"
        max={duration || 0.001}
        step="0.1"
        value={currentTime}
        oninput={handleSeekBar}
        onchange={handleSeekCommit}
        aria-label="シーク"
        disabled={!video || !duration}
      />
      {#if abLoop.in != null}
        <span
          class="ab-marker in"
          style:left="{((abLoop.in / (duration || 1)) * 100).toFixed(2)}%"
          title="A 点"
        ></span>
      {/if}
      {#if abLoop.out != null}
        <span
          class="ab-marker out"
          style:left="{((abLoop.out / (duration || 1)) * 100).toFixed(2)}%"
          title="B 点"
        ></span>
      {/if}
    </div>
    <span class="time" style:min-width="{timeMinCh}ch">{formatDuration(duration)}</span>
  </div>

  <div class="controls">
    <button type="button" class="btn primary" onclick={onTogglePlay} aria-label="再生/一時停止">
      <span class="btn-icon">{paused ? '▶' : '❚❚'}</span>
      <span class="classic-label">{paused ? '再生' : '一時停止'}</span>
    </button>
    <div class="volume">
      <button type="button" class="btn" onclick={onToggleMute} aria-label="ミュート">
        <span class="btn-icon">{muted || volume === 0 ? '🔇' : volume < 0.5 ? '🔉' : '🔊'}</span>
        <span class="classic-label">音量</span>
      </button>
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={muted ? 0 : volume}
        oninput={handleVolumeBar}
        aria-label="音量"
      />
    </div>

    <div class="rate-picker">
      <span class="rate-label">速度</span>
      <button
        type="button"
        class="rate-btn"
        aria-haspopup="listbox"
        aria-expanded={rateOpen}
        onclick={toggleRate}>{rateLabel(playbackRate)} ▾</button
      >
      {#if rateOpen}
        <div class="rate-menu" role="listbox">
          {#each speeds as s (s)}
            <button
              type="button"
              role="option"
              aria-selected={s === playbackRate}
              class:current={s === playbackRate}
              onclick={() => pickRate(s)}>{rateLabel(s)}</button
            >
          {/each}
        </div>
      {/if}
    </div>

    {#if uniqueLevels.length > 1}
      <label class="select">
        画質
        <select
          value={displayLevel}
          onchange={(e) => onQuality(Number((e.currentTarget as HTMLSelectElement).value))}
        >
          {#each uniqueLevels as { index, level } (index)}
            <option value={index}>{qualityLabel(level)}</option>
          {/each}
        </select>
      </label>
    {/if}

    <div class="ab" role="group" aria-label="A-B リピート">
      <button type="button" class="btn small" onclick={onSetAbIn} title="A 点 (I)">A</button>
      <button type="button" class="btn small" onclick={onSetAbOut} title="B 点 (O)">B</button>
      <button
        type="button"
        class="btn small"
        class:active={abLoop.enabled}
        onclick={onToggleAb}
        title="ループ ON/OFF (L)"
        disabled={abLoop.in == null || abLoop.out == null}>↻</button
      >
      <button type="button" class="btn small" onclick={onClearAb} title="クリア">×</button>
      <span class="ab-classic-label">A B S</span>
    </div>

    <div class="comments-controls">
      <button
        type="button"
        class="btn"
        class:active={commentsEnabled}
        onclick={onToggleComments}
        title="コメ表示 (C)">コメ {commentsEnabled ? 'ON' : 'OFF'}</button
      >
      <input
        type="range"
        min="0.1"
        max="1"
        step="0.05"
        value={commentOpacity}
        oninput={(e) => onCommentOpacity(Number((e.currentTarget as HTMLInputElement).value))}
        title="コメ透明度"
      />
    </div>

    {#if showPip}
      <button
        type="button"
        class="btn pip-btn"
        class:active={pipActive}
        onclick={() => onTogglePip?.()}
        title="ミニプレイヤー (P)"
        aria-label="ミニプレイヤー"
        aria-pressed={pipActive}
      >
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
          <path d="M3 5h18v14H3V5zm2 2v10h14V7H5zm7 4h6v4h-6v-4z" fill="currentColor" />
        </svg>
        <span class="classic-label">ミニプレイヤー</span>
      </button>
    {/if}
    <button type="button" class="btn" onclick={() => onScreenshot?.()} title="スクリーンショット">
      <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
        <path
          d="M9 2L7.17 4H4c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V6c0-1.1-.9-2-2-2h-3.17L15 2H9zm3 15c-2.76 0-5-2.24-5-5s2.24-5 5-5 5 2.24 5 5-2.24 5-5 5z"
          fill="currentColor"
        />
      </svg>
      <span class="classic-label">静止画</span>
    </button>
    <button type="button" class="btn" onclick={onFullscreen} title="全画面 (F)">
      <span class="btn-icon">⛶</span>
      <span class="classic-label">全画面</span>
    </button>
    <button
      type="button"
      class="btn loop-btn"
      class:active={loop}
      onclick={onToggleLoop}
      title="リピート再生"
    >
      ループ
    </button>
  </div>
</div>

<style>
  .bar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: linear-gradient(0deg, rgba(0, 0, 0, 0.8) 0%, rgba(0, 0, 0, 0) 100%);
    padding: 24px 12px 12px;
    position: relative;
    z-index: 10;
    flex-shrink: 0;
  }
  .classic-label,
  .ab-classic-label {
    display: none;
  }
  .seek {
    display: grid;
    grid-template-columns: max-content 1fr max-content;
    align-items: center;
    gap: 8px;
    color: var(--theme-text);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .seek-track {
    position: relative;
  }
  .seek-track input[type='range'] {
    width: 100%;
  }
  .ab-marker {
    position: absolute;
    top: 50%;
    width: 4px;
    height: 14px;
    transform: translate(-50%, -50%);
    border-radius: 2px;
    pointer-events: none;
  }
  .ab-marker.in {
    background: var(--theme-success-strong);
  }
  .ab-marker.out {
    background: var(--theme-warning-text);
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    color: var(--theme-text);
    font-size: 13px;
  }
  .btn {
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: var(--theme-text);
    padding: 4px 10px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 14px;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .btn.small {
    padding: 2px 8px;
    font-size: 12px;
  }
  .btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.16);
  }
  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .btn.primary {
    background: var(--theme-accent);
    border-color: var(--theme-accent);
  }
  .btn.active {
    background: var(--theme-accent);
    border-color: var(--theme-accent);
  }
  :global(html[data-theme='niconico-classic']) .btn {
    background: linear-gradient(180deg, #fffdf8 0%, #ebe2d4 100%);
    border: 1px solid #c4b39c;
    color: #251d17;
    border-radius: 3px;
    box-shadow: 0 1px 0 rgba(255, 255, 255, 0.85) inset;
    padding: 5px 10px;
    min-height: 30px;
  }
  :global(html[data-theme='niconico-classic']) .btn:hover:not(:disabled) {
    background: linear-gradient(180deg, #fff8ef 0%, #e6d6c2 100%);
  }
  :global(html[data-theme='niconico-classic']) .btn.primary,
  :global(html[data-theme='niconico-classic']) .btn.active {
    background: linear-gradient(180deg, #edf4ff 0%, #dbe7fb 100%);
    border-color: #b3c6e2;
    color: #2a4d78;
  }
  .loop-btn {
    font-size: 12px;
    padding: 4px 8px;
    min-width: 48px;
  }
  .volume {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  :global(html[data-theme='niconico-classic']) .volume {
    padding-right: 8px;
    border-right: 1px solid #d8ccba;
  }
  .volume input[type='range'] {
    width: 80px;
  }
  .select {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--theme-text-soft);
  }
  .select select {
    background: var(--theme-surface-3);
    color: var(--theme-text);
    border: 1px solid var(--theme-border-strong);
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 12px;
  }
  :global(html[data-theme='niconico-classic']) .select select {
    background: linear-gradient(180deg, #fffdf8 0%, #ebe2d4 100%);
    color: #251d17;
    border-color: #c4b39c;
    border-radius: 3px;
  }
  .select select option {
    /* popup だけ光らせて選択肢が読めるように */
    background: var(--theme-surface-2);
    color: var(--theme-text);
  }
  /* 速度ピッカー (上方向に開くカスタムドロップダウン) */
  .rate-picker {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--theme-text-soft);
  }
  :global(html[data-theme='niconico-classic']) .rate-picker,
  :global(html[data-theme='niconico-classic']) .select,
  :global(html[data-theme='niconico-classic']) .comments-controls {
    padding-right: 8px;
    border-right: 1px solid #d8ccba;
  }
  .rate-label {
    font-size: 12px;
  }
  .rate-btn {
    background: var(--theme-surface-3);
    color: var(--theme-text);
    border: 1px solid var(--theme-border-strong);
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 12px;
    cursor: pointer;
    min-width: 56px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .rate-btn:hover {
    background: var(--theme-border-strong);
  }
  :global(html[data-theme='niconico-classic']) .rate-btn {
    background: linear-gradient(180deg, #fffdf8 0%, #ebe2d4 100%);
    color: #251d17;
    border-color: #c4b39c;
    border-radius: 3px;
  }
  :global(html[data-theme='niconico-classic']) .rate-btn:hover {
    background: linear-gradient(180deg, #fff8ef 0%, #e6d6c2 100%);
  }
  .rate-menu {
    position: absolute;
    bottom: calc(100% + 4px); /* 上に開く - 全画面の下端でも切れない */
    right: 0;
    background: var(--theme-surface-3);
    border: 1px solid var(--theme-border-strong);
    border-radius: 6px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 -4px 12px rgba(0, 0, 0, 0.6);
    z-index: 30;
    min-width: 70px;
  }
  :global(html[data-theme='niconico-classic']) .rate-menu {
    background: #fffdf8;
    border-color: #c4b39c;
    box-shadow: 0 -4px 12px rgba(75, 55, 34, 0.2);
  }
  .rate-menu button {
    background: transparent;
    border: none;
    color: var(--theme-text);
    padding: 4px 10px;
    border-radius: 3px;
    font-size: 12px;
    cursor: pointer;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .rate-menu button:hover {
    background: var(--theme-border-strong);
  }
  .rate-menu button.current {
    background: var(--theme-accent);
    color: white;
  }
  :global(html[data-theme='niconico-classic']) .rate-menu button.current {
    background: linear-gradient(180deg, #edf4ff 0%, #dbe7fb 100%);
    color: #2a4d78;
  }
  .ab {
    display: inline-flex;
    gap: 2px;
    padding: 0 6px;
    border-left: 1px solid var(--theme-border-strong);
    border-right: 1px solid var(--theme-border-strong);
    margin: 0 4px;
  }
  :global(html[data-theme='niconico-classic']) .ab {
    gap: 4px;
    align-items: center;
    padding: 0 8px 0 0;
    border-left: none;
    border-right: 1px solid #d8ccba;
    margin: 0;
  }
  .comments-controls {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .comments-controls input[type='range'] {
    width: 80px;
  }
  .pip-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 4px 8px;
  }
  .pip-btn svg {
    display: block;
  }
  :global(html[data-theme='niconico-classic']) .bar {
    gap: 10px;
    background: linear-gradient(
      180deg,
      rgba(255, 255, 255, 0.96) 0%,
      rgba(234, 225, 211, 0.98) 100%
    );
    padding: 10px 12px 12px;
    border-top: 1px solid #c4b39c;
    color: #251d17;
  }
  :global(html[data-theme='niconico-classic']) .seek {
    color: #4a4038;
    gap: 10px;
  }
  :global(html[data-theme='niconico-classic']) .seek-track input[type='range'] {
    accent-color: #3f73b3;
  }
  :global(html[data-theme='niconico-classic']) .controls {
    gap: 8px 10px;
    align-items: center;
    color: #4a4038;
    font-size: 12px;
  }
  :global(html[data-theme='niconico-classic']) .btn-icon {
    font-size: 13px;
    line-height: 1;
  }
  :global(html[data-theme='niconico-classic']) .classic-label {
    display: inline;
    font-size: 12px;
    line-height: 1;
  }
  :global(html[data-theme='niconico-classic']) .ab-classic-label {
    display: inline;
    margin-left: 2px;
    font-size: 12px;
    color: #4a4038;
    letter-spacing: 0.08em;
  }
  :global(html[data-theme='niconico-classic']) .btn.small {
    min-width: 28px;
    justify-content: center;
  }
  :global(html[data-theme='niconico-classic']) .loop-btn {
    min-width: 60px;
  }
</style>
