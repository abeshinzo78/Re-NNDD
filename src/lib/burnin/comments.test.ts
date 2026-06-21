import { describe, expect, test } from 'vitest';

import type { PlayerComment } from '../player/types';
import { buildConfigOverride, sanitizeContent, toIsoPostedAt, toV1Threads } from './comments';

const cmt = (over: Partial<PlayerComment>): PlayerComment => ({
  id: 'x',
  no: 1,
  vposMs: 0,
  content: 'c',
  mail: '',
  commands: [],
  fork: 'main',
  isOwner: false,
  ...over,
});

describe('sanitizeContent', () => {
  test('制御文字を除去する', () => {
    expect(sanitizeContent('a\x07b')).toBe('ab');
    expect(sanitizeContent('a\x00b\x1Fc')).toBe('abc');
  });

  test('行/段落区切り (U+2028/U+2029) を改行へ統一', () => {
    expect(sanitizeContent('a' + String.fromCharCode(0x2028) + 'b')).toBe('a\nb');
    expect(sanitizeContent('a' + String.fromCharCode(0x2029) + 'b')).toBe('a\nb');
  });

  test('通常テキスト・改行・タブは保持', () => {
    expect(sanitizeContent('hello\nworld\t!')).toBe('hello\nworld\t!');
  });

  test('空入力は空文字', () => {
    expect(sanitizeContent('')).toBe('');
  });
});

describe('toV1Threads', () => {
  test('fork ごとにスレッドへまとめる', () => {
    const threads = toV1Threads([
      cmt({ id: 'o1', content: 'owner', fork: 'owner', isOwner: true, vposMs: 100 }),
      cmt({ id: 'm1', content: 'main1', commands: ['red'], mail: 'red', vposMs: 200 }),
      cmt({ id: 'm2', content: 'main2', no: 2, vposMs: 300 }),
    ]);
    const owner = threads.find((t) => t.fork === 'owner');
    const main = threads.find((t) => t.fork === 'main');
    expect(owner?.comments.length).toBe(1);
    expect(main?.comments.length).toBe(2);
    expect(main?.commentCount).toBe(2);
    expect(main?.comments[0].body).toBe('main1');
    expect(main?.comments[0].commands).toEqual(['red']);
    expect(main?.comments[0].vposMs).toBe(200);
  });

  test('fork 未指定は main 扱い', () => {
    const threads = toV1Threads([cmt({ fork: '' })]);
    expect(threads[0].fork).toBe('main');
  });

  test('本文は sanitize される', () => {
    const threads = toV1Threads([cmt({ content: 'a\x07b' })]);
    expect(threads[0].comments[0].body).toBe('ab');
  });

  test('ローカルの Unix 秒 postedAt は ISO 8601 へ変換される', () => {
    // niconicomments v1 は Date.parse(postedAt) でモード判定するため ISO が必須。
    const threads = toV1Threads([cmt({ postedAt: '1170000000' })]);
    expect(threads[0].comments[0].postedAt).toBe('2007-01-28T16:00:00.000Z');
  });
});

describe('toIsoPostedAt', () => {
  test('Unix 秒文字列を ISO へ', () => {
    expect(toIsoPostedAt('1170000000')).toBe('2007-01-28T16:00:00.000Z');
  });
  test('既に ISO ならそのまま', () => {
    expect(toIsoPostedAt('2024-01-01T00:00:00+09:00')).toBe('2024-01-01T00:00:00+09:00');
  });
  test('空/undefined は空文字', () => {
    expect(toIsoPostedAt('')).toBe('');
    expect(toIsoPostedAt(undefined)).toBe('');
  });
});

describe('buildConfigOverride', () => {
  test('fonts のみを上書きする (座標系 config は触らない)', () => {
    const c = buildConfigOverride();
    expect(Object.keys(c)).toEqual(['fonts']);
    expect(c.fonts.html5.defont.font).toContain('Noto Sans CJK JP');
    expect(c.fonts.flash.gulim).toContain('gulim');
  });
});
