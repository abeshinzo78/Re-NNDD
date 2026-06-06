#!/usr/bin/env node
// Re:NNDD end-to-end WebDriver harness — launches the REAL Tauri app and drives it.
//
// This is a zero-dependency W3C-WebDriver client. It spawns `tauri-driver`
// (which in turn spawns WebKitWebDriver and the app binary), opens a session
// against the live app, and exposes primitives to screenshot / click / eval the
// running UI. `app.test.mjs` imports `launch()` + `ROUTES` to assert against the
// live app; this file also keeps a small CLI for manual screenshot tours.
//
// It MUST run under an X server (xvfb-run) because the Tauri app opens a real
// WebKitGTK window (see the CLI usage below).
//
// CLI usage (always under xvfb-run -a):
//   node tests/e2e/driver.mjs tour [outDir]        # screenshot every main route
//   node tests/e2e/driver.mjs shot <route> <file>  # one route -> one PNG
//   node tests/e2e/driver.mjs eval <route> '<js>'  # navigate, run JS, print result
//
// Env knobs:
//   APP_BIN   path to the app binary (default: target/debug/nndd-next)
//   PORT      tauri-driver port (default: 4444)
//   NATIVE_PORT  native WebKitWebDriver port tauri-driver proxies to (default: PORT+1)
//   OUT_DIR   default screenshot dir (default: /tmp/re-nndd-shots)
//   NO_VITE=1 skip the defensive Vite dev-server launch (pure prod binary)
//   REUSE_VITE=1 reuse a server already on :1420 instead of failing closed

import { spawn } from 'node:child_process';
import { writeFileSync, mkdirSync, existsSync, openSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = dirname(fileURLToPath(import.meta.url));
// this file lives at <repo>/tests/e2e -> repo root is 2 up.
export const REPO = resolve(HERE, '..', '..');
const PORT = Number(process.env.PORT || 4444);
// tauri-driver runs TWO WebDriver ends; if you move PORT off 4444, the native
// WebKitWebDriver must move off its 4445 default too or it can collide/attach to
// a stale native driver. Track PORT by default.
const NATIVE_PORT = Number(process.env.NATIVE_PORT || PORT + 1);
const BASE = `http://127.0.0.1:${PORT}`;
// NB: this is a cargo *workspace*, so the binary lands in the workspace-root
// target/, not src-tauri/target/.
const APP_BIN = process.env.APP_BIN || resolve(REPO, 'target/debug/nndd-next');
const OUT_DIR = process.env.OUT_DIR || '/tmp/re-nndd-shots';

// The W3C element id key returned for every located element.
const ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';

// The nine built-in sidebar routes (name is just a stable label for filenames).
export const ROUTES = [
  ['/', 'home'],
  ['/library', 'library'],
  ['/ranking', 'ranking'],
  ['/search', 'search'],
  ['/playlists', 'playlists'],
  ['/downloads', 'downloads'],
  ['/history', 'history'],
  ['/ng', 'ng'],
  ['/settings', 'settings'],
];

// WebKitGTK under Xvfb has no GPU; force software rendering or the window
// comes up blank / the app crashes in the GL stack. These propagate through
// tauri-driver -> WebKitWebDriver -> the app process. Caller-set values win.
for (const [k, v] of Object.entries({
  WEBKIT_DISABLE_COMPOSITING_MODE: '1',
  WEBKIT_DISABLE_DMABUF_RENDERER: '1',
  LIBGL_ALWAYS_SOFTWARE: '1',
  GDK_BACKEND: 'x11',
})) {
  if (!process.env[k]) process.env[k] = v;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function wd(method, path, body) {
  const res = await fetch(BASE + path, {
    method,
    headers: { 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  let json;
  try {
    json = text ? JSON.parse(text) : {};
  } catch {
    json = { raw: text };
  }
  if (!res.ok) {
    const err = json?.value?.error || json?.value?.message || text;
    throw new Error(`WD ${method} ${path} -> ${res.status}: ${err}`);
  }
  return json.value;
}

async function waitForDriver(proc, timeoutMs = 20000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    // If the spawned tauri-driver died (e.g. port already bound, or not
    // installed), fail now instead of silently accepting a stranger on PORT.
    if (proc.spawnError || proc.exitCode !== null || proc.signalCode) {
      throw new Error(
        `tauri-driver exited before binding ${BASE} ` +
          `(${proc.spawnError || `code ${proc.exitCode ?? proc.signalCode}`}). ` +
          `Is it installed (cargo install tauri-driver) and is the port free?`,
      );
    }
    try {
      const r = await fetch(BASE + '/status');
      if (r.ok || r.status === 500) return; // our tauri-driver is answering
    } catch {
      /* not up yet */
    }
    await sleep(250);
  }
  throw new Error(`tauri-driver did not open ${BASE} within ${timeoutMs}ms`);
}

async function httpUp(url) {
  try {
    await fetch(url, { signal: AbortSignal.timeout(1000) });
    return true; // any HTTP answer means something is listening
  } catch {
    return false;
  }
}

// Tauri *debug* builds default to dev mode and load the Vite devUrl
// (http://localhost:1420); only `npx tauri build` bakes the frontend in for a
// prod-style load. A plain `cargo build`/`cargo test` on the workspace silently
// rebuilds the binary in dev mode -> it then shows "Could not connect to
// localhost" under the driver. To be robust to BOTH binary flavours we start
// our OWN Vite (from this repo) before launching: a prod binary ignores it, a
// dev binary loads it. Set NO_VITE=1 to skip (pure prod binary, slightly faster).
async function ensureVite() {
  if (process.env.NO_VITE) return null;
  // Fail closed if :1420 is already taken: a dev-mode binary would load whatever
  // is there (devUrl=localhost:1420), which could be a stale build or a totally
  // different app/checkout — and we'd silently drive the wrong thing. We can't
  // reliably prove an arbitrary server is *this* checkout, so refuse it.
  // REUSE_VITE=1 opts in (e.g. you already run `npm run dev` for this repo).
  if (await httpUp('http://127.0.0.1:1420')) {
    if (process.env.REUSE_VITE) {
      console.log('reusing the server already on :1420 (REUSE_VITE set)');
      return null;
    }
    throw new Error(
      'port 1420 is already in use — a dev-mode binary loads whatever lives ' +
        'there (devUrl=localhost:1420), which may be a stale or unrelated app, ' +
        'so the run could drive the wrong thing. Free it ' +
        '(pkill -f node_modules/.bin/vite), or set REUSE_VITE=1 to use it as-is, ' +
        'or NO_VITE=1 if you built a prod binary (npx tauri build).',
    );
  }
  console.log('starting Vite dev server (this repo) for dev-mode binary…');
  const log = '/tmp/re-nndd-vite.log';
  const out = openSync(log, 'w');
  // Spawn the vite binary directly (NOT `npm run dev`): npm swallows SIGTERM and
  // orphans its vite child. detached:true makes vite its own group leader so the
  // group SIGTERM on exit also reaps vite's esbuild child.
  const vite = spawn(resolve(REPO, 'node_modules/.bin/vite'), [], {
    cwd: REPO,
    stdio: ['ignore', out, out],
    env: process.env,
    detached: true,
  });
  const t0 = Date.now();
  while (Date.now() - t0 < 40000) {
    if (await httpUp('http://127.0.0.1:1420')) {
      console.log('Vite up on :1420');
      return vite;
    }
    await sleep(400);
  }
  killGroup(vite);
  throw new Error(`Vite did not come up on :1420 in 40s (see ${log})`);
}

async function startDriver() {
  // Refuse to attach to a pre-existing listener on PORT: if something else owns
  // it our spawned tauri-driver can't bind, and /session would silently hit the
  // stranger while cleanup only kills our (dead) child. A fresh run expects the
  // port free — the driver reaps its own.
  if (await httpUp(BASE + '/status')) {
    throw new Error(
      `something is already listening on ${BASE} — refusing to attach. ` +
        `Free it (pkill -f tauri-driver) or set PORT to a free port.`,
    );
  }
  // tauri-driver passes our env (incl. WEBKIT_* software-render flags) down to
  // WebKitWebDriver and the app it launches.
  const proc = spawn(
    'tauri-driver',
    ['--port', String(PORT), '--native-port', String(NATIVE_PORT)],
    {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: process.env,
    },
  );
  // Record the failure instead of process.exit() here: exiting from this async
  // event handler bypasses teardown and would orphan the Vite server ensureVite()
  // started. waitForDriver() sees spawnError and throws, so teardown still runs.
  proc.spawnError = null;
  proc.on('error', (e) => {
    proc.spawnError = e.message;
  });
  return proc;
}

async function newSession() {
  // Retry: the very first New Session after launch can race the app's window.
  let lastErr;
  for (let i = 0; i < 3; i++) {
    try {
      const value = await wd('POST', '/session', {
        capabilities: {
          firstMatch: [{}],
          alwaysMatch: { 'tauri:options': { application: APP_BIN } },
        },
      });
      return value.sessionId;
    } catch (e) {
      lastErr = e;
      await sleep(1000);
    }
  }
  throw lastErr;
}

const css = (sid, selector) =>
  wd('POST', `/session/${sid}/element`, { using: 'css selector', value: selector });
const cssAll = (sid, selector) =>
  wd('POST', `/session/${sid}/elements`, { using: 'css selector', value: selector });
const elClick = (sid, eid) => wd('POST', `/session/${sid}/element/${eid}/click`, {});
const execute = (sid, script, args = []) =>
  wd('POST', `/session/${sid}/execute/sync`, { script, args });

async function waitForCss(sid, selector, timeoutMs = 15000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeoutMs) {
    try {
      return await css(sid, selector);
    } catch {
      await sleep(200);
    }
  }
  throw new Error(`selector not found in ${timeoutMs}ms: ${selector}`);
}

// Rendered text of the first element matching `selector` (waits for presence).
async function elementText(sid, selector, timeoutMs = 15000) {
  const el = await waitForCss(sid, selector, timeoutMs);
  return wd('GET', `/session/${sid}/element/${el[ELEMENT_KEY]}/text`);
}

// Poll an element's text until `predicate(text)` holds (e.g. an async value that
// starts as a placeholder and settles after an IPC round-trip).
async function waitForText(sid, selector, predicate, timeoutMs = 15000) {
  const t0 = Date.now();
  let last = '';
  while (Date.now() - t0 < timeoutMs) {
    try {
      last = await elementText(sid, selector, 1000);
      if (predicate(last)) return last;
    } catch {
      /* not present / not settled yet */
    }
    await sleep(200);
  }
  throw new Error(
    `text predicate not met in ${timeoutMs}ms for ${selector} (last: ${JSON.stringify(last)})`,
  );
}

async function screenshot(sid, file) {
  const b64 = await wd('GET', `/session/${sid}/screenshot`);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, Buffer.from(b64, 'base64'));
  return file;
}

// Poll until the webview actually reports the target path. A fixed sleep can
// fire while the SvelteKit route chunk is still loading (cold Vite / load),
// which would act on the *previous* page.
async function waitForPath(sid, route, timeoutMs = 12000) {
  const t0 = Date.now();
  let at = '';
  while (Date.now() - t0 < timeoutMs) {
    at = await execute(sid, 'return location.pathname;');
    if (at === route) return;
    await sleep(150);
  }
  throw new Error(`navigation to ${route} did not settle (still at ${at})`);
}

// Click the sidebar <a> whose href matches, then wait for the URL to settle.
async function navTo(sid, route) {
  // Prefer clicking the real nav link (exercises SvelteKit client routing).
  const links = await cssAll(sid, 'a.nav-item');
  for (const l of links) {
    const eid = l[ELEMENT_KEY];
    const href = await wd('GET', `/session/${sid}/element/${eid}/attribute/href`);
    if (href && new URL(href, 'http://x').pathname === route) {
      await elClick(sid, eid);
      await waitForPath(sid, route);
      return;
    }
  }
  // Fallback: SvelteKit programmatic navigation.
  await execute(sid, 'window.location.assign(arguments[0]);', [route]);
  await waitForPath(sid, route);
}

function killGroup(proc) {
  if (!proc) return;
  try {
    process.kill(-proc.pid, 'SIGTERM'); // negative pid = whole process group
  } catch {
    proc.kill('SIGTERM');
  }
}

async function teardownRaw(sid, driver, vite) {
  try {
    if (sid) await wd('DELETE', `/session/${sid}`);
  } catch {
    /* session may already be gone */
  }
  if (driver) driver.kill('SIGTERM');
  if (vite) killGroup(vite);
}

// Launch the real app and return a session handle. The caller MUST eventually
// call handle.teardown() (e.g. in an `after` hook) to reap tauri-driver + Vite.
export async function launch() {
  if (!existsSync(APP_BIN)) {
    throw new Error(
      `app binary not found: ${APP_BIN}\n` + `Build it first: npx tauri build --debug --no-bundle`,
    );
  }
  let vite = null;
  let driver = null;
  let sid = null;
  try {
    vite = await ensureVite();
    driver = await startDriver();
    await waitForDriver(driver);
    sid = await newSession();
    // Home always renders the brand; use it as the "app is alive" signal. The
    // ~100MB debug binary + WebKitGTK cold start can take >15s under CPU load
    // (e.g. a concurrent cargo build), so give it generous headroom.
    await waitForCss(sid, '.brand', 45000);
    await sleep(800); // let onMount/invoke settle
  } catch (e) {
    await teardownRaw(sid, driver, vite);
    throw e;
  }
  return {
    sid,
    execute: (script, args) => execute(sid, script, args),
    css: (selector) => css(sid, selector),
    cssAll: (selector) => cssAll(sid, selector),
    text: (selector, timeoutMs) => elementText(sid, selector, timeoutMs),
    waitForCss: (selector, timeoutMs) => waitForCss(sid, selector, timeoutMs),
    waitForText: (selector, predicate, timeoutMs) =>
      waitForText(sid, selector, predicate, timeoutMs),
    navTo: (route) => navTo(sid, route),
    screenshot: (file) => screenshot(sid, file),
    teardown: () => teardownRaw(sid, driver, vite),
  };
}

async function main() {
  const [cmd = 'tour', a1, a2] = process.argv.slice(2);
  const app = await launch();
  try {
    if (cmd === 'tour') {
      const dir = a1 || OUT_DIR;
      let i = 0;
      for (const [route, name] of ROUTES) {
        await app.navTo(route);
        await sleep(600);
        const f = `${dir}/${String(i).padStart(2, '0')}-${name}.png`;
        await app.screenshot(f);
        const title = await app.execute('return document.title || location.pathname;');
        console.log(`shot ${route.padEnd(12)} -> ${f}  (${title})`);
        i++;
      }
    } else if (cmd === 'shot') {
      if (!a1 || !a2) throw new Error('usage: shot <route> <file>');
      await app.navTo(a1);
      await sleep(600);
      console.log('wrote', await app.screenshot(a2));
    } else if (cmd === 'eval') {
      if (!a1 || !a2) throw new Error('usage: eval <route> <js>');
      await app.navTo(a1);
      await sleep(600);
      const script = a2.trim().startsWith('return') ? a2 : `return (${a2});`;
      console.log(JSON.stringify(await app.execute(script), null, 2));
    } else {
      throw new Error(`unknown command: ${cmd}`);
    }
  } finally {
    await app.teardown();
  }
}

// Run the CLI only when invoked directly (`node tests/e2e/driver.mjs ...`), not
// when imported by the e2e test.
const invokedDirectly =
  process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (invokedDirectly) {
  main().catch((e) => {
    console.error('DRIVER ERROR:', e.message);
    process.exit(1);
  });
}
