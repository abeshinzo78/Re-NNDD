import { describe, expect, it, vi } from 'vitest';
import { PositionSync, type PositionSyncInfo } from './positionSync';

const INFO: PositionSyncInfo = { isGuest: false, okReason: 'PURELY', durationSec: 320 };

function setup(eligible = true) {
  const saver = vi.fn((_videoId: string, _seconds: number) => Promise.resolve());
  const sync = new PositionSync(saver, () => eligible);
  sync.reset('sm9', INFO);
  return { saver, sync };
}

describe('PositionSync', () => {
  it('saves nothing until playback actually starts', () => {
    const { saver, sync } = setup();
    sync.saveAt(12);
    expect(saver).not.toHaveBeenCalled();
    sync.started();
    sync.saveAt(12.9);
    expect(saver).toHaveBeenCalledWith('sm9', 12);
  });

  it('does not send the same position twice', () => {
    const { saver, sync } = setup();
    sync.started();
    sync.saveAt(30);
    sync.saveAt(30.4);
    sync.saveAt(31);
    expect(saver.mock.calls).toEqual([
      ['sm9', 30],
      ['sm9', 31],
    ]);
  });

  it('saves 0 at the end and keeps it when leaving afterwards', () => {
    const { saver, sync } = setup();
    sync.started();
    sync.finished();
    sync.saveAt(320);
    expect(saver.mock.calls).toEqual([['sm9', 0]]);
  });

  it('resumes saving the position after rewinding a finished video', () => {
    const { saver, sync } = setup();
    sync.started();
    sync.finished();
    sync.timeUpdate(319.5); // まだ終端
    sync.saveAt(319.5);
    sync.timeUpdate(30);
    sync.saveAt(32);
    expect(saver.mock.calls).toEqual([
      ['sm9', 0],
      ['sm9', 32],
    ]);
  });

  it('stops after a hand-off until playback starts again', () => {
    const { saver, sync } = setup();
    sync.started();
    sync.saveAt(100);
    sync.handOff();
    sync.saveAt(100.5); // 引き継ぎ時点の位置で上書きしない
    sync.started();
    sync.saveAt(150);
    expect(saver.mock.calls).toEqual([
      ['sm9', 100],
      ['sm9', 150],
    ]);
  });

  it('respects eligibility and resets per video', () => {
    const { saver, sync } = setup(false);
    sync.started();
    sync.saveAt(10);
    expect(saver).not.toHaveBeenCalled();

    const other = setup();
    other.sync.started();
    other.sync.reset('sm10', INFO);
    other.sync.saveAt(10); // 新しい動画はまだ再生が始まっていない
    other.sync.reset(null);
    other.sync.started();
    other.sync.saveAt(10);
    expect(other.saver).not.toHaveBeenCalled();
  });
});
