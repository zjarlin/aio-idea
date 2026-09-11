const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { chromium } = require("playwright");

const baseURL = process.env.AIO_URL || "http://127.0.0.1:4174";
const screenshotDir = process.env.AIO_SCREENSHOT_DIR || os.tmpdir();

async function authenticate(context, page) {
  if (process.env.AIO_COOKIE_FILE) {
    const cookies = fs.readFileSync(process.env.AIO_COOKIE_FILE, "utf8")
      .split(/\r?\n/)
      .filter((line) => line && (!line.startsWith("#") || line.startsWith("#HttpOnly_")))
      .map((line) => {
        const [, , cookiePath, secure, expires, name, value] = line.replace(/^#HttpOnly_/, "").split("\t");
        return { name, value, url: new URL(cookiePath, baseURL).href, httpOnly: line.startsWith("#HttpOnly_"), secure: new URL(baseURL).protocol === "https:" && secure === "TRUE", ...(Number(expires) > 0 ? { expires: Number(expires) } : {}) };
      });
    await context.addCookies(cookies);
  }
  await page.goto(baseURL, { waitUntil: "domcontentloaded" });
  await page.locator('.application-shell, input[aria-label="账号"]').first().waitFor();
  if (await page.getByRole("button", { name: "登录", exact: true }).isVisible()) {
    assert(process.env.AIO_ACCOUNT && process.env.AIO_PASSWORD, "需要有效 Cookie 或 AIO_ACCOUNT/AIO_PASSWORD");
    await page.getByLabel("账号", { exact: true }).fill(process.env.AIO_ACCOUNT);
    await page.getByLabel("密码", { exact: true }).fill(process.env.AIO_PASSWORD);
    const login = page.waitForResponse((response) => response.url().endsWith("/api/auth/login"));
    const reload = page.waitForNavigation({ waitUntil: "domcontentloaded" });
    await page.getByRole("button", { name: "登录", exact: true }).click();
    assert((await login).ok(), "登录请求失败");
    await reload;
  }
  if (process.env.AIO_TENANT_ID) {
    const response = await context.request.post(`${baseURL}/api/tenants/switch`, { data: { tenant_id: process.env.AIO_TENANT_ID } });
    assert(response.ok(), `切换测试租户失败: ${response.status()}`);
    await page.goto(baseURL, { waitUntil: "domcontentloaded" });
  }
  await page.locator(".application-shell").waitFor({ state: "visible" });
}

async function assertFits(page) {
  const dimensions = await page.evaluate(() => ({ width: document.documentElement.clientWidth, scrollWidth: document.documentElement.scrollWidth }));
  assert(dimensions.scrollWidth <= dimensions.width, JSON.stringify(dimensions));
}

async function openAccountPage(page, label, mobile) {
  if (mobile) await page.getByRole("button", { name: "打开菜单", exact: true }).click();
  const container = mobile ? page.getByRole("dialog") : page.locator(".application-shell__sidebar");
  await container.locator('button[aria-label$="的账户菜单"]').click();
  await page.getByRole("menuitem", { name: label, exact: true }).click();
  await page.locator(".application-fullscreen:visible").waitFor();
  assert.equal(await page.locator(".application-shell:visible").count(), 0);
  assert.equal(await page.locator(".application-shell__mobile-dialog:visible").count(), 0);
  assert.equal(await page.getByRole("navigation", { name: "场景" }).count(), 0);
  await assertFits(page);
}

async function returnToWorkspace(page) {
  await page.getByRole("button", { name: "返回主后台", exact: true }).click();
  await page.locator(".application-fullscreen:visible").waitFor({ state: "hidden" });
  await page.locator(".application-shell").waitFor({ state: "visible" });
}

async function scenario(browser, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 }, isMobile: mobile });
  // 只在测试响应中加入社区账户页，验证通用挂载契约，不修改租户数据。
  await context.route("**/api/runtime/catalog", async (route) => {
    const response = await route.fetch();
    const catalog = await response.json();
    if (!catalog.data.pages.some((page) => page.label === "Hello")) {
      catalog.data.pages.push({ id: "navigation-hello-test", label: "Hello", icon: null, scene: { id: "workspace", label: "工作区" }, menu_path: [], required_permission: null, body: { kind: "text", title: "Hello", content: "导航测试" } });
    }
    catalog.data.pages.push({ id: "navigation-state-test", label: "导航状态", icon: null, scene: { id: "community", label: "社区插件" }, menu_path: [], required_permission: null, body: { kind: "counter", title: "导航状态", button: "状态 +1" } });
    catalog.data.pages.push({ id: "account-extension-test", label: "社区账户扩展", icon: null, scene: { id: "community", label: "社区插件" }, menu_path: [], required_permission: null, body: { kind: "counter", title: "社区账户扩展", button: "扩展 +1" } });
    catalog.data.account_items.push({ id: "open-account-extension-test", label: "社区账户扩展", icon: null, page_id: "account-extension-test", required_permission: null });
    await route.fulfill({ response, json: catalog });
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => { if (message.type() === "error") errors.push(message.text()); });
  try {
    await authenticate(context, page);
    const sidebar = page.locator(".application-shell__sidebar");
    const sceneTabs = page.getByRole("navigation", { name: "场景" });
    const labels = (container) => container.locator(".application-shell__navigation-button").evaluateAll((buttons) => buttons.map((button) => button.getAttribute("aria-label")));
    const menus = () => labels(sidebar);
    assert.deepEqual(await menus(), ["首页", "Hello"]);
    assert.equal(await sidebar.locator(".application-shell__navigation-heading").count(), 0);
    for (const label of ["个人资料", "设置中心", "插件市场", "租户管理", "社区账户扩展"]) {
      assert.equal(await sidebar.getByRole("button", { name: label, exact: true }).count(), 0);
    }

    await sceneTabs.getByRole("button", { name: "系统", exact: true }).click();
    if (mobile) await page.getByRole("button", { name: "打开菜单", exact: true }).click();
    const systemNavigation = mobile ? page.getByRole("dialog") : sidebar;
    assert.deepEqual(await labels(systemNavigation), [
      "系统管理",
      "用户管理",
      "角色管理",
      "字典管理",
      "基础设施",
      "文件管理",
      "文件列表",
    ]);
    const systemManagement = systemNavigation.getByRole("button", { name: "系统管理", exact: true });
    const infrastructure = systemNavigation.getByRole("button", { name: "基础设施", exact: true });
    const fileManagement = systemNavigation.getByRole("button", { name: "文件管理", exact: true });
    for (const group of [systemManagement, infrastructure, fileManagement]) {
      assert.equal(await group.getAttribute("aria-expanded"), "true");
    }
    await systemManagement.click();
    assert.equal(await systemManagement.getAttribute("aria-expanded"), "false");
    for (const leaf of ["用户管理", "角色管理", "字典管理"]) {
      assert.equal(await systemNavigation.getByRole("button", { name: leaf, exact: true }).count(), 0);
    }
    await systemManagement.click();
    assert.equal(await systemManagement.getAttribute("aria-expanded"), "true");
    await fileManagement.click();
    assert.equal(await systemNavigation.getByRole("button", { name: "文件列表", exact: true }).count(), 0);
    await fileManagement.click();
    assert.equal(await systemNavigation.getByRole("button", { name: "文件列表", exact: true }).count(), 1);
    if (mobile) await page.waitForTimeout(200);
    await page.screenshot({ path: path.join(screenshotDir, `aio-system-tree-${mobile ? "mobile" : "desktop"}.png`), fullPage: true });
    if (mobile) await page.getByRole("button", { name: "关闭菜单", exact: true }).click();
    await sceneTabs.getByRole("button", { name: "社区插件", exact: true }).click();
    assert((await menus()).some((text) => text.includes("导航状态")));
    assert(!(await menus()).some((text) => /首页|Hello|系统管理|用户管理|角色管理|字典管理|基础设施|文件管理/.test(text)));

    if (mobile) await page.getByRole("button", { name: "打开菜单", exact: true }).click();
    const navigation = mobile ? page.getByRole("dialog") : sidebar;
    await navigation.getByRole("button", { name: "导航状态", exact: true }).click();
    const content = page.locator(".application-shell__content");
    const counterButton = content.getByRole("button").first();
    await counterButton.click();
    const stateBeforeAccount = await content.innerText();

    for (const label of ["个人资料", "设置中心", "插件市场", "切换租户", "社区账户扩展"]) {
      await openAccountPage(page, label, mobile);
      await page.locator(".application-fullscreen:visible .application-fullscreen__content h2").first().waitFor();
      await page.waitForFunction(() => [...document.querySelectorAll(".application-fullscreen__content")].filter(element => element.checkVisibility()).every(element => !element.innerText.includes("正在读取")));
      assert.equal(await page.locator('.application-fullscreen:visible [role="alert"]').count(), 0);
      if (label === "社区账户扩展") {
        await page.getByRole("button", { name: "扩展 +1", exact: true }).click();
        await page.locator(".application-fullscreen:visible .application-fullscreen__content").getByText("计数：1", { exact: true }).waitFor();
      }
      if (label === "设置中心") {
        await page.screenshot({ path: path.join(screenshotDir, `aio-account-fullscreen-${mobile ? "mobile" : "desktop"}.png`), fullPage: true });
      }
      await returnToWorkspace(page);
      assert.equal(await content.innerText(), stateBeforeAccount, "返回必须保留原页面状态");
      assert.equal(await sceneTabs.getByRole("button", { name: "社区插件", exact: true }).getAttribute("aria-pressed"), "true");
    }
    await page.screenshot({ path: path.join(screenshotDir, `aio-scene-root-${mobile ? "mobile" : "desktop"}.png`), fullPage: true });
    await assertFits(page);
    assert.deepEqual(errors, []);
    return { viewport: mobile ? "mobile" : "desktop", sceneFiltering: true, fullscreenAccount: true, runtimeAccountFixture: true, preservedPageState: true, errors };
  } finally {
    await context.close();
  }
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  try {
    console.log(JSON.stringify([await scenario(browser, false), await scenario(browser, true)], null, 2));
  } finally {
    await browser.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
