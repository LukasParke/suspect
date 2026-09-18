#!/usr/bin/env node
// TEST ONLY. Real Chrome page checks; default server receives GETs only.
// All click-to-run checks use the separate controlled_server.py entry point.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const args = process.argv.slice(2);
function option(name, fallback) {
  const at = args.indexOf(name);
  return at === -1 ? fallback : args[at + 1];
}
const liveURL = new URL(option('--url', 'http://127.0.0.1:8765')).origin;
const output = path.resolve(option('--output', path.join(repo, 'target/sdk-demo-web-20260911-01/checks/browser-01')));
assert(path.relative(path.join(repo, 'target'), output).startsWith('sdk-demo-web-20260911-'), 'Use fresh web-owned browser evidence');
const tools = option('--tools', '/private/var/folders/cp/c0_kzhh92pngpr3xyxx0h9w00000gn/T/opencode/sdk-editor-native-host-tl2581zs/tools');
const require = createRequire(path.join(tools, 'package.json'));
const { chromium } = require('playwright-core');
const canary = 'web-browser-controlled-canary-only';
const checks = [];
const screenshots = [];
const traffic = [];
const consoleErrors = [];
const violations = [];
let browser;
let controlled;
let controlledOutput = '';
let controlledError = '';
let report;

async function check(name, action) {
  const started = performance.now();
  const detail = await action();
  checks.push({ name, passed: true, durationMs: Math.round(performance.now() - started), ...(detail || {}) });
  console.log(`PASS ${name}`);
}

async function eventually(action, timeout = 10000) {
  const end = performance.now() + timeout;
  while (performance.now() < end) {
    try {
      const value = await action();
      if (value) return value;
    } catch (error) {
      if (error.code !== 'ENOENT') throw error;
    }
    await delay(50);
  }
  throw new Error('Controlled browser setup did not become ready');
}

async function capture(page, name, fullPage = false, mode = 'actual-default-page') {
  const filename = `${name}.png`;
  await page.screenshot({ path: path.join(output, filename), fullPage, animations: 'disabled' });
  screenshots.push({ filename, mode, viewport: page.viewportSize(), fullPage });
}

async function localAPI(page, endpoint) {
  return page.evaluate(async endpoint => {
    const response = await fetch(endpoint, { headers: { 'X-SDK-Demo-Nonce': document.querySelector('meta[name="sdk-demo-nonce"]').content }, cache: 'no-store' });
    if (!response.ok) throw new Error('Local observation failed');
    return response.json();
  }, endpoint);
}

async function installGuards(context, origin, mode) {
  await context.route('**/*', async route => {
    const request = route.request();
    const url = new URL(request.url());
    const allowed = url.origin === origin && !url.search && !url.hash && !(mode === 'actual-default-page' && request.method() !== 'GET');
    const hasCanary = (request.postData() || '').includes(canary);
    if (traffic.length < 512) traffic.push({ mode, method: request.method(), path: url.pathname, credentialBody: hasCanary });
    if (!allowed || request.url().includes(canary) || (hasCanary && url.pathname !== '/api/credential')) {
      violations.push({ mode, method: request.method(), path: url.pathname });
      await route.abort();
    } else await route.continue();
  });
  context.on('page', page => {
    page.on('pageerror', error => consoleErrors.push({ mode, type: 'pageerror', message: String(error.message).replaceAll(canary, '[CONTROLLED CANARY OMITTED]').slice(0, 512) }));
    page.on('console', message => {
      if (message.type() === 'error') consoleErrors.push({ mode, type: 'console', message: message.text().replaceAll(canary, '[CONTROLLED CANARY OMITTED]').slice(0, 512) });
    });
  });
}

async function assertNoOverflow(page, width, height) {
  await page.setViewportSize({ width, height });
  const size = await page.evaluate(() => ({ width: innerWidth, document: document.documentElement.scrollWidth, body: document.body.scrollWidth }));
  assert(size.document <= width && size.body <= width, `No horizontal page overflow at ${width}px`);
  return size;
}

await mkdir(output, { recursive: false });
await mkdir(path.join(output, 'browser-tmp'));
try {
  const launchEnvironment = Object.fromEntries(Object.entries(process.env).filter(([name]) => name !== 'OPENROUTER_API_KEY' && !name.startsWith('SDK_DEMO_')));
  launchEnvironment.TMPDIR = path.join(output, 'browser-tmp');
  browser = await chromium.launch({
    executablePath: option('--chrome', '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'),
    headless: true,
    env: launchEnvironment,
    args: ['--disable-background-networking', '--disable-component-update', '--disable-sync', '--no-first-run', '--disable-crash-reporter', `--crash-dumps-dir=${path.join(output, 'browser-tmp')}`],
  });
  const actualContext = await browser.newContext({ viewport: { width: 1440, height: 1080 }, deviceScaleFactor: 1, reducedMotion: 'reduce' });
  await installGuards(actualContext, liveURL, 'actual-default-page');
  await actualContext.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: liveURL });
  const actual = await actualContext.newPage();
  let actualReady;
  await check('Actual default page: twelve ready native cards, zero live confirmations, no injected executor', async () => {
    const start = performance.now();
    await actual.goto(liveURL, { waitUntil: 'networkidle' });
    await actual.locator('.sdk-card[data-native="true"]').last().waitFor();
    actualReady = await localAPI(actual, '/api/readiness');
    assert.equal(actualReady.mode, 'live');
    assert.equal(actualReady.cards.filter(card => card.native && card.ready).length, 12);
    assert.equal(await actual.locator('.sdk-card[data-native="true"]').count(), 12);
    assert.equal(await actual.locator('#confirmed-count').textContent(), '0');
    assert.equal(await actual.locator('#test-banner').isVisible(), false);
    assert.equal(await actual.locator('#connection-error').isVisible(), false);
    const snapshot = await localAPI(actual, '/api/jobs');
    assert.equal(snapshot.submitted, 0);
    assert.equal(snapshot.jobs.length, 0);
    return { firstReadyMs: Math.round(performance.now() - start), readyNativeTargets: 12, liveJobsSubmitted: 0 };
  });
  await check('Actual default page: exact four-line excerpts and working clipboard copy', async () => {
    for (const card of actualReady.cards) {
      const displayed = await actual.locator(`#sdk-${card.id} .line-content`).allTextContents();
      assert.equal(displayed.length, 4);
      assert.equal(displayed.join('\n'), card.code);
    }
    await actual.getByRole('button', { name: 'Copy TypeScript example', exact: true }).click();
    await actual.waitForFunction(() => document.querySelector('#sdk-typescript .copy-code span').textContent === 'Copied');
    const clipboard = await actual.evaluate(() => navigator.clipboard.readText());
    assert.equal(clipboard, actualReady.cards.find(card => card.id === 'typescript').code);
    return { snippetsVerified: 13, clipboardVerified: true };
  });
  await check('Actual default page: desktop, tablet, and mobile layouts do not scroll horizontally', async () => {
    await actual.waitForFunction(() => document.querySelector('#toast').hidden);
    await actual.evaluate(() => scrollTo(0, 0));
    const desktop = await assertNoOverflow(actual, 1440, 1080);
    await capture(actual, 'actual-desktop');
    await capture(actual, 'actual-desktop-full', true);
    await actual.locator('#sdk-typescript').scrollIntoViewIfNeeded();
    await capture(actual, 'actual-sdk-cards');
    const tablet = await assertNoOverflow(actual, 768, 1024);
    await actual.evaluate(() => scrollTo(0, 0));
    await capture(actual, 'actual-tablet');
    const mobile = await assertNoOverflow(actual, 390, 844);
    await actual.evaluate(() => scrollTo(0, 0));
    await capture(actual, 'actual-mobile');
    await capture(actual, 'actual-mobile-full', true);
    const compact = await assertNoOverflow(actual, 320, 740);
    await actual.evaluate(() => scrollTo(0, 0));
    await capture(actual, 'actual-compact-mobile');
    return { desktop, tablet, mobile, compact };
  });
  await check('Actual default page: keyboard focus, labels, live region, and full-source links', async () => {
    await actual.setViewportSize({ width: 1440, height: 1080 });
    await actual.evaluate(() => { scrollTo(0, 0); document.activeElement?.blur(); });
    await actual.keyboard.press('Tab');
    const focus = await actual.evaluate(() => {
      const element = document.activeElement;
      const style = getComputedStyle(element);
      return { tag: element.tagName, outline: style.outlineStyle, width: style.outlineWidth };
    });
    assert.notEqual(focus.outline, 'none');
    assert.notEqual(focus.width, '0px');
    assert.equal(await actual.getByLabel('OpenRouter API key', { exact: true }).getAttribute('type'), 'password');
    assert.equal(await actual.locator('#activity-announcement').getAttribute('aria-live'), 'polite');
    for (const card of actualReady.cards) {
      for (const endpoint of [card.sourceUrl, card.docsUrl]) {
        const status = await actual.evaluate(async endpoint => (await fetch(endpoint)).status, endpoint);
        assert.equal(status, 200);
      }
    }
    const stats = await localAPI(actual, '/api/jobs');
    assert.equal(stats.submitted, 0);
    return { sourceAndReadmeLinks: 26, keyboardFocusVisible: true };
  });

  const controlledRoot = path.join(output, 'controlled-server');
  controlled = spawn('python3', ['-B', path.join(here, 'controlled_server.py'), '--port', '0', '--evidence-dir', controlledRoot], { cwd: repo, env: launchEnvironment, stdio: ['ignore', 'pipe', 'pipe'] });
  controlled.stdout.on('data', data => { controlledOutput = (controlledOutput + data.toString()).slice(0, 16384); });
  controlled.stderr.on('data', data => { controlledError = (controlledError + data.toString()).slice(0, 16384); });
  const serverInfo = await eventually(async () => JSON.parse(await readFile(path.join(controlledRoot, 'server.json'), 'utf8')));
  const controlledURL = new URL(serverInfo.url).origin;
  const context = await browser.newContext({ viewport: { width: 1440, height: 1080 }, deviceScaleFactor: 1, reducedMotion: 'reduce' });
  await installGuards(context, controlledURL, 'controlled-ui-test');
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: controlledURL });
  const page = await context.newPage();
  await page.goto(controlledURL, { waitUntil: 'networkidle' });
  await page.locator('.sdk-card[data-native="true"]').last().waitFor();
  await check('Controlled UI: test banner, no automatic run, password-only key entry', async () => {
    assert.equal(await page.locator('#test-banner').isVisible(), true);
    assert.equal((await localAPI(page, '/api/jobs')).submitted, 0);
    await page.locator('#run-all').click();
    assert.equal(await page.locator('#key-error').isVisible(), true);
    assert.equal(await page.evaluate(() => document.activeElement.id), 'api-key');
    assert.equal((await localAPI(page, '/api/jobs')).submitted, 0);
    await page.getByLabel('OpenRouter API key', { exact: true }).fill(canary);
    assert.equal(await page.locator('#api-key').getAttribute('type'), 'password');
    await page.locator('#save-key').click();
    await page.waitForFunction(() => document.querySelector('#credential-state').textContent.includes('loaded in memory'));
    assert.equal(await page.locator('#api-key').inputValue(), '');
    assert.equal((await localAPI(page, '/api/jobs')).submitted, 0);
    assert.equal(await page.locator('body').innerText().then(text => text.includes(canary)), false);
  });
  await check('Controlled UI: Run all queues twelve, runs three, stays interactive', async () => {
    const start = performance.now();
    await page.locator('#run-all').click();
    await page.waitForFunction(() => Number(document.querySelector('#queued-count').textContent) > 0 && Number(document.querySelector('#running-count').textContent) === 3);
    const queuedMs = Math.round(performance.now() - start);
    const status = await localAPI(page, '/api/jobs');
    assert.equal(status.submitted, 12);
    assert.equal(status.jobs.some(job => job.language === 'javascript'), false);
    const apiStart = performance.now();
    await localAPI(page, '/api/readiness');
    const readinessMs = Math.round(performance.now() - apiStart);
    assert(readinessMs < 500, 'Readiness remains responsive while children run');
    await page.getByRole('button', { name: 'Copy TypeScript example', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('#sdk-typescript .copy-code span').textContent === 'Copied');
    await page.evaluate(() => scrollTo(0, 0));
    await capture(page, 'controlled-queue-running', false, 'controlled-ui-test');
    return { submittedControlledJobs: 12, concurrency: 3, buttonToQueuedMs: queuedMs, readinessWhileRunningMs: readinessMs };
  });
  await check('Controlled UI: decoded 200 and typed 401 produce truthful distinct results', async () => {
    await page.waitForFunction(() => document.querySelector('#confirmed-count').textContent === '11' && document.querySelector('#failed-count').textContent === '1' && document.querySelector('#queued-count').textContent === '0' && document.querySelector('#running-count').textContent === '0', null, { timeout: 15000 });
    assert.equal(await page.locator('#sdk-typescript').getAttribute('data-state'), 'completed');
    assert.equal(await page.locator('#sdk-python').getAttribute('data-state'), 'failed');
    assert.equal(await page.locator('#sdk-python .result-details').getAttribute('open'), '');
    assert.equal(JSON.parse(await page.locator('#sdk-python .result-json').textContent()).status, 401);
    const typed = JSON.parse(await page.locator('#sdk-typescript .result-json').textContent());
    assert.equal(typed.status, 200);
    assert.equal(typed.usage, '9007199254740993.000000000000000001');
    assert.equal(typed.freeTier, false);
    await page.locator('#sdk-typescript .result-details summary').click();
    assert.equal(await page.locator('#sdk-typescript .result-json').isVisible(), true);
    await page.evaluate(() => scrollTo(0, document.querySelector('#sdk-typescript').getBoundingClientRect().top + scrollY - 100));
    await capture(page, 'controlled-native-style-results', false, 'controlled-ui-test');
    return { controlledSuccessCards: 11, controlledFailureCards: 1, actualNativeSdkExecutions: 0 };
  });
  await check('Controlled UI: optional JavaScript runs outside the twelve-native count', async () => {
    await page.locator('#toggle-bonus').click();
    assert.equal(await page.locator('#toggle-bonus').getAttribute('aria-expanded'), 'true');
    await page.getByRole('button', { name: 'Run JavaScript live request', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('#sdk-javascript').dataset.state === 'completed');
    assert.equal(await page.locator('#confirmed-count').textContent(), '11');
    await capture(page, 'controlled-javascript-bonus', false, 'controlled-ui-test');
  });
  await check('Controlled UI: per-job cancel and Cancel all stop queued and active work', async () => {
    await page.locator('#run-all').click();
    await page.waitForFunction(() => document.querySelector('#sdk-cpp').dataset.state === 'queued');
    await page.getByRole('button', { name: 'Cancel C++ request', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('#sdk-cpp').dataset.state === 'cancelled');
    await page.locator('#cancel-all').click();
    await page.waitForFunction(() => document.querySelector('#queued-count').textContent === '0' && document.querySelector('#running-count').textContent === '0' && Number(document.querySelector('#cancelled-count').textContent) > 0);
    const jobs = await localAPI(page, '/api/jobs');
    assert(jobs.counts.cancelled > 0);
    assert.equal(jobs.jobs.some(job => ['running', 'queued'].includes(job.state)), false);
    await page.evaluate(() => scrollTo(0, 0));
    await capture(page, 'controlled-cancelled', false, 'controlled-ui-test');
  });
  await check('Controlled UI: clearing the key cancels work and leaves no browser credential storage', async () => {
    await page.locator('#run-all').click();
    await page.waitForFunction(() => Number(document.querySelector('#running-count').textContent) > 0);
    await page.locator('#clear-key').click();
    await page.waitForFunction(() => document.querySelector('#credential-state').textContent === 'No key loaded');
    await page.waitForFunction(() => document.querySelector('#queued-count').textContent === '0' && document.querySelector('#running-count').textContent === '0');
    const state = await localAPI(page, '/api/jobs');
    assert.equal(state.credential.ready, false);
    assert.equal(await page.locator('#api-key').inputValue(), '');
    assert.equal(await page.evaluate(value => document.body.innerText.includes(value), canary), false);
    assert.deepEqual(await page.evaluate(() => ({ local: localStorage.length, session: sessionStorage.length })), { local: 0, session: 0 });
    assert.equal((await context.cookies()).length, 0);
    await page.getByRole('button', { name: 'Run Python live request', exact: true }).click();
    assert.equal((await localAPI(page, '/api/jobs')).submitted, state.submitted);
    assert.equal(await page.locator('#key-error').isVisible(), true);
    await assertNoOverflow(page, 390, 844);
    await page.evaluate(() => scrollTo(0, 0));
    await capture(page, 'controlled-mobile-cleared', false, 'controlled-ui-test');
  });
  await check('Browser traffic is local-only; default server still has zero submitted jobs', async () => {
    assert.equal(violations.length, 0);
    assert.equal(consoleErrors.length, 0);
    const final = await localAPI(actual, '/api/jobs');
    assert.equal(final.submitted, 0);
    assert.equal(final.jobs.length, 0);
    for (const name of await readdir(path.join(controlledRoot, 'jobs'))) {
      const text = await readFile(path.join(controlledRoot, 'jobs', name), 'utf8');
      assert.equal(text.includes(canary), false);
      assert.equal(JSON.parse(text).mode, 'controlled-test');
    }
    return { defaultSubmittedJobs: 0, externalPageRequests: 0, browserConsoleErrors: 0 };
  });
  report = { status: 'passed', mode: 'real-browser-local-pages-with-isolated-controlled-jobs', actualDefaultURL: liveURL,
    browserVersion: browser.version(), checks, screenshots, traffic, consoleErrors, violations,
    nativeSdkExecutions: 0, openRouterRequests: 0, realTokenUsed: false,
    preparedNativeTargets: 12, nativeControlledMatrixReplayed: false };
} catch (error) {
  report = { status: 'failed', mode: 'real-browser-local-pages-with-isolated-controlled-jobs', checks, screenshots, traffic, consoleErrors, violations,
    error: String(error.stack || error).replaceAll(canary, '[CONTROLLED CANARY OMITTED]').slice(0, 8192), nativeSdkExecutions: 0, openRouterRequests: 0, realTokenUsed: false };
  process.exitCode = 1;
} finally {
  if (browser) await browser.close();
  if (controlled) {
    controlled.kill('SIGTERM');
    await eventually(() => controlled.exitCode !== null || controlled.signalCode !== null, 6000).catch(() => controlled.kill('SIGKILL'));
    await writeFile(path.join(output, 'controlled-server.stdout.log'), controlledOutput.replaceAll(canary, '[CONTROLLED CANARY OMITTED]'), { flag: 'wx' });
    await writeFile(path.join(output, 'controlled-server.stderr.log'), controlledError.replaceAll(canary, '[CONTROLLED CANARY OMITTED]'), { flag: 'wx' });
  }
  await writeFile(path.join(output, 'REPORT.json'), JSON.stringify(report, null, 2) + '\n', { flag: 'wx' });
  console.log(JSON.stringify({ status: report.status, checks: checks.length, screenshots: screenshots.length, report: path.join(output, 'REPORT.json') }));
  if (report.error) console.error(report.error);
}
