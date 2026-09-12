const { test } = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const { createHash, webcrypto } = require('node:crypto');
const { runInNewContext } = require('node:vm');

function fixture() {
  const stores = new Map();
  const requests = [];
  const bytes = Buffer.from('export const value = 42;');
  const sha = createHash('sha256').update(bytes).digest('hex');
  const config = { revision: 'revision-a', session_context: 'login-a', assets: { 'app.js': sha } };
  const caches = {
    keys: async () => [...stores.keys()],
    delete: async name => stores.delete(name),
    open: async name => {
      if (stores.has(name)) return stores.get(name);
      const entries = new Map();
      const cache = {
        entries,
        keys: async () => [...entries.keys()],
        match: async key => entries.get(key)?.clone(),
        delete: async key => entries.delete(key),
        put: async (key, response) => entries.set(key, response.clone()),
      };
      stores.set(name, cache);
      return cache;
    },
  };
  const env = {
    window: {}, caches, crypto: webcrypto, Response, URL, location: { origin: 'https://aio.test' },
    fetch: async (url, options) => {
      requests.push({ url, options });
      return new Response(bytes, { headers: { 'content-type': 'text/javascript' } });
    },
  };
  const create = runInNewContext(readFileSync('src/runtime/frontend_cache.js', 'utf8') + '\ncreateFrontendAssetCache', env);
  return { create, config, caches, stores, requests, bytes, env };
}

test('same-session instances and recreated hosts reuse verified immutable bytes, never business responses', async () => {
  const f = fixture();
  const first = f.create(f.config);
  const a = await first('app.js', 'ticket-a');
  const b = await f.create(f.config)('app.js', 'new-ticket');
  assert.deepEqual(Buffer.from(a.bytes), f.bytes);
  assert.deepEqual(Buffer.from(b.bytes), f.bytes);
  assert.equal(f.requests.length, 1);
  assert.equal(f.requests[0].options.credentials, 'same-origin');
  assert.equal(f.requests[0].options.redirect, 'error');
  await assert.rejects(first('/tasks', 'ticket-a'), /未声明/);
  await assert.rejects(first('../app.js', 'ticket-a'), /未声明/);
  assert.equal(f.requests.length, 1);
});

test('damaged cache is replaced and damaged network artifacts are never admitted', async () => {
  const f = fixture();
  const load = f.create(f.config);
  await load('app.js', 'ticket');
  const cache = f.stores.get('aio-plugin-assets-v1-login-a');
  const [key] = await cache.keys();
  await cache.put(key, new Response('tampered'));
  await load('app.js', 'ticket');
  assert.equal(f.requests.length, 2);
  assert.equal(await (await cache.match(key)).text(), f.bytes.toString());
  await cache.delete(key);
  f.env.fetch = async () => new Response('wrong digest');
  await assert.rejects(load('app.js', 'ticket'), /摘要/);
  assert.equal((await cache.keys()).length, 0);
});

test('concurrent mounts coalesce downloads and login changes remove the old namespace', async () => {
  const f = fixture();
  await Promise.all([f.create(f.config)('app.js', 'one'), f.create(f.config)('app.js', 'two')]);
  assert.equal(f.requests.length, 1);
  await f.create({ ...f.config, session_context: 'login-b' })('app.js', 'three');
  assert.deepEqual(await f.caches.keys(), ['aio-plugin-assets-v1-login-b']);
  assert.equal(f.requests.length, 2);
  const keys = await f.stores.get('aio-plugin-assets-v1-login-b').keys();
  assert(!keys[0].includes('three'), 'Cache keys must not contain authorization tickets');
});

test('cache denial falls back to verified downloads', async () => {
  const f = fixture();
  f.caches.open = async () => { throw new Error('disabled'); };
  assert.deepEqual(Buffer.from((await f.create(f.config)('app.js', 'ticket')).bytes), f.bytes);
  assert.equal(f.requests.length, 1);
});

test('writes from different plugin mounts obey the shared entry quota', async () => {
  const f = fixture();
  const cache = await f.caches.open('aio-plugin-assets-v1-login-a');
  for (let i = 0; i < 512; i++) await cache.put(`old-${i}`, new Response('x', { headers: { 'content-length': '1' } }));
  await Promise.all([
    f.create(f.config)('app.js', 'one'),
    f.create({ ...f.config, revision: 'revision-b' })('app.js', 'two'),
  ]);
  const keys = await cache.keys();
  assert.equal(keys.length, 512);
  assert(keys.some(key => key.includes('revision-a')));
  assert(keys.some(key => key.includes('revision-b')));
  assert(!keys.includes('old-0'));
  assert(!keys.includes('old-1'));
});
