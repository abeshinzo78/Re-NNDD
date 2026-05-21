import { beforeEach, describe, expect, test } from 'vitest';
import {
  advanceQueue,
  clearQueue,
  currentItem,
  getQueue,
  hasNextInQueue,
  itemHref,
  nextItem,
  prevItem,
  rewindQueue,
  setQueue,
  setQueueIndexByVideoId,
  subscribeQueue,
  type PlaybackQueueItem,
} from './playbackQueue';

function makeItems(n: number, source: 'online' | 'local' = 'online'): PlaybackQueueItem[] {
  return Array.from({ length: n }, (_, i) => ({
    videoId: `sm${i + 1}`,
    title: `Video ${i + 1}`,
    source,
  }));
}

beforeEach(() => {
  localStorage.clear();
});

describe('playbackQueue', () => {
  test('getQueue returns null when nothing stored', () => {
    expect(getQueue()).toBeNull();
  });

  test('setQueue persists and is readable', () => {
    setQueue('series', 'srs1', 'My Series', makeItems(3));
    const q = getQueue();
    expect(q).not.toBeNull();
    expect(q?.context).toBe('series');
    expect(q?.label).toBe('My Series');
    expect(q?.items).toHaveLength(3);
    expect(q?.index).toBe(0);
  });

  test('setQueue with empty items clears the queue', () => {
    setQueue('mylist', 'a', 'X', makeItems(2));
    expect(getQueue()).not.toBeNull();
    setQueue('mylist', 'b', 'Y', []);
    expect(getQueue()).toBeNull();
  });

  test('setQueue with startIndex clamps to range', () => {
    setQueue('series', 's1', 'S', makeItems(3), 99);
    expect(getQueue()?.index).toBe(2);
    setQueue('series', 's2', 'S', makeItems(3), -5);
    expect(getQueue()?.index).toBe(0);
  });

  test('advanceQueue advances and returns the new item', () => {
    setQueue('series', 's', 'S', makeItems(3));
    const a = advanceQueue();
    expect(a?.videoId).toBe('sm2');
    expect(getQueue()?.index).toBe(1);
  });

  test('advanceQueue returns null at the end and does not move index', () => {
    setQueue('series', 's', 'S', makeItems(2));
    advanceQueue();
    expect(getQueue()?.index).toBe(1);
    expect(advanceQueue()).toBeNull();
    expect(getQueue()?.index).toBe(1);
  });

  test('rewindQueue returns previous and stops at 0', () => {
    setQueue('series', 's', 'S', makeItems(3), 2);
    expect(rewindQueue()?.videoId).toBe('sm2');
    expect(rewindQueue()?.videoId).toBe('sm1');
    expect(rewindQueue()).toBeNull();
    expect(getQueue()?.index).toBe(0);
  });

  test('currentItem / nextItem / prevItem reflect the index', () => {
    setQueue('series', 's', 'S', makeItems(3), 1);
    expect(currentItem()?.videoId).toBe('sm2');
    expect(nextItem()?.videoId).toBe('sm3');
    expect(prevItem()?.videoId).toBe('sm1');
  });

  test('nextItem returns null at the end', () => {
    setQueue('series', 's', 'S', makeItems(2), 1);
    expect(nextItem()).toBeNull();
  });

  test('setQueueIndexByVideoId aligns to current video', () => {
    setQueue('series', 's', 'S', makeItems(3));
    setQueueIndexByVideoId('sm3');
    expect(getQueue()?.index).toBe(2);
  });

  test('setQueueIndexByVideoId is a no-op for unknown id', () => {
    setQueue('series', 's', 'S', makeItems(3), 1);
    setQueueIndexByVideoId('not-in-queue');
    expect(getQueue()?.index).toBe(1);
  });

  test('clearQueue removes the queue', () => {
    setQueue('series', 's', 'S', makeItems(3));
    clearQueue();
    expect(getQueue()).toBeNull();
  });

  test('subscribeQueue fires on write and respects unsubscribe', () => {
    let count = 0;
    const unsub = subscribeQueue(() => count++);
    setQueue('series', 's', 'S', makeItems(2));
    expect(count).toBe(1);
    advanceQueue();
    expect(count).toBe(2);
    unsub();
    clearQueue();
    expect(count).toBe(2);
  });

  test('hasNextInQueue is true mid-queue and false at the tail / absent', () => {
    expect(hasNextInQueue('sm1')).toBe(false); // no queue
    setQueue('series', 's', 'S', makeItems(3));
    expect(hasNextInQueue('sm1')).toBe(true);
    expect(hasNextInQueue('sm2')).toBe(true);
    expect(hasNextInQueue('sm3')).toBe(false);
    expect(hasNextInQueue('not-in-queue')).toBe(false);
  });

  test('itemHref points at /video/ for online and /library/ for local', () => {
    const online: PlaybackQueueItem = { videoId: 'sm1', source: 'online' };
    const local: PlaybackQueueItem = { videoId: 'sm2', source: 'local' };
    expect(itemHref(online)).toBe('/video/sm1?from=queue');
    expect(itemHref(local)).toBe('/library/sm2?from=queue');
    expect(itemHref(online, false)).toBe('/video/sm1');
  });

  test('survives malformed localStorage', () => {
    localStorage.setItem('nndd:playback-queue', '{not json');
    expect(getQueue()).toBeNull();
  });
});
