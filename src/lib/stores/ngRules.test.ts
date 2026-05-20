import { describe, expect, test } from 'vitest';
import {
  isRankingItemBlocked,
  type NgRule,
  type RankingItemLike,
  type RankingTagInfo,
} from './ngRules';

function makeRule(
  overrides: Partial<NgRule> & Pick<NgRule, 'targetType' | 'matchMode' | 'pattern'>,
): NgRule {
  return {
    id: 'r1',
    scopeRanking: true,
    scopeSearch: false,
    scopeComment: false,
    enabled: true,
    createdAt: 0,
    hitCount: 0,
    ...overrides,
  } as NgRule;
}

describe('isRankingItemBlocked', () => {
  const item: RankingItemLike = {
    id: 'sm12345',
    title: '【替え歌】テスト動画',
    owner: { id: '999', name: '太郎', ownerType: 'user' },
  };

  test('video_id exact match blocks', () => {
    const rules = [makeRule({ targetType: 'video_id', matchMode: 'exact', pattern: 'sm12345' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(true);
  });

  test('video_id non-match does not block', () => {
    const rules = [makeRule({ targetType: 'video_id', matchMode: 'exact', pattern: 'sm99' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(false);
  });

  test('video_title partial match blocks', () => {
    const rules = [
      makeRule({ targetType: 'video_title', matchMode: 'partial', pattern: '替え歌' }),
    ];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(true);
  });

  test('uploader matches by numeric id', () => {
    const rules = [makeRule({ targetType: 'uploader', matchMode: 'exact', pattern: '999' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(true);
  });

  test('uploader matches by user/{id}', () => {
    const rules = [makeRule({ targetType: 'uploader', matchMode: 'exact', pattern: 'user/999' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(true);
  });

  test('uploader_name exact', () => {
    const rules = [makeRule({ targetType: 'uploader_name', matchMode: 'exact', pattern: '太郎' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(true);
  });

  test('uploader_name partial does not match exact-only rule', () => {
    const rules = [makeRule({ targetType: 'uploader_name', matchMode: 'exact', pattern: '太' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(false);
  });

  test('uploader_name partial matches partial-mode rule', () => {
    const rules = [makeRule({ targetType: 'uploader_name', matchMode: 'partial', pattern: '太' })];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(true);
  });

  test('tag rule with kind=both matches any tag', () => {
    const tags: RankingTagInfo[] = [
      { name: 'ロック太郎', isLocked: true },
      { name: 'ニコニコ', isLocked: false },
    ];
    const rules = [
      makeRule({ targetType: 'tag', matchMode: 'partial', pattern: 'ニコニコ', tagKind: 'both' }),
    ];
    expect(isRankingItemBlocked(rules, item, tags).blocked).toBe(true);
  });

  test('tag rule with kind=lock skips user tags', () => {
    const tags: RankingTagInfo[] = [{ name: 'ニコニコ', isLocked: false }];
    const rules = [
      makeRule({ targetType: 'tag', matchMode: 'partial', pattern: 'ニコニコ', tagKind: 'lock' }),
    ];
    expect(isRankingItemBlocked(rules, item, tags).blocked).toBe(false);
  });

  test('tag rule with kind=lock matches lock tag', () => {
    const tags: RankingTagInfo[] = [{ name: 'カテゴリ', isLocked: true }];
    const rules = [
      makeRule({ targetType: 'tag', matchMode: 'exact', pattern: 'カテゴリ', tagKind: 'lock' }),
    ];
    expect(isRankingItemBlocked(rules, item, tags).blocked).toBe(true);
  });

  test('tag rule with kind=user skips lock tags', () => {
    const tags: RankingTagInfo[] = [{ name: 'カテゴリ', isLocked: true }];
    const rules = [
      makeRule({ targetType: 'tag', matchMode: 'exact', pattern: 'カテゴリ', tagKind: 'user' }),
    ];
    expect(isRankingItemBlocked(rules, item, tags).blocked).toBe(false);
  });

  test('tag rule is skipped when tags are not provided yet', () => {
    const rules = [makeRule({ targetType: 'tag', matchMode: 'partial', pattern: 'ニコニコ' })];
    // tags=undefined → ルール無効化扱い (= ブロックしない)
    expect(isRankingItemBlocked(rules, item, undefined).blocked).toBe(false);
  });

  test('disabled rule does not block', () => {
    const rules = [
      makeRule({
        targetType: 'video_id',
        matchMode: 'exact',
        pattern: 'sm12345',
        enabled: false,
      }),
    ];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(false);
  });

  test('rule without scopeRanking does not block', () => {
    const rules = [
      makeRule({
        targetType: 'video_id',
        matchMode: 'exact',
        pattern: 'sm12345',
        scopeRanking: false,
      }),
    ];
    expect(isRankingItemBlocked(rules, item).blocked).toBe(false);
  });
});
