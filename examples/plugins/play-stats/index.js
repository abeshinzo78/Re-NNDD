// 再生統計プラグイン。
//
// 何をするか:
//   - `player:time` イベントを 200ms 間隔で購読し、動画ごとの累積再生秒数を
//     集計する。同じ動画への重複・前後シーク等は素朴に都度 +200ms 加算。
//   - 集計結果は plugin:<id>:stats 設定キーに JSON で永続化する
//     (起動時にロード、変更時に書き戻し)。
//   - サイドバーの「再生統計」をクリックするとダッシュボード ページが
//     開き、上位 N 件をリスト表示する (N は設定可能)。
//   - コマンドパレットから「再生統計を消去」できる。
//
// permissions:
//   - settings.read / settings.write: 集計結果を永続化するため
//   - notify: notify.toast での通知 (ctx.ui.toast は不要だが、Rust 経由
//     の例として 1 度だけ起動完了時に使う)

const KEY_STATS = 'plugin:com.example.play-stats:stats';
const KEY_TOPN = 'plugin:com.example.play-stats:topN';

export async function activate(ctx) {
  ctx.log.info('play-stats activated');

  // ---- 設定項目を 1 つ宣言 ----
  ctx.settings.register({
    key: KEY_TOPN,
    label: 'ダッシュボードに表示する上位件数',
    description: '1〜50 の整数。',
    kind: 'number',
    default: 10,
    min: 1,
    max: 50,
    step: 1,
  });

  // ---- 過去の集計を復元 ----
  /** @type {Record<string, { title: string|null, seconds: number }>} */
  let stats = {};
  try {
    const raw = await ctx.settings.get(KEY_STATS);
    if (typeof raw === 'string') stats = JSON.parse(raw);
    if (typeof stats !== 'object' || stats === null) stats = {};
  } catch (e) {
    ctx.log.warn('failed to load stats:', e);
    stats = {};
  }

  // ---- 永続化のスロットル (連続書き込みを 5 秒間隔に抑える) ----
  let persistTimer = null;
  function schedulePersist() {
    if (persistTimer) return;
    persistTimer = setTimeout(() => {
      persistTimer = null;
      void ctx.settings
        .set(KEY_STATS, JSON.stringify(stats))
        .catch((e) => ctx.log.warn('persist failed:', e));
    }, 5000);
  }

  // ---- player:time の購読 (200ms スロットル済み) ----
  // 同じイベントが連発するので、200ms 間隔の素朴加算で「再生時間 ≒ wall clock」
  // とみなす。シーク中も加算してしまうが MVP として許容。
  ctx.events.on('player:time', (ev) => {
    if (!ev || !ev.videoId) return;
    const id = ev.videoId;
    if (!stats[id]) stats[id] = { title: null, seconds: 0 };
    stats[id].seconds += 0.2;
    schedulePersist();
  });

  // ---- player:play でタイトルを覚えておく (player:time に title が無いので) ----
  // タイトルは現状 player:play / player:time に乗っていないため、library から
  // 取れる範囲だけ memo する (= ローカル DL 済み動画のみ)。今回は省略し、
  // タイトル不明のままビデオ ID で表示する。

  // ---- コマンドパレット項目 ----
  ctx.commands.register({
    id: 'play-stats.reset',
    title: '再生統計: すべての集計を消去',
    keywords: ['reset', 'clear'],
    handler: async () => {
      stats = {};
      try {
        await ctx.settings.set(KEY_STATS, '{}');
        ctx.ui.toast('再生統計をリセットしました', 'ok');
      } catch (e) {
        ctx.ui.toast('リセット失敗: ' + e, 'error');
      }
    },
  });

  // ---- ダッシュボードページ ----
  ctx.pages.register('dashboard', async (el) => {
    // 直前に表示している stats をフラッシュ (持っている in-memory が最新)。
    const topN = await getTopN(ctx);
    const list = Object.entries(stats)
      .map(([id, s]) => ({ id, seconds: s.seconds, title: s.title }))
      .sort((a, b) => b.seconds - a.seconds)
      .slice(0, topN);

    const root = document.createElement('div');
    root.innerHTML = `
      <h2 style="margin-top:0">再生統計ダッシュボード</h2>
      <p style="color: var(--theme-text-muted); font-size: 13px;">
        プラグイン読み込み後の累積再生時間 (200ms ごと加算)。表示件数は設定で変更できます。
      </p>
    `;
    if (list.length === 0) {
      const empty = document.createElement('p');
      empty.style.color = 'var(--theme-text-muted)';
      empty.textContent = 'まだ集計データがありません。動画を再生すると蓄積されます。';
      root.appendChild(empty);
    } else {
      const table = document.createElement('table');
      table.style.cssText = 'width:100%; border-collapse: collapse; font-size: 13px;';
      table.innerHTML = `
        <thead>
          <tr style="border-bottom: 1px solid var(--theme-border);">
            <th style="text-align:left; padding:6px 8px;">順位</th>
            <th style="text-align:left; padding:6px 8px;">動画 ID</th>
            <th style="text-align:right; padding:6px 8px;">累積再生時間</th>
          </tr>
        </thead>
      `;
      const tbody = document.createElement('tbody');
      list.forEach((row, idx) => {
        const tr = document.createElement('tr');
        tr.style.borderBottom = '1px solid var(--theme-surface-3)';
        const mins = Math.floor(row.seconds / 60);
        const secs = Math.floor(row.seconds % 60);
        tr.innerHTML = `
          <td style="padding:6px 8px;">${idx + 1}</td>
          <td style="padding:6px 8px;"><a href="/video/${row.id}" style="color: var(--theme-accent-soft);">${row.id}</a></td>
          <td style="padding:6px 8px; text-align:right; font-variant-numeric: tabular-nums;">${mins}分 ${secs}秒</td>
        `;
        tbody.appendChild(tr);
      });
      table.appendChild(tbody);
      root.appendChild(table);
    }
    el.appendChild(root);
  });

  ctx.nav.addPage({
    href: '/plugin/com.example.play-stats/dashboard',
    label: '再生統計',
  });

  ctx.ui.toast('Play Stats: 集計を開始しました', 'info');
}

async function getTopN(ctx) {
  try {
    const v = await ctx.settings.get(KEY_TOPN);
    const n = typeof v === 'string' ? Number(v) : Number(v);
    if (Number.isFinite(n) && n >= 1 && n <= 50) return Math.floor(n);
  } catch {
    /* fall through */
  }
  return 10;
}

export function deactivate() {
  // host が listener とページを掃除するので、追加処理は不要。
  // 集計データは settings に永続化済み。
}
