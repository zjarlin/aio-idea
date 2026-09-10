const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const assetURL = new URL(config.src, window.location.href);
if (assetURL.origin !== window.location.origin || !assetURL.pathname.startsWith(`/api/runtime/frontend/assets/${config.token}/`)) {
  throw new Error("插件资产地址无效");
}
const requests = new Map();
let disposed = false;
const reply = (message) => {
  if (!disposed) frame.contentWindow?.postMessage({ channel: "aio-plugin", token: config.token, ...message }, "*");
};
const receive = async (event) => {
  const message = event.data;
  if (disposed || event.source !== frame.contentWindow || event.origin !== "null" || !message ||
      message.channel !== "aio-plugin" || message.token !== config.token ||
      typeof message.id !== "string" || !/^[0-9]{1,16}$/.test(message.id)) return;
  if (requests.has(message.id)) return;
  if (requests.size >= 16) return reply({ id: message.id, error: "插件请求超过并发配额" });
  let body;
  try {
    body = JSON.stringify(message.request);
    if (!body || body.length > 1048576) throw new Error("插件请求体过大");
  } catch (error) {
    return reply({ id: message.id, error: error.message });
  }
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 25000);
  requests.set(message.id, controller);
  try {
    const response = await fetch(`/api/runtime/frontend/${config.token}/request`, {
      method: "POST", credentials: "same-origin", redirect: "error",
      headers: { "content-type": "application/json" }, body, signal: controller.signal,
    });
    const payload = await response.json();
    if (!response.ok) throw new Error(payload.error || `HTTP ${response.status}`);
    reply({ id: message.id, response: payload.data });
  } catch (error) {
    reply({ id: message.id, error: error.message });
  } finally {
    clearTimeout(timeout);
    requests.delete(message.id);
  }
};
window.addEventListener("message", receive);
frame.src = assetURL.href;
try {
  await dioxus.recv();
} finally {
  disposed = true;
  window.removeEventListener("message", receive);
  for (const controller of requests.values()) controller.abort();
  requests.clear();
  frame.removeAttribute("src");
  fetch(`/api/runtime/frontend/${config.token}`, {
    method: "DELETE", credentials: "same-origin", keepalive: true, redirect: "error",
  }).catch(() => {});
}
