const assert = require('node:assert/strict');
const { mkdir, writeFile } = require('node:fs/promises');
const { resolve } = require('node:path');
const { randomUUID } = require('node:crypto');
const { chromium } = require('playwright');

const base = process.env.AIO_URL;
assert(new URL(base).hostname === '127.0.0.1', 'Only the isolated local test host is allowed');
const output = resolve('target/admin-ui-test');

async function scenario(browser, mobile) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 }, isMobile: mobile, hasTouch: mobile });
  const viewer = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  const account = `rbac-${randomUUID().slice(0, 8)}`;
  const role = `${account}-editor`;
  const password = 'rbac-test-password-123';
  const request = context.request;
  let userId;
  async function data() { return (await (await request.get(`${base}/api/rbac`)).json()).data; }
  async function navigate(name) {
    if (mobile) await page.getByRole('button', { name: '打开菜单', exact: true }).click();
    const nav = mobile ? page.getByRole('dialog') : page.locator('.application-shell__sidebar');
    await nav.getByRole('button', { name, exact: true }).click();
    await page.getByRole('heading', { name, exact: true }).waitFor();
  }
  async function save(dialog, method, path) {
    const result = page.waitForResponse(response => response.url().endsWith(path) && response.request().method() === method);
    await dialog.getByRole('button', { name: '保存', exact: true }).click();
    const response = await result;
    if (!response.ok()) throw new Error(await response.text());
    await dialog.waitFor({ state: 'detached' });
  }
  try {
    assert((await request.post(`${base}/api/auth/login`, { data: { account: process.env.AIO_BOOTSTRAP_ACCOUNT, password: process.env.AIO_BOOTSTRAP_PASSWORD } })).ok());
    await page.goto(base);
    await navigate('角色管理');
    await page.getByRole('button', { name: '新建角色', exact: true }).click();
    const roleDialog = page.getByRole('dialog', { name: '新建角色', exact: true });
    await roleDialog.getByLabel('角色 ID', { exact: true }).fill(role);
    await roleDialog.getByRole('checkbox', { name: '权限 file:manage', exact: true }).click();
    await save(roleDialog, 'POST', '/api/rbac/roles');
    await page.getByRole('searchbox', { name: '搜索角色', exact: true }).fill(role);
    await page.getByRole('table', { name: '角色', exact: true }).getByText(role, { exact: true }).waitFor();
    await page.getByRole('button', { name: `编辑 ${role}`, exact: true }).click();
    const roleEdit = page.getByRole('dialog', { name: '编辑角色', exact: true });
    await roleEdit.getByRole('checkbox', { name: '权限 dictionary:manage', exact: true }).click();
    await save(roleEdit, 'PUT', `/api/rbac/roles/${role}`);
    assert.deepEqual((await data()).roles.find(item => item.id === role).permissions.sort(), ['dictionary:manage', 'file:manage']);
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-roles.png`) });

    await navigate('用户管理');
    await page.getByRole('button', { name: '新建用户', exact: true }).click();
    const newUser = page.getByRole('dialog', { name: '新建用户', exact: true });
    await newUser.getByLabel('用户账号', { exact: true }).fill(account);
    await newUser.getByLabel('用户显示名称', { exact: true }).fill('待授权成员');
    await newUser.getByLabel('初始密码', { exact: true }).fill(password);
    await save(newUser, 'POST', '/api/rbac/users');
    userId = (await data()).users.find(user => user.account === account).id;
    assert((await viewer.request.post(`${base}/api/auth/login`, { data: { account, password } })).ok());
    assert.equal((await viewer.request.get(`${base}/api/rbac`)).status(), 403);
    assert.equal((await viewer.request.post(`${base}/api/rbac/roles`, { data: { role_id: 'forbidden-role', permissions: ['rbac:manage'] } })).status(), 403);
    assert.equal((await viewer.request.get(`${base}/api/files`)).status(), 403);
    await page.getByRole('searchbox', { name: '搜索用户', exact: true }).fill(account);
    await page.getByRole('button', { name: `编辑 ${account}`, exact: true }).click();
    const userEdit = page.getByRole('dialog', { name: '编辑用户', exact: true });
    await userEdit.getByLabel('用户显示名称', { exact: true }).fill('文件与字典管理员');
    await save(userEdit, 'PUT', `/api/rbac/users/${userId}`);
    assert.equal((await data()).users.find(user => user.id === userId).display_name, '文件与字典管理员');
    await page.getByRole('button', { name: `分配角色 ${account}`, exact: true }).click();
    const assignments = page.getByRole('dialog', { name: '分配角色', exact: true });
    await assignments.getByRole('checkbox', { name: `角色 ${role}`, exact: true }).click();
    await save(assignments, 'PUT', `/api/rbac/users/${userId}/roles`);
    assert((await viewer.request.get(`${base}/api/files`)).ok());
    assert((await viewer.request.get(`${base}/api/dictionaries`)).ok());
    assert.equal((await viewer.request.get(`${base}/api/rbac`)).status(), 403);
    assert.equal((await request.delete(`${base}/api/rbac/roles/${role}`)).status(), 400, 'Assigned role must not be deleted');
    await page.getByRole('table', { name: '用户', exact: true }).getByText(role, { exact: true }).waitFor();
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'User page overflow');
    await page.screenshot({ path: resolve(output, `${mobile ? 'mobile' : 'desktop'}-users.png`) });

    await page.getByRole('button', { name: `分配角色 ${account}`, exact: true }).click();
    await assignments.getByRole('checkbox', { name: `角色 ${role}`, exact: true }).click();
    await save(assignments, 'PUT', `/api/rbac/users/${userId}/roles`);
    assert.equal((await viewer.request.get(`${base}/api/files`)).status(), 403, 'Permission revocation must affect existing sessions');
    await page.getByRole('button', { name: `移出 ${account}`, exact: true }).click();
    const removal = page.getByRole('dialog', { name: '移出租户', exact: true });
    await removal.getByRole('button', { name: '取消', exact: true }).click();
    assert((await data()).users.some(user => user.id === userId));
    await page.getByRole('button', { name: `移出 ${account}`, exact: true }).click();
    await removal.getByRole('button', { name: '确认删除', exact: true }).click();
    await removal.waitFor({ state: 'detached' });
    assert(!(await data()).users.some(user => user.id === userId));
    assert.equal((await viewer.request.get(`${base}/api/files`)).status(), 401, 'Removed member session must be revoked');

    await navigate('角色管理');
    await page.getByRole('button', { name: '刷新角色', exact: true }).click();
    await page.getByRole('searchbox', { name: '搜索角色', exact: true }).fill(role);
    await page.getByRole('button', { name: `删除 ${role}`, exact: true }).click();
    const deletion = page.getByRole('dialog', { name: '删除角色', exact: true });
    await deletion.getByRole('button', { name: '确认删除', exact: true }).click();
    await deletion.waitFor({ state: 'detached' });
    assert(!(await data()).roles.some(item => item.id === role));
    const current = (await data()).current_user_id;
    assert.equal((await request.put(`${base}/api/rbac/users/${current}/roles`, { data: { roles: ['member'] } })).status(), 400, 'Self-lockout must be rejected');
    assert.equal((await request.delete(`${base}/api/rbac/roles/platform-admin`)).status(), 400);
    assert.deepEqual(errors, []);
    return { viewport: mobile ? 'mobile' : 'desktop', userCreateEdit: true, roleCreateEditDelete: true, assignments: true, revokeExistingSessionPermissions: true, removeTenantMembership: true, memberDenied: true, selfLockoutDenied: true, systemRoleProtected: true };
  } catch (error) {
    await page.screenshot({ path: resolve(output, 'rbac-failure.png') });
    throw error;
  } finally {
    if (userId) await request.delete(`${base}/api/rbac/users/${userId}`);
    await request.delete(`${base}/api/rbac/roles/${role}`);
    await viewer.close(); await context.close();
  }
}

(async () => {
  await mkdir(output, { recursive: true });
  const browser = await chromium.launch({ channel: 'chrome', headless: true });
  try {
    const report = [await scenario(browser, false), await scenario(browser, true)];
    await writeFile(resolve(output, 'rbac-report.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; });
