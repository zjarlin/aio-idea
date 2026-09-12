function createFrontendAssetCache(config) {
  const prefix = 'aio-plugin-assets-v1-';
  const name = prefix + config.session_context;
  const limit = 96 * 1024 * 1024;
  const shared = window[Symbol.for('aio.asset-cache')] ||= { downloads: new Map(), writes: Promise.resolve() };
  const inflight = shared.downloads;
  const storage = typeof caches === 'undefined' ? Promise.resolve(null) : caches.open(name).catch(() => null);
  const digest = async bytes => Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)), byte => byte.toString(16).padStart(2, '0')).join('');
  if (typeof caches !== 'undefined') {
    void caches.keys().then(names => Promise.all(names.filter(key => key.startsWith(prefix) && key !== name).map(key => caches.delete(key)))).catch(() => {});
  }
  const save = (cache, key, bytes, type) => {
    shared.writes = shared.writes.catch(() => {}).then(async () => {
      let size = bytes.byteLength;
      const keys = await cache.keys();
      const sizes = await Promise.all(keys.map(async key => Number((await cache.match(key))?.headers.get('content-length') || 0)));
      size += sizes.reduce((sum, value) => sum + value, 0);
      while (keys.length && (size > limit || keys.length >= 512)) {
        await cache.delete(keys.shift());
        size -= sizes.shift();
      }
      await cache.put(key, new Response(bytes, { headers: { 'content-type': type, 'content-length': String(bytes.byteLength) } }));
    });
    return shared.writes;
  };
  return async (path, ticket, signal) => {
    if (!Object.hasOwn(config.assets, path)) throw new Error('插件未声明该资源');
    const expected = config.assets[path];
    const key = new URL(`/_aio_cache/${config.revision}/${expected}/${path.split('/').map(encodeURIComponent).join('/')}`, location.origin).href;
    const pendingKey = name + key;
    if (inflight.has(pendingKey)) return inflight.get(pendingKey);
    const operation = (async () => {
      const cache = await storage;
      const cached = await cache?.match(key).catch(() => null);
      if (cached) {
        const bytes = await cached.arrayBuffer();
        if (await digest(bytes) === expected) return { bytes, type: cached.headers.get('content-type') };
        await cache.delete(key).catch(() => {});
      }
      const response = await fetch(`/api/runtime/frontend/assets/${ticket}/${path.split('/').map(encodeURIComponent).join('/')}`, { credentials: 'same-origin', redirect: 'error', signal });
      if (!response.ok) throw new Error(`读取插件资源失败: HTTP ${response.status}`);
      const bytes = await response.arrayBuffer();
      if (bytes.byteLength > limit || await digest(bytes) !== expected) throw new Error('插件资源摘要或大小校验失败');
      const type = response.headers.get('content-type') || 'application/octet-stream';
      if (cache) await save(cache, key, bytes, type).catch(() => {});
      return { bytes, type };
    })();
    inflight.set(pendingKey, operation);
    try { return await operation; }
    finally { if (inflight.get(pendingKey) === operation) inflight.delete(pendingKey); }
  };
}
