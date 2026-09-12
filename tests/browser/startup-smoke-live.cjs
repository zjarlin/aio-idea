const assert = require('node:assert/strict');
const {mkdir, writeFile} = require('node:fs/promises');
const {resolve} = require('node:path');
const {PNG} = require('pngjs');
const {contextFor, select, marketplace, launchBrowser, closeContext, closeBrowser, viewportScreenshot} = require('./live-session.cjs');
const base = process.env.AIO_URL || 'https://aio.addzero.site';
const output = resolve('target/startup-test/plugins');

async function run(browser, mobile) {
  const context = await contextFor(browser, base, mobile);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  try {
    await page.goto(base, {waitUntil: 'domcontentloaded', timeout: 60000});
    await page.waitForFunction(() => {
      const shell = document.querySelector('.application-shell');
      return shell && getComputedStyle(shell).display === 'grid';
    }, null, {timeout: 60000});
    await page.getByRole('navigation', {name: '场景'}).getByRole('button', {name: '社区插件', exact: true}).click();
    await select(page, mobile, 'KMP 全栈示例');
    const iframe = page.locator('iframe[title="KMP 全栈示例"]');
    const frame = page.frameLocator('iframe[title="KMP 全栈示例"]');
    await frame.getByRole('button', {name: 'Counter', exact: true}).waitFor({timeout: 120000});
    await frame.getByRole('button', {name: 'Counter', exact: true}).click({force: true});
    await frame.getByText('KMP Counter1', {exact: true}).waitFor();
    await frame.getByText('0', {exact: true}).waitFor();
    await page.waitForTimeout(350);
    const canvas = frame.locator('canvas').first();
    const before = PNG.sync.read(await canvas.screenshot());
    const increment = await frame.getByRole('button', {name: '+1', exact: true}).boundingBox();
    assert(increment);
    await page.mouse.click(increment.x + increment.width / 2, increment.y + increment.height / 2);
    await frame.getByText('1', {exact: true}).waitFor();
    await page.mouse.move(0, 0);
    const after = PNG.sync.read(await canvas.screenshot());
    let changed = 0;
    for (let i = 0; i < before.data.length; i += 4) if (before.data.readUInt32BE(i) !== after.data.readUInt32BE(i)) changed++;
    assert(changed > 30);
    const src = await iframe.getAttribute('src');
    const marker = await frame.locator('body').evaluate(() => window.__startupMarker = Math.random());
    const poll = page.waitForResponse(response => response.url().endsWith('/api/runtime/bootstrap') && response.status() === 304);
    await page.evaluate(() => dispatchEvent(new Event('aio:catalog-invalidated')));
    await poll;
    assert.equal(await iframe.getAttribute('src'), src);
    assert.equal(await frame.locator('body').evaluate(() => window.__startupMarker), marker);
    await frame.getByText('1', {exact: true}).waitFor();
    await viewportScreenshot(page, resolve(output, `${mobile ? 'mobile' : 'desktop'}-counter.png`));
    await marketplace(page, mobile);
    await page.getByRole('textbox', {name: '搜索插件', exact: true}).fill('KMP');
    await page.getByRole('treeitem').filter({hasText: 'KMP 全栈示例'}).click();
    await page.locator('.dx-markdown h1').waitFor({timeout: 60000});
    await page.waitForFunction(() => {
      const image = document.querySelector('.dx-markdown img');
      return image?.complete && image.naturalWidth > 0;
    }, null, {timeout: 60000});
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    await viewportScreenshot(page, resolve(output, `${mobile ? 'mobile' : 'desktop'}-marketplace.png`));
    assert.deepEqual(errors, []);
    return {viewport: mobile ? 'mobile' : 'desktop', connection: process.env.AIO_BROWSER_PROXY ? 'explicit-test-proxy' : 'direct', title: 'KMP Counter1', counter: 1, changedCanvasPixels: changed,
      unchanged304: true, iframeRetained: true, readmeImage: true, consoleErrors: errors};
  } catch (error) {
    await viewportScreenshot(page, resolve(output, `${mobile ? 'mobile' : 'desktop'}-failure.png`));
    throw error;
  } finally {await closeContext(context);}
}
(async () => {
  await mkdir(output, {recursive: true});
  const browser = await launchBrowser();
  try {
    const result = [];
    for (const mobile of [false, true].filter(value => !process.env.AIO_SMOKE_VIEWPORT || process.env.AIO_SMOKE_VIEWPORT === (value ? 'mobile' : 'desktop'))) {
      result.push(await run(browser, mobile));
      await writeFile(resolve(output, `${mobile ? 'mobile' : 'desktop'}-report.json`), JSON.stringify(result.at(-1), null, 2));
      console.log(JSON.stringify(result.at(-1)));
    }
  } finally {await closeBrowser(browser);}
})().catch(error => {console.error(error.message.split('Call log:')[0]); process.exitCode = 1;});
