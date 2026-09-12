const assert = require('node:assert/strict');
const {createServer} = require('node:http');
const {readFile, mkdir, writeFile, realpath} = require('node:fs/promises');
const {resolve, extname, sep} = require('node:path');
const {chromium} = require('playwright');
const {parse, serialize} = require('parse5');

const root = resolve('target/dx/aio-idea/release/web/public');
const output = resolve('target/startup-test/local');
const mime = {'.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.svg': 'image/svg+xml'};
let origin, mode = 'ok', requests = [], revision = 'a';
const catalog = {
  session_context: 'login-a', context: 'permissions-a', tenant: {id: 'startup', label: 'Startup'},
  user: {label: 'Tester', handle: '@tester', initials: 'T'}, plugins: [], account_items: [], page_versions: {},
  pages: [{id: 'startup-test', label: 'Startup test', scene: {id: 'startup', label: 'Startup'}, menu_path: [],
    required_permission: null, body: {kind: 'text', title: 'Startup test', content: 'Ready'}}],
};
const server = createServer(async (req, res) => {
  try {
    const path = new URL(req.url, origin).pathname;
    if (path.startsWith('/api/')) {
      requests.push({path, etag: req.headers['if-none-match'], mode});
      if (path !== '/api/runtime/bootstrap') return res.writeHead(404).end();
      if (mode === 'hang') return;
      if (mode === 'body-hang') { res.writeHead(200, {'content-type': 'application/json'}); res.flushHeaders(); return; }
      if (mode === 'fail') return res.writeHead(503).end();
      if (mode === 'fail-once') { mode = 'ok'; return res.writeHead(503).end(); }
      const etag = `"${revision}"`;
      const headers = {'content-type': 'application/json', 'cache-control': 'private, no-store', etag};
      if (req.headers['if-none-match'] === etag) return res.writeHead(304, headers).end();
      return res.writeHead(200, headers).end(JSON.stringify({data: mode === 'logout' ? null : {permissions: [], catalog}}));
    }
    if (path === '/favicon.ico') return res.writeHead(204).end();
    const full = await realpath(resolve(root, path === '/' ? 'index.html' : path.slice(1)));
    assert(full.startsWith(await realpath(root) + sep));
    let bytes = await readFile(full);
    if (path === '/' && mode === 'embedded') {
      const document = parse(bytes.toString());
      const head = document.childNodes.find(node => node.tagName === 'html').childNodes.find(node => node.tagName === 'head');
      const script = {nodeName: 'script', tagName: 'script', namespaceURI: 'http://www.w3.org/1999/xhtml',
        attrs: [{name: 'id', value: 'aio-startup-snapshot'}, {name: 'type', value: 'application/json'}], childNodes: [], parentNode: head};
      script.childNodes.push({nodeName: '#text', value: JSON.stringify({snapshot: {permissions: [], catalog}, etag: `"${revision}"`}).replaceAll('<', '\\u003c'), parentNode: script});
      head.childNodes.unshift(script);
      bytes = Buffer.from(serialize(document));
    }
    res.writeHead(200, {'content-type': mime[extname(full)] || 'application/octet-stream'}).end(bytes);
  } catch (error) { res.writeHead(500).end(error.message); }
});
const invalidated = page => page.evaluate(() => dispatchEvent(new Event('aio:catalog-invalidated')));
async function run(browser, mobile) {
  const context = await browser.newContext({viewport: mobile ? {width: 390, height: 844} : {width: 1440, height: 1000}});
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const shell = page.locator('.application-shell');
  try {
    mode = 'embedded'; revision = 'a'; requests = [];
    await page.goto(origin); await shell.waitFor();
    await page.waitForFunction(() => getComputedStyle(document.querySelector('.application-shell')).display === 'grid');
    assert.equal(requests.length, 0, 'An authenticated HTML snapshot must render without an extra startup request');
    assert.equal(await page.locator('#aio-startup-snapshot').count(), 0, 'Consume the document snapshot only once');
    const embeddedPoll = page.waitForResponse(r => r.url().endsWith('/bootstrap') && r.status() === 304);
    await invalidated(page); await embeddedPoll;
    assert.equal(requests[0].etag, '"a"');
    mode = 'ok'; revision = 'a'; requests = [];
    const start = performance.now();
    await page.goto(origin);
    await shell.waitFor();
    await page.waitForFunction(() => getComputedStyle(document.querySelector('.application-shell')).display === 'grid');
    const coldMs = performance.now() - start;
    assert.equal(requests.length, 1, 'Startup should use one HTTP snapshot');
    await shell.evaluate(node => node.dataset.startupMarker = 'retained');
    const unchanged = page.waitForResponse(r => r.url().endsWith('/bootstrap') && r.status() === 304);
    await invalidated(page); await unchanged;
    assert.equal(requests[1].etag, '"a"');
    assert.equal(await shell.getAttribute('data-startup-marker'), 'retained');

    mode = 'fail';
    const failure = page.waitForResponse(r => r.url().endsWith('/bootstrap') && requests.length >= 4);
    await invalidated(page); await failure;
    await shell.waitFor();
    assert.equal(await shell.getAttribute('data-startup-marker'), 'retained');
    assert.equal(await page.getByRole('alert').count(), 0, 'Background failure should keep the workspace');
    await page.screenshot({path: resolve(output, `${mobile ? 'mobile' : 'desktop'}.png`)});

    mode = 'fail-once'; requests = [];
    await page.reload(); await shell.waitFor();
    assert.equal(requests.length, 2, 'A transient startup failure retries once');

    const bounds = [];
    for (const timeoutMode of ['hang', 'body-hang']) {
      mode = timeoutMode; requests = [];
      await page.reload();
      await page.getByRole('status').waitFor();
      const pending = performance.now();
      for (let i = 0; i < 3; i++) {
        await page.evaluate(() => dispatchEvent(new Event('focus')));
        await page.waitForTimeout(150);
      }
      assert.equal(requests.length, 1, 'Focus must not cancel and restart a pending load');
      await page.getByRole('alert').waitFor({timeout: 19000});
      const elapsed = performance.now() - pending;
      assert(elapsed < 18000, `Request and retry must be bounded: ${elapsed}`);
      assert.equal(requests.length, 2);
      bounds.push({mode: timeoutMode, elapsedMs: elapsed, attempts: requests.length});
      mode = 'ok';
      await page.getByRole('button', {name: '重试', exact: true}).click();
      await shell.waitFor();
    }
    mode = 'logout'; revision = 'logged-out';
    await invalidated(page);
    await shell.waitFor({state: 'detached'});
    await page.getByRole('button', {name: '登录', exact: true}).waitFor();
    assert.deepEqual(errors, []);
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    return {viewport: mobile ? 'mobile' : 'desktop', coldMs, embeddedStartupRequests: 0, fallbackStartupRequests: 1, unchanged304: true,
      workspaceRetained: true, transientRetry: true, focusDoesNotRestart: true, logoutClearsWorkspace: true, bounds, consoleErrors: errors};
  } catch (error) {
    await page.screenshot({path: resolve(output, 'failure.png')}); throw error;
  } finally { await context.close(); }
}
(async () => {
  await mkdir(output, {recursive: true});
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({channel: 'chrome', headless: true});
  try {
    const results = [await run(browser, false), await run(browser, true)];
    await writeFile(resolve(output, 'report.json'), JSON.stringify(results, null, 2));
    console.log(JSON.stringify(results, null, 2));
  } finally { await browser.close(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => {console.error(error); process.exitCode = 1;});
