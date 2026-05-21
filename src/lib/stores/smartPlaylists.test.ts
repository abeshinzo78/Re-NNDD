import { beforeEach, describe, expect, test } from 'vitest';
import {
  createSmartPlaylist,
  deleteSmartPlaylist,
  filterToQueryParams,
  filterToSearchQuery,
  getSmartPlaylist,
  listSmartPlaylists,
  normalizeFilter,
  subscribeSmartPlaylists,
  summarizeFilter,
  updateSmartPlaylist,
} from './smartPlaylists';

beforeEach(() => {
  localStorage.clear();
});

describe('smartPlaylists CRUD', () => {
  test('listSmartPlaylists is empty by default', () => {
    expect(listSmartPlaylists()).toEqual([]);
  });

  test('createSmartPlaylist persists name + filter', () => {
    const p = createSmartPlaylist('My picks', { tags: ['vocaloid'], limit: 50 });
    expect(p.id).toMatch(/^sp_/);
    expect(p.name).toBe('My picks');
    expect(p.filter.tags).toEqual(['vocaloid']);
    expect(p.filter.limit).toBe(50);
    expect(listSmartPlaylists()).toHaveLength(1);
  });

  test('createSmartPlaylist falls back to a default name when blank', () => {
    const p = createSmartPlaylist('   ', {});
    expect(p.name.length).toBeGreaterThan(0);
  });

  test('getSmartPlaylist round-trips by id', () => {
    const p = createSmartPlaylist('A', { q: 'foo' });
    const found = getSmartPlaylist(p.id);
    expect(found?.id).toBe(p.id);
    expect(found?.filter.q).toBe('foo');
  });

  test('updateSmartPlaylist patches name and filter', () => {
    const p = createSmartPlaylist('A', { tags: ['a'] });
    const updated = updateSmartPlaylist(p.id, { name: 'B', filter: { tags: ['b'] } });
    expect(updated?.name).toBe('B');
    expect(updated?.filter.tags).toEqual(['b']);
    expect(updated?.updatedAt).toBeGreaterThanOrEqual(p.createdAt);
  });

  test('updateSmartPlaylist returns undefined for unknown id', () => {
    expect(updateSmartPlaylist('nope', { name: 'X' })).toBeUndefined();
  });

  test('deleteSmartPlaylist removes the entry', () => {
    const p = createSmartPlaylist('A', {});
    expect(deleteSmartPlaylist(p.id)).toBe(true);
    expect(getSmartPlaylist(p.id)).toBeUndefined();
    expect(deleteSmartPlaylist(p.id)).toBe(false);
  });

  test('subscribeSmartPlaylists fires on mutation', () => {
    let n = 0;
    const unsub = subscribeSmartPlaylists(() => n++);
    const p = createSmartPlaylist('A', {});
    expect(n).toBe(1);
    updateSmartPlaylist(p.id, { name: 'B' });
    expect(n).toBe(2);
    deleteSmartPlaylist(p.id);
    expect(n).toBe(3);
    unsub();
    createSmartPlaylist('C', {});
    expect(n).toBe(3);
  });
});

describe('normalizeFilter', () => {
  test('drops empty strings and zero numbers', () => {
    const f = normalizeFilter({
      q: '   ',
      tags: ['', '  ', 'a'],
      tagsAny: ['b', 'b', ''],
      uploaderId: '',
      minDuration: 0,
      maxDuration: -1,
      resolution: '   ',
      sortBy: '',
      limit: 0,
    });
    expect(f.q).toBeUndefined();
    expect(f.tags).toEqual(['a']);
    expect(f.tagsAny).toEqual(['b']);
    expect(f.uploaderId).toBeUndefined();
    expect(f.minDuration).toBeUndefined();
    expect(f.maxDuration).toBeUndefined();
    expect(f.resolution).toBeUndefined();
    expect(f.sortBy).toBeUndefined();
    expect(f.limit).toBeUndefined();
  });

  test('preserves valid sort order and floors numbers', () => {
    const f = normalizeFilter({
      sortBy: 'view_count',
      sortOrder: 'asc',
      minDuration: 30.7,
      limit: 100.9,
    });
    expect(f.sortOrder).toBe('asc');
    expect(f.minDuration).toBe(30);
    expect(f.limit).toBe(100);
  });

  test('drops an invalid sortOrder value silently', () => {
    const f = normalizeFilter({ sortOrder: 'sideways' as unknown as 'asc' });
    expect(f.sortOrder).toBeUndefined();
  });
});

describe('filterToQueryParams', () => {
  test('maps every populated field 1:1', () => {
    const params = filterToQueryParams({
      q: 'a',
      tags: ['t1'],
      tagsAny: ['t2'],
      uploaderId: 'u1',
      minDuration: 10,
      maxDuration: 99,
      resolution: '1920x1080',
      sortBy: 'view_count',
      sortOrder: 'asc',
      limit: 200,
    });
    expect(params).toEqual({
      q: 'a',
      tags: ['t1'],
      tagsAny: ['t2'],
      uploaderId: 'u1',
      minDuration: 10,
      maxDuration: 99,
      resolution: '1920x1080',
      sortBy: 'view_count',
      sortOrder: 'asc',
      limit: 200,
    });
  });

  test('omits empty arrays and undefined fields', () => {
    expect(filterToQueryParams({ tags: [], tagsAny: [] })).toEqual({});
  });
});

describe('summarizeFilter', () => {
  test('returns a useful label for non-empty filters', () => {
    const s = summarizeFilter({ q: 'foo', tags: ['a', 'b'], limit: 10 });
    expect(s).toContain('"foo"');
    expect(s).toContain('タグAND');
    expect(s).toContain('上限');
  });

  test('returns a fallback for empty filters', () => {
    expect(summarizeFilter({})).toBe('条件なし (全件)');
  });
});

describe('filterToSearchQuery (online snapshot search)', () => {
  test('keyword becomes q with title+tags targets', () => {
    const q = filterToSearchQuery({ q: 'ボカロ' });
    expect(q.q).toBe('ボカロ');
    expect(q.targets).toEqual(['title', 'tags']);
    expect(q.filters).toEqual([]);
  });

  test('AND tags without keyword: first tag is q (tagsExact), rest become filters', () => {
    const q = filterToSearchQuery({ tags: ['ボカロ', '名作'] });
    expect(q.q).toBe('ボカロ');
    expect(q.targets).toEqual(['tagsExact']);
    expect(q.filters).toEqual([{ field: 'tagsExact', op: 'eq', value: '名作' }]);
  });

  test('OR tags only: joined with OR operator', () => {
    const q = filterToSearchQuery({ tagsAny: ['替え歌', '弾いてみた'] });
    expect(q.q).toBe('替え歌 OR 弾いてみた');
    expect(q.targets).toEqual(['tagsExact']);
  });

  test('keyword + AND tags: tags become tagsExact filters', () => {
    const q = filterToSearchQuery({ q: 'ライブ', tags: ['ボカロ'] });
    expect(q.q).toBe('ライブ');
    expect(q.targets).toEqual(['title', 'tags']);
    expect(q.filters).toContainEqual({ field: 'tagsExact', op: 'eq', value: 'ボカロ' });
  });

  test('uploader / duration become snapshot search filters', () => {
    const q = filterToSearchQuery({
      q: 'a',
      uploaderId: '12345',
      minDuration: 60,
      maxDuration: 600,
    });
    expect(q.filters).toContainEqual({ field: 'userId', op: 'eq', value: '12345' });
    expect(q.filters).toContainEqual({ field: 'lengthSeconds', op: 'gte', value: '60' });
    expect(q.filters).toContainEqual({ field: 'lengthSeconds', op: 'lte', value: '600' });
  });

  test('sort maps to snapshot search field names', () => {
    const q = filterToSearchQuery({ q: 'a', sortBy: 'view_count', sortOrder: 'desc' });
    expect(q.sort).toEqual({ field: 'viewCounter', direction: 'desc' });
  });

  test('local-only sort fields are dropped (no sort emitted)', () => {
    const q = filterToSearchQuery({ q: 'a', sortBy: 'play_count', sortOrder: 'desc' });
    expect(q.sort).toBeUndefined();
  });

  test('limit is capped to snapshot search max (100)', () => {
    const q = filterToSearchQuery({ q: 'a', limit: 500 });
    expect(q.limit).toBeLessThanOrEqual(100);
  });

  test('limit honors user value when small', () => {
    const q = filterToSearchQuery({ q: 'a', limit: 25 });
    expect(q.limit).toBe(25);
  });

  test('empty filter yields empty q (caller surfaces a UX error)', () => {
    const q = filterToSearchQuery({});
    expect(q.q).toBe('');
  });

  test('resolution is silently ignored online', () => {
    const q = filterToSearchQuery({ q: 'a', resolution: '1280x720' });
    const hasResolutionFilter = (q.filters ?? []).some(
      (c) => String(c.field).toLowerCase() === 'resolution',
    );
    expect(hasResolutionFilter).toBe(false);
  });
});
