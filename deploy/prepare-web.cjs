const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {gzipSync} = require('node:zlib');
const {parse, serialize} = require('parse5');
const root = path.resolve(process.argv[2] || 'target/dx/aio-idea/release/web/public');
const assets = path.join(root, 'assets');
const file = path.join(root, 'index.html');
const document = parse(fs.readFileSync(file, 'utf8'));
const head = document.childNodes.find(node => node.tagName === 'html').childNodes.find(node => node.tagName === 'head');
const scripts=[];
function visit(node){
  if(node.tagName==='script'&&node.attrs.some(a=>a.name==='type'&&a.value==='module')){
    const src=node.attrs.find(a=>a.name==='src')?.value;
    if(src){const url=new URL(src,'https://build.invalid/');const name=path.posix.basename(url.pathname);
      if(url.origin==='https://build.invalid'&&/^aio-idea-dxh[a-f0-9]+\.js$/.test(name))scripts.push(fs.readFileSync(path.join(assets,name),'utf8'));
    }
  }
  for(const child of node.childNodes||[])visit(child);
}
visit(document);
assert.equal(scripts.length,1,'The current HTML must reference exactly one shell module');
const wasm=fs.readdirSync(assets).filter(name=>/^aio-idea_bg-dxh[a-f0-9]+\.wasm$/.test(name)&&scripts[0].includes(name));
assert.equal(wasm.length,1,'The current shell module must reference exactly one Wasm asset');
const href = `/assets/${wasm[0]}`;
if (!head.childNodes.some(node => node.tagName === 'link' && node.attrs.some(attr => attr.name === 'href' && attr.value === href))) {
  head.childNodes.push({nodeName: 'link', tagName: 'link', namespaceURI: 'http://www.w3.org/1999/xhtml',
    attrs: Object.entries({rel: 'preload', as: 'fetch', type: 'application/wasm', href, crossorigin: ''}).map(([name, value]) => ({name, value})),
    childNodes: [], parentNode: head});
}
fs.writeFileSync(file, serialize(document));
const report = [];
for (const name of fs.readdirSync(assets, {recursive: true})) {
  if (!/\.(wasm|js|css|svg|json)$/.test(name)) continue;
  const full = path.join(assets, name);
  if (!fs.statSync(full).isFile()) continue;
  const bytes = fs.readFileSync(full);
  const compressed = gzipSync(bytes, {level: 9});
  if (compressed.length < bytes.length) {
    fs.writeFileSync(`${full}.gz`, compressed);
    report.push({path: name, bytes: bytes.length, gzipBytes: compressed.length});
  }
}
console.log(JSON.stringify({preload: href, assets: report}));
