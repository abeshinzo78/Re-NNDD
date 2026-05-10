import { describe, expect, test } from 'vitest';
import { sanitizeDescriptionHtml } from './sanitize';

describe('sanitizeDescriptionHtml', () => {
  test('strips script tags', () => {
    // 中身のテキストはサニタイザの出力ではプレーンテキスト化される。
    // 再パースしても `<script>` タグが現れない事だけが安全条件。
    const out = sanitizeDescriptionHtml('hello <script>alert(1)</script> world');
    expect(out).not.toContain('<script');
    expect(out).not.toContain('</script');
    expect(out).toContain('hello');
    expect(out).toContain('world');
  });

  test('strips event handlers and javascript: URLs', () => {
    const out = sanitizeDescriptionHtml(
      '<img src=x onerror="invoke(1)"><a href="javascript:alert(1)">x</a>',
    );
    expect(out).not.toContain('onerror');
    expect(out).not.toContain('javascript:');
    expect(out).not.toContain('<img');
  });

  test('keeps allowed anchor and adds noopener', () => {
    const out = sanitizeDescriptionHtml('<a href="https://example.com">ex</a>');
    expect(out).toContain('href="https://example.com"');
    expect(out).toContain('rel="noopener noreferrer"');
    expect(out).toContain('target="_blank"');
  });

  test('keeps br and basic formatting', () => {
    const out = sanitizeDescriptionHtml('a<br><b>bold</b><font color="red">red</font>');
    expect(out).toContain('<br>');
    expect(out).toContain('<b>bold</b>');
    expect(out).toContain('<font color="red">red</font>');
  });

  test('rejects malformed font color', () => {
    const out = sanitizeDescriptionHtml('<font color="expression(alert(1))">x</font>');
    expect(out).not.toContain('expression');
    expect(out).not.toContain('color=');
  });

  test('drops iframes entirely', () => {
    const out = sanitizeDescriptionHtml('<iframe src="https://evil"></iframe>tail');
    expect(out).not.toContain('<iframe');
    expect(out).toContain('tail');
  });

  test('handles empty / null input', () => {
    expect(sanitizeDescriptionHtml('')).toBe('');
    expect(sanitizeDescriptionHtml(null)).toBe('');
    expect(sanitizeDescriptionHtml(undefined)).toBe('');
  });

  test('keeps niconico-style timestamp anchors', () => {
    const out = sanitizeDescriptionHtml('<a href="#1:23">jump</a>');
    expect(out).toContain('href="#1:23"');
  });
});
