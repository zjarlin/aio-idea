const assert = require('node:assert/strict');
const { mkdir, writeFile, readFile } = require('node:fs/promises');
const { resolve } = require('node:path');
const { randomUUID } = require('node:crypto');
const { chromium } = require('playwright');
const contrast = require('./admin-contrast.cjs');

const base = process.env.AIO_URL;
assert(new URL(base).hostname === '127.0.0.1', 'Only the isolated local test host is allowed');
const output = resolve('target/admin-ui-test');
const browserErrors = [];
const run = randomUUID().slice(0, 8);

async function scenario(browser, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 }, isMobile: mobile, hasTouch: mobile });
  const prefix = `admin-${run}-${mobile ? 'mobile' : 'desktop'}`;
  const created = new Set();
  const page = await context.newPage();
  const active = page.locator('[data-aio-page-active="true"]');
  page.on('pageerror', error => browserErrors.push(error.message));
  const request = context.request;
  const login = await request.post(`${base}/api/auth/login`, { data: { account: process.env.AIO_BOOTSTRAP_ACCOUNT, password: process.env.AIO_BOOTSTRAP_PASSWORD } });
  assert(login.ok(), `Login failed: ${login.status()}`);
  async function upload(name, bytes, type = 'text/plain') {
    const response = await request.post(`${base}/api/files?filename=${encodeURIComponent(name)}`, { data: bytes, headers: { 'content-type': type } });
    assert(response.ok(), await response.text());
    const file = (await response.json()).data;
    created.add(file.id);
    return file;
  }
  async function fits() {
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'Page overflow');
  }
  async function openFiles() {
    if (mobile) await page.getByRole('button', { name: '打开菜单', exact: true }).click();
    const nav = mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
    await nav.getByRole('button', { name: '文件列表', exact: true }).click();
    await page.getByRole('heading', { name: '文件管理', exact: true }).waitFor();
    await page.getByRole('searchbox', { name: '搜索文件', exact: true }).waitFor();
  }
  try {
    for (let i = 0; i < 13; i++) await upload(`${prefix}-${String(i).padStart(2, '0')}.txt`, Buffer.alloc(10 + i, 65));
    await page.goto(base);
    assert.equal(await page.getByRole('button', { name: '首页', exact: true }).count(), 0);
    await openFiles();
    const search = page.getByRole('searchbox', { name: '搜索文件', exact: true });
    const table = page.getByRole('table', { name: '文件', exact: true });
    await search.fill(prefix);
    await active.getByText('第 1–10 项，共 13 项', { exact: true }).waitFor();
    await page.getByRole('button', { name: '下一页', exact: true }).click();
    await active.getByText('第 11–13 项，共 13 项', { exact: true }).waitFor();
    await search.fill(`${prefix}-00`);
    await active.getByText('第 1–1 项，共 1 项', { exact: true }).waitFor();
    await search.fill('no-matching-file');
    await active.getByText('没有匹配结果', { exact: true }).waitFor();
    await search.fill(prefix);
    await page.getByRole('button', { name: '排序：大小', exact: true }).click();
    assert.equal(await table.locator('thead th[aria-sort="ascending"]').count(), 1);
    await page.getByRole('button', { name: '排序：大小', exact: true }).click();
    assert.equal(await table.locator('thead th[aria-sort="descending"]').count(), 1);
    await table.getByRole('checkbox').nth(1).click();
    await active.getByText('已选择 1 项', { exact: true }).waitFor();
    assert.equal(await table.getByRole('checkbox', { name: '选择当前页' }).getAttribute('aria-checked'), 'mixed');
    await table.getByRole('checkbox', { name: '选择当前页' }).click();
    await active.getByText('已选择 10 项', { exact: true }).waitFor();
    await page.getByRole('button', { name: '取消选择', exact: true }).click();
    await page.getByRole('button', { name: '文件类型', exact: true }).click();
    await page.getByRole('option', { name: '图片', exact: true }).click();
    await active.getByText('暂无文件', { exact: true }).waitFor();
    await page.getByRole('button', { name: '文件类型', exact: true }).click();
    await page.getByRole('option', { name: '全部类型', exact: true }).click();
    await search.fill(prefix);
    await page.getByRole('button', { name: '上传文件', exact: true }).click();
    let dialog = page.getByRole('dialog', { name: '上传文件', exact: true });
    await dialog.waitFor();
    for (let i = 0; i < 7; i++) { await page.keyboard.press('Tab'); assert(await dialog.evaluate(el => el.contains(document.activeElement)), 'Dialog focus escaped'); }
    await page.keyboard.press('Escape');
    await dialog.waitFor({ state: 'detached' });
    assert.equal(await page.evaluate(() => document.activeElement.textContent.trim()), '上传文件');
    await page.getByRole('button', { name: '上传文件', exact: true }).click();
    const filename = `${prefix}-上传验证.txt`;
    const payload = Buffer.from('AIO 文件上传与下载校验');
    await dialog.getByLabel('选择文件', { exact: true }).setInputFiles({ name: filename, mimeType: 'text/plain', buffer: payload });
    const uploaded = page.waitForResponse(response => response.url().includes('/api/files?') && response.request().method() === 'POST');
    await dialog.getByRole('button', { name: '上传', exact: true }).click();
    const item = (await (await uploaded).json()).data;
    created.add(item.id);
    await dialog.waitFor({ state: 'detached' });
    await search.fill(filename);
    await table.getByRole('button', { name: filename, exact: true }).click();
    const details = page.getByRole('dialog', { name: '文件详情', exact: true });
    await details.getByText(item.sha256, { exact: true }).waitFor();
    const downloading = page.waitForEvent('download');
    await details.getByRole('button', { name: '下载文件', exact: true }).click();
    const downloaded = await downloading;
    assert.deepEqual(await readFile(await downloaded.path()), payload);
    await details.getByRole('button', { name: '关闭', exact: true }).click();
    await table.getByRole('button', { name: `删除 ${filename}`, exact: true }).click();
    const deletion = page.getByRole('dialog', { name: '删除文件', exact: true });
    await deletion.getByRole('button', { name: '取消', exact: true }).click();
    assert((await request.get(`${base}/api/files/${item.id}`)).ok(), 'Cancel deleted the file');
    await table.getByRole('button', { name: `删除 ${filename}`, exact: true }).click();
    await deletion.getByRole('button', { name: '确认删除', exact: true }).click();
    await deletion.waitFor({ state: 'detached' });
    assert.equal((await request.get(`${base}/api/files/${item.id}`)).status(), 404);
    await search.fill(prefix);
    await table.getByRole('checkbox').nth(1).click();
    await table.getByRole('checkbox').nth(2).click();
    await page.getByRole('button', { name: '删除选中 (2)', exact: true }).click();
    await deletion.getByRole('button', { name: '确认删除', exact: true }).click();
    await deletion.waitFor({ state: 'detached' });
    await active.getByText('第 1–10 项，共 11 项', { exact: true }).waitFor();
    await active.locator('.data-table-viewport').evaluate(el => { el.scrollLeft = 0; });
    await page.evaluate(() => document.fonts.ready);
    assert(await page.evaluate(() => document.fonts.check('14px Inter')), 'Inter font missing');
    assert(await page.evaluate(() => [...document.fonts].some(font => font.family.includes('Noto Sans SC') && font.status === 'loaded')), 'Chinese font missing');
    await fits();
    const lightContrast = await contrast(page);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-files.png`) });
    await page.evaluate(() => { document.documentElement.dataset.theme = 'dark'; });
    await page.waitForFunction(() => getComputedStyle(document.querySelector('.admin-page-header .dx-button[data-style="primary"]')).backgroundColor === 'rgb(250, 250, 250)');
    const darkContrast = await contrast(page);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-dark.png`) });
    await fits();
    await page.evaluate(() => { document.documentElement.dataset.theme = 'light'; document.body.style.zoom = '2'; });
    await page.waitForFunction(() => getComputedStyle(document.querySelector('.admin-page-header .dx-button[data-style="primary"]')).backgroundColor === 'rgb(24, 24, 27)');
    await fits();
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-zoom.png`) });
    await page.evaluate(() => { document.body.style.zoom = ''; });
    await page.route('**/api/files', route => route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: '测试服务暂时不可用' }) }));
    await page.getByRole('button', { name: '刷新文件', exact: true }).click();
    await page.getByRole('alert').filter({ hasText: '测试服务暂时不可用' }).waitFor();
    await page.unroute('**/api/files');
    await page.getByRole('button', { name: '重试', exact: true }).click();
    await table.waitFor();
    return { viewport: mobile ? 'mobile' : 'desktop', search: true, filter: true, sort: true, pagination: true, selection: true, upload: true, downloadBytes: true, deleteConfirmation: true, bulkDelete: true, focusTrap: true, fonts: true, dark: true, zoom200: true, retry: true, lightContrast, darkContrast };
  } catch (error) {
    await page.screenshot({ path: resolve(output, 'failure.png') });
    throw error;
  } finally {
    for (const id of created) await request.delete(`${base}/api/files/${id}`);
    await context.close();
  }
}

(async () => {
  await mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const report = [await scenario(browser, false), await scenario(browser, true)];
    assert.deepEqual(browserErrors, []);
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
