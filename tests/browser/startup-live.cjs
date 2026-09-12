const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {contextFor, launchBrowser, closeContext, closeBrowser} = require('./live-session.cjs');
const base = process.env.AIO_URL || 'https://aio.addzero.site';
const output = path.resolve(process.env.AIO_PERFORMANCE_OUTPUT || 'target/startup-test');
(async () => {
  fs.mkdirSync(output, {recursive: true});
  const browser = await launchBrowser();
  const report = [];
  try {
    for (const mobile of [false, true]) {
      const context = await contextFor(browser, base, mobile);
      const page = await context.newPage();
      for (const phase of ['cold', 'warm']) {
      const requests = [];
      const failures = [];
      const onFailure = request => failures.push({path: new URL(request.url()).pathname, error: request.failure()?.errorText});
      const onResponse = response => {
        const url = new URL(response.url());
        if (url.origin === base && (url.pathname.startsWith('/api/') || /\.(wasm|js|css)$/.test(url.pathname))) {
          const request = response.request();
          requests.push({path: url.pathname, status: response.status(), timing: request.timing(),
            encoding: response.headers()['content-encoding'], cache: response.headers()['cache-control'],
            serverTiming: response.headers()['server-timing']});
        }
      };
      page.on('requestfailed', onFailure);
      page.on('response', onResponse);
      const started = Date.now();
      let failure;
      let documentHeaders;
      let workspaceDomMs;
      try {
        const document = await page.goto(base, {waitUntil: 'domcontentloaded', timeout: 60000});
        documentHeaders = {cache: document.headers()['cache-control'], serverTiming: document.headers()['server-timing']};
        await page.locator('.application-shell:visible').waitFor({timeout: 60000});
        workspaceDomMs = await page.evaluate(() => performance.now());
        await page.waitForFunction(() => {
          const shell = document.querySelector('.application-shell');
          return shell && getComputedStyle(shell).display === 'grid';
        }, null, {timeout: 60000});
      }
      catch (error) { failure = error.message.split('Call log:')[0]; }
      const readyMs = Date.now() - started;
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      const metrics = await page.evaluate(() => ({navigation: performance.getEntriesByType('navigation')[0]?.toJSON(),
        resources: performance.getEntriesByType('resource').map(({name, startTime, responseStart, responseEnd, transferSize, encodedBodySize, decodedBodySize}) =>
          ({path: new URL(name).pathname, startTime, responseStart, responseEnd, transferSize, encodedBodySize, decodedBodySize}))}));
      const startupApiRequests = metrics.resources.filter(resource => resource.startTime <= workspaceDomMs && ['/api/runtime/bootstrap', '/api/runtime/catalog', '/api/auth/session'].includes(resource.path)).length;
      await page.screenshot({path: path.join(output, `${mobile ? 'mobile' : 'desktop'}-${phase}-startup.png`)});
      report.push({viewport: mobile ? 'mobile' : 'desktop', phase, readyMs, workspaceDomMs, startupApiRequests, documentHeaders, requests, failures, failure,
        visibleText: failure ? await page.locator('body').innerText() : undefined, metrics});
      fs.writeFileSync(path.join(output, 'startup-report.json'), JSON.stringify(report, null, 2));
      console.log(JSON.stringify({viewport: mobile ? 'mobile' : 'desktop', phase, readyMs, startupApiRequests, documentHeaders,
        api: requests.filter(request => request.path.startsWith('/api/'))}));
      page.off('requestfailed', onFailure);
      page.off('response', onResponse);
      if (failure) throw new Error(failure);
      if (process.env.AIO_EXPECT_EMBEDDED === '1') {
        assert.equal(startupApiRequests, 0, 'Initial workspace must use its freshly authenticated HTML snapshot');
        assert.equal(documentHeaders.cache, 'private, no-store');
      }
      }
      await closeContext(context);
    }
    fs.writeFileSync(path.join(output, 'startup-report.json'), JSON.stringify(report, null, 2));
  } finally { await closeBrowser(browser); }
})().catch(error => { console.error(error.message.split('Call log:')[0]); process.exitCode = 1; });
