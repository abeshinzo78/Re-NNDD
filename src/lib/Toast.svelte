<script lang="ts">
  // アプリ全体に 1 つだけ存在するトースト表示コンテナ。
  // `+layout.svelte` の末尾に <Toast /> を 1 つ置く。
  //
  // 重要な不変条件:
  // - プラグイン寄与 0 件のとき DOM は **完全に空** になる (display: none ではなく
  //   each ループの結果が 0 要素)。プラグイン未使用時の DOM フットプリントを
  //   増やさない設計。
  // - eventBus 経由で notify:toast を購読する。プラグインの notify.toast 発火
  //   時 (Rust dispatcher → emit_event) にこのコンポーネントが拾い上げる。
  import { onDestroy, onMount } from 'svelte';
  import * as pluginBus from '$lib/plugins/eventBus';
  import { dismissToast, listToasts, showToast } from '$lib/toastStore.svelte';

  // pluginBus.on は owner が必要。host (アプリ本体) は固有 owner 名で識別。
  const OWNER = '__host_toast__';

  let toasts = $derived(listToasts());

  onMount(() => {
    pluginBus.on(OWNER, 'notify:toast', (payload: unknown) => {
      // payload の shape は dispatcher::handle_notify_toast 由来:
      //   { pluginId: string, message: string, kind: string }
      if (!payload || typeof payload !== 'object') return;
      const p = payload as { pluginId?: unknown; message?: unknown; kind?: unknown };
      if (typeof p.message !== 'string') return;
      const kind = typeof p.kind === 'string' ? p.kind : 'info';
      const pluginId = typeof p.pluginId === 'string' ? p.pluginId : null;
      // 未知 kind は info として扱う (forward-compat)。
      const normalized: 'info' | 'ok' | 'warn' | 'error' =
        kind === 'ok' || kind === 'warn' || kind === 'error' ? kind : 'info';
      showToast(p.message, normalized, { pluginId });
    });
  });

  onDestroy(() => {
    // host owner の listener を一括解除。layout は通常 unmount しないが、
    // テスト環境や HMR で再 mount される場合に古い listener が残らないように。
    pluginBus.offAllByOwner(OWNER);
  });
</script>

{#if toasts.length > 0}
  <div class="toast-stack" role="status" aria-live="polite" aria-atomic="false">
    {#each toasts as t (t.id)}
      <div class="toast {t.kind}">
        {#if t.pluginId}
          <span class="src">プラグイン: <code>{t.pluginId}</code></span>
        {/if}
        <span class="msg">{t.message}</span>
        <button
          type="button"
          class="close"
          onclick={() => dismissToast(t.id)}
          aria-label="閉じる"
          title="閉じる">×</button
        >
      </div>
    {/each}
  </div>
{/if}

<style>
  .toast-stack {
    position: fixed;
    bottom: 24px;
    right: 24px;
    z-index: 9000;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 360px;
    /* MiniPlayer (z-index 8500 想定) より上、しかし全画面 video の上には
       出さない (全画面中はそもそも DOM ツリー上のここは隠れる)。 */
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--theme-surface-2);
    border: 1px solid var(--theme-border);
    color: var(--theme-text);
    box-shadow: var(--theme-menu-shadow);
    font-size: 13px;
    line-height: 1.5;
    animation: slide-in 180ms ease-out;
  }
  @keyframes slide-in {
    from {
      transform: translateY(8px);
      opacity: 0;
    }
    to {
      transform: translateY(0);
      opacity: 1;
    }
  }
  .toast.ok {
    background: var(--theme-success-bg-2);
    border-color: var(--theme-success-border);
    color: var(--theme-success-text);
  }
  .toast.warn {
    background: var(--theme-warning-bg);
    border-color: var(--theme-warning-border);
    color: var(--theme-warning-text);
  }
  .toast.error {
    background: var(--theme-danger-bg);
    border-color: var(--theme-danger-border);
    color: var(--theme-danger-text);
  }
  .src {
    font-size: 11px;
    color: var(--theme-text-muted);
    flex-shrink: 0;
    align-self: center;
  }
  .src code {
    background: var(--theme-bg);
    border: 1px solid var(--theme-border);
    border-radius: 3px;
    padding: 0 4px;
  }
  .msg {
    flex: 1;
    word-break: break-word;
  }
  .close {
    background: transparent;
    border: none;
    color: inherit;
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    padding: 0 4px;
    opacity: 0.6;
  }
  .close:hover {
    opacity: 1;
  }
</style>
