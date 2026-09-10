(() => {
  const token = document.currentScript.dataset.token;
  const pending = new Map();
  let sequence = 0;
  const request = ({ method = "GET", path, query = null, body = "" }) => new Promise((resolve, reject) => {
    if (pending.size >= 16) return reject(new Error("请求数量超过限制"));
    const id = String(++sequence);
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error("插件请求超时")); }, 30000);
    pending.set(id, { resolve, reject, timeout });
    parent.postMessage({ channel: "aio-plugin", token, id, request: { method, path, query, body } }, "*");
  });
  addEventListener("message", (event) => {
    if (event.source !== parent || event.data?.channel !== "aio-plugin" || event.data.token !== token) return;
    const item = pending.get(event.data.id);
    if (!item) return;
    clearTimeout(item.timeout);
    pending.delete(event.data.id);
    if (event.data.error) item.reject(new Error(event.data.error));
    else item.resolve(event.data.response);
  });
  Object.defineProperty(window, "aioPlugin", { value: Object.freeze({ request }), writable: false, configurable: false });
})();
