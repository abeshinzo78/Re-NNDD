import { beforeEach, describe, expect, test } from 'vitest';
import {
  createSmartPlaylist,
  deleteSmartPlaylist,
  filterToQueryParams,
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
