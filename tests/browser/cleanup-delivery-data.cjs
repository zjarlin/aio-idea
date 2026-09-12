const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {parseEnv} = require('node:util');
const {Client} = require('/opt/aio-delivery/ops/node_modules/pg');
const repositories = process.argv.slice(2);
assert(repositories.length > 0);
assert(repositories.every(name => /^aio-delivery-(e2e|acceptance)-(rust|kotlin|typescript)$/.test(name)));
const gits = repositories.map(name => `https://github.com/zjarlin/${name}.git`);
const settings = parseEnv(fs.readFileSync('/opt/aio-idea/runtime.env', 'utf8'));
const client = new Client({connectionString:settings.AIO_DATABASE_URL});

(async()=>{
  await client.connect();
  try {
    await client.query('BEGIN');
    const {rows:sources} = await client.query('SELECT id,git FROM plugin_sources WHERE git=ANY($1) FOR UPDATE',[gits]);
    const ids = sources.map(source=>source.id);
    const {rows:bindings} = await client.query('SELECT tenant_id,source_id FROM tenant_plugin_bindings WHERE source_id=ANY($1)',[ids]);
    assert.equal(bindings.length,0,'必须先通过正式接口卸载测试插件');
    const {rows:busy} = await client.query("SELECT id FROM delivery_jobs WHERE git=ANY($1) AND state IN ('queued','building','uploaded','publishing')",[gits]);
    assert.equal(busy.length,0,'测试仓库仍有进行中的构建');
    const {rows:packages} = await client.query('SELECT revision FROM plugin_packages WHERE git=ANY($1)',[gits]);
    await client.query('DELETE FROM delivery_jobs WHERE git=ANY($1)',[gits]);
    await client.query('DELETE FROM delivery_sources WHERE git=ANY($1)',[gits]);
    await client.query('DELETE FROM marketplace_entries WHERE git=ANY($1)',[gits]);
    await client.query('DELETE FROM plugin_publish_jobs WHERE git=ANY($1)',[gits]);
    await client.query('DELETE FROM plugin_publish_credentials WHERE git=ANY($1)',[gits]);
    await client.query('DELETE FROM plugin_lifecycle_events WHERE source_id=ANY($1)',[ids]);
    await client.query('DELETE FROM plugin_runtime_instances WHERE revision_id IN (SELECT id FROM plugin_revisions WHERE source_id=ANY($1))',[ids]);
    await client.query('DELETE FROM plugin_sources WHERE id=ANY($1)',[ids]);
    await client.query('DELETE FROM plugin_packages WHERE git=ANY($1)',[gits]);
    await client.query('COMMIT');
    const cache = path.resolve('/opt/aio-idea',settings.AIO_PLUGIN_CACHE||'plugin-cache');
    for(const {revision} of packages){
      assert(/^[a-f0-9]{64}$/.test(revision));
      fs.rmSync(path.join(cache,revision),{recursive:true,force:true});
    }
    console.log(JSON.stringify({repositories,removedSources:sources.length,removedPackages:packages.length}));
  } catch(error) {
    await client.query('ROLLBACK');
    throw error;
  } finally {await client.end();}
})().catch(error=>{console.error(error.message);process.exitCode=1;});
