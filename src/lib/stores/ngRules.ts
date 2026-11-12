/**
 * NG (block) rules — local storage backed.
 *
 * Mirrors the `ng_rules` table from CLAUDE.md so the eventual SQLite
 * migration is mechanical. Filtering is done at draw-time / render-time
 * (the source data is never mutated) so toggling a rule applies
 * immediately without re-fetching.
 */

import { createPersistedStore } from './persistedStore';

export type NgTargetType =
  | 'video_title'
  | 'uploader'
  | 'uploader_name'
  | 'video_id'
  | 'tag'
  | 'category'
  | 'comment_body'
  | 'comment_user';

export type NgMatchMode = 'exact' | 'partial' | 'regex';

export type NgScope = 'ranking' | 'search' | 'comment';

/** `tag` ルール専用: ロックタグ / ユーザータグ / 両方 のどれにマッチさせるか。 */
export type NgTagKind = 'lock' | 'user' | 'both';

export type NgRule = {
  id: string;
  targetType: NgTargetType;
  matchMode: NgMatchMode;
  pattern: string;
  scopeRanking: boolean;
  scopeSearch: boolean;
  scopeComment: boolean;
  enabled: boolean;
  /** `targetType === 'tag'` のときのみ意味を持つ。未指定なら `both`。 */
  tagKind?: NgTagKind;
  /** 大文字小文字を区別しない (ニコニコの NG 設定から取り込んだ word など)。
   *  未指定なら区別する (従来どおり)。 */
  ignoreCase?: boolean;
  /** 改行を取り除いてから照合する (ニコニコの NG 設定から取り込んだ word。
   *  ニコニコは改行を除いた本文で判定する)。未指定なら従来どおり。 */
  ignoreNewlines?: boolean;
  note?: string;
  createdAt: number;
  hitCount: number;
  lastHitAt?: number;
};

const KEY = 'nndd:ngRules';

const {
  read,
  write,
  subscribe: subscribeNgRules,
} = createPersistedStore<NgRule[]>({
  key: KEY,
  fallback: () => [],
  validate: (parsed) => (Array.isArray(parsed) ? (parsed as NgRule[]) : []),
});
export { subscribeNgRules };

export function listNgRules(): NgRule[] {
  return read();
}

export function addNgRule(rule: Omit<NgRule, 'id' | 'createdAt' | 'hitCount'>): NgRule {
  const r: NgRule = {
    ...rule,
    id: `ng_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
    createdAt: Date.now(),
    hitCount: 0,
  };
  const list = read();
  list.push(r);
  write(list);
  return r;
}

export function updateNgRule(id: string, patch: Partial<NgRule>): void {
  const list = read();
  const idx = list.findIndex((r) => r.id === id);
  if (idx < 0) return;
  list[idx] = { ...list[idx], ...patch, id: list[idx].id };
  write(list);
}

export function deleteNgRule(id: string): void {
  const list = read().filter((r) => r.id !== id);
  write(list);
}

export function clearAllNgRules(): void {
  write([]);
}

/** Bulk import — used by CSV/JSON loaders. Skips invalid entries. */
export function importNgRules(rules: Partial<NgRule>[]): number {
  const list = read();
  let added = 0;
  for (const r of rules) {
    if (!r.targetType || !r.matchMode || !r.pattern) continue;
    list.push({
      id: `ng_${Date.now()}_${Math.random().toString(36).slice(2, 8)}_${added}`,
      targetType: r.targetType,
      matchMode: r.matchMode,
      pattern: r.pattern,
      scopeRanking: r.scopeRanking ?? false,
      scopeSearch: r.scopeSearch ?? true,
      scopeComment: r.scopeComment ?? true,
      enabled: r.enabled ?? true,
      tagKind: r.tagKind,
      ignoreCase: r.ignoreCase === true ? true : undefined,
      ignoreNewlines: r.ignoreNewlines === true ? true : undefined,
      note: r.note,
      createdAt: Date.now(),
      hitCount: 0,
    });
    added += 1;
  }
  write(list);
  return added;
}

/* ------------------------------------------------------------------------ */
/*  ニコニコ側の視聴者 NG 設定の取り込み (オプション機能)。                    */
/* ------------------------------------------------------------------------ */

/** `fetch_niconico_ng_settings` の 1 件 (`type` は `word` / `id` / `command`)。 */
export type NiconicoNgItemLike = { type: string; source: string };

export type NiconicoNgImportResult = {
  /** 新しく足したルール数 */
  added: number;
  /** 同じルールが既にあって足さなかった数 */
  duplicates: number;
  /** ローカルに対応する種別が無くて取り込めなかった数 (`command` など) */
  unsupported: number;
};

export const NICONICO_NG_NOTE = 'ニコニコの NG 設定から取り込み';

/**
 * ニコニコの NG 1 件をローカルのルールへ写す。判定は Web プレイヤーに合わせる:
 * `word` は改行を除いた本文の部分一致で大文字小文字を区別しない、`id` はコメントの
 * userId と一致。`command` (コマンドの全トークン一致) はローカルに対応する種別が
 * 無いので `null`。
 */
export function niconicoNgToRule(
  item: NiconicoNgItemLike,
): Omit<NgRule, 'id' | 'createdAt' | 'hitCount'> | null {
  if (!item.source || !item.source.trim()) return null;
  const base = {
    pattern: item.source,
    scopeComment: true,
    scopeRanking: false,
    scopeSearch: false,
    enabled: true,
    note: NICONICO_NG_NOTE,
  };
  switch (item.type) {
    case 'word':
      return {
        ...base,
        targetType: 'comment_body',
        matchMode: 'partial',
        ignoreCase: true,
        ignoreNewlines: true,
      };
    case 'id':
      return { ...base, targetType: 'comment_user', matchMode: 'exact' };
    default:
      return null;
  }
}

/**
 * ニコニコの NG をローカルへ取り込む。足すだけで、ローカルのルールは消さない。
 * 照合のしかた (ターゲット・モード・パターン・大文字小文字・改行の扱い) まで同じ
 * ルールが既にある時だけ足さない (例えば大文字小文字を区別する同じパターンのルールが
 * あっても、区別しないルールは別に足す)。
 */
export function importNiconicoNg(items: readonly NiconicoNgItemLike[]): NiconicoNgImportResult {
  const list = read();
  const keyOf = (
    r: Pick<NgRule, 'targetType' | 'matchMode' | 'pattern' | 'ignoreCase' | 'ignoreNewlines'>,
  ) =>
    [
      r.targetType,
      r.matchMode,
      r.pattern,
      r.ignoreCase === true ? 'i' : '',
      r.ignoreNewlines === true ? 'n' : '',
    ].join('\u0000');
  const existing = new Set(list.map(keyOf));
  const result: NiconicoNgImportResult = { added: 0, duplicates: 0, unsupported: 0 };
  const now = Date.now();
  for (const item of items) {
    const rule = niconicoNgToRule(item);
    if (!rule) {
      result.unsupported += 1;
      continue;
    }
    const key = keyOf(rule);
    if (existing.has(key)) {
      result.duplicates += 1;
      continue;
    }
    existing.add(key);
    list.push({
      ...rule,
      id: `ng_${now}_${Math.random().toString(36).slice(2, 8)}_${result.added}`,
      createdAt: now,
      hitCount: 0,
    });
    result.added += 1;
  }
  if (result.added > 0) write(list);
  return result;
}

/** Compile a rule into a matcher function. Invalid regex returns `null`. */
export function compileRule(rule: NgRule): ((value: string) => boolean) | null {
  if (!rule.enabled) return null;
  const match = compileMatcher(rule);
  if (!match || rule.ignoreNewlines !== true) return match;
  return (v) => match(v.replace(/[\r\n]/g, ''));
}

function compileMatcher(rule: NgRule): ((value: string) => boolean) | null {
  const ignoreCase = rule.ignoreCase === true;
  switch (rule.matchMode) {
    case 'exact': {
      if (!ignoreCase) return (v) => v === rule.pattern;
      const pattern = rule.pattern.toLowerCase();
      return (v) => v.toLowerCase() === pattern;
    }
    case 'partial': {
      if (!ignoreCase) return (v) => v.includes(rule.pattern);
      const pattern = rule.pattern.toLowerCase();
      return (v) => v.toLowerCase().includes(pattern);
    }
    case 'regex': {
      try {
        const re = new RegExp(rule.pattern, ignoreCase ? 'i' : '');
        return (v) => re.test(v);
      } catch {
        return null;
      }
    }
  }
}

export function isValidRegex(src: string): boolean {
  try {
    new RegExp(src);
    return true;
  } catch {
    return false;
  }
}

/** Increment hit_count for one rule. Debounced lightly via batching. */
const pendingHits = new Map<string, number>();
let flushTimer: ReturnType<typeof setTimeout> | null = null;
export function recordHit(id: string) {
  pendingHits.set(id, (pendingHits.get(id) ?? 0) + 1);
  if (flushTimer) return;
  flushTimer = setTimeout(() => {
    flushTimer = null;
    if (pendingHits.size === 0) return;
    const list = read();
    let dirty = false;
    const now = Date.now();
    for (const r of list) {
      const inc = pendingHits.get(r.id);
      if (inc) {
        r.hitCount += inc;
        r.lastHitAt = now;
        dirty = true;
      }
    }
    pendingHits.clear();
    if (dirty) write(list);
  }, 1000);
}

/* ------------------------------------------------------------------------ */
/*  Apply helpers — kept here so callers don't reinvent the matching logic. */
/* ------------------------------------------------------------------------ */

export type SearchHitLike = {
  contentId?: string;
  title?: string;
  tags?: string;
  userId?: number;
  channelId?: number;
  categoryTags?: string;
  genre?: string;
};

/** Returns true if the hit should be HIDDEN under the search scope. */
export function isHitBlocked(
  rules: NgRule[],
  hit: SearchHitLike,
): { blocked: boolean; ruleId?: string } {
  for (const r of rules) {
    if (!r.enabled || !r.scopeSearch) continue;
    const match = compileRule(r);
    if (!match) continue;
    let value: string | undefined;
    switch (r.targetType) {
      case 'video_title':
        value = hit.title;
        break;
      case 'video_id':
        value = hit.contentId;
        break;
      case 'tag':
        if (hit.tags) {
          for (const tag of hit.tags.split(/\s+/).filter(Boolean)) {
            if (match(tag)) return { blocked: true, ruleId: r.id };
          }
        }
        continue;
      case 'category':
        value = hit.categoryTags ?? hit.genre;
        break;
      case 'uploader':
        value =
          hit.userId != null
            ? `user/${hit.userId}`
            : hit.channelId != null
              ? `channel/${hit.channelId}`
              : undefined;
        break;
      default:
        continue;
    }
    if (value != null && match(value)) return { blocked: true, ruleId: r.id };
  }
  return { blocked: false };
}

export type CommentLike = {
  content: string;
  userId?: string;
};

export function isCommentBlocked(
  rules: NgRule[],
  c: CommentLike,
): { blocked: boolean; ruleId?: string } {
  for (const r of rules) {
    if (!r.enabled || !r.scopeComment) continue;
    const match = compileRule(r);
    if (!match) continue;
    if (r.targetType === 'comment_body') {
      if (match(c.content)) return { blocked: true, ruleId: r.id };
    } else if (r.targetType === 'comment_user') {
      if (c.userId && match(c.userId)) return { blocked: true, ruleId: r.id };
    }
  }
  return { blocked: false };
}

/** Pure filtering helper. Hit counters must be updated outside derived/render calculations. */
export function filterSearchHits<T extends SearchHitLike>(rules: NgRule[], hits: T[]): T[] {
  return hits.filter((h) => !isHitBlocked(rules, h).blocked);
}

export function filterComments<T extends CommentLike>(rules: NgRule[], comments: T[]): T[] {
  return comments.filter((c) => !isCommentBlocked(rules, c).blocked);
}

/* ------------------------------------------------------------------------ */
/*  Ranking-specific apply helpers.                                         */
/* ------------------------------------------------------------------------ */

export type RankingItemLike = {
  id: string;
  title: string;
  owner?: {
    id?: string | null;
    name?: string | null;
    ownerType?: string | null;
  } | null;
};

export type RankingTagInfo = {
  name: string;
  isLocked: boolean;
};

/**
 * ランキング項目を NG ルールでフィルタする際のマッチ判定。
 * `tags` が `undefined` の場合は「タグ未取得」として扱い、タグ系ルールはスキップする。
 */
export function isRankingItemBlocked(
  rules: NgRule[],
  item: RankingItemLike,
  tags?: ReadonlyArray<RankingTagInfo>,
): { blocked: boolean; ruleId?: string } {
  for (const r of rules) {
    if (!r.enabled || !r.scopeRanking) continue;
    const match = compileRule(r);
    if (!match) continue;

    switch (r.targetType) {
      case 'video_id':
        if (match(item.id)) return { blocked: true, ruleId: r.id };
        break;
      case 'video_title':
        if (match(item.title)) return { blocked: true, ruleId: r.id };
        break;
      case 'uploader': {
        // user/12345, channel/ch12345, 数字のみ なども許容。
        const ownerId = item.owner?.id;
        if (!ownerId) break;
        const ownerType = item.owner?.ownerType ?? 'user';
        const candidates = [ownerId, `${ownerType}/${ownerId}`, `user/${ownerId}`];
        if (candidates.some((c) => match(c))) return { blocked: true, ruleId: r.id };
        break;
      }
      case 'uploader_name': {
        const name = item.owner?.name;
        if (name && match(name)) return { blocked: true, ruleId: r.id };
        break;
      }
      case 'tag': {
        if (!tags) break;
        const kind: NgTagKind = r.tagKind ?? 'both';
        for (const t of tags) {
          if (kind === 'lock' && !t.isLocked) continue;
          if (kind === 'user' && t.isLocked) continue;
          if (match(t.name)) return { blocked: true, ruleId: r.id };
        }
        break;
      }
      default:
        break;
    }
  }
  return { blocked: false };
}
