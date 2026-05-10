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
    onFullscreen: () => void;
    onToggleLoop: () => void;
    onQuality: (levelIndex: number) => void;
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
    onFullscreen,
    onToggleLoop,
    onQuality,
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
</script>

<div class="bar">
  <div class="seek">
    <span class="time">{formatDuration(currentTime)}</span>
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
    <span class="time">{formatDuration(duration)}</span>
  </div>

  <div class="controls">
    <button type="button" class="btn primary" onclick={onTogglePlay} aria-label="再生/一時停止">
      {paused ? '▶' : '❚❚'}
    </button>
    <div class="volume">
      <button type="button" class="btn" onclick={onToggleMute} aria-label="ミュート">
        {muted || volume === 0 ? '🔇' : volume < 0.5 ? '🔉' : '🔊'}
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

    <button type="button" class="btn" onclick={onFullscreen} title="全画面 (F)">⛶</button>
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
  .seek {
    display: grid;
    grid-template-columns: max-content 1fr max-content;
    align-items: center;
    gap: 8px;
    color: #f5f5f5;
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
    background: #41d2c5;
  }
  .ab-marker.out {
    background: #f59e0b;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    color: #eaeaea;
    font-size: 13px;
  }
  .btn {
    background: rgba(255, 255, 255, 0.08);
    border: 1px solid rgba(255, 255, 255, 0.12);
    color: #eaeaea;
    padding: 4px 10px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 14px;
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
    background: #2563eb;
    border-color: #2563eb;
  }
  .btn.active {
    background: #2563eb;
    border-color: #2563eb;
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
  .volume input[type='range'] {
    width: 80px;
  }
  .select {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: #cfcfcf;
  }
  .select select {
    background: #1a1a1a;
    color: #eaeaea;
    border: 1px solid #2f2f2f;
    border-radius: 4px;
    padding: 2px 6px;
    font-size: 12px;
  }
  .select select option {
    /* popup だけ光らせて選択肢が読めるように */
    background: #ffffff;
    color: #000000;
  }
  /* 速度ピッカー (上方向に開くカスタムドロップダウン) */
  .rate-picker {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: #cfcfcf;
  }
  .rate-label {
    font-size: 12px;
  }
  .rate-btn {
    background: #1a1a1a;
    color: #eaeaea;
    border: 1px solid #2f2f2f;
    border-radius: 4px;
    padding: 2px 8px;
    font-size: 12px;
    cursor: pointer;
    min-width: 56px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .rate-btn:hover {
    background: #2a2a2a;
  }
  .rate-menu {
    position: absolute;
    bottom: calc(100% + 4px); /* 上に開く - 全画面の下端でも切れない */
    right: 0;
    background: #1a1a1a;
    border: 1px solid #2f2f2f;
    border-radius: 6px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    box-shadow: 0 -4px 12px rgba(0, 0, 0, 0.6);
    z-index: 30;
    min-width: 70px;
  }
  .rate-menu button {
    background: transparent;
    border: none;
    color: #eaeaea;
    padding: 4px 10px;
    border-radius: 3px;
    font-size: 12px;
    cursor: pointer;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .rate-menu button:hover {
    background: #2a2a2a;
  }
  .rate-menu button.current {
    background: #2563eb;
    color: white;
  }
  .ab {
    display: inline-flex;
    gap: 2px;
    padding: 0 6px;
    border-left: 1px solid #2a2a2a;
    border-right: 1px solid #2a2a2a;
    margin: 0 4px;
  }
  .comments-controls {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .comments-controls input[type='range'] {
    width: 80px;
  }
</style>
