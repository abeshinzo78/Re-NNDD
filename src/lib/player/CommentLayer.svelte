<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import NiconiComments from '@xpadev-net/niconicomments';
  import type { PlayerComment } from './types';

  type Props = {
    video: HTMLVideoElement | null;
    comments: PlayerComment[];
    enabled: boolean;
    opacity: number;
  };

  let { video, comments, enabled, opacity }: Props = $props();

  let canvas: HTMLCanvasElement | null = $state(null);
  let nc: NiconiComments | null = null;
  let rafId = 0;
  let lastVpos = -1;
  let prevFingerprint = '';

  let cacheElW = 0;
  let cacheElH = 0;
  // nc 作成時点の canvas 内部解像度。tick 中に canvas.width/height が
  // ずれていたら nc の internal scale も陳腐化してるので強制再作成。
  // これで「初期 layout 未確定で nc が小さい canvas で作られて、後で
  // canvas が大きくなっても scale 古いまま → 右上の小さい領域だけに
  // 描画される」現象を確実に直す。
  let ncCanvasW = 0;
  let ncCanvasH = 0;

  let resizeObserver: ResizeObserver | null = null;

  // niconicomments のプラットフォーム判定は Linux/X11 を "other" に落として
  // generic な sans-serif/serif しか指定しない。WebKitGTK の Canvas2D は
  // CSS の per-glyph フォールバックを完全には行わないことがあるため、
  // 和文 + 罫線（┃ ━ ┌ 等のコメ職人 AA で多用される文字）を両方持つ
  // フォントを上位に置く。
  // VL Gothic / Noto Sans CJK JP / IPA Gothic は和文 + 罫線を両方持つ。
  // 末尾の DejaVu Sans / sans-serif は罫線フォールバック用の保険。
  const JP_GOTHIC =
    '"Noto Sans CJK JP", "Noto Sans JP", "Source Han Sans JP", ' +
    '"VL Gothic", "VL PGothic", "VL ゴシック", "VL Pゴシック", ' +
    '"IPAexGothic", "IPAPGothic", "IPAGothic", "IPA Pゴシック", "IPAゴシック", ' +
    '"Takao P Gothic", "Takao Gothic", ' +
    '"Hiragino Kaku Gothic ProN", "Hiragino Sans", ' +
    '"Yu Gothic UI", "Yu Gothic", YuGothic, ' +
    '"BIZ UDPGothic", "Meiryo", ' +
    '"MS PGothic", MS-PGothic, ' +
    '"DejaVu Sans", "FreeSans", ' +
    '"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif';
  const JP_MINCHO =
    '"Noto Serif CJK JP", "Noto Serif JP", ' +
    '"IPAexMincho", "IPAPMincho", "IPAMincho", "IPA明朝", ' +
    '"Hiragino Mincho ProN", "Yu Mincho", YuMincho, ' +
    '"MS PMincho", MS-PMincho, "DejaVu Serif", "FreeSerif", serif';

  function buildFontPlatform() {
    return {
      defont: { font: JP_GOTHIC, offset: 0, weight: 600 },
      gothic: { font: JP_GOTHIC, offset: -0.04, weight: 400 },
      mincho: { font: JP_MINCHO, offset: -0.01, weight: 400 },
    };
  }

  // niconicomments の設定の `fonts` フィールドだけ上書きする。
  // 他のキーは defaultConfig の値が浅いマージで残るので触らなくてよい。
  function buildConfigOverride() {
    const html5 = buildFontPlatform();
    return {
      fonts: {
        html5,
        flash: {
          gulim: `normal 600 [size]px gulim, ${html5.gothic.font}`,
          simsun: `normal 400 [size]px simsun, batang, "PMingLiU", MingLiU-ExtB, ${html5.mincho.font}`,
        },
      },
    } as const;
  }

  // 制御文字や正規化前の結合文字で niconicomments の文字計測が崩れる/
  // 豆腐化することがあるので、Canvas に渡す直前で軽く整形する。
  // ・NFC 正規化（結合文字をプリコンポーズ）
  // ・C0/C1/DEL 制御文字（改行とタブ以外）を除去
  // ・行区切り(U+2028)/段落区切り(U+2029) を改行に統一
  // ・孤立サロゲートを除去
  // eslint-disable-next-line no-control-regex
  const RE_CONTROL = /[\x00-\x08\x0B\x0C\x0E-\x1F\x7F-\u009F]/g;
  const RE_LINESEP = /[\u2028\u2029]/g;
  const RE_LONE_HIGH = /[\uD800-\uDBFF](?![\uDC00-\uDFFF])/g;
  const RE_LONE_LOW = /(^|[^\uD800-\uDBFF])[\uDC00-\uDFFF]/g;

  function sanitizeContent(raw: string): string {
    if (!raw) return '';
    let s: string;
    try {
      s = raw.normalize('NFC');
    } catch {
      s = raw;
    }
    s = s.replace(RE_CONTROL, '');
    s = s.replace(RE_LINESEP, '\n');
    s = s.replace(RE_LONE_HIGH, '').replace(RE_LONE_LOW, '$1');
    return s;
  }

  function fingerprint(cs: PlayerComment[]): string {
    if (cs.length === 0) return '';
    return `${cs.length}:${cs[0].id}:${cs[cs.length - 1].id}`;
  }

  function destroyNc() {
    if (!nc) return;
    // destroy() は内部 active-instance counter をデクリメントしてくれる。
    // clear() はピクセルだけ消す。両方呼んでおく。
    try {
      (nc as unknown as { destroy?: () => void }).destroy?.();
    } catch {
      /* */
    }
    try {
      nc.clear?.();
    } catch {
      /* */
    }
    nc = null;
    forceClearCanvas();
    lastVpos = -1;
  }

  function forceClearCanvas() {
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    ctx?.clearRect(0, 0, canvas.width, canvas.height);
  }

  type SizeResult = {
    /** video が有効サイズを持ち canvas.width を必ず触っているか */
    valid: boolean;
    /** 直前の cache から実際にサイズが変わったか */
    changed: boolean;
  };

  function sizeCanvas(): SizeResult {
    if (!canvas || !video) return { valid: false, changed: false };
    const dw = video.clientWidth;
    const dh = video.clientHeight;
    if (dw === 0 || dh === 0) return { valid: false, changed: false };
    if (dw === cacheElW && dh === cacheElH) return { valid: true, changed: false };
    cacheElW = dw;
    cacheElH = dh;
    // niconicomments の internal canvas は 1920x1080 を基準にしている。
    // DPR を考慮して目標バッファサイズを決め、内部スケールに合わせて 1920x1080
    // を上限にすることで、コメ職人系の AA や細い線をボヤけさせずに描く。
    const dpr = Math.max(
      1,
      Math.min(2, (typeof window !== 'undefined' && window.devicePixelRatio) || 1),
    );
    const wantW = dw * dpr;
    const wantH = dh * dpr;
    const scale = Math.min(1, 1920 / wantW, 1080 / wantH);
    const pw = Math.max(2, Math.round(wantW * scale));
    const ph = Math.max(2, Math.round(wantH * scale));
    canvas.width = pw; // canvas.width 代入で内部バッファがクリアされる
    canvas.height = ph;
    canvas.style.width = dw + 'px';
    canvas.style.height = dh + 'px';
    return { valid: true, changed: true };
  }

  function createNc() {
    destroyNc();
    if (!canvas || comments.length === 0) return;
    const r = sizeCanvas();
    // sizeCanvas が valid 返さない場合は次の resize を待つ
    if (!r.valid) return;
    if (canvas.width === 0 || canvas.height === 0) return;

    // Force Canvas2D renderer — WebGL2 on WebKitGTK is slow/unstable.
    // Pre-binding '2d' makes getContext('webgl2') return null inside NC.
    canvas.getContext('2d');

    const byFork = new Map<string, ReturnType<typeof toV1Comment>[]>();
    for (const c of comments) {
      const fork = c.fork || 'main';
      const arr = byFork.get(fork) ?? [];
      arr.push(toV1Comment(c));
      byFork.set(fork, arr);
    }
    const threads = Array.from(byFork.entries()).map(([fork, arr]) => ({
      id: fork,
      fork,
      commentCount: arr.length,
      comments: arr,
    }));
    nc = new NiconiComments(canvas, threads as never, {
      format: 'v1',
      // V1 コメは HTML5 系レンダラで描く。default は legacy/flash に
      // 切り替わる経路があり、稀に解釈ズレが起きるので明示する。
      mode: 'html5',
      // Linux/X11 で defaultConfig が generic フォントになるのを上書き
      config: buildConfigOverride() as never,
    });
    ncCanvasW = canvas.width;
    ncCanvasH = canvas.height;
  }

  function toV1Comment(c: PlayerComment) {
    return {
      id: c.id,
      no: c.no,
      vposMs: c.vposMs,
      body: sanitizeContent(c.content),
      commands: c.commands,
      userId: c.userId ?? '',
      isPremium: false,
      score: c.score ?? 0,
      postedAt: c.postedAt ?? '',
      nicoruCount: c.nicoruCount ?? 0,
      nicoruId: null,
      source: 'leaf',
      isMyPost: false,
    };
  }

  function tick() {
    // 毎フレーム size を確認し、nc の整合性を取る。これが root of truth。
    // ResizeObserver / 各種 effect の発火タイミングに依存しない。
    if (canvas && video) {
      const r = sizeCanvas();
      if (r.valid && comments.length > 0) {
        // nc 未作成 / size 変わった / nc が違う size で作られてる
        // のどれかなら作り直す。
        const stale = !nc || r.changed || canvas.width !== ncCanvasW || canvas.height !== ncCanvasH;
        if (stale) createNc();
      } else if (comments.length === 0 && nc) {
        destroyNc();
      }
    }

    if (nc && video && enabled) {
      if (video.seeking) {
        forceClearCanvas();
        lastVpos = -1;
      } else {
        const vpos = Math.floor(video.currentTime * 100);
        if (vpos !== lastVpos) {
          lastVpos = vpos;
          // forceRendering=true で niconicomments の cache を毎回バイパス。
          // 「pre-render された古いスケールの comment image」を使い回されて
          // 描画位置が偏るバグの保険。
          nc.drawCanvas(vpos, true);
        }
      }
    } else if (nc && !enabled && lastVpos !== -1) {
      forceClearCanvas();
      lastVpos = -1;
    }
    rafId = requestAnimationFrame(tick);
  }

  onMount(() => {
    rafId = requestAnimationFrame(tick);
  });

  onDestroy(() => {
    cancelAnimationFrame(rafId);
    destroyNc();
    resizeObserver?.disconnect();
  });

  // 再作成は tick() の毎フレーム検査に集約。ここでは何もしない。
  // ResizeObserver はあえて残してあるが、目的は tick が見る size cache を
  // 早めにウォームアップするためだけ (sizeCanvas を 1 回呼ぶ効果のみ)。
  function onElementResize() {
    sizeCanvas();
  }

  // ResizeObserver: track video element size changes (fullscreen, window resize)
  $effect(() => {
    if (!video) {
      cacheElW = 0;
      cacheElH = 0;
      return;
    }
    onElementResize();
    resizeObserver?.disconnect();
    resizeObserver = new ResizeObserver(onElementResize);
    resizeObserver.observe(video);
    return () => {
      resizeObserver?.disconnect();
      resizeObserver = null;
    };
  });

  // comments が変わった時は単に nc を捨てるだけ。tick が次フレームで
  // 適切な canvas size を見て作り直す。ここで createNc を直接呼ぶと
  // layout 確定前のサイズで nc が固定されて描画が偏る原因になる。
  $effect(() => {
    const fp = fingerprint(comments);
    untrack(() => {
      if (fp === prevFingerprint) return;
      prevFingerprint = fp;
      forceClearCanvas();
      destroyNc();
    });
  });
</script>

<canvas bind:this={canvas} class="layer" style:opacity={enabled ? opacity : 0}></canvas>

<style>
  .layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
    transition: opacity 0.15s linear;
    /* Canvas を表示位置で再サンプリングする際にぼやけにくくする */
    image-rendering: -webkit-optimize-contrast;
  }
</style>
