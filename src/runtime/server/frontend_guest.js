(() => {
  const token = document.currentScript.dataset.token;
  const pending = new Map();
  let sequence = 0;
  let visible = true;
  const visibilityListeners = new Set();
  const request = ({ method = "GET", path, query = null, body = "" }) => new Promise((resolve, reject) => {
    if (pending.size >= 16) return reject(new Error("请求数量超过限制"));
    const id = String(++sequence);
    const timeout = setTimeout(() => { pending.delete(id); reject(new Error("插件请求超时")); }, 30000);
    pending.set(id, { resolve, reject, timeout });
    parent.postMessage({ channel: "aio-plugin", token, id, request: { method, path, query, body } }, "*");
  });
  addEventListener("message", (event) => {
    if (event.source !== parent || event.data?.channel !== "aio-plugin" || event.data.token !== token) return;
    if (event.data.lifecycle === "visibility" && typeof event.data.visible === "boolean") {
      visible = event.data.visible;
      for (const listener of visibilityListeners) {
        try { listener(visible); } catch (error) { console.error(error); }
      }
      return;
    }
    const item = pending.get(event.data.id);
    if (!item) return;
    clearTimeout(item.timeout);
    pending.delete(event.data.id);
    if (event.data.error) item.reject(new Error(event.data.error));
    else item.resolve(event.data.response);
  });
  const json = async (method, path, value) => {
    const response = await request({ method, path, body: value === undefined ? "" : JSON.stringify(value) });
    const result = response.body ? JSON.parse(response.body) : null;
    if (response.status < 200 || response.status >= 300) throw new Error(result?.error || `HTTP ${response.status}`);
    return result;
  };
  const onVisibilityChange = (listener) => {
    if (typeof listener !== "function") throw new TypeError("可见性监听器必须是函数");
    visibilityListeners.add(listener);
    listener(visible);
    return () => visibilityListeners.delete(listener);
  };
  Object.defineProperty(window, "aioPlugin", { value: Object.freeze({ request, json, get visible() { return visible; }, onVisibilityChange }), writable: false, configurable: false });
})();
