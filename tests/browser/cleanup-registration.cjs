const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const {parseEnv} = require('node:util');
const {Client} = require(process.env.AIO_TEST_PG_MODULE || '/opt/aio-delivery/ops/node_modules/pg');
const accounts = JSON.parse(readFileSync(0, 'utf8'));
assert(accounts.length > 0 && accounts.length <= 10);
for (const account of accounts) {
  assert(/^registration_test_[a-f0-9]{8}_(desktop|mobile)$/.test(account.account));
  assert(/^[a-f0-9-]{36}$/.test(account.user_id));
  assert(/^[a-f0-9-]{36}$/.test(account.tenant_id));
}
const settings = process.env.AIO_REGISTRATION_DATABASE_URL ? process.env : parseEnv(readFileSync('/opt/aio-idea/runtime.env', 'utf8'));
const client = new Client({connectionString: settings.AIO_REGISTRATION_DATABASE_URL || settings.AIO_DATABASE_URL});
(async () => {
  await client.connect();
  try {
    await client.query('BEGIN');
    for (const account of accounts) {
      const {rows} = await client.query('SELECT id FROM identity_users WHERE id=$1 AND account=$2 FOR UPDATE', [account.user_id, account.account]);
      assert.equal(rows.length, 1, '只允许清理本次创建的精确账号');
      const memberships = await client.query('SELECT tenant_id,user_id FROM tenant_memberships WHERE user_id=$1 OR tenant_id=$2', [account.user_id, account.tenant_id]);
      assert.deepEqual(memberships.rows, [{tenant_id: account.tenant_id, user_id: account.user_id}]);
      const bindings = await client.query('SELECT source_id FROM tenant_plugin_bindings WHERE tenant_id=$1', [account.tenant_id]);
      assert.equal(bindings.rows.length, 0, '有插件安装记录时拒绝清理');
      for (const table of ['auth_sessions', 'tenant_member_roles', 'role_permissions', 'tenant_memberships']) {
        await client.query(`DELETE FROM ${table} WHERE tenant_id=$1`, [account.tenant_id]);
      }
      await client.query('DELETE FROM tenants WHERE id=$1', [account.tenant_id]);
      await client.query('DELETE FROM identity_users WHERE id=$1 AND account=$2', [account.user_id, account.account]);
    }
    await client.query('COMMIT');
    console.log(JSON.stringify({removedTestAccounts: accounts.length, removedTestWorkspaces: accounts.length}));
  } catch (error) {await client.query('ROLLBACK'); throw error;}
  finally {await client.end();}
})().catch(error => {console.error(error.message); process.exitCode = 1;});
