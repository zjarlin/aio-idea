const assert = require('node:assert/strict');
const { mkdir, writeFile } = require('node:fs/promises');
const { resolve } = require('node:path');
const { chromium } = require('playwright');
const verifyCounterState = require('./counter-state.cjs');

const base = process.env.AIO_URL;
assert(base && process.env.AIO_STORAGE_STATE, '需要宿主地址和受保护的浏览器登录状态');
const output = resolve('target/public-admin-test');
const expected = ['https://github.com/zjarlin/aio-plugin-dioxus-fullstack.git', 'https://github.com/zjarlin/aio-plugin-kmp-example.git'];

async function run(browser, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 }, storageState: process.env.AIO_STORAGE_STATE });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  const nav = async () => {
    if (mobile) await page.getByRole('button', { name: '打开菜单', exact: true }).click();
    return mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
  };
  try {
    const response = await context.request.get(`${base}/api/runtime/catalog`);
    assert(response.ok());
    const catalog = (await response.json()).data;
    assert.deepEqual(catalog.plugins.map(p => p.git).sort(), expected);
    await page.goto(base);
    await page.locator('.application-shell:visible').waitFor();
    await page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: '系统', exact: true }).click();
    for (const [label, table] of [['用户管理', '用户'], ['角色管理', '角色'], ['字典管理', null], ['文件列表', '文件']]) {
      await (await nav()).getByRole('button', { name: label, exact: true }).click();
      const active = page.locator('[data-aio-page-active="true"]');
      await active.getByRole('heading', { level: 1, name: label === '文件列表' ? '文件管理' : label, exact: true }).waitFor();
      if (table) await active.getByRole('table', { name: table, exact: true }).waitFor();
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-${table || 'dictionary'}.png`) });
    }
    await page.getByRole('navigation', { name: '场景' }).getByRole('button', { name: '社区插件', exact: true }).click();
    const community = await nav();
    assert.deepEqual(await community.locator('.application-shell__navigation-button').evaluateAll(buttons => buttons.map(button => button.getAttribute('aria-label'))), ['计数器示例', '任务工作台示例']);
    await community.getByRole('button', { name: '计数器示例', exact: true }).click();
    const frame = page.frameLocator('iframe[title="计数器示例"]');
    const counter = await verifyCounterState(page, context);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-dioxus.png`) });
    await (await nav()).locator('button[aria-label$="的账户菜单"]').click();
    await page.getByRole('menuitem', { name: '插件市场', exact: true }).click();
    await page.getByRole('table', { name: '插件', exact: true }).waitFor();
    assert.equal(await page.getByRole('table', { name: '插件', exact: true }).getByRole('row').count(), 3);
    assert.equal(await page.locator('.application-shell:visible').count(), 0);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-marketplace.png`) });
    await page.getByRole('button', { name: '返回主后台', exact: true }).click();
    await frame.getByText(`计数：${counter.count}`, { exact: true }).waitFor();
    assert.deepEqual(errors, []);
    return { viewport: mobile ? 'mobile' : 'desktop', systemTables: true, onlyTwoCommunityPlugins: true, noHomeOrHello: true, ...counter, fullscreenMarketReturn: true, consoleErrors: 0 };
  } catch (error) {
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-failure.png`) });
    console.error(await page.locator('body').innerText(), errors);
    throw error;
  } finally { await context.close(); }
}

(async () => {
  await mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const report = [await run(browser, false), await run(browser, true)];
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
