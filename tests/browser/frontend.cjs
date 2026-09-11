const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const crypto = require("node:crypto");
const { execFileSync } = require("node:child_process");
const { chromium } = require("playwright");
const verifyCounterState = require('./counter-state.cjs');

const base = process.env.AIO_URL;
const cli = process.env.AIO_TEST_CLI;
const plugin = process.env.AIO_TEST_FRONTEND_PLUGIN;
const output = process.env.AIO_TEST_OUTPUT;
const tenant = process.env.AIO_TEST_TENANT;
assert(base && cli && plugin && output && tenant, "需要测试宿主、CLI、已构建插件、输出目录与隔离租户");
const git = `https://example.com/dioxus-${crypto.randomUUID()}.git`;

async function workspaceFixture(context) {
  await context.route('**/api/runtime/catalog', async route => {
    const response = await route.fetch();
    const catalog = await response.json();
    catalog.data.pages.push({ id: 'frontend-workspace-test', label: '测试工作区', icon: null, scene: { id: 'workspace', label: '工作区' }, menu_path: [], required_permission: null, body: { kind: 'text', title: '测试工作区', content: '浏览器测试' } });
    await route.fulfill({ response, json: catalog });
  });
}

async function json(context, method, endpoint, data) {
  const response = await context.request.fetch(`${base}${endpoint}`, { method, data });
  if (response.status() === 204) return null;
  const body = await response.json();
  assert(response.ok(), `${method} ${endpoint}: ${response.status()} ${JSON.stringify(body)}`);
  return body.data;
}

async function run(browser, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  await workspaceFixture(context);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  try {
    await json(context, "POST", "/api/auth/login", { account: process.env.AIO_BOOTSTRAP_ACCOUNT, password: process.env.AIO_BOOTSTRAP_PASSWORD });
    await json(context, "POST", "/api/tenants/switch", { tenant_id: tenant });
    await page.goto(base);
    await page.getByRole("navigation", { name: "场景" }).getByRole("button", { name: "社区插件", exact: true }).click();
    if (mobile) await page.getByRole("button", { name: "打开菜单", exact: true }).click();
    const sidebar = mobile ? page.getByRole("dialog") : page.locator(".application-shell__sidebar");
    await sidebar.getByRole("button", { name: "Dioxus 全栈计数器", exact: true }).click();
    const frame = page.frameLocator('iframe[title="Dioxus 全栈计数器"]');
    const counter = await verifyCounterState(page, context, tenant);
    assert.deepEqual(errors, [], "正常插件加载和调用不能产生控制台错误");
    const dimensions = await page.evaluate(() => ({ width: innerWidth, scroll: document.documentElement.scrollWidth }));
    assert(dimensions.scroll <= dimensions.width, JSON.stringify(dimensions));
    const child = page.frames().find(candidate => candidate.url().includes("/frontend/assets/"));
    assert(child, "必须实际挂载插件文档");
    const isolation = await child.evaluate(() => {
      let parentBlocked = false;
      let cookieBlocked = false;
      try { void parent.document.body; } catch { parentBlocked = true; }
      try { void document.cookie; } catch { cookieBlocked = true; }
      return { parentBlocked, cookieBlocked };
    });
    assert.deepEqual(isolation, { parentBlocked: true, cookieBlocked: true });
    const src = await page.locator("iframe").getAttribute("src");
    const token = new URL(src).pathname.split("/")[5];
    const denied = await context.request.post(`${base}/api/runtime/frontend/${token}/request`, { data: { method: "GET", path: "/private", query: null, body: "" } });
    assert.equal(denied.status(), 404);
    await page.screenshot({ path: path.join(os.tmpdir(), `aio-dioxus-fullstack-${mobile ? "mobile" : "desktop"}.png`), fullPage: true });
    await page.getByRole("navigation", { name: "场景" }).getByRole("button", { name: "工作区", exact: true }).click();
    await page.locator(`iframe[src="${src}"]`).waitFor({ state: "hidden" });
    assert.equal((await context.request.get(src)).status(), 200, "切换菜单不能销毁前端挂载");
    await page.getByRole("navigation", { name: "场景" }).getByRole("button", { name: "社区插件", exact: true }).click();
    await frame.getByText(`计数：${counter.count}`, { exact: true }).waitFor();
    assert.equal(await page.locator('iframe[title="Dioxus 全栈计数器"]').getAttribute("src"), src);
    return { viewport: mobile ? "mobile" : "desktop", realDioxus: true, ...counter, tenant, isolation, retained: true };
  } catch (error) {
    console.error("宿主页面:", await page.locator("body").innerText());
    console.error("控制台:", errors);
    throw error;
  } finally { await context.close(); }
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  const publisher = await browser.newContext();
  try {
    await json(publisher, "POST", "/api/auth/login", { account: process.env.AIO_BOOTSTRAP_ACCOUNT, password: process.env.AIO_BOOTSTRAP_PASSWORD });
    await json(publisher, "POST", "/api/tenants/switch", { tenant_id: tenant });
    const credential = await json(publisher, "POST", "/api/runtime/publish-credentials", { git });
    const file = path.join(output, "dioxus.aio-plugin");
    execFileSync(cli, ["plugin", "package", plugin, "--git", git, "--version", "1.0.0", "--output", file], { timeout: 180000 });
    execFileSync(cli, ["plugin", "publish", file], { timeout: 180000, env: { ...process.env, AIO_PLUGIN_PUBLISH_TOKEN: credential.token, AIO_PLUGIN_PUBLISH_URL: `${base}/api/runtime/plugins/publish` } });
    const catalog = await json(publisher, "GET", "/api/runtime/catalog");
    const installed = catalog.plugins.find(item => item.git === git);
    assert(installed, "真实 CLI 发布后必须激活插件");
    const results = [await run(browser, false), await run(browser, true)];
    await workspaceFixture(publisher);
    const page = await publisher.newPage();
    let documents = 0;
    page.on('request', request => { if (request.isNavigationRequest() && request.frame() === page.mainFrame()) documents++; });
    await page.goto(base);
    await page.locator('.application-shell__sidebar button[aria-label$="的账户菜单"]').click();
    await page.getByRole('menuitem', { name: '插件市场', exact: true }).click();
    await page.getByRole('searchbox', { name: '搜索插件', exact: true }).fill(git);
    const title = 'Dioxus Fullstack Counter';
    const row = page.getByRole('table', { name: '插件', exact: true }).getByRole('row').filter({ has: page.getByRole('button', { name: `停用 ${title}`, exact: true }) });
    const disabled = page.waitForResponse(response => response.url().endsWith('/disable'));
    await row.getByRole('button', { name: `停用 ${title}`, exact: true }).click();
    assert((await disabled).ok());
    try { await page.getByRole('button', { name: `启用 ${title}`, exact: true }).waitFor(); }
    catch (error) { console.error('Market state:', await page.locator('body').innerText()); throw error; }
    assert(!(await json(publisher, 'GET', '/api/runtime/catalog')).pages.some(p => p.id === 'dioxus-fullstack-counter'));
    await page.getByRole('button', { name: `启用 ${title}`, exact: true }).click();
    await page.getByRole('button', { name: `停用 ${title}`, exact: true }).waitFor();
    assert((await json(publisher, 'GET', '/api/runtime/catalog')).pages.some(p => p.id === 'dioxus-fullstack-counter'));
    await page.getByRole('button', { name: `详情 ${title}`, exact: true }).click();
    const details = page.getByRole('dialog', { name: title, exact: true });
    await details.getByRole('heading', { name: '生命周期记录', exact: true }).waitFor();
    await details.getByRole('button', { name: '关闭', exact: true }).click();
    await page.getByRole('button', { name: `卸载 ${title}`, exact: true }).click();
    let confirm = page.getByRole('dialog', { name: '卸载插件', exact: true });
    await confirm.getByRole('button', { name: '取消', exact: true }).click();
    assert((await json(publisher, 'GET', '/api/runtime/catalog')).pages.some(p => p.id === 'dioxus-fullstack-counter'));
    await page.getByRole('button', { name: `卸载 ${title}`, exact: true }).click();
    await confirm.getByRole('button', { name: '确认卸载', exact: true }).click();
    await confirm.waitFor({ state: 'detached' });
    assert(!(await json(publisher, "GET", "/api/runtime/catalog")).pages.some(page => page.id === "dioxus-fullstack-counter"));
    await page.getByRole('button', { name: `安装 ${title}`, exact: true }).click();
    const install = page.getByRole('dialog', { name: '安装插件', exact: true });
    await install.getByRole('button', { name: '安装', exact: true }).click();
    await install.waitFor({ state: 'detached', timeout: 60000 });
    assert((await json(publisher, 'GET', '/api/runtime/catalog')).pages.some(p => p.id === 'dioxus-fullstack-counter'));
    await page.getByRole('button', { name: `卸载 ${title}`, exact: true }).click();
    await confirm.getByRole('button', { name: '确认卸载', exact: true }).click();
    await confirm.waitFor({ state: 'detached' });
    assert.equal(documents, 1, 'Market lifecycle actions must not reload the shell');
    console.log(JSON.stringify({ publishedBinaryBytes: fs.statSync(file).size, results, uninstalled: true }, null, 2));
  } finally { await publisher.close(); await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
