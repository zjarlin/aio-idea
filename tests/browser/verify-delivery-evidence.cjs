const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const output = path.resolve('target/delivery-test');
const read = name => JSON.parse(fs.readFileSync(path.join(output, name), 'utf8'));
const publications = read('acceptance-timing-report.json');
const calls = read('acceptance-all-cli-live-report.json');
assert.equal(publications.length, 3);
assert.equal(calls.length, 3);
const firstPushes = publications.map(publication => {
  const call = calls.find(call => call.entry.git === publication.git);
  assert(call);
  assert.equal(call.details.source_revision, publication.source);
  assert.equal(call.entry.active_revision, publication.revision);
  assert(call.calls.some(call => call.status === 200 && call.body.data.status === 200));
  const seconds = (call.observedAt - publication.pushedAt) / 1000;
  assert(seconds >= 0 && seconds <= 3600, `${publication.language}: first push exceeded 60 minutes`);
  return {language: publication.language, job: publication.jobId, source: publication.source,
    revision: publication.revision, secondsToRealBackendCall: seconds};
});
const updates = ['counter', 'readme'].flatMap(mode => {
  const push = read(`${mode}-push.json`);
  const snapshot = read(`${mode}-publication.json`);
  if (mode === 'readme') assert.deepEqual(push.files, ['README.md']);
  return read(`${mode}-report.json`).map(page => {
    assert.equal(page.latest.details.source_revision, push.source);
    assert.equal(page.navigationsDuringUpdate, 0);
    const activation = snapshot.activations.find(event => event.tenant_id === 'default'
      && event.revision === page.latest.entry.rev && event.lifecycle === 'activate');
    assert(activation);
    const activatedAt = Date.parse(activation.created_at);
    const seconds = (page.observedAt - push.pushedAt) / 1000;
    assert(seconds >= 0 && seconds <= 3600, `${mode}: push exceeded 60 minutes`);
    let secondsToStart;
    if (mode === 'counter') {
      const mount = page.mounts.find(mount => mount.at >= activatedAt && mount.body.page_id === 'kmp-fullstack');
      assert(mount);
      secondsToStart = (mount.at - activatedAt) / 1000;
      assert(secondsToStart <= 60, `${page.viewport}: activation exceeded 60 seconds before remount`);
      assert(page.changedCanvasPixels > 30);
    }
    return {mode, viewport: page.viewport, source: push.source, revision: page.latest.entry.rev,
      secondsFromPush: seconds, secondsToStart, navigationsDuringUpdate: page.navigationsDuringUpdate};
  });
});
const report = {verifiedAt: Date.now(), firstPushes, updates};
fs.writeFileSync(path.join(output, 'acceptance-summary.json'), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report));
