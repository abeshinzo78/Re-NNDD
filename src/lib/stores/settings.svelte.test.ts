// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { SETTING_DEFS } from './settings.svelte';

describe('SETTING_DEFS (regression guard for plugin system)', () => {
  // プラグイン機構追加で 1 件 (plugins.enabled) が増える。これが unexpected に
  // 増減したら気付けるよう、厳密な数で固定する。新しい設定追加時はここも更新する。
  // (built-in 16 件 + plugins.enabled 1 件 + yt-dlp アップデート 3 件
  //  + ニコニコへの視聴記録 1 件 = 21)
  it('has exactly the expected number of built-in settings', () => {
    expect(SETTING_DEFS.length).toBe(21);
  });

  // niconico の watch は nvapi v4 で副作用が無くなり、視聴記録は再生開始時に
  // 明示的に送る。旧 watch ページは開いただけで記録されていたので既定 ON。
  it('records views on niconico by default', () => {
    const def = SETTING_DEFS.find((d) => d.key === 'playback.record_niconico_view');
    expect(def).toBeTruthy();
    expect(def?.default).toBe(true);
    expect(def?.kind).toBe('bool');
    expect(def?.section).toBe('playback');
  });

  it('contains the plugins.enabled kill switch', () => {
    const def = SETTING_DEFS.find((d) => d.key === 'plugins.enabled');
    expect(def).toBeTruthy();
    expect(def?.default).toBe(true);
    expect(def?.kind).toBe('bool');
    expect(def?.section).toBe('advanced');
  });

  it('contains the appearance.expand_description toggle (default off)', () => {
    const def = SETTING_DEFS.find((d) => d.key === 'appearance.expand_description');
    expect(def).toBeTruthy();
    expect(def?.default).toBe(false);
    expect(def?.kind).toBe('bool');
    expect(def?.section).toBe('appearance');
  });

  it('all setting keys are unique', () => {
    const keys = SETTING_DEFS.map((d) => d.key);
    expect(new Set(keys).size).toBe(keys.length);
  });

  // ---- yt-dlp アップデート追従 ----
  //
  // キーは Rust 側 `downloader::ytdlp_update` の KEY_CHANNEL / KEY_AUTO_CHECK /
  // KEY_AUTO_INSTALL と文字列一致していないと、設定画面と起動時の自動チェックが
  // 別々の行を読むことになる。片方だけリネームされたらここで落ちる。
  it('exposes the yt-dlp update settings with keys matching the Rust side', () => {
    const channel = SETTING_DEFS.find((d) => d.key === 'ytdlp.update_channel');
    expect(channel).toBeTruthy();
    expect(channel?.kind).toBe('select');
    expect(channel?.default).toBe('stable');
    expect(channel?.section).toBe('download');
    // Rust の `UpdateChannel::as_str` が返す値だけを選べること。
    expect(channel?.options?.map((o) => o.value)).toEqual(['stable', 'nightly']);

    const autoCheck = SETTING_DEFS.find((d) => d.key === 'ytdlp.auto_check');
    expect(autoCheck?.kind).toBe('bool');
    // 既定 ON。Rust 側も「明示的に false の時だけ止める」で揃えてある。
    expect(autoCheck?.default).toBe(true);

    const autoInstall = SETTING_DEFS.find((d) => d.key === 'ytdlp.auto_install');
    expect(autoInstall?.kind).toBe('bool');
    // 勝手にバイナリを差し替えないよう既定 OFF。
    expect(autoInstall?.default).toBe(false);
  });
});
