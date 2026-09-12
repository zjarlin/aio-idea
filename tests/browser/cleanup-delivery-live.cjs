const assert = require('node:assert/strict');
const {writeFile} = require('node:fs/promises');
const {resolve} = require('node:path');
const {contextFor, getJson, launchBrowser, closeBrowser} = require('./live-session.cjs');
const base = process.env.AIO_URL || 'https://aio.addzero.site';
const repositories = ['e2e', 'acceptance'].flatMap(group =>
  ['rust', 'kotlin', 'typescript'].map(language => `aio-delivery-${group}-${language}`));
const gits = repositories.map(name => `https://github.com/zjarlin/${name}.git`);
(async () => {
  const browser = await launchBrowser();
  try {
    const context = await contextFor(browser, base, false);
    const before = (await getJson(context, `${base}/api/runtime/marketplace`)).data;
    const verify = process.argv[2] === 'verify';
    if (verify) {
      assert(before.every(entry => !gits.includes(entry.git)), 'Temporary market entries remain');
    } else {
      for (const entry of before.filter(entry => gits.includes(entry.git) && entry.installed)) {
        assert(entry.source_id);
        const response = await context.request.post(
          `${base}/api/runtime/plugins/${encodeURIComponent(entry.source_id)}/uninstall`, {timeout: 60000});
        assert(response.ok(), `${entry.git}: uninstall returned ${response.status()}`);
      }
    }
    const after = (await getJson(context, `${base}/api/runtime/marketplace`)).data;
    assert(after.filter(entry => gits.includes(entry.git)).every(entry => !entry.installed));
    for (const entry of before.filter(entry => !gits.includes(entry.git))) {
      assert.equal(after.find(item => item.git === entry.git)?.active_revision, entry.active_revision);
    }
    const report = {observedAt: Date.now(), repositories, before, after};
    await writeFile(resolve('target/delivery-test', verify ? 'cleanup-market-report.json' : 'uninstall-report.json'),
      JSON.stringify(report, null, 2));
    console.log(JSON.stringify({verified: verify, remaining: after.map(({git, installed}) => ({git, installed}))}));
  } finally { await closeBrowser(browser); }
})().catch(error => { console.error(error.message.split('Call log:')[0]); process.exitCode = 1; });
