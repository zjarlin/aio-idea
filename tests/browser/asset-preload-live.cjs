const assert = require('node:assert/strict');
const path = require('node:path');
const { mkdirSync, writeFileSync } = require('node:fs');
const { PNG } = require('pngjs');
const { launchBrowser, contextFor, closeBrowser } = require('./live-session.cjs');

const base = process.env.AIO_URL || 'https://aio.addzero.site';
const baseline = process.argv.includes('baseline');
const directory = path.resolve('target/component-delivery/asset-preload');
mkdirSync(directory, { recursive: true });

async function run() {
  const browser = await launchBrowser();
  try {
    const context = await contextFor(browser, base, false);
    const page = await context.newPage();
    page.setDefaultTimeout(120000);
    const wasm = [], errors = [], temporary = new Set(), released = new Set(), expectedWasm = new Set();
    let backgroundBusiness = 0;
    let opening = false;
    const redact = value => value.replace(/\/components\/assets\/[^/]+/g, '/components/assets/[token]').replace(/\/components\/[^/]+\/(request|renew)/g, '/components/[token]/$1');
    page.on('pageerror', error => errors.push(redact(error.message)));
    page.on('console', message => { if (message.type() === 'error') errors.push(redact(message.text())); });
    page.on('request', request => {
      const url = new URL(request.url());
      if (/\/components\/assets\/.*\.wasm$/.test(url.pathname)) wasm.push({ file: url.pathname.split('/').at(-1), at: Date.now() });
      if (!opening && /\/components\/[^/]+\/request$/.test(url.pathname)) backgroundBusiness++;
      if (request.method() === 'DELETE' && url.pathname.startsWith('/api/runtime/frontend/')) released.add(url.pathname.split('/').at(-1));
    });
    page.on('response', response => {
      if (!opening && response.url().endsWith('/api/runtime/frontend/mount') && response.ok()) void response.json().then(({ data }) => {
        temporary.add(data.token);
        if (data.abi === 2) for (const [name, digest] of Object.entries(data.assets)) if (name.endsWith('.wasm')) expectedWasm.add(digest);
        console.log(JSON.stringify({ backgroundMount: data.abi || 1, assetCount: Object.keys(data.assets).length }));
      }).catch(() => {});
    });
    const catalog = (await (await context.request.get(`${base}/api/runtime/catalog`)).json()).data;
    const pages = catalog.pages.filter(item => item.body.kind === 'frontend').length;
    await page.goto(base);
    await page.locator('.application-shell:visible').waitFor();
    const shellAt = Date.now();
    if (!baseline) {
      for (let i = 0; i < 240; i++) {
        if (temporary.size >= pages && [...temporary].every(token => released.has(token))) break;
        await page.waitForTimeout(1000);
      }
      console.log(JSON.stringify({ backgroundMounts: temporary.size, releasedMounts: released.size, backgroundWasm: wasm.map(item => item.file), backgroundBusiness, errors }));
      assert(temporary.size >= pages, 'Background queue did not visit all accessible plugins');
      assert([...temporary].every(token => released.has(token)), 'Background mount was not released');
      assert.equal(backgroundBusiness, 0, 'Warming executed plugin business requests');
      assert(expectedWasm.size >= 3, 'Agent and Memory manifests were not received');
      assert(await page.evaluate(async digests => {
        const keys = (await Promise.all((await caches.keys()).filter(name => name.startsWith('aio-plugin-assets-v1-')).map(async name => (await (await caches.open(name)).keys()).map(key => key.url)))).flat();
        return digests.every(digest => keys.some(key => key.includes(`/${digest}/`)));
      }, [...expectedWasm]), 'The background budget omitted an Agent or Memory Wasm');
    }
    const warmingMs = baseline ? null : Date.now() - shellAt;
    const measurements = [];
    for (const mode of ['first', 'refresh']) {
      if (mode === 'refresh') {
        await page.reload();
        await page.locator('.application-shell:visible').waitFor();
        await page.waitForTimeout(2000);
      }
      opening = true;
      const before = wasm.length;
      const start = Date.now();
      await page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: '社区插件', exact: true }).click();
      const navigation = page.locator('.application-shell__sidebar');
      const memory = navigation.getByRole('button', { name: '记忆图谱', exact: true });
      if (!await memory.isVisible()) await navigation.getByRole('button', { name: '智能体', exact: true }).click();
      await memory.click();
      const frame = page.frameLocator('iframe[title="记忆图谱"]');
      await frame.locator('canvas').first().waitFor();
      await frame.getByRole('button', { name: '新建记忆', exact: true }).first().waitFor();
      measurements.push({ mode, readyMs: Date.now() - start, wasmDownloads: wasm.length - before });
      console.log(JSON.stringify(measurements.at(-1)));
      if (!baseline) assert.equal(wasm.length - before, 0, 'Opening downloaded a prewarmed Wasm again');
      for (const [name, viewport] of [['desktop', { width: 1440, height: 1000 }], ['mobile', { width: 390, height: 844 }]]) {
        await page.setViewportSize(viewport);
        await page.waitForTimeout(800);
        const png = PNG.sync.read(await page.screenshot({ path: path.join(directory, `${baseline ? 'baseline' : 'warm'}-${mode}-${name}.png`) }));
        let painted = 0;
        for (let i = 0; i < png.data.length; i += 4) if (png.data[i + 1] > png.data[i] + 15 && png.data[i + 1] > png.data[i + 2]) painted++;
        assert(painted > 100, 'Memory canvas was blank');
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      }
      await page.setViewportSize({ width: 1440, height: 1000 });
    }
    assert.deepEqual(errors, []);
    const report = { base, baseline, warmingMs, backgroundBusiness, measurements, wasmRequests: wasm.length, errors };
    writeFileSync(path.join(directory, `${baseline ? 'baseline' : 'warm'}.json`), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally { await closeBrowser(browser); }
}
run().catch(error => { console.error(error.message.split('Call log:')[0]); process.exitCode = 1; });
