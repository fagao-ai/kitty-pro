// Layout smoke test for the Kitty Pro web shell.
//
// Runs against a real server so the numbers checked here are the numbers a user
// gets: KITTY_WEB_URL points at `dx serve` during development and at the
// standalone `kitty-pro-web-server` in CI.
const { defineConfig } = require('@playwright/test');

const baseURL = process.env.KITTY_WEB_URL || 'http://127.0.0.1:7897';

// CI installs Playwright's own Chromium. Locally, set KITTY_BROWSER=chrome to
// reuse an installed Google Chrome and skip the ~150 MB download.
const useSystemChrome = process.env.KITTY_BROWSER === 'chrome';

module.exports = defineConfig({
  testDir: __dirname,
  timeout: 120000,
  expect: { timeout: 15000 },
  fullyParallel: false,
  workers: 1,
  reporter: process.env.CI ? [['list'], ['github']] : [['list']],
  use: {
    baseURL,
    viewport: { width: 1440, height: 900 },
    trace: 'retain-on-failure',
    // app.css collapses every animation under this query, which keeps entry
    // transitions (the modal slides 8px) from skewing geometry measurements.
    reducedMotion: 'reduce',
    ...(useSystemChrome ? { channel: 'chrome' } : {}),
  },
});
