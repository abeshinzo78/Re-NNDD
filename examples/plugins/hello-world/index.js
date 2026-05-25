// 最小サンプル: Hello World プラグイン。
//
// 何をするか:
//   - 有効化時に挨拶トーストを出す
//   - コマンドパレット (Ctrl/⌘+K) に「Hello, world!」コマンドを追加
//   - 動画カードの ⋯ メニューに「動画ID を console.log」アクションを追加
//   - プレイヤーのコントロールバーに「👋」ボタンを追加 (キー: g)
//   - サイドバーに「ハロー」ページを追加 + そのページの DOM を描画
//   - `player:play` イベントで「再生開始」をログに残す
//
// permissions:
//   - "notify" のみ (notify.toast 経由トーストを使う場合; ctx.ui.toast は
//     権限不要なのでこの permission は実は notify.toast を使うときのみ必要)。

export async function activate(ctx) {
  ctx.log.info('Hello, world!', ctx.manifest.version);

  // 起動時の挨拶。ctx.ui.toast はホスト UI を直接叩くので permission 不要。
  ctx.ui.toast('Hello World プラグインが起動しました', 'ok');

  // コマンドパレット (Ctrl/⌘+K) への登録。permission は不要 (host UI への
  // 寄与のみ)。
  ctx.commands.register({
    id: 'hello.greet',
    title: 'Hello World: 挨拶を表示',
    hint: 'トーストで挨拶',
    keywords: ['hello', 'greet', 'こんにちは'],
    handler: () => {
      ctx.ui.toast('こんにちは!', 'info');
    },
  });

  // 動画カードのメニューに「ID を console に出す」アクション。
  ctx.items.addAction({
    label: 'Hello: ID を console に',
    handler: (hit) => {
      const id = hit?.contentId ?? hit?.videoId ?? '(unknown)';
      ctx.log.info('hit:', id);
      ctx.ui.toast(`console に ${id} を出力しました`, 'info');
    },
  });

  // プレイヤーコントロールバーの右端にボタン。`g` キーでも発火する
  // (組込みショートカット未使用キーのみ有効)。
  ctx.player.addAction({
    label: 'Hi',
    icon: '👋',
    key: 'g',
    handler: async () => {
      const state = ctx.player.getState();
      const at = Number.isFinite(state.currentTime) ? state.currentTime.toFixed(1) : '?';
      ctx.ui.toast(`現在 ${at}s を再生中`, 'info');
    },
  });

  // サイドバーナビ + 専用ページ。`/plugin/<id>/main` に対応するルートに
  // 自前 DOM をマウントする。
  ctx.pages.register('main', (el) => {
    const root = document.createElement('div');
    root.innerHTML = `
      <h2 style="margin-top:0">Hello World プラグイン</h2>
      <p>これは <code>ctx.pages.register("main", render)</code> で登録された
      プラグイン専用ページです。</p>
      <p>サイドバーの「ハロー」リンクから何度でも開けます。</p>
      <button id="hello-greet-btn" style="
        padding: 8px 16px; border-radius: 6px;
        background: var(--theme-accent); color: var(--theme-accent-fg);
        border: none; cursor: pointer; font-size: 14px;
      ">挨拶トーストを出す</button>
    `;
    el.appendChild(root);
    const btn = root.querySelector('#hello-greet-btn');
    const onClick = () => ctx.ui.toast('Hello, world from page!', 'ok');
    btn.addEventListener('click', onClick);
    // ページ離脱時に listener 解除 (戻り値の cleanup 関数)。
    return () => {
      btn.removeEventListener('click', onClick);
    };
  });
  ctx.nav.addPage({ href: '/plugin/com.example.hello-world/main', label: 'ハロー' });

  // 標準イベントの購読例。`player:play` 発火時にログを残す。
  ctx.events.on('player:play', (ev) => {
    ctx.log.info('played', ev.videoId, 'at', ev.currentTime);
  });
}

export function deactivate() {
  // host が registry / eventBus / pages を一括解除するので、追加でやることは無し。
  // タイマー等を自前で保持していたらここで clear する。
}
