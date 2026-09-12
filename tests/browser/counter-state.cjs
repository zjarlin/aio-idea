const assert = require('node:assert/strict');

module.exports = async function verifyCounterState(page, context, tenant) {
  const frame = page.frameLocator('iframe[title="计数器示例"]');
  const increment = frame.getByRole('button', { name: '+1', exact: true });
  const backend = frame.getByRole('button', { name: '请求后端 +1', exact: true });
  const requests = [];
  const capture = request => {
    if (/\/api\/runtime\/(frontend\/[^/]+\/request|pages\/action)(?:\?|$)/.test(request.url())) {
      requests.push(request);
    }
  };
  await increment.waitFor({ timeout: 60000 });
  await frame.getByText('计数：0', { exact: true }).waitFor();
  page.on('request', capture);
  try {
    await increment.click();
    await frame.getByText('计数：1', { exact: true }).waitFor({ timeout: 1000 });
    await context.setOffline(true);
    for (let i = 0; i < 20; i++) await increment.click();
    await frame.getByText('计数：21', { exact: true }).waitFor({ timeout: 1000 });
  } finally {
    await context.setOffline(false);
    page.off('request', capture);
  }
  assert.equal(requests.length, 0, '本地 Signal 更新不能发送 action 或服务请求，包括断网重试');

  // Hold the actual service call to prove its pending state cannot block local UI.
  let release;
  const pending = new Promise(resolve => { release = resolve; });
  const endpoint = '**/api/runtime/frontend/*/request';
  const hold = async route => { await pending; await route.continue(); };
  await context.route(endpoint, hold);
  let response;
  try {
    const sent = page.waitForRequest(request => /\/frontend\/[^/]+\/request$/.test(request.url()));
    response = page.waitForResponse(response => /\/frontend\/[^/]+\/request$/.test(response.url()));
    await backend.click();
    await sent;
    assert(await backend.isDisabled());
    await increment.click();
    await frame.getByText('计数：22', { exact: true }).waitFor({ timeout: 1000 });
  } finally {
    release();
    if (response) assert((await response).ok());
    await context.unroute(endpoint, hold);
  }
  await frame.getByText('服务端结果：1', { exact: true }).waitFor();
  if (tenant) await frame.getByText(`租户：${tenant}`, { exact: true }).waitFor();
  else await frame.getByText(/^租户：.+/).waitFor();
  await frame.getByText('计数：22', { exact: true }).waitFor();
  return { count: 22, offlineClicks: 20, localCounterRequests: requests.length, componentRequest: true, responsiveWhileBackendPending: true };
};
