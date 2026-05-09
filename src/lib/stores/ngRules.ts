/**
 * NG (block) rules — local storage backed.
 *
 * Mirrors the `ng_rules` table from CLAUDE.md so the eventual SQLite
 * migration is mechanical. Filtering is done at draw-time / render-time
 * (the source data is never mutated) so toggling a rule applies
 * immediately without re-fetching.
 */

export type NgTargetType =
  | 'video_title'
  | 'uploader'
  | 'video_id'
  | 'tag'
  | 'category'
  | 'comment_body'
  | 'comment_user';

export type NgMatchMode = 'exact' | 'partial' | 'regex';

export type NgScope = 'ranking' | 'search' | 'comment';

export type NgRule = {
  id: string;
  targetType: NgTargetType;
  matchMode: NgMatchMode;
  pattern: string;
  scopeRanking: boolean;
  scopeSearch: boolean;
  scopeComment: boolean;
  enabled: boolean;
  note?: string;
  createdAt: number;
  hitCount: number;
  lastHitAt?: number;
};

const KEY = 'nndd:ngRules';

const listeners = new Set<() => void>();
function notify() {
  for (const fn of listeners) fn();
}
export function subscribeNgRules(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function read(): NgRule[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as NgRule[];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

function write(list: NgRule[]) {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(KEY, JSON.stringify(list));
  notify();
}

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
      note: r.note,
      createdAt: Date.now(),
      hitCount: 0,
    });
    added += 1;
  }
  write(list);
  return added;
}

/** Compile a rule into a matcher function. Invalid regex returns `null`. */
export function compileRule(rule: NgRule): ((value: string) => boolean) | null {
  if (!rule.enabled) return null;
  switch (rule.matchMode) {
    case 'exact':
      return (v) => v === rule.pattern;
    case 'partial':
      return (v) => v.includes(rule.pattern);
    case 'regex': {
      try {
        const re = new RegExp(rule.pattern);
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
export function isHitBlocked(rules: NgRule[], hit: SearchHitLike): { blocked: boolean; ruleId?: string } {
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
        value = hit.userId != null ? `user/${hit.userId}` : hit.channelId != null ? `channel/${hit.channelId}` : undefined;
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

export function isCommentBlocked(rules: NgRule[], c: CommentLike): { blocked: boolean; ruleId?: string } {
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
  const out: T[] = [];
  for (const h of hits) {
    const r = isHitBlocked(rules, h);
    if (r.blocked) {
      continue;
    }
    out.push(h);
  }
  return out;
}

export function filterComments<T extends CommentLike>(rules: NgRule[], comments: T[]): T[] {
  const out: T[] = [];
  for (const c of comments) {
    const r = isCommentBlocked(rules, c);
    if (r.blocked) {
      continue;
    }
    out.push(c);
  }
  return out;
}
