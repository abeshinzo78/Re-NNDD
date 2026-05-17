<script lang="ts">
  let { title, phase, description } = $props<{
    title: string;
    phase: string;
    description: string;
  }>();
</script>

<section>
  <h2>{title} <span class="badge">{phase}</span></h2>
  <p class="muted">{description}</p>
  <p class="muted">この画面はまだ未実装です。CLAUDE.md のフェーズ表も参照。</p>
</section>

<style>
  h2 {
    margin-top: 0;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .muted {
    color: var(--theme-text-muted);
  }
  .badge {
    background: var(--theme-border-strong);
    color: var(--theme-text-soft);
    font-size: 11px;
    font-weight: 400;
    padding: 2px 8px;
    border-radius: 999px;
  }
</style>
