const assert = require('node:assert/strict');
const fs = require('node:fs');
const {spawnSync} = require('node:child_process');
const repositories = ['e2e', 'acceptance'].flatMap(group =>
  ['rust', 'kotlin', 'typescript'].map(language => `aio-delivery-${group}-${language}`));
const request = args => spawnSync('gh', args, {encoding: 'utf8', timeout: 60000});
const report = [];
for (const name of repositories) {
  const endpoint = `repos/zjarlin/${name}`;
  let deleted = false;
  for (let attempt = 0; attempt < 3; attempt++) {
    const current = request(['api', endpoint, '--jq', '{name,archived}']);
    if (current.status !== 0) {
      if (current.stderr.includes('HTTP 404')) { deleted = true; break; }
      continue;
    }
    const repository = JSON.parse(current.stdout);
    assert.equal(repository.name, name);
    assert.equal(repository.archived, true, 'Archive test repositories before removing delivery data');
    const removed = request(['repo', 'delete', `zjarlin/${name}`, '--yes']);
    if (removed.status !== 0 && removed.stderr.includes('HTTP 403')) {
      throw new Error(`${name}: GitHub requires delete_repo authorization`);
    }
    const check = request(['api', endpoint, '--jq', '.name']);
    if (check.status !== 0 && check.stderr.includes('HTTP 404')) { deleted = true; break; }
  }
  assert(deleted, `${name}: deletion could not be verified`);
  report.push({name, deleted: true, verifiedAt: Date.now()});
  console.log(`Deleted and verified: ${name}`);
}
fs.writeFileSync('target/delivery-test/deleted-repositories.json', JSON.stringify(report, null, 2));
