<script lang="ts">
  // プラグイン専用ページ。`ctx.pages.register(subpath, render)` で登録された
  // renderer を mount する。SvelteKit の rest セグメント `[...path]` を使い、
  // `/plugin/<id>/` 配下なら任意の URL 階層を受ける。
  //
  // 重要な不変条件:
  // - renderer の戻り値が cleanup 関数なら、ページ離脱時に必ず呼ぶ。
  // - 同一プラグインの違う subpath への遷移でも、前の renderer は cleanup
  //   される (`page.params.id` / `page.params.path` を $effect 依存にする)。
  // - 登録のないプラグイン / subpath は「ページ未登録」を案内する。
  //   404 にすると UX が悪い (プラグインの addPage と pages.register の
  //   組み合わせミスをデバッグしづらい) ため、404 ではなくその場で表示。

  import { onDestroy } from 'svelte';
  import { page } from '$app/state';
  import { pluginPage } from '$lib/plugins/registry';

  let mountEl: HTMLDivElement | null = $state(null);
  let cleanup: (() => void) | null = null;
  let errorMessage = $state<string | null>(null);
  // ルート変化のたびにインクリメントする世代カウンタ。renderer が async で
  // 戻ってきたとき、自分の世代が最新かどうかを確認して stale な resolve が
  // 新しい cleanup を上書きしないようにする (Codex r3298977876)。
  let mountGeneration = 0;

  // ページ遷移 (id / path の変化) で renderer を mount し直す。`mountEl` が
  // bind:this で取れた直後の最初の effect 評価でマウントが起きる。
  $effect(() => {
    // 依存源を明示。`page.params.id` / `page.params.path` を読むことで
    // SvelteKit のルート変化に reactive。`id` は [id] dynamic セグメントなので
    // 必ず string が入る (型上は undefined 含むが実体は string)。
    const id = page.params.id ?? '';
    const subpath = page.params.path ?? '';
    if (!mountEl) return;
    if (!id) return; // 念のため (起こらないはず)
    // 世代を 1 つ進める。この effect のクロージャ内で `gen` をキャプチャし、
    // 後続の Promise then の中で `gen === mountGeneration` を確認することで、
    // 高速な subpath 切替で古い renderer が遅れて resolve しても新ページの
    // cleanup を上書きしない。
    mountGeneration += 1;
    const gen = mountGeneration;
    // 前の renderer の cleanup を先に呼ぶ (subpath 切替時の listener 漏れ防止)。
    try {
      cleanup?.();
    } catch (e) {
      console.error('[plugin page] cleanup of previous renderer threw:', e);
    }
    cleanup = null;
    // プラグインがレンダラで appendChild した DOM を全部捨てる。`mountEl` は
    // SvelteKit のテンプレートでは bind:this で取った空 div で、Svelte は
    // この div の中身を一切管理しない (= プラグイン境界として明示的に空けて
    // ある) ので、replaceChildren で空にしても runtime に混乱は起きない。
    // eslint-disable-next-line svelte/no-dom-manipulating
    mountEl.replaceChildren();
    errorMessage = null;

    const renderer = pluginPage(id, subpath);
    if (!renderer) {
      // 未登録。サンプル文言で原因を示す。
      errorMessage =
        `プラグイン "${id}" の "${subpath || '/'}" には ` +
        `ページが登録されていません。プラグイン側で ctx.pages.register("${subpath || '/'}", render) を呼んでください。`;
      return;
    }
    // renderer が Promise を返すケースに備えて await し、cleanup を取り出す。
    Promise.resolve()
      .then(() => renderer(mountEl!))
      .then((maybeCleanup) => {
        // 世代チェック: この呼出しが走っている間に subpath 遷移していたら、
        // ここで得た cleanup は古いページ用なので即捨てる + 何も書き換えない。
        if (gen !== mountGeneration) {
          if (typeof maybeCleanup === 'function') {
            try {
              maybeCleanup();
            } catch (e) {
              console.error('[plugin page] stale renderer cleanup threw:', e);
            }
          }
          return;
        }
        if (typeof maybeCleanup === 'function') cleanup = maybeCleanup;
      })
      .catch((e) => {
        if (gen !== mountGeneration) return; // stale な reject は無視
        console.error(`[plugin page] renderer for ${id}/${subpath} threw:`, e);
        errorMessage = `プラグインのレンダラがエラーを投げました: ${e}`;
      });
  });

  onDestroy(() => {
    // 未解決の renderer Promise が後から resolve しても stale 扱いになるよう、
    // 世代を 1 つ進めてから cleanup する。これがないと unmount 後に async
    // renderer が resolve したとき `gen === mountGeneration` が成立して
    // `cleanup = maybeCleanup` が代入され、その cleanup が永遠に呼ばれない
    // (listener/timer リーク; Codex review r3299045278)。
    mountGeneration += 1;
    try {
      cleanup?.();
    } catch (e) {
      console.error('[plugin page] cleanup on unmount threw:', e);
    }
    cleanup = null;
  });
</script>

<section class="plugin-page">
  {#if errorMessage}
    <div class="error">
      <h3>プラグインページ</h3>
      <p>{errorMessage}</p>
    </div>
  {/if}
  <!-- renderer がここに DOM を書き込む。プラグイン側のスタイルは scoped されない
       (svelte の :global() を通さない素 HTML/CSS) ので、プラグイン作者は wrapper
       要素にユニークな id/class を付ける運用が推奨。 -->
  <div bind:this={mountEl} class="plugin-mount"></div>
</section>

<style>
  .plugin-page {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .error {
    background: var(--theme-warning-bg);
    border: 1px solid var(--theme-warning-border);
    color: var(--theme-warning-text);
    padding: 12px 16px;
    border-radius: 8px;
  }
  .error h3 {
    margin: 0 0 4px;
    font-size: 14px;
  }
  .error p {
    margin: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  .plugin-mount {
    /* プラグインが描画する領域。最小高さで「空 div で空白に見える」UX を回避。 */
    min-height: 80px;
  }
</style>
