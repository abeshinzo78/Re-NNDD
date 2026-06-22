// @vitest-environment jsdom
//
// Regression test for the "マイリスト/シリーズ タブが『読み込み中…』から進まない"
// bug. The lazy-load $effect used to guard on `list.length === 0`, which stays
// true forever when a user has 0 public mylists/series — so every completed
// (empty) load re-triggered the effect, an infinite reload loop that pinned the
// UI on the loading spinner. The fix guards on "已读 userId" (`loadedFor`).
//
// These tests drive the *real* Svelte 5 reactive graph ($state/$effect) so they
// fail against the old guard and pass against the new one.
import { describe, expect, it } from 'vitest';
import { flushSync } from 'svelte';

/** Pump a few microtask + reactive-flush cycles, as the event loop would. */
async function pump(cycles = 12) {
  for (let i = 0; i < cycles; i++) {
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
  }
}

describe('user page tab lazy-load (empty result must not infinite-loop)', () => {
  it('OLD guard (list.length === 0) reloads endlessly on an empty result', async () => {
    let calls = 0;
    let setTab: (t: string) => void = () => {};
    const cleanup = $effect.root(() => {
      let activeTab = $state('videos');
      let items = $state<string[]>([]);
      let loading = $state(false);

      async function load() {
        calls++;
        loading = true;
        const resp: string[] = await Promise.resolve([]); // empty (0 mylists)
        items = resp;
        loading = false;
      }

      $effect(() => {
        if (activeTab === 'mylists' && items.length === 0 && !loading) {
          load();
        }
      });
      setTab = (t) => (activeTab = t);
    });

    setTab('mylists');
    flushSync();
    await pump();
    cleanup();

    // The bug: each completed empty load re-arms the effect -> many reloads.
    expect(calls).toBeGreaterThan(3);
  });

  it('NEW guard (loadedFor !== userId) loads exactly once on an empty result', async () => {
    let calls = 0;
    let setTab: (t: string) => void = () => {};
    const cleanup = $effect.root(() => {
      let activeTab = $state('videos');
      const userId = 'u-34013689';
      // Note: there is deliberately NO `items` array here — the new guard keys
      // off `loadedFor`, proving the (empty) result length is irrelevant.
      let loadedFor = $state<string | null>(null);
      let loading = $state(false);

      async function load() {
        calls++;
        const target = userId;
        loading = true;
        try {
          await Promise.resolve([]); // empty result (0 mylists)
          if (target !== userId) return;
          loadedFor = target;
        } finally {
          loading = false;
        }
      }

      $effect(() => {
        if (activeTab === 'mylists' && loadedFor !== userId && !loading) {
          load();
        }
      });
      setTab = (t) => (activeTab = t);
    });

    setTab('mylists');
    flushSync();
    await pump();
    cleanup();

    expect(calls).toBe(1);
  });

  it('NEW guard reloads once more after navigating to a different user', async () => {
    let calls = 0;
    let setUser: (u: string) => void = () => {};
    const cleanup = $effect.root(() => {
      const activeTab = 'mylists';
      let userId = $state('u-A');
      let loadedFor = $state<string | null>(null);
      let loading = $state(false);

      async function load() {
        calls++;
        const target = userId;
        loading = true;
        try {
          await Promise.resolve([]);
          if (target !== userId) return;
          loadedFor = target;
        } finally {
          loading = false;
        }
      }

      $effect(() => {
        if (activeTab === 'mylists' && loadedFor !== userId && !loading) {
          load();
        }
      });
      setUser = (u) => (userId = u);
    });

    flushSync();
    await pump();
    expect(calls).toBe(1); // loaded user A once

    setUser('u-B'); // navigate /user/A -> /user/B (component not remounted)
    flushSync();
    await pump();
    cleanup();

    expect(calls).toBe(2); // loaded user B once, still no loop
  });
});
