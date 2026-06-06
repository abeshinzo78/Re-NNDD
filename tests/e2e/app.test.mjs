// Re:NNDD end-to-end smoke test.
//
// Drives the REAL Tauri app (Rust backend + WebKitGTK frontend + IPC) through
// tauri-driver and asserts the things a pure unit/component test cannot:
//   - the app boots and the Svelte shell mounts,
//   - the home page reports the app version from the Rust backend over IPC
//     (proves the Tauri command bridge works AND that the version is in sync),
//   - every built-in sidebar route navigates and renders real content.
//
// Prerequisites: a built app binary (`npx tauri build --debug --no-bundle`),
// `tauri-driver` on PATH (`cargo install tauri-driver`), and an X server — run
// under `xvfb-run -a`. The whole suite shares ONE app instance:
// it is launched once in `before` and torn down once in `after`.
//
//   xvfb-run -a npm run test:e2e

import { test, before, after } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { launch, ROUTES, REPO } from './driver.mjs';

const SEMVER = /^\d+\.\d+\.\d+$/;
const pkg = JSON.parse(readFileSync(resolve(REPO, 'package.json'), 'utf8'));

// Cold WebKitGTK start on the debug binary can take a while under CI load.
const LAUNCH_TIMEOUT = 180_000;
const STEP_TIMEOUT = 60_000;

let app;

before(
  async () => {
    app = await launch();
  },
  { timeout: LAUNCH_TIMEOUT },
);

after(async () => {
  if (app) await app.teardown();
});

test('boots and mounts the Svelte shell', { timeout: STEP_TIMEOUT }, async () => {
  const brand = (await app.text('.brand')).trim();
  assert.equal(brand, 'Re:NNDD');

  // All nine built-in nav links render.
  const links = await app.cssAll('a.nav-item');
  assert.ok(
    links.length >= ROUTES.length,
    `expected >= ${ROUTES.length} nav items, got ${links.length}`,
  );
});

test(
  'home reports the app version from the Rust backend over IPC',
  { timeout: STEP_TIMEOUT },
  async () => {
    await app.navTo('/');
    // The version starts as a placeholder ("取得中…") and settles once the
    // get_app_version IPC call resolves — poll until it is a real semver.
    const shown = (await app.waitForText('.env dd', (t) => SEMVER.test(t.trim()))).trim();

    assert.equal(shown, pkg.version, `live app version ${shown} != package.json ${pkg.version}`);
    // get_app_version() returns env!("CARGO_PKG_VERSION"); package.json,
    // Cargo.toml and tauri.conf.json are kept in sync, so this also guards the
    // three-file version bump from drifting.
  },
);

// Every sidebar route must navigate (client-side routing) and render content.
for (const [route, name] of ROUTES) {
  test(`route ${route} (${name}) navigates and renders`, { timeout: STEP_TIMEOUT }, async () => {
    await app.navTo(route);

    const at = await app.execute('return location.pathname;');
    assert.equal(at, route, `pathname did not settle on ${route}`);

    // The route's page rendered *something* into the content area (not a blank
    // page / "could not connect" error screen).
    const childCount = await app.execute(
      'return document.querySelector("main.content")?.childElementCount ?? 0;',
    );
    assert.ok(childCount > 0, `main.content is empty on ${route}`);

    // The matching sidebar entry is marked active.
    const activeHref = await app.execute(
      'return document.querySelector("a.nav-item.active")?.getAttribute("href") ?? null;',
    );
    assert.equal(activeHref, route, `active nav item is ${activeHref}, expected ${route}`);
  });
}
