<script lang="ts">
  // SearchHitCard を使っていない動画カード (ranking / library / history / user
  // ページ等) で、プラグインの `ctx.items.addAction()` を活かすための共通ボタン。
  //
  // 既存の SearchHit ベース handler を最大限再利用するため、引数は SearchHit
  // と互換のあるサブセット (`contentId`, `title`, ...) を受け取る。型を緩い
  // VideoLike にしておけば、各ルートが自由に渡せる。
  //
  // appliesTo() が plugin-supplied なので必ず try/catch で囲む — render
  // 中の throw でカードが壊れない設計 (SearchHitCard と同じ防御方針)。

  import { pluginItemActions } from '$lib/plugins/registry';
  import { showToast } from '$lib/toastStore.svelte';
  import type { PluginItemAction } from '$lib/plugins/types';

  /** 各ルートのカード行が渡す動画情報の最小公倍数。
   *  最低限 contentId / title はある前提だが、プラグイン handler は他フィールド
   *  にもアクセスできるよう生オブジェクトをそのまま渡す。 */
  export type VideoLike = {
    contentId?: string | null;
    /** ローカルライブラリ等で `videoId` 名で持ってるケースに対応。 */
    videoId?: string | null;
    title?: string | null;
    thumbnailUrl?: string | null;
    lengthSeconds?: number | null;
    viewCounter?: number | null;
    [k: string]: unknown;
  };

  type Props = {
    video: VideoLike;
    /** ⋯ ボタンを非表示にして「プラグインアクション 0 件のとき何も出さない」場合に
     *  true を渡す。各ページが NG メニュー等を別途持っているときに重複を避けるため
     *  にも使える (今回は ⋯ ボタンを単独で出す)。 */
    compact?: boolean;
  };
  let { video, compact = false }: Props = $props();

  // VideoLike → SearchHit 互換オブジェクト。pluginItemActions の handler は
  // SearchHit 形を期待しているケースが多い (docs/plugins.md のサンプルが
  // そう書いてある) ので、`contentId` を確実に埋める。
  let hitLike = $derived.by(() => ({
    ...video,
    contentId: video.contentId ?? video.videoId ?? null,
  }));

  let actions: PluginItemAction[] = $derived(
    pluginItemActions().filter((a) => {
      if (!a.appliesTo) return true;
      try {
        return !!a.appliesTo(hitLike);
      } catch (e) {
        console.error('[plugin item action] appliesTo threw — excluding:', e);
        return false;
      }
    }),
  );

  let menuOpen = $state(false);
  let menuBtn: HTMLButtonElement | null = $state(null);
  let menuEl: HTMLDivElement | null = $state(null);

  async function runAction(a: PluginItemAction) {
    menuOpen = false;
    try {
      await a.handler(hitLike);
    } catch (e) {
      console.error('[plugin item action] threw:', e);
      showToast('プラグイン処理でエラー', 'error');
    }
  }

  function onDocClick(e: MouseEvent) {
    const t = e.target as Node;
    if (menuEl?.contains(t) || menuBtn?.contains(t)) return;
    menuOpen = false;
  }

  // ranking / library 等で 100 件以上のカードが並ぶページでは、各カードが
  // 常時 document mousedown を listen していると、クリックごとに全件分の
  // handler が走って性能劣化する (Codex r3298977877)。
  // メニューが open のときだけ listener を貼り、close 時に外す。
  $effect(() => {
    if (!menuOpen) return;
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  });
</script>

{#if actions.length > 0}
  <div class="wrap" class:compact>
    <button
      type="button"
      class="trigger"
      bind:this={menuBtn}
      onclick={() => (menuOpen = !menuOpen)}
      aria-haspopup="true"
      aria-expanded={menuOpen}
      aria-label="プラグインアクション"
      title="プラグインアクション">⋯</button
    >
    {#if menuOpen}
      <div class="menu" bind:this={menuEl} role="menu">
        <div class="head">プラグイン</div>
        {#each actions as a, i (i)}
          <button type="button" onclick={() => runAction(a)}>{a.label}</button>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  /* プラグインアクション 0 件のとき DOM ごと出ないので、style だけ書いておけば
     プラグイン無効時のレイアウト影響はゼロ。 */
  .wrap {
    position: relative;
    display: inline-block;
  }
  .trigger {
    background: var(--theme-surface-2);
    color: var(--theme-text-soft);
    border: 1px solid var(--theme-border-strong);
    border-radius: 6px;
    padding: 0 8px;
    font-size: 16px;
    line-height: 22px;
    cursor: pointer;
  }
  .trigger:hover {
    background: var(--theme-border-strong);
    color: var(--theme-text);
  }
  .compact .trigger {
    padding: 0 6px;
    font-size: 14px;
    line-height: 18px;
  }
  .menu {
    position: absolute;
    top: 28px;
    right: 0;
    z-index: 40;
    width: 220px;
    background: var(--theme-surface-4);
    border: 1px solid var(--theme-surface-hover);
    border-radius: 8px;
    box-shadow: var(--theme-menu-shadow);
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .head {
    font-size: 10px;
    color: var(--theme-text-muted);
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--theme-border-strong);
    margin-bottom: 4px;
    text-transform: uppercase;
  }
  .menu button {
    display: block;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--theme-text);
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 12px;
    cursor: pointer;
  }
  .menu button:hover {
    background: var(--theme-border-strong);
  }
</style>
