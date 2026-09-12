const assert = require('node:assert/strict');
const {mkdir, writeFile} = require('node:fs/promises');
const {resolve} = require('node:path');
const {randomUUID} = require('node:crypto');
const {launchBrowser, viewportScreenshot} = require('./live-session.cjs');

const base = process.env.AIO_URL;
assert(['127.0.0.1', 'aio.addzero.site'].includes(new URL(base).hostname));
const output = resolve(process.env.AIO_REGISTRATION_OUTPUT || 'target/registration-test');
const accounts = [];

async function scenario(browser, mobile) {
  const mode = mobile ? 'mobile' : 'desktop';
  const context = await browser.newContext({viewport: mobile ? {width: 390, height: 844} : {width: 1440, height: 1000}, isMobile: mobile, hasTouch: mobile});
  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  const account = `registration_test_${randomUUID().slice(0, 8)}_${mode}`;
  const password = `Registration-${randomUUID()}`;
  const errors = [];
  let registrations = 0;
  let navigations = 0;
  page.on('pageerror', e => errors.push(e.message));
  page.on('request', request => {
    if (new URL(request.url()).pathname === '/api/auth/register') registrations++;
    if (request.isNavigationRequest() && request.frame() === page.mainFrame()) navigations++;
  });
  async function session() {
    const response = await context.request.get(`${base}/api/auth/session`);
    assert(response.ok());
    return (await response.json()).data;
  }
  async function submit(dialog, expected) {
    const pending = page.waitForResponse(r => new URL(r.url()).pathname === '/api/auth/register');
    await dialog.getByRole('button', {name: '注册并登录', exact: true}).click();
    const response = await pending;
    assert.equal(response.status(), expected);
    return response.json();
  }
  try {
    await page.goto(base);
    await page.getByRole('button', {name: '注册账号', exact: true}).click();
    let dialog = page.getByRole('dialog', {name: '注册账号', exact: true});
    await dialog.waitFor();
    assert.equal(await dialog.locator('input').count(), 3);
    await viewportScreenshot(page, resolve(output, `${mode}-registration.png`));
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
    const box = await dialog.boundingBox();
    const viewport = page.viewportSize();
    assert(box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width + 1 && box.y + box.height <= viewport.height + 1);
    await dialog.getByRole('button', {name: '取消', exact: true}).click();
    await dialog.waitFor({state: 'detached'});
    assert.equal(registrations, 0);
    await page.getByRole('button', {name: '注册账号', exact: true}).click();
    dialog = page.getByRole('dialog', {name: '注册账号', exact: true});
    await dialog.getByLabel('账号', {exact: true}).fill(account);
    await dialog.getByLabel('密码', {exact: true}).fill(password);
    await dialog.getByLabel('确认密码', {exact: true}).fill('different');
    await dialog.getByRole('button', {name: '注册并登录', exact: true}).click();
    await dialog.getByRole('alert').filter({hasText: '两次输入的密码不一致'}).waitFor();
    assert.equal(registrations, 0);
    await dialog.getByLabel('密码', {exact: true}).fill('short');
    await dialog.getByLabel('确认密码', {exact: true}).fill('short');
    await submit(dialog, 400);
    await dialog.getByRole('alert').waitFor();
    await dialog.getByLabel('密码', {exact: true}).fill(password);
    await dialog.getByLabel('确认密码', {exact: true}).fill(password);
    const registered = (await submit(dialog, 201)).data;
    accounts.push({account, user_id: registered.user_id, tenant_id: registered.tenant_id});
    await writeFile(resolve(output, 'accounts.json'), JSON.stringify(accounts, null, 2));
    await page.locator('.application-shell:visible').waitFor();
    assert.equal(navigations, 1, '注册无需重载整页');
    assert.equal(registered.account, account);
    assert.notEqual(registered.tenant_id, 'default');
    assert.equal((await session()).user_id, registered.user_id);
    const tenants = await context.request.get(`${base}/api/tenants`);
    assert(tenants.ok());
    assert.deepEqual((await tenants.json()).data.map(t => t.id), [registered.tenant_id]);
    assert.equal((await context.request.post(`${base}/api/tenants/switch`, {data: {tenant_id: 'default'}})).status(), 403);
    assert.equal((await context.request.get(`${base}/api/rbac`)).status(), 200);
    assert.equal((await context.request.get(`${base}/api/files`)).status(), 200);
    await page.reload();
    await page.locator('.application-shell:visible').waitFor();
    assert.equal((await session()).tenant_id, registered.tenant_id);
    await viewportScreenshot(page, resolve(output, `${mode}-workspace.png`));
    if (mobile) await page.getByRole('button', {name: '打开菜单', exact: true}).click();
    const nav = mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
    await nav.locator('button[aria-label$="的账户菜单"]').click();
    await page.getByRole('menuitem', {name: '退出登录', exact: true}).click();
    await page.getByRole('button', {name: '登录', exact: true}).waitFor();
    assert.equal(await session(), null);
    await page.getByRole('button', {name: '注册账号', exact: true}).click();
    dialog = page.getByRole('dialog', {name: '注册账号', exact: true});
    await dialog.getByLabel('账号', {exact: true}).fill(account);
    await dialog.getByLabel('密码', {exact: true}).fill(password);
    await dialog.getByLabel('确认密码', {exact: true}).fill(password);
    await submit(dialog, 409);
    await dialog.getByRole('alert').filter({hasText: '账号已被使用'}).waitFor();
    assert.equal(await dialog.getByLabel('账号', {exact: true}).inputValue(), account);
    await dialog.getByRole('button', {name: '取消', exact: true}).click();
    await dialog.waitFor({state: 'detached'});
    await page.getByLabel('账号', {exact: true}).fill(account);
    await page.getByLabel('密码', {exact: true}).fill(password);
    await page.getByRole('button', {name: '登录', exact: true}).click();
    await page.locator('.application-shell:visible').waitFor();
    assert.equal((await session()).user_id, registered.user_id);
    assert.deepEqual(errors, []);
    return {viewport: mode, registration: true, automaticLogin: true, cancelUnmount: true, confirmation: true, duplicateAccount: true, refreshRecovery: true, loginAgain: true, isolatedWorkspace: true, consoleErrors: errors};
  } catch (error) {
    await viewportScreenshot(page, resolve(output, `${mode}-failure.png`)).catch(() => {});
    throw error;
  } finally {await context.close();}
}

(async () => {
  await mkdir(output, {recursive: true});
  const browser = await launchBrowser();
  try {
    const results = [await scenario(browser, false), await scenario(browser, true)];
    assert.notEqual(accounts[0].tenant_id, accounts[1].tenant_id);
    const report = {base, checkedAt: new Date().toISOString(), results};
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally {await browser.close();}
})().catch(error => {console.error(error.message.split('Call log:')[0]); process.exitCode = 1;});
