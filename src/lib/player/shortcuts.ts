// Keyboard shortcut wiring per CLAUDE.md "キーボードショートカット" table.
// Returns a teardown function. Caller is responsible for not registering when
// the player is hidden.

export type PlayerActions = {
  togglePlay: () => void;
  seekDelta: (deltaSec: number) => void;
  jumpToFraction: (frac: number) => void;
  toggleComments: () => void;
  toggleFullscreen: () => void;
  toggleMute: () => void;
  setAbIn: () => void;
  setAbOut: () => void;
  toggleAbLoop: () => void;
  volumeDelta: (delta: number) => void;
  frameStep: (forward: boolean) => void;
  togglePip?: () => void;
  /** プラグインが要求したキーバインド (key → handler)。組込みショートカット
   *  にマッチしないキーのみ評価する (組込み優先 = プラグインで上書き不可)。
   *  未指定 / 空オブジェクトなら何も評価しない (= 既存挙動と同じ)。 */
  pluginKeys?: Record<string, () => void>;
};

export function bindShortcuts(target: HTMLElement | Window, a: PlayerActions): () => void {
  function isTextField(el: EventTarget | null): boolean {
    if (!(el instanceof HTMLElement)) return false;
    const tag = el.tagName;
    return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable;
  }

  function onKey(event: KeyboardEvent) {
    if (isTextField(event.target)) return;
    if (event.altKey || event.metaKey) return;

    const big = event.shiftKey ? 1 : event.ctrlKey ? 30 : 5;
    switch (event.key) {
      case ' ':
        event.preventDefault();
        a.togglePlay();
        return;
      case 'ArrowLeft':
        event.preventDefault();
        a.seekDelta(-big);
        return;
      case 'ArrowRight':
        event.preventDefault();
        a.seekDelta(big);
        return;
      case 'ArrowUp':
        event.preventDefault();
        a.volumeDelta(0.05);
        return;
      case 'ArrowDown':
        event.preventDefault();
        a.volumeDelta(-0.05);
        return;
      case ',':
        event.preventDefault();
        a.frameStep(false);
        return;
      case '.':
        event.preventDefault();
        a.frameStep(true);
        return;
      case 'c':
      case 'C':
        a.toggleComments();
        return;
      case 'f':
      case 'F':
        a.toggleFullscreen();
        return;
      case 'm':
      case 'M':
        a.toggleMute();
        return;
      case 'i':
      case 'I':
        a.setAbIn();
        return;
      case 'o':
      case 'O':
        a.setAbOut();
        return;
      case 'l':
      case 'L':
        a.toggleAbLoop();
        return;
      case 'p':
      case 'P':
        if (a.togglePip) {
          event.preventDefault();
          a.togglePip();
        }
        return;
      case 'Escape': {
        const d = document as Document & {
          fullscreenElement?: Element | null;
          webkitFullscreenElement?: Element | null;
        };
        const inFs = d.fullscreenElement ?? d.webkitFullscreenElement;
        if (inFs) a.toggleFullscreen();
        return;
      }
      default:
        if (/^[0-9]$/.test(event.key)) {
          a.jumpToFraction(Number(event.key) / 10);
          return;
        }
        // プラグイン提供のキーバインド (組込みショートカットに当たらなかった場合のみ)。
        // 大文字小文字は呼び出し側が自由に登録できるよう、入力 key そのまま検索。
        if (a.pluginKeys) {
          const handler = a.pluginKeys[event.key];
          if (handler) {
            try {
              handler();
            } catch (err) {
              console.error('[plugin shortcut] threw:', err);
            }
          }
        }
    }
  }

  target.addEventListener('keydown', onKey as EventListener);
  return () => target.removeEventListener('keydown', onKey as EventListener);
}
