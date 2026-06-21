//! REAL end-to-end verification harness for the comment burn-in export feature.
//!
//! Fetches REAL niconico comments + the REAL video for sm9 using an authenticated
//! cookie, renders niconicomments frame-by-frame under @napi-rs/canvas, streams
//! the PNG frames into the ffmpeg sidecar (image2pipe) with the EXACT filter graph from
//! src-tauri/src/downloader/burnin.rs, and writes output.mp4 + sample frames.
//!
//! Mirrors the Rust path:
//!   - api/video.rs   : watch-page fetch + server-response meta extraction + html_unescape
//!   - api/comment.rs : POST {server}/v1/threads
//!   - downloader/burnin.rs : overlay_filter() + spawn_session() ffmpeg args
//!   - lib/burnin/core.ts + comments.ts : the shared frame loop + v1 projection

import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { createCanvas, GlobalFonts, Canvas, Path2D, Image } from '@napi-rs/canvas';
import NiconiComments from '@xpadev-net/niconicomments';

import { buildNiconiOptions } from '../../src/lib/burnin/core';
import { runFrameLoop } from '../../src/lib/burnin/core';
import { toV1Threads } from '../../src/lib/burnin/comments';
import type { FrameSink } from '../../src/lib/burnin/core';
import type { PlayerComment } from '../../src/lib/player/types';

// ---------------------------------------------------------------------------
// Constants (mirror the Rust constants verbatim where applicable)
// ---------------------------------------------------------------------------
// The niconico `user_session` cookie value. Provide it via the NICO_USER_SESSION
// env var — NEVER hardcode a real session token in source (it is a secret).
const COOKIE_VALUE = process.env.NICO_USER_SESSION ?? '';
if (!COOKIE_VALUE) {
  console.error(
    'Set NICO_USER_SESSION to your niconico user_session cookie value, e.g.\n' +
      '  NICO_USER_SESSION="user_session_..." NODE_TLS_REJECT_UNAUTHORIZED=0 \\\n' +
      '  NODE_PATH=node_modules node scripts/burnin-verify/verify.cjs',
  );
  process.exit(1);
}
const COOKIE_HEADER = `user_session=${COOKIE_VALUE}`;
// api/video.rs BROWSER_UA
const BROWSER_UA =
  'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';
const FRONTEND_ID = '6';
const FRONTEND_VERSION = '0';

const VIDEO_ID = 'sm9';
const WATCH_URL = `https://www.nicovideo.jp/watch/${VIDEO_ID}`;

// 安全な一意の一時ディレクトリを作る (CodeQL: 予測可能な temp パスを避ける)。
const OUT_DIR = mkdtempSync(join(tmpdir(), 'burnin-verify-'));
const INPUT_MP4 = `${OUT_DIR}/input.mp4`;
const OUTPUT_MP4 = `${OUT_DIR}/output.mp4`;
const COOKIE_FILE = `${OUT_DIR}/cookies.txt`;

// リポジトリルートから実行する前提 (npm スクリプトと同じ)。triple サフィックス
// 付きのサイドカーバイナリを src-tauri/binaries/ から探す。
const BIN_DIR = `${process.cwd()}/src-tauri/binaries`;
function findSidecar(prefix: string): string {
  const hit = readdirSync(BIN_DIR).find((f) => f.startsWith(`${prefix}-`));
  if (!hit) {
    throw new Error(
      `${prefix} sidecar not found in ${BIN_DIR}. Run: bash scripts/fetch-binaries.sh`,
    );
  }
  return `${BIN_DIR}/${hit}`;
}
const FFMPEG = findSidecar('ffmpeg');
const YTDLP = findSidecar('yt-dlp');

// Render parameters (fps 30, 1280x720).
const WIDTH = 1280;
const HEIGHT = 720;
const FPS = 30;
const SS = 0;
// Default: burn in the FULL video — the exact production path. The real video ends
// at a fractional second while the frontend feeds ceil(duration)*fps frames, so a
// few surplus tail frames are produced; the OLD code died there with a broken pipe
// (os error 109 on Windows / EPIPE on Linux). Set BURNIN_VERIFY_TO=<seconds, may be
// fractional> to cap the render for a faster run while still exercising that tail.
const TO: number | undefined = process.env.BURNIN_VERIFY_TO
  ? Number(process.env.BURNIN_VERIFY_TO)
  : undefined;
const OPACITY = 1.0;

// ---------------------------------------------------------------------------
// niconicomments polyfill setup — copied VERBATIM from smoke.ts
// ---------------------------------------------------------------------------
GlobalFonts.loadSystemFonts();
console.log('[fonts] loaded families:', GlobalFonts.families.length);

const g = globalThis as Record<string, unknown>;
g.HTMLCanvasElement = Canvas;
g.Path2D = Path2D;
g.Image = Image;
g.window = {
  setTimeout: (fn: (...a: unknown[]) => void, ms?: number) => setTimeout(fn, ms),
  clearTimeout: (id: unknown) => clearTimeout(id as ReturnType<typeof setTimeout>),
  devicePixelRatio: 1,
};
g.document = {
  createElement: (tag: string) => (tag === 'canvas' ? createCanvas(1, 1) : {}),
  fonts: { ready: Promise.resolve() },
};
g.OffscreenCanvas = class {
  width: number;
  height: number;
  private c: Canvas;
  constructor(w: number, h: number) {
    this.width = w;
    this.height = h;
    this.c = createCanvas(w, h);
  }
  getContext(type: string) {
    return this.c.getContext(type as '2d');
  }
};

// ---------------------------------------------------------------------------
// html_unescape — port of api/video.rs::html_unescape (same replace order)
// ---------------------------------------------------------------------------
function htmlUnescape(s: string): string {
  const intermediate = s
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&#39;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');
  // Decode numeric character references: &#NN; (decimal) and &#xNN; (hex)
  return intermediate.replace(/&#(x?[0-9a-fA-F]+);/g, (whole, num: string) => {
    let codePoint: number | undefined;
    if (num[0] === 'x' || num[0] === 'X') {
      const parsed = Number.parseInt(num.slice(1), 16);
      codePoint = Number.isNaN(parsed) ? undefined : parsed;
    } else {
      const parsed = Number.parseInt(num, 10);
      codePoint = Number.isNaN(parsed) ? undefined : parsed;
    }
    if (codePoint === undefined) return whole;
    try {
      return String.fromCodePoint(codePoint);
    } catch {
      return whole;
    }
  });
}

// <meta name="server-response" content="([^"]*)"  (api/video.rs meta_regex)
const META_RE = /<meta name="server-response" content="([^"]*)"/;

type NvCommentSetup = { server: string; threadKey: string; params: unknown };

// ---------------------------------------------------------------------------
// Step 1: fetch the watch page and extract duration + nvComment
// ---------------------------------------------------------------------------
async function fetchWatchData(): Promise<{ durationSec: number; nv: NvCommentSetup }> {
  console.log(`[step1] GET ${WATCH_URL}`);
  const res = await fetch(WATCH_URL, {
    headers: {
      Cookie: COOKIE_HEADER,
      'User-Agent': BROWSER_UA,
      Accept: 'text/html,application/xhtml+xml',
    },
  });
  if (!res.ok) {
    throw new Error(`watch page returned ${res.status} ${res.statusText}`);
  }
  const html = await res.text();
  const m = META_RE.exec(html);
  if (!m) {
    throw new Error('watch page missing <meta name="server-response">');
  }
  const decoded = htmlUnescape(m[1]);
  const root = JSON.parse(decoded) as unknown;
  const response = (root as { data?: { response?: Record<string, unknown> } })?.data?.response;
  if (!response) throw new Error('missing /data/response');

  const video = response.video as { duration?: number; title?: string } | undefined;
  const duration = video?.duration;
  if (typeof duration !== 'number') throw new Error('missing /data/response/video/duration');

  const nvNode = (response.comment as { nvComment?: Record<string, unknown> } | undefined)
    ?.nvComment;
  if (!nvNode) throw new Error('missing /data/response/comment/nvComment');
  const nv: NvCommentSetup = {
    server: String(nvNode.server),
    threadKey: String(nvNode.threadKey),
    params: nvNode.params,
  };
  console.log(`[step1] title=${JSON.stringify(video?.title)} duration=${duration}s`);
  console.log(`[step1] nvComment.server=${nv.server} threadKey.len=${nv.threadKey.length}`);
  return { durationSec: duration, nv };
}

// ---------------------------------------------------------------------------
// Step 1 (cont.): POST {server}/v1/threads  (api/comment.rs)
// ---------------------------------------------------------------------------
function parseThreads(envelope: unknown): PlayerComment[] {
  const threads = (envelope as { data?: { threads?: unknown[] } })?.data?.threads;
  if (!Array.isArray(threads)) return [];
  const out: PlayerComment[] = [];
  for (const thread of threads) {
    const t = thread as { fork?: string; comments?: unknown[] };
    const fork = typeof t.fork === 'string' ? t.fork : 'main';
    const isOwner = fork === 'owner';
    if (!Array.isArray(t.comments)) continue;
    for (const cv of t.comments) {
      const c = cv as Record<string, unknown>;
      const body = c.body;
      if (typeof body !== 'string') continue;
      const no = typeof c.no === 'number' ? c.no : 0;
      const vposMs = typeof c.vposMs === 'number' ? c.vposMs : 0;
      const id = typeof c.id === 'string' ? c.id : `${fork}-${no}`;
      const commands = Array.isArray(c.commands)
        ? (c.commands.filter((x) => typeof x === 'string') as string[])
        : [];
      const userIdRaw = c.userId;
      const userId =
        typeof userIdRaw === 'string'
          ? userIdRaw
          : typeof userIdRaw === 'number'
            ? String(userIdRaw)
            : undefined;
      out.push({
        id,
        no,
        vposMs,
        content: body,
        commands,
        mail: commands.join(' '),
        userId,
        postedAt: typeof c.postedAt === 'string' ? c.postedAt : undefined,
        fork,
        isOwner,
        nicoruCount: typeof c.nicoruCount === 'number' ? c.nicoruCount : undefined,
        score: typeof c.score === 'number' ? c.score : undefined,
      });
    }
  }
  return out;
}

async function fetchComments(nv: NvCommentSetup): Promise<PlayerComment[]> {
  const url = `${nv.server.replace(/\/+$/, '')}/v1/threads`;
  console.log(`[step1] POST ${url}`);
  const body = JSON.stringify({ params: nv.params, threadKey: nv.threadKey, additionals: {} });
  const res = await fetch(url, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'x-frontend-id': FRONTEND_ID,
      'x-frontend-version': FRONTEND_VERSION,
      Cookie: COOKIE_HEADER,
      'User-Agent': BROWSER_UA,
    },
    body,
  });
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`threads API ${res.status}: ${text.slice(0, 500)}`);
  }
  const envelope = JSON.parse(text) as unknown;
  const metaStatus = (envelope as { meta?: { status?: number } })?.meta?.status;
  if (metaStatus !== 200) {
    throw new Error(`unexpected meta.status from threads API: ${metaStatus}`);
  }
  const comments = parseThreads(envelope);
  const byFork = new Map<string, number>();
  for (const c of comments) byFork.set(c.fork, (byFork.get(c.fork) ?? 0) + 1);
  console.log(
    `[step1] fetched ${comments.length} comments; per-fork:`,
    JSON.stringify(Object.fromEntries(byFork)),
  );
  return comments;
}

// ---------------------------------------------------------------------------
// Step 2: download the REAL video via yt-dlp sidecar
// ---------------------------------------------------------------------------
function run(
  cmd: string,
  args: string[],
  opts: { collectStderr?: boolean } = {},
): Promise<{ code: number | null; stderr: string; stdout: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let stderr = '';
    let stdout = '';
    child.stdout.on('data', (d: Buffer) => {
      stdout += d.toString();
    });
    child.stderr.on('data', (d: Buffer) => {
      stderr += d.toString();
    });
    child.on('error', reject);
    child.on('close', (code) => resolve({ code, stderr, stdout }));
    void opts;
  });
}

async function downloadVideo(): Promise<void> {
  // sm9 only exposes HLS (m3u8) tracks split into video-only + audio-only,
  // so yt-dlp must mux them with ffmpeg. Point it at our sidecar ffmpeg and
  // ask it to merge into mp4. Try a height-capped pair first (per the task's
  // "best[height<=480]/best" intent), then plain best+bestaudio, then bare best.
  const common = [
    '--no-check-certificate',
    '--ffmpeg-location',
    FFMPEG,
    '--merge-output-format',
    'mp4',
    '--cookies',
    COOKIE_FILE,
    '-o',
    INPUT_MP4,
  ];
  const attempts: { label: string; fmt?: string }[] = [
    {
      label: 'bestvideo[height<=480]+bestaudio/best[height<=480]/best',
      fmt: 'bestvideo[height<=480]+bestaudio/best[height<=480]/best',
    },
    { label: 'bestvideo+bestaudio/best', fmt: 'bestvideo+bestaudio/best' },
    { label: '(no -f)' },
  ];

  let lastErr = '';
  for (const attempt of attempts) {
    // A stale partial output would make existsSync lie; clear it first.
    await run('rm', ['-f', INPUT_MP4]);
    const args = attempt.fmt ? [...common, '-f', attempt.fmt, WATCH_URL] : [...common, WATCH_URL];
    console.log(`[step2] yt-dlp -f "${attempt.label}"`);
    const res = await run(YTDLP, args);
    if (res.code === 0 && existsSync(INPUT_MP4) && statSync(INPUT_MP4).size > 0) {
      const sz = statSync(INPUT_MP4).size;
      console.log(`[step2] downloaded input.mp4 (${sz} bytes) via "${attempt.label}"`);
      return;
    }
    lastErr = `code=${res.code}\n${res.stderr.split('\n').slice(-20).join('\n')}`;
    console.log(`[step2] attempt "${attempt.label}" failed: ${lastErr}`);
  }
  throw new Error(`yt-dlp failed after all attempts. last:\n${lastErr}`);
}

// Probe via ffmpeg -i (writes to stderr).
async function probe(file: string): Promise<string> {
  const res = await run(FFMPEG, ['-hide_banner', '-i', file]);
  // ffmpeg -i with no output returns non-zero; the info is in stderr.
  const lines = res.stderr
    .split('\n')
    .filter((l) => /Duration|Stream|Video:|Audio:|Input #/.test(l))
    .join('\n');
  return lines;
}

// ---------------------------------------------------------------------------
// Step 3: overlay_filter — port of downloader/burnin.rs::overlay_filter
// ---------------------------------------------------------------------------
function overlayFilter(width: number, height: number, fps: number, opacity: number): string {
  const op = Math.min(1, Math.max(0, opacity));
  const base =
    `[0:v]fps=fps=${fps},pad=width=max(iw\\,ih*(16/9)):height=ow/(16/9):x=(ow-iw)/2:y=(oh-ih)/2,scale=w=${width}:h=${height}[video];` +
    `[1:v]format=yuva444p,colorspace=bt709:iall=bt601-6-525:fast=1[baseImage];` +
    `[1:v]format=rgba,alphaextract[alpha];` +
    `[baseImage][alpha]alphamerge[image]`;
  if (op >= 0.999) {
    return `${base};[video][image]overlay=eof_action=pass[output]`;
  }
  return `${base};[image]format=rgba,colorchannelmixer=aa=${op.toFixed(4)}[imageop];[video][imageop]overlay=eof_action=pass[output]`;
}

// ---------------------------------------------------------------------------
// Step 3: spawn ffmpeg session — port of downloader/burnin.rs::spawn_session
// ---------------------------------------------------------------------------
type FfmpegSession = {
  args: string[];
  /** Returns true if accepted, false if ffmpeg closed stdin (= it has enough
   *  frames). Mirrors the production Rust path: a broken pipe on the surplus
   *  tail frames is NOT an error — success is judged by the exit code at finish. */
  writePng: (png: Uint8Array) => Promise<boolean>;
  finish: () => Promise<void>;
  getStderr: () => string;
};

function isSinkClosed(code: string | undefined): boolean {
  return code === 'EPIPE' || code === 'ERR_STREAM_DESTROYED' || code === 'ERR_STREAM_WRITE_AFTER_END';
}

function spawnFfmpegSession(filter: string): FfmpegSession {
  const args = [
    '-hide_banner',
    '-loglevel',
    'error',
    '-nostats',
    '-y',
    '-sws_flags',
    'spline+accurate_rnd+full_chroma_int',
    // Cap the input only when BURNIN_VERIFY_TO is set; otherwise burn the FULL
    // video (production parity — no -t).
    ...(TO !== undefined ? ['-t', String(TO)] : []),
    '-i',
    INPUT_MP4,
    '-f',
    'image2pipe',
    '-framerate',
    String(FPS),
    '-i',
    'pipe:0',
    '-filter_complex',
    filter,
    '-map',
    '[output]',
    '-map',
    '0:a:0?',
    '-c:v',
    'libx264',
    '-preset',
    'veryfast',
    '-crf',
    '20',
    '-pix_fmt',
    'yuv420p',
    '-color_range',
    'tv',
    '-colorspace',
    'bt709',
    '-color_primaries',
    'bt709',
    '-color_trc',
    'bt709',
    '-c:a',
    'aac',
    '-b:a',
    '192k',
    '-movflags',
    '+faststart',
    OUTPUT_MP4,
  ];

  const child = spawn(FFMPEG, args, { stdio: ['pipe', 'ignore', 'pipe'] });
  let stderr = '';
  child.stderr.on('data', (d: Buffer) => {
    stderr += d.toString();
  });

  let spawnError: Error | null = null;
  child.on('error', (e) => {
    spawnError = e;
  });

  // Capture the process exit ONCE at spawn time. In the surplus-tail case ffmpeg
  // can emit 'close' between the broken-pipe write and the finish() call, so a
  // listener registered lazily inside finish() would miss it and hang forever.
  // Registering here guarantees the exit is observed regardless of timing.
  const exit: Promise<number | null> = new Promise((resolve) => {
    child.once('close', (code) => resolve(code));
    child.once('error', () => resolve(null)); // spawn failure: unblock finish()
  });

  const stdin = child.stdin;
  let closed = false;
  // Swallow any stray async EPIPE so an error between writes never crashes the
  // process; the exit code is the source of truth.
  stdin.on('error', (e: NodeJS.ErrnoException) => {
    if (isSinkClosed(e.code)) closed = true;
  });

  const writePng = (png: Uint8Array): Promise<boolean> =>
    new Promise((resolve, reject) => {
      if (spawnError) {
        reject(spawnError);
        return;
      }
      if (closed || stdin.destroyed) {
        resolve(false);
        return;
      }
      // The write callback fires after flush (or error), which also gives us
      // natural backpressure handling.
      stdin.write(Buffer.from(png), (err?: NodeJS.ErrnoException | null) => {
        if (!err) {
          resolve(true);
        } else if (isSinkClosed(err.code)) {
          closed = true;
          resolve(false);
        } else {
          reject(err);
        }
      });
    });

  const finish = async (): Promise<void> => {
    try {
      stdin.end();
    } catch {
      /* already closed */
    }
    const code = await exit;
    if (spawnError) throw spawnError;
    if (code !== 0) {
      throw new Error(
        `ffmpeg exited with code ${code}\n${stderr.split('\n').slice(0, 40).join('\n')}`,
      );
    }
  };

  return { args, writePng, finish, getStderr: () => stderr };
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main() {
  const t0 = Date.now();

  // Setup: OUT_DIR is already created by mkdtempSync. Generate the yt-dlp
  // Netscape cookie file from the NICO_USER_SESSION env var (self-contained).
  console.log(`[setup] work dir: ${OUT_DIR}`);
  const expiry = Math.floor(Date.now() / 1000) + 31_536_000;
  writeFileSync(
    COOKIE_FILE,
    `# Netscape HTTP Cookie File\n.nicovideo.jp\tTRUE\t/\tTRUE\t${expiry}\tuser_session\t${COOKIE_VALUE}\n`,
  );

  // Step 1: comments
  const { durationSec, nv } = await fetchWatchData();
  const comments = await fetchComments(nv);
  if (comments.length === 0) {
    throw new Error('no comments fetched — aborting (expected thousands for sm9)');
  }

  // Production parity: downloaded snapshots store `posted_at` as a Unix-seconds
  // STRING (load_snapshot_comments serializes i64.to_string()), unlike the online
  // API's ISO `postedAt`. Simulate that here so we exercise the SAME input the
  // real burn-in path sees. toV1Threads()/toIsoPostedAt() must normalize it back
  // to ISO, otherwise niconicomments misclassifies (0.2.x) or drops (0.3.x) every
  // comment with a postedAt.
  if (process.env.SIMULATE_LOCAL !== '0') {
    for (const c of comments) {
      if (c.postedAt) {
        const ms = Date.parse(c.postedAt);
        if (!Number.isNaN(ms)) c.postedAt = String(Math.floor(ms / 1000));
      }
    }
    console.log('[step1] simulated local storage: postedAt -> Unix-seconds string');
  }

  // Step 2: video
  await downloadVideo();
  const probeInfo = await probe(INPUT_MP4);
  console.log('[step2] ffmpeg probe of input.mp4:\n' + probeInfo);

  // Step 3: render + overlay
  const filter = overlayFilter(WIDTH, HEIGHT, FPS, OPACITY);
  console.log('[step3] filter_complex:\n' + filter);

  const canvas = createCanvas(WIDTH, HEIGHT);

  // Capture the transparent empty PNG BEFORE creating niconicomments (blank canvas).
  const emptyPng: Uint8Array = canvas.toBuffer('image/png');
  console.log(`[step3] empty (transparent) PNG captured: ${emptyPng.length} bytes`);

  const opts = buildNiconiOptions({ format: 'v1', mode: 'default', scale: 1 });
  const threads = toV1Threads(comments);
  const nico = new NiconiComments(canvas as never, threads as never, opts as never);
  const timelineKeys = Object.keys(
    (nico as unknown as { timeline: Record<number, unknown[]> }).timeline,
  );
  console.log(`[step3] niconicomments timeline keys: ${timelineKeys.length}`);

  const session = spawnFfmpegSession(filter);
  console.log('[step3] ffmpeg command:\n' + FFMPEG + ' ' + session.args.map(quoteArg).join(' '));

  const toPng = (): Promise<Uint8Array> => Promise.resolve(canvas.toBuffer('image/png'));

  let framesDrawn = 0;
  let emptyFrames = 0;
  let sinkClosedAt = -1;
  const sink: FrameSink = {
    async frame(index: number, png: Uint8Array) {
      const ok = await session.writePng(png);
      if (ok) framesDrawn++;
      else if (sinkClosedAt < 0) sinkClosedAt = index;
      return ok;
    },
    async empty(index: number) {
      const ok = await session.writePng(emptyPng);
      if (ok) emptyFrames++;
      else if (sinkClosedAt < 0) sinkClosedAt = index;
      return ok;
    },
  };

  const total = await runFrameLoop({
    nico,
    toPng,
    fps: FPS,
    durationSec,
    ssSec: SS,
    toSec: TO, // undefined => full video (production parity)
    sink,
    onProgress: (rendered, totalFrames) => {
      if (rendered % 60 === 0 || rendered === totalFrames) {
        console.log(`[step3] progress ${rendered}/${totalFrames}`);
      }
    },
  });
  const renderedTo = TO ?? durationSec;
  const requestedFrames = Math.ceil(renderedTo - SS) * FPS;
  console.log(
    `[step3] frame loop done: total=${total} drawn=${framesDrawn} empty=${emptyFrames} ` +
      `requested=${requestedFrames} sinkClosedAt=${sinkClosedAt} ` +
      `(surplus tail absorbed: ${sinkClosedAt >= 0 ? 'yes' : 'no — fit in pipe buffer'})`,
  );

  await session.finish();
  console.log('[step3] ffmpeg finished (exit 0)');

  if (!existsSync(OUTPUT_MP4)) throw new Error('output.mp4 was not produced');
  const outSz = statSync(OUTPUT_MP4).size;
  console.log(`[step3] output.mp4 size: ${outSz} bytes`);
  if (outSz < 10000) throw new Error(`output.mp4 suspiciously small (${outSz} bytes)`);

  const outProbe = await probe(OUTPUT_MP4);
  console.log('[step3] ffmpeg probe of output.mp4:\n' + outProbe);

  // Assert the output covers (about) the full requested span — i.e. the surplus
  // tail did NOT truncate the video. Parse "Duration: HH:MM:SS.ss" from the probe.
  const durMatch = /Duration:\s*(\d+):(\d+):(\d+(?:\.\d+)?)/.exec(outProbe);
  if (durMatch) {
    const outDur = Number(durMatch[1]) * 3600 + Number(durMatch[2]) * 60 + Number(durMatch[3]);
    const expected = renderedTo - SS;
    console.log(`[step3] output duration=${outDur.toFixed(2)}s expected≈${expected.toFixed(2)}s`);
    // Allow 1s slack (container rounding / audio padding).
    if (outDur < expected - 1) {
      throw new Error(
        `output truncated: ${outDur.toFixed(2)}s < expected ${expected.toFixed(2)}s — ` +
          `the surplus-tail broken pipe likely cut the render short`,
      );
    }
  }

  // Step 4: extract sample frames across the rendered span, incl. the very tail.
  console.log('[step4] extracting sample frames');
  const span = renderedTo - SS;
  const SAMPLE_TIMES = [0.05, 0.25, 0.5, 0.75, 0.97]
    .map((f) => +(SS + span * f).toFixed(2))
    .filter((t) => t >= 0);
  const framePaths: { t: number; path: string; size: number }[] = [];
  for (const t of SAMPLE_TIMES) {
    const framePath = `${OUT_DIR}/frame_${t}.png`;
    const res = await run(FFMPEG, [
      '-hide_banner',
      '-loglevel',
      'error',
      '-y',
      '-ss',
      String(t),
      '-i',
      OUTPUT_MP4,
      '-frames:v',
      '1',
      framePath,
    ]);
    if (res.code !== 0 || !existsSync(framePath)) {
      throw new Error(
        `frame extraction at t=${t} failed (code=${res.code}): ${res.stderr.slice(0, 300)}`,
      );
    }
    const size = statSync(framePath).size;
    framePaths.push({ t, path: framePath, size });
    console.log(`[step4] frame_${t}.png: ${size} bytes`);
  }

  // Final structured summary for the report.
  const summary = {
    videoId: VIDEO_ID,
    commentCount: comments.length,
    durationSec,
    render: { width: WIDTH, height: HEIGHT, fps: FPS, ss: SS, to: TO ?? 'full' },
    framesTotal: total,
    framesDrawn,
    emptyFrames,
    sinkClosedAt,
    inputMp4: INPUT_MP4,
    inputSize: statSync(INPUT_MP4).size,
    outputMp4: OUTPUT_MP4,
    outputSize: outSz,
    filter,
    ffmpegCommand: FFMPEG + ' ' + session.args.map(quoteArg).join(' '),
    frames: framePaths,
    elapsedSec: Math.round((Date.now() - t0) / 1000),
  };
  console.log('\n===SUMMARY_JSON_BEGIN===');
  console.log(JSON.stringify(summary, null, 2));
  console.log('===SUMMARY_JSON_END===');
}

function quoteArg(a: string): string {
  return /[\s"'\\|()[\]]/.test(a) ? `'${a.replace(/'/g, "'\\''")}'` : a;
}

main().catch((e) => {
  console.error('\n[FATAL]', e instanceof Error ? e.stack || e.message : e);
  process.exit(1);
});
