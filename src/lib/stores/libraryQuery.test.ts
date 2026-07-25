// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';

import { filterToSearchQuery, normalizeFilter } from './smartPlaylists';
import {
  activeFilterCount,
  addTagFilter,
  buildLibraryQuery,
  canSearchComments,
  checkSmartPlaylistSave,
  clampPage,
  clearLibraryFilter,
  clearLibraryFilters,
  commentHitHref,
  DEFAULT_LIBRARY_FILTER,
  isLibrarySortKey,
  LIBRARY_PAGE_SIZES,
  LIBRARY_SORT_OPTIONS,
  loadLibraryFilter,
  minutesToSeconds,
  newRandomSeed,
  normalizeLibraryFilter,
  pageCount,
  parseStartTime,
  patchLibraryFilter,
  removeTagFilter,
  RANDOM_SEED_MAX,
  resultRangeLabel,
  saveLibraryFilter,
  secondsToMinutes,
  sortLabel,
  summarizeLibraryFilter,
  toSmartPlaylistFilter,
  type LibraryFilterState,
} from './libraryQuery';

function state(patch: Partial<LibraryFilterState> = {}): LibraryFilterState {
  return { ...DEFAULT_LIBRARY_FILTER, ...patch };
}

describe('minutesToSeconds / secondsToMinutes', () => {
  it('分入力を秒へ変換する', () => {
    expect(minutesToSeconds('5')).toBe(300);
    expect(minutesToSeconds(' 10 ')).toBe(600);
    expect(minutesToSeconds('1.5')).toBe(90);
    expect(minutesToSeconds('0')).toBe(0);
  });

  it('空 / 非数値 / 負値は undefined (＝条件なし)', () => {
    expect(minutesToSeconds('')).toBeUndefined();
    expect(minutesToSeconds('   ')).toBeUndefined();
    expect(minutesToSeconds('abc')).toBeUndefined();
    expect(minutesToSeconds('-3')).toBeUndefined();
    expect(minutesToSeconds('NaN')).toBeUndefined();
  });

  it('秒 → 分の逆変換', () => {
    expect(secondsToMinutes(300)).toBe('5');
    expect(secondsToMinutes(90)).toBe('1.5');
    expect(secondsToMinutes(null)).toBe('');
    expect(secondsToMinutes(undefined)).toBe('');
    expect(secondsToMinutes(-1)).toBe('');
  });
});

describe('buildLibraryQuery', () => {
  it('既定状態では並び順とページングだけを送る', () => {
    expect(buildLibraryQuery(DEFAULT_LIBRARY_FILTER)).toEqual({
      sortBy: 'downloaded_at',
      sortOrder: 'desc',
      limit: 48,
      offset: 0,
    });
  });

  it('未指定の絞り込みはキーごと落とす', () => {
    const q = buildLibraryQuery(state({ q: '  ' }));
    expect('q' in q).toBe(false);
    expect('tags' in q).toBe(false);
    expect('uploaderId' in q).toBe(false);
    expect('resolution' in q).toBe(false);
    expect('isShort' in q).toBe(false);
    expect('minDuration' in q).toBe(false);
    expect('maxDuration' in q).toBe(false);
  });

  it('全条件を指定すると query.rs のパラメータ名へ写像する', () => {
    const q = buildLibraryQuery(
      state({
        q: ' ミク ',
        tags: ['VOCALOID', '初音ミク'],
        uploaderId: '1234',
        resolution: '1280x720',
        isShort: true,
        minMinutes: '2',
        maxMinutes: '10',
        sortBy: 'view_count',
        sortOrder: 'asc',
        pageSize: 24,
        page: 3,
      }),
    );
    expect(q).toEqual({
      q: 'ミク',
      tags: ['VOCALOID', '初音ミク'],
      uploaderId: '1234',
      // 投稿者を指定したら種別も必ず送る (空文字 = 種別不明グループ)
      uploaderType: '',
      resolution: '1280x720',
      isShort: true,
      minDuration: 120,
      maxDuration: 600,
      sortBy: 'view_count',
      sortOrder: 'asc',
      limit: 24,
      offset: 72,
    });
  });

  it('isShort=false は「絞り込みなし」なので送らない', () => {
    expect('isShort' in buildLibraryQuery(state({ isShort: false }))).toBe(false);
  });

  it('tags は状態の配列を共有しない (後からの破壊的変更を防ぐ)', () => {
    const s = state({ tags: ['東方'] });
    const q = buildLibraryQuery(s);
    q.tags?.push('汚染');
    expect(s.tags).toEqual(['東方']);
  });

  it('offset は page × pageSize', () => {
    expect(buildLibraryQuery(state({ page: 0, pageSize: 96 })).offset).toBe(0);
    expect(buildLibraryQuery(state({ page: 2, pageSize: 96 })).offset).toBe(192);
  });

  it('UI に出す並び順は query.rs の ALLOWED_SORT の部分集合', () => {
    // query.rs 側 ALLOWED_SORT (is_short は UI 非表示)
    const allowed = [
      'title',
      'downloaded_at',
      'posted_at',
      'view_count',
      'duration_sec',
      'play_count',
      'last_played_at',
      'mylist_count',
      'comment_count',
      'is_short',
      'random',
    ];
    for (const opt of LIBRARY_SORT_OPTIONS) {
      expect(allowed).toContain(opt.value);
    }
    expect(isLibrarySortKey('view_count')).toBe(true);
    expect(isLibrarySortKey('drop table')).toBe(false);
  });
});

describe('patchLibraryFilter', () => {
  it('page 以外を変更したらページを 0 に戻す', () => {
    const s = state({ page: 4 });
    expect(patchLibraryFilter(s, { q: 'ミク' }).page).toBe(0);
    expect(patchLibraryFilter(s, { sortBy: 'title' }).page).toBe(0);
    expect(patchLibraryFilter(s, { pageSize: 24 }).page).toBe(0);
  });

  it('ページ送りだけならページを保つ', () => {
    expect(patchLibraryFilter(state({ page: 4 }), { page: 5 }).page).toBe(5);
  });

  it('明示的な page 付きパッチはその値を採用する', () => {
    expect(patchLibraryFilter(state({ page: 4 }), { q: 'a', page: 2 }).page).toBe(2);
  });

  it('元の状態を破壊しない', () => {
    const s = state({ q: '前' });
    patchLibraryFilter(s, { q: '後' });
    expect(s.q).toBe('前');
  });
});

describe('タグフィルタ操作', () => {
  it('追加・重複無視・空白無視', () => {
    let s = state({ page: 3 });
    s = addTagFilter(s, ' 東方 ');
    expect(s.tags).toEqual(['東方']);
    expect(s.page).toBe(0);
    s = addTagFilter(s, '東方');
    expect(s.tags).toEqual(['東方']);
    expect(addTagFilter(s, '   ')).toBe(s);
  });

  it('削除', () => {
    const s = state({ tags: ['東方', 'ゆっくり'] });
    expect(removeTagFilter(s, '東方').tags).toEqual(['ゆっくり']);
    expect(removeTagFilter(s, '無い')).toBe(s);
  });

  it('clearLibraryFilters はフリーワードと並び順を残す', () => {
    const s = state({
      q: 'ミク',
      tags: ['VOCALOID'],
      uploaderId: '1',
      uploaderType: 'user',
      resolution: '1280x720',
      isShort: true,
      minMinutes: '1',
      maxMinutes: '9',
      sortBy: 'title',
      sortOrder: 'asc',
    });
    const cleared = clearLibraryFilters(s);
    expect(cleared).toMatchObject({
      q: 'ミク',
      tags: [],
      uploaderId: '',
      uploaderType: '',
      resolution: '',
      isShort: false,
      minMinutes: '',
      maxMinutes: '',
      sortBy: 'title',
      sortOrder: 'asc',
      page: 0,
    });
  });
});

describe('activeFilterCount', () => {
  it('フリーワード・並び順・ページは数えない', () => {
    expect(activeFilterCount(state({ q: 'ミク', sortBy: 'title', page: 3 }))).toBe(0);
  });

  it('絞り込み項目を数える', () => {
    expect(
      activeFilterCount(
        state({
          tags: ['A', 'B'],
          uploaderId: '1',
          resolution: '1280x720',
          isShort: true,
          minMinutes: '1',
          maxMinutes: '5',
        }),
      ),
    ).toBe(7);
  });

  it('不正な分入力は数えない', () => {
    expect(activeFilterCount(state({ minMinutes: 'abc' }))).toBe(0);
  });
});

describe('ランダム順の種', () => {
  it('種は sortBy=random のときだけ送る', () => {
    expect('randomSeed' in buildLibraryQuery(state({ sortBy: 'title', randomSeed: 42 }))).toBe(
      false,
    );
    expect(buildLibraryQuery(state({ sortBy: 'random', randomSeed: 42 })).randomSeed).toBe(42);
  });

  it('種が未設定 (0) なら送らない', () => {
    expect('randomSeed' in buildLibraryQuery(state({ sortBy: 'random', randomSeed: 0 }))).toBe(
      false,
    );
  });

  it('newRandomSeed は Rust 側の法に収まる正の整数', () => {
    for (let i = 0; i < 200; i++) {
      const seed = newRandomSeed();
      expect(Number.isInteger(seed)).toBe(true);
      expect(seed).toBeGreaterThan(0);
      expect(seed).toBeLessThanOrEqual(RANDOM_SEED_MAX);
    }
  });

  it('ページ送りでは種が保たれる (並びが変わらない)', () => {
    const s = patchLibraryFilter(state({ sortBy: 'random', randomSeed: 777 }), { page: 2 });
    const q = buildLibraryQuery(s);
    expect(q.randomSeed).toBe(777);
    expect(q.offset).toBe(2 * DEFAULT_LIBRARY_FILTER.pageSize);
  });
});

describe('toSmartPlaylistFilter', () => {
  it('オンライン検索に無い条件 (解像度・ショート) は渡さない', () => {
    const f = toSmartPlaylistFilter(
      state({
        q: 'ミク',
        tags: ['VOCALOID'],
        uploaderId: '1234',
        resolution: '1280x720',
        isShort: true,
        minMinutes: '2',
      }),
    );
    expect(f).toEqual({
      q: 'ミク',
      tags: ['VOCALOID'],
      uploaderId: '1234',
      minDuration: 120,
      sortBy: 'downloaded_at',
      sortOrder: 'desc',
    });
  });

  it('投稿者の種別 (channel/user) を引き継ぐ', () => {
    expect(
      toSmartPlaylistFilter(state({ q: 'a', uploaderId: 'ch123', uploaderType: 'channel' }))
        .uploaderType,
    ).toBe('channel');
    expect(
      toSmartPlaylistFilter(state({ q: 'a', uploaderId: 'u1', uploaderType: 'user' })).uploaderType,
    ).toBe('user');
  });

  it('種別不明なら渡さない (呼び出し先は user 扱い)', () => {
    const f = toSmartPlaylistFilter(state({ q: 'a', uploaderId: 'u1', uploaderType: '' }));
    expect('uploaderType' in f).toBe(false);
  });

  it('チャンネル投稿は channelId で検索される', () => {
    const f = normalizeFilter(
      toSmartPlaylistFilter(state({ q: 'a', uploaderId: 'ch123', uploaderType: 'channel' })),
    );
    const query = filterToSearchQuery(f);
    expect(query.filters).toContainEqual({ field: 'channelId', op: 'eq', value: 'ch123' });
  });

  it('ユーザ投稿 / 種別不明は userId で検索される', () => {
    for (const uploaderType of ['user', '']) {
      const f = normalizeFilter(
        toSmartPlaylistFilter(state({ q: 'a', uploaderId: '1', uploaderType })),
      );
      expect(filterToSearchQuery(f).filters).toContainEqual({
        field: 'userId',
        op: 'eq',
        value: '1',
      });
    }
  });
});

describe('checkSmartPlaylistSave', () => {
  it('キーワードかタグが要る (投稿者だけでは保存させない)', () => {
    expect(checkSmartPlaylistSave(state({ q: 'ミク' })).canSave).toBe(true);
    expect(checkSmartPlaylistSave(state({ tags: ['東方'] })).canSave).toBe(true);
    // Snapshot Search は q 必須で、投稿者 ID からは q を組み立てられない
    expect(checkSmartPlaylistSave(state({ uploaderId: '1234' })).canSave).toBe(false);
    expect(checkSmartPlaylistSave(DEFAULT_LIBRARY_FILTER).canSave).toBe(false);
  });

  it('落ちる条件を列挙する (解像度・ショート・ローカル専用の並び順)', () => {
    const { dropped } = checkSmartPlaylistSave(
      state({ q: 'ミク', resolution: '1280x720', isShort: true, sortBy: 'downloaded_at' }),
    );
    expect(dropped).toEqual(['解像度', 'ショート', '並び順「DL 日時」']);
  });

  it('最長 0 分は保存されないので落ちる条件に挙げる', () => {
    // normalizeFilter は 0 を「未指定」として捨てるため、黙って全再生時間が
    // 対象になってしまう (ローカルでは duration_sec <= 0 の絞り込み)。
    const { dropped } = checkSmartPlaylistSave(
      state({ q: 'ミク', maxMinutes: '0', sortBy: 'view_count' }),
    );
    expect(dropped).toEqual(['最長 0 分']);
    // 最短 0 分はローカルでも条件として無意味なので警告しない。
    expect(
      checkSmartPlaylistSave(state({ q: 'ミク', minMinutes: '0', sortBy: 'view_count' })).dropped,
    ).toEqual([]);
  });

  it('キーワードの対象範囲が変わる点を注意書きにする', () => {
    // ローカルはタイトル/タグ/保存コメント、オンラインはタイトル/タグのみ。
    const { notes } = checkSmartPlaylistSave(state({ q: 'ミク', sortBy: 'view_count' }));
    expect(notes).toEqual(['キーワードの対象はタイトルとタグのみ (保存コメントは対象外)']);
    // キーワード無し (タグのみ) なら注意書きは要らない。
    expect(checkSmartPlaylistSave(state({ tags: ['東方'], sortBy: 'view_count' })).notes).toEqual(
      [],
    );
  });

  it('種別不明の投稿者は userId 扱いになる旨を注意書きにする', () => {
    const { notes } = checkSmartPlaylistSave(
      state({ tags: ['東方'], uploaderId: '123', uploaderType: '', sortBy: 'view_count' }),
    );
    expect(notes).toEqual(['投稿者の種別が不明のため、ユーザー ID (userId) として検索します']);
    // 種別が分かっていれば注意書きは出ない。
    for (const uploaderType of ['user', 'channel']) {
      expect(
        checkSmartPlaylistSave(
          state({ tags: ['東方'], uploaderId: '123', uploaderType, sortBy: 'view_count' }),
        ).notes,
      ).toEqual([]);
    }
    // 投稿者未指定なら関係ない。
    expect(checkSmartPlaylistSave(state({ tags: ['東方'], sortBy: 'view_count' })).notes).toEqual(
      [],
    );
  });

  it('オンラインでも使える並び順なら並び順は落ちない', () => {
    expect(checkSmartPlaylistSave(state({ q: 'ミク', sortBy: 'view_count' })).dropped).toEqual([]);
  });

  it('投稿者の種別はローカル検索にも渡す (ID の名前空間が別のため)', () => {
    const q = buildLibraryQuery(state({ uploaderId: '2632720', uploaderType: 'channel' }));
    expect(q.uploaderId).toBe('2632720');
    expect(q.uploaderType).toBe('channel');
  });

  it('種別不明の投稿者は空文字を送る (Rust 側で uploader_type IS NULL になる)', () => {
    // 一覧が種別ごとに束ねている以上、種別不明の行を選んだ時に他の名前空間の
    // 動画まで出てはいけない。
    const q = buildLibraryQuery(state({ uploaderId: '123', uploaderType: '' }));
    expect(q.uploaderId).toBe('123');
    expect(q.uploaderType).toBe('');
  });

  it('投稿者未指定なら種別も送らない', () => {
    expect('uploaderType' in buildLibraryQuery(state({ uploaderType: 'channel' }))).toBe(false);
  });

  it('ローカル専用の並び順はすべて警告対象', () => {
    for (const sortBy of [
      'downloaded_at',
      'title',
      'play_count',
      'last_played_at',
      'random',
    ] as const) {
      const { dropped } = checkSmartPlaylistSave(state({ q: 'ミク', sortBy }));
      expect(dropped).toEqual([`並び順「${sortLabel(sortBy)}」`]);
    }
  });

  it('ローカル専用の並び順は smartPlaylists 側の正規化で落ちる', () => {
    const f = normalizeFilter(toSmartPlaylistFilter(state({ q: 'ミク', sortBy: 'downloaded_at' })));
    expect(f.sortBy).toBeUndefined();
    expect(f.q).toBe('ミク');
  });

  it('オンラインでも使える並び順は残る', () => {
    const f = normalizeFilter(
      toSmartPlaylistFilter(state({ q: 'ミク', sortBy: 'view_count', sortOrder: 'asc' })),
    );
    expect(f.sortBy).toBe('view_count');
    expect(f.sortOrder).toBe('asc');
  });
});

describe('コメント横断検索のガード', () => {
  it('3 文字以上でだけ検索できる', () => {
    expect(canSearchComments('弾幕で')).toBe(true);
    expect(canSearchComments('ｗｗ')).toBe(false);
    expect(canSearchComments('  a  ')).toBe(false);
    expect(canSearchComments('')).toBe(false);
  });

  it('サロゲートペアも 1 文字として数える (Rust の chars().count() と同じ)', () => {
    expect(canSearchComments('😀😀')).toBe(false);
    expect(canSearchComments('😀😀😀')).toBe(true);
  });
});

describe('コメントヒットからの頭出しリンク', () => {
  it('vpos(ms) を秒に落として ?t= を付ける', () => {
    expect(commentHitHref('sm9', 0)).toBe('/library/sm9?from=library&t=0');
    expect(commentHitHref('sm9', 12_800)).toBe('/library/sm9?from=library&t=12');
    expect(commentHitHref('sm9', -500)).toBe('/library/sm9?from=library&t=0');
    expect(commentHitHref('sm9', Number.NaN)).toBe('/library/sm9?from=library&t=0');
  });

  it('動画 ID はエスケープする', () => {
    expect(commentHitHref('so 1/2', 1000)).toBe('/library/so%201%2F2?from=library&t=1');
  });

  it('?t= の解釈: 秒へ、不正値は null', () => {
    expect(parseStartTime('12')).toBe(12);
    expect(parseStartTime('12.9')).toBe(12);
    expect(parseStartTime('0')).toBe(0);
    expect(parseStartTime(null)).toBeNull();
    expect(parseStartTime('')).toBeNull();
    expect(parseStartTime('  ')).toBeNull();
    expect(parseStartTime('abc')).toBeNull();
    expect(parseStartTime('-5')).toBeNull();
    expect(parseStartTime('Infinity')).toBeNull();
  });

  it('リンクと解釈が往復する', () => {
    const href = commentHitHref('sm500873', 95_400);
    const t = new URL(href, 'http://localhost').searchParams.get('t');
    expect(parseStartTime(t)).toBe(95);
  });
});

describe('ページャ表示', () => {
  it('件数レンジ', () => {
    expect(resultRangeLabel(120, 48, 48)).toBe('120 件中 49–96 件');
    expect(resultRangeLabel(1234, 0, 48)).toBe('1,234 件中 1–48 件');
    expect(resultRangeLabel(0, 0, 0)).toBe('0 件');
  });

  it('総ページ数は最低 1', () => {
    expect(pageCount(0, 48)).toBe(1);
    expect(pageCount(48, 48)).toBe(1);
    expect(pageCount(49, 48)).toBe(2);
    expect(pageCount(100, 0)).toBe(1);
  });

  it('件数が減ったら最終ページへ丸める', () => {
    // 100 件 / 48 = 3 ページ (0,1,2) なので末尾は 2
    expect(clampPage(state({ page: 5, pageSize: 48 }), 100).page).toBe(2);
    expect(clampPage(state({ page: 0, pageSize: 48 }), 0).page).toBe(0);
    const s = state({ page: 1, pageSize: 48 });
    expect(clampPage(s, 100)).toBe(s); // 範囲内なら同一参照
  });
});

describe('summarizeLibraryFilter', () => {
  it('条件なし', () => {
    expect(summarizeLibraryFilter(DEFAULT_LIBRARY_FILTER)).toBe('条件なし');
  });

  it('指定した条件だけを並べる', () => {
    expect(
      summarizeLibraryFilter(
        state({ q: 'ミク', tags: ['VOCALOID', '初音ミク'], resolution: '1280x720' }),
      ),
    ).toBe('"ミク" / タグ: VOCALOID + 初音ミク / 1280x720');
    expect(summarizeLibraryFilter(state({ minMinutes: '2', maxMinutes: '10' }))).toBe('2〜10 分');
    expect(summarizeLibraryFilter(state({ minMinutes: '2' }))).toBe('2 分以上');
    expect(summarizeLibraryFilter(state({ maxMinutes: '10' }))).toBe('10 分以下');
  });
});

describe('normalizeLibraryFilter', () => {
  it('不正な値を既定へ倒す', () => {
    const s = normalizeLibraryFilter({
      mode: 'nope',
      q: 42,
      tags: ['A', '', '  A  ', 7],
      uploaderId: ' 1234 ',
      uploaderType: 'channel',
      resolution: ' 1280x720 ',
      isShort: 'yes',
      minMinutes: 'abc',
      maxMinutes: 90,
      sortBy: 'DROP TABLE videos',
      sortOrder: 'sideways',
      randomSeed: -5,
      pageSize: 999,
      page: -3,
    });
    expect(s).toEqual({
      mode: 'videos',
      q: '',
      tags: ['A'],
      uploaderId: '1234',
      uploaderType: 'channel',
      resolution: '1280x720',
      isShort: false,
      minMinutes: '',
      maxMinutes: '90',
      sortBy: 'downloaded_at',
      sortOrder: 'desc',
      randomSeed: 0,
      pageSize: DEFAULT_LIBRARY_FILTER.pageSize,
      page: 0,
    });
  });

  it('投稿者が選ばれていない / 不明な種別は落とす', () => {
    expect(normalizeLibraryFilter({ uploaderType: 'channel' }).uploaderType).toBe('');
    expect(normalizeLibraryFilter({ uploaderId: 'u1', uploaderType: 'both' }).uploaderType).toBe(
      '',
    );
    expect(normalizeLibraryFilter({ uploaderId: 'u1', uploaderType: 'user' }).uploaderType).toBe(
      'user',
    );
  });

  it('ランダム順の種は範囲外を捨てる', () => {
    expect(normalizeLibraryFilter({ randomSeed: 12345 }).randomSeed).toBe(12345);
    expect(normalizeLibraryFilter({ randomSeed: 0 }).randomSeed).toBe(0);
    expect(normalizeLibraryFilter({ randomSeed: -1 }).randomSeed).toBe(0);
    expect(normalizeLibraryFilter({ randomSeed: RANDOM_SEED_MAX + 1 }).randomSeed).toBe(0);
    expect(normalizeLibraryFilter({ randomSeed: 'abc' }).randomSeed).toBe(0);
  });

  it('null / 非オブジェクトは既定値', () => {
    expect(normalizeLibraryFilter(null)).toEqual(DEFAULT_LIBRARY_FILTER);
    expect(normalizeLibraryFilter('{}')).toEqual(DEFAULT_LIBRARY_FILTER);
  });

  it('妥当な値はそのまま通る', () => {
    const src = state({
      mode: 'comments',
      q: '弾幕',
      tags: ['東方'],
      pageSize: LIBRARY_PAGE_SIZES[2],
      page: 2,
      sortBy: 'random',
      sortOrder: 'asc',
    });
    expect(normalizeLibraryFilter(src)).toEqual(src);
  });
});

describe('sessionStorage への保存・復元', () => {
  beforeEach(() => {
    clearLibraryFilter();
  });

  it('保存した状態を復元できる', () => {
    const s = state({ q: 'ミク', tags: ['VOCALOID'], page: 2, sortBy: 'view_count' });
    saveLibraryFilter(s);
    expect(loadLibraryFilter()).toEqual(s);
  });

  it('未保存なら既定値', () => {
    expect(loadLibraryFilter()).toEqual(DEFAULT_LIBRARY_FILTER);
  });

  it('壊れた JSON でも既定値へフォールバックする', () => {
    sessionStorage.setItem('nndd:libraryQuery', '{ not json');
    expect(loadLibraryFilter()).toEqual(DEFAULT_LIBRARY_FILTER);
  });

  it('古い / 改竄された値は正規化して読む', () => {
    sessionStorage.setItem(
      'nndd:libraryQuery',
      JSON.stringify({ sortBy: 'evil', pageSize: 100000, tags: 'nope' }),
    );
    const s = loadLibraryFilter();
    expect(s.sortBy).toBe('downloaded_at');
    expect(s.pageSize).toBe(DEFAULT_LIBRARY_FILTER.pageSize);
    expect(s.tags).toEqual([]);
  });
});
