const config = await dioxus.recv();
const frame = document.getElementById(config.id);
if (!frame) throw new Error("插件容器已卸载");
const assetURL = new URL(config.src, window.location.href);
if (assetURL.origin !== window.location.origin || !assetURL.pathname.startsWith(`/api/runtime/frontend/assets/${config.token}/`)) {
  throw new Error("插件资产地址无效");
}
const requests = new Map();
let disposed = false;
let renewing = false;
const page = frame.closest("[data-aio-page-active]");
const visible = () => !document.hidden && (!page || page.dataset.aioPageActive === "true");
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
const visibility = () => {
  reply({ lifecycle: "visibility", visible: visible() });
};
const revoke = () => fetch(`/api/runtime/frontend/${config.token}`, {
  method: "DELETE", credentials: "same-origin", keepalive: true, redirect: "error",
}).catch(() => {});
const cleanup = () => {
  if (disposed) return;
  disposed = true;
  clearInterval(heartbeat);
  observer.disconnect();
  document.removeEventListener("visibilitychange", activity);
  window.removeEventListener("pagehide", leave);
  window.removeEventListener("pageshow", activity);
  window.removeEventListener("message", receive);
  frame.removeEventListener("load", visibility);
  for (const controller of requests.values()) controller.abort();
  requests.clear();
  frame.removeAttribute("src");
  revoke();
};
const renew = async () => {
  if (disposed || renewing) return;
  renewing = true;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 10000);
  requests.set("lease", controller);
  try {
    const response = await fetch(`/api/runtime/frontend/${config.token}/renew`, {
      method: "POST", credentials: "same-origin", redirect: "error", signal: controller.signal,
    });
    if ([401, 403, 404].includes(response.status) && !disposed) {
      cleanup();
      dioxus.send({ error: "插件挂载已失效，请重新打开页面" });
      window.dispatchEvent(new Event("aio:catalog-invalidated"));
    }
  } catch (_) {
    // 临时断网不销毁已加载界面；服务端仍逐次校验业务请求权限。
  } finally {
    clearTimeout(timeout);
    requests.delete("lease");
    renewing = false;
  }
};
const activity = () => { visibility(); if (visible()) void renew(); };
const leave = event => { if (!event.persisted) cleanup(); };
const observer = new MutationObserver(activity);
if (page) observer.observe(page, { attributes: true, attributeFilter: ["data-aio-page-active"] });
document.addEventListener("visibilitychange", activity);
window.addEventListener("pagehide", leave);
window.addEventListener("pageshow", activity);
frame.addEventListener("load", visibility);
const heartbeat = setInterval(renew, 60000);
frame.src = assetURL.href;
try {
  await dioxus.recv();
} finally {
  cleanup();
}
