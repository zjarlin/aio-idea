const assert = require('node:assert/strict');
const { readFile, mkdir, writeFile } = require('node:fs/promises');
const { resolve } = require('node:path');
const { chromium } = require('playwright');

const origin = process.env.AIO_TEST_ORIGIN || 'https://aio.addzero.site';
const output = resolve('target/tenant-cache-live');

async function run(browser, cookie, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  await context.addCookies([{ name: 'aio_session', value: cookie, url: origin, httpOnly: true, secure: true, sameSite: 'Lax' }]);
  const get = async path => {
    const response = await context.request.get(origin + path);
    assert.equal(response.status(), 200, path);
    return (await response.json()).data;
  };
  const session = await get('/api/auth/session');
  assert(session, 'A valid session cookie is required');
  const original = session.tenant_id;
  const other = process.env.AIO_TEST_OTHER_TENANT;
  assert(other && other !== original, 'Set AIO_TEST_OTHER_TENANT to a different authorized tenant with both examples installed');
  const page = await context.newPage();
  const errors = [];
  const downloads = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('request', request => {
    const path = new URL(request.url()).pathname;
    if (/\/frontend\/assets\//.test(path)) downloads.push(path);
  });
  const scene = () => page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: '社区插件', exact: true }).click();
  const select = async label => {
    if (mobile) await page.getByRole('button', { name: '打开菜单', exact: true }).click();
    const nav = mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
    await nav.getByRole('button', { name: label, exact: true }).click();
  };
  const selector = (tenant, label) => `[data-aio-workspace="${tenant}"] iframe[title="${label}"]`;
  const switchTo = async tenant_id => {
    const response = await context.request.post(origin + '/api/tenants/switch', { data: { tenant_id } });
    assert.equal(response.status(), 204, 'Tenant switch');
    const catalog = page.waitForResponse(r => r.url().endsWith('/api/runtime/bootstrap'));
    await page.evaluate(() => dispatchEvent(new Event('aio:catalog-invalidated')));
    await catalog;
    await page.locator(`[data-aio-workspace="${tenant_id}"][data-aio-workspace-active="true"]`).first().waitFor({ state: 'attached' });
  };
  const counter = async (tenant, value) => {
    const frame = page.frameLocator(selector(tenant, 'KMP 全栈示例'));
    await frame.getByRole('button', { name: 'Counter', exact: true }).waitFor({ timeout: 180000 });
    await frame.getByRole('button', { name: 'Counter', exact: true }).click({ force: true });
    await frame.getByRole('button', { name: '+1', exact: true }).waitFor();
    await frame.getByText(String(value), { exact: true }).waitFor();
    return frame;
  };
  try {
    await page.goto(origin);
    await scene();
    const rust = page.frameLocator(selector(original, 'Dioxus 全栈计数器'));
    await rust.getByRole('button', { name: '+1', exact: true }).waitFor({ timeout: 90000 });
    console.log(`${mobile ? 'mobile' : 'desktop'}: Dioxus loaded`);
    await rust.getByRole('button', { name: '+1', exact: true }).click();
    await select('KMP 全栈示例');
    const frame = await counter(original, 0);
    console.log(`${mobile ? 'mobile' : 'desktop'}: Compose loaded`);
    await frame.getByRole('button', { name: '+1', exact: true }).click({ force: true });
    await frame.getByText('1', { exact: true }).waitFor();
    const marker = await frame.locator('body').evaluate(() => window.__tenantCacheProbe = Math.random());
    const source = await page.locator(selector(original, 'KMP 全栈示例')).getAttribute('src');
    const oldToken = source.match(/assets\/([^/]+)/)[1];
    await switchTo(other);
    await page.locator(selector(original, 'KMP 全栈示例')).waitFor({ state: 'hidden' });
    assert.match(await frame.locator('body').evaluate(() => window.aioPlugin.json('GET', '/tasks').catch(e => e.message)), /暂停/);
    await scene();
    await select('KMP 全栈示例');
    const frameB = await counter(other, 0);
    await frameB.getByRole('button', { name: '+1', exact: true }).click({ force: true });
    await frameB.getByText('1', { exact: true }).waitFor();
    const before = downloads.length;
    const start = performance.now();
    await switchTo(original);
    await frame.getByText('1', { exact: true }).waitFor({ timeout: 1000 });
    const returnMs = performance.now() - start;
    assert.equal(await page.locator(selector(original, 'KMP 全栈示例')).getAttribute('src'), source);
    assert.equal(await frame.locator('body').evaluate(() => window.__tenantCacheProbe), marker);
    assert.equal(downloads.length, before, 'A return must not fetch assets');
    const stale = await context.request.post(origin + `/api/runtime/frontend/${oldToken}/renew`);
    assert.equal(stale.status(), 401, 'Revoked tickets must remain unauthorized after returning');
    const backend = await frame.locator('body').evaluate(() => window.aioPlugin.json('GET', '/tasks'));
    assert.equal(backend.tenantId, original);
    console.log(`${mobile ? 'mobile' : 'desktop'}: tenant return and backend authorization verified`);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}.png`) });
    const reloaded = downloads.length;
    await page.reload();
    await scene();
    await page.frameLocator(selector(original, 'Dioxus 全栈计数器')).getByRole('button', { name: '+1', exact: true }).waitFor({ timeout: 90000 });
    await select('KMP 全栈示例');
    await counter(original, 0);
    const repeatedModules = downloads.slice(reloaded).filter(path => /\.(wasm|mjs|js)$/.test(path) && !/\/(__aio_|import-map-loader|startup)/.test(path));
    assert.deepEqual(repeatedModules, []);
    assert.deepEqual(errors, []);
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    return { viewport: mobile ? 'mobile' : 'desktop', returnMs, sameIframe: true, stateRetained: true, oldTicketRevoked: true, backendTenantVerified: true, reloadModuleDownloads: 0, consoleErrors: 0 };
  } catch (error) {
    await page.screenshot({ path: resolve(output, 'failure.png') }).catch(() => {});
    console.error(errors);
    throw error;
  } finally {
    const restored = await context.request.post(origin + '/api/tenants/switch', { data: { tenant_id: original } });
    assert.equal(restored.status(), 204, 'Restore original tenant');
    await context.close();
  }
}

(async () => {
  assert(process.env.AIO_TEST_COOKIE_FILE, 'Set AIO_TEST_COOKIE_FILE to a private Netscape cookie file');
  const content = await readFile(process.env.AIO_TEST_COOKIE_FILE, 'utf8');
  const cookie = content.split('\n').map(line => line.split('\t')).find(fields => fields[5] === 'aio_session')?.[6]?.trim();
  assert(cookie, 'Session cookie missing');
  await mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const report = [await run(browser, cookie, false), await run(browser, cookie, true)];
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
