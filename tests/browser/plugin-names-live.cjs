const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {launchBrowser, contextFor, marketplace, closeBrowser, getJson} = require('./live-session.cjs');

const base = process.env.AIO_URL || 'https://aio.addzero.site';
const output = path.resolve('target/component-delivery/chinese-names');
const names = new Map([
  ['aio-plugin-dioxus-fullstack', '计数器示例'],
  ['aio-plugin-kmp-example', '任务工作台示例'],
  ['aio-plugin-agent-memory', '智能体记忆'],
  ['aio-plugin-screen', '数据大屏'],
]);

async function run() {
  const browser = await launchBrowser();
  const errors = [];
  const result = [];
  fs.mkdirSync(output, {recursive: true});
  try {
    for (const mobile of [false, true]) {
      const context = await contextFor(browser, base, mobile);
      const entries = (await getJson(context, `${base}/api/runtime/marketplace`)).data;
      for (const [repository, title] of names) {
        const entry = entries.find(e => e.git === `https://github.com/zjarlin/${repository}.git`);
        assert.equal(entry?.title, title);
      }
      const memory = entries.find(e => e.title === '智能体记忆');
      assert.equal(memory.parent_title, '智能体');
      const page = await context.newPage();
      page.setDefaultTimeout(60000);
      page.on('pageerror', e => errors.push(e.message));
      await page.goto(base);
      await page.locator('.application-shell:visible').waitFor();
      await marketplace(page, mobile);
      await page.getByRole('textbox', {name: '搜索插件', exact: true}).fill('');
      await page.getByRole('button', {name: '收起 智能体', exact: true}).waitFor();
      const visibleNames = await page.locator('.extension-browser__item strong').allTextContents();
      for (const title of [...names.values(), '智能体']) assert(visibleNames.includes(title));
      const viewport = mobile ? 'mobile' : 'desktop';
      await page.screenshot({path: path.join(output, `${viewport}-list.png`)});
      await page.getByRole('treeitem').filter({hasText: '智能体记忆'}).click();
      await page.locator('.extension-browser__heading h1').getByText('智能体记忆', {exact: true}).waitFor();
      await page.getByText('正在读取 README', {exact: true}).waitFor({state: 'hidden'});
      await page.getByText('父插件：智能体', {exact: true}).waitFor();
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
      await page.screenshot({path: path.join(output, `${viewport}-details.png`)});
      result.push({viewport, names: visibleNames, revision: memory.rev});
    }
    assert.deepEqual(errors, []);
    const report = {base, checkedAt: new Date().toISOString(), result, errors};
    fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report));
  } finally {
    await closeBrowser(browser);
  }
}
run().catch(e => {console.error(e.message.split('Call log:')[0]); process.exitCode = 1;});
