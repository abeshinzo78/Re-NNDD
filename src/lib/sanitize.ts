// 動画説明文（niconico の APIが返す HTML）を `{@html}` で描画する前に通す
// 軽量サニタイザ。
//
// 動機: 説明文には作者がリンク (`<a>`)、改行 (`<br>`)、太字 (`<b>`/`<strong>`)、
// 文字色 (`<font color>`) など軽い装飾を埋めることが多いので、まるごと
// テキスト化するのは惜しい。が、`<img onerror>` や `<script>` を素通しに
// すると Tauri 環境で任意 invoke を呼ばれて PC のファイルを消されかねない
// ので、許可リスト方式で書き直す。
//
// DOMPurify を入れる手もあるが、依存を増やしたくないので必要十分な
// 自前実装でカバーする。

const ALLOWED_TAGS = new Set(['A', 'B', 'BR', 'EM', 'FONT', 'I', 'P', 'S', 'SPAN', 'STRONG', 'U']);

// タグごとに許可する属性。値は別途バリデートする。
const ALLOWED_ATTRS: Record<string, ReadonlySet<string>> = {
  A: new Set(['href']),
  FONT: new Set(['color']),
};

const SAFE_URL_SCHEMES = new Set(['http:', 'https:', 'mailto:']);
// `#1:23` のような時刻アンカーは niconico の説明欄でよく使われる。
function isSafeRelativeHref(href: string): boolean {
  // ハッシュリンク・サイト相対 (`/watch/sm...`) は許可。
  return href.startsWith('#') || href.startsWith('/');
}

function isSafeHref(href: string): boolean {
  const trimmed = href.trim();
  if (!trimmed) return false;
  if (isSafeRelativeHref(trimmed)) return true;
  try {
    const u = new URL(trimmed);
    return SAFE_URL_SCHEMES.has(u.protocol);
  } catch {
    return false;
  }
}

// `red` `#fff` `#ffffff` `rgb(0,0,0)` のみ許可（CSS 式注入を避けるため
// `expression(`、`url(`、改行などは弾く）。
function isSafeColor(value: string): boolean {
  const v = value.trim();
  if (v.length === 0 || v.length > 32) return false;
  return /^(#[0-9a-fA-F]{3,8}|[a-zA-Z]+|rgba?\([\d.,\s%]+\))$/.test(v);
}

function sanitizeNode(node: Node, out: Node, doc: Document): void {
  for (const child of Array.from(node.childNodes)) {
    if (child.nodeType === Node.TEXT_NODE) {
      out.appendChild(doc.createTextNode(child.nodeValue ?? ''));
      continue;
    }
    if (child.nodeType !== Node.ELEMENT_NODE) {
      // コメント・処理命令などは捨てる。
      continue;
    }
    const el = child as Element;
    const tag = el.tagName.toUpperCase();
    if (!ALLOWED_TAGS.has(tag)) {
      // タグごと捨てるが中身のテキストは活かしたいので再帰だけする。
      sanitizeNode(el, out, doc);
      continue;
    }
    const replacement = doc.createElement(tag.toLowerCase());
    const allowedAttrs = ALLOWED_ATTRS[tag];
    if (allowedAttrs) {
      for (const attr of Array.from(el.attributes)) {
        if (!allowedAttrs.has(attr.name.toLowerCase())) continue;
        const name = attr.name.toLowerCase();
        const value = attr.value;
        if (tag === 'A' && name === 'href') {
          if (!isSafeHref(value)) continue;
          replacement.setAttribute('href', value);
          // 外部リンクは新規タブ。`opener` を渡さないことで XSS 経由の
          // window.opener 攻撃を遮断。
          replacement.setAttribute('rel', 'noopener noreferrer');
          replacement.setAttribute('target', '_blank');
        } else if (tag === 'FONT' && name === 'color') {
          if (!isSafeColor(value)) continue;
          replacement.setAttribute('color', value);
        }
      }
    }
    sanitizeNode(el, replacement, doc);
    out.appendChild(replacement);
  }
}

/**
 * 信頼できない HTML 文字列を、許可リスト方式でサニタイズした HTML 文字列に
 * 変換する。`{@html}` に渡しても XSS にならないことを目標に書いてある。
 *
 * SSR 中（DOMParser が無い）はタグを全部落とした素のテキストを返す。
 */
export function sanitizeDescriptionHtml(input: string | null | undefined): string {
  if (!input) return '';
  if (typeof DOMParser === 'undefined') {
    // SSR フォールバック: タグを全部剥がす。
    return input.replace(/<[^>]*>/g, '');
  }
  const doc = new DOMParser().parseFromString(`<!doctype html><body>${input}`, 'text/html');
  const body = doc.body;
  const out = doc.createElement('div');
  sanitizeNode(body, out, doc);
  return out.innerHTML;
}
