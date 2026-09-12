const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { readFile, mkdir, writeFile } = require('node:fs/promises');
const { resolve, extname } = require('node:path');
const { chromium } = require('playwright');
const { PNG } = require('pngjs');

const root = resolve('target/dx/aio-idea/release/web/public');
const output = resolve('target/marketplace-test');
let entries;
let revision;
const png = new PNG({ width: 240, height: 80 });
for (let y=0;y<80;y++) for(let x=0;x<240;x++) {
  const offset=(y*240+x)*4;
  png.data.set(x<80?[29,125,95,255]:x<160?[230,237,242,255]:[70,100,143,255],offset);
}
function reset() {
  revision = 'a'.repeat(64);
  entries = ['Dioxus Counter','KMP Counter','TypeScript Counter'].map((title,i)=>({
    git:`https://github.com/example/plugin-${i}.git`,rev:revision,title,summary:'前端交互与独立后端服务',license:'MIT',tags:['fullstack'],
    installed:i<2,source_id:`source-${i}`,state:i<2?'active':null,active_revision:i<2?revision:null,runtime:'process',capabilities:{network:[],filesystem:[],database:false},
  }));
}
const session = {user_id:'test',account:'test',display_name:'Test',tenant_id:'test',tenant_label:'Test',permissions:['plugin:manage']};
const catalog = {session_context:'test',context:'test',tenant:{id:'test',label:'Test'},user:{label:'Test',handle:'test',initials:'T'},plugins:[],pages:[],page_versions:{},account_items:[]};
const server = createServer(async (req,res)=>{
  const send=data=>res.writeHead(200,{'content-type':'application/json','cache-control':'no-store'}).end(JSON.stringify({data}));
  try {
    const path=new URL(req.url,'http://localhost').pathname;
    if(path==='/api/auth/session') return send(session);
    if(path==='/api/runtime/bootstrap') return send({catalog,permissions:session.permissions});
    if(path==='/api/runtime/catalog') return send(catalog);
    if(path==='/api/runtime/marketplace') return send(entries);
    if(path.endsWith('/details')) return send({readme:`# ${revision[0]==='a'?'Counter workspace':'Updated documentation'}\n\nAIO Fullstack\n\n## Modules\n\n| Module | Runtime |\n|---|---|\n| Frontend | Browser |\n| Backend | Process |\n\n\`\`\`typescript\nconst count = 1;\n\`\`\`\n\n![Preview](docs/preview.png)\n\n[Source](shared/model.ts)\n\n<script>window.__readmeExecuted=true</script>\n\n[unsafe](javascript:alert(1))`,version:'0.0.0-dev.1',source_revision:'c'.repeat(40),versions:[{revision,version:'0.0.0-dev.1',source_revision:'c'.repeat(40),created_at:'2026-09-12'}],builds:[]});
    if(path.includes('/images/')) return res.writeHead(200,{'content-type':'image/png'}).end(PNG.sync.write(png));
    if(path==='/api/runtime/plugins/install') {
      const chunks=[];for await(const chunk of req)chunks.push(chunk);
      const body=JSON.parse(Buffer.concat(chunks));const entry=entries.find(e=>e.git===body.git);
      assert(entry);entry.installed=true;entry.state='active';return send(catalog);
    }
    if(path.startsWith('/api/runtime/plugins/')) {
      const [,id,action]=path.match(/\/plugins\/([^/]+)\/(\w+)/)||[];
      const entry=entries.find(e=>e.source_id===id);assert(entry);
      if(action==='uninstall'){entry.installed=false;entry.state=null;}else if(action==='disable')entry.state='disabled';else entry.state='active';
      return send(catalog);
    }
    if(path.startsWith('/api/')) return send([]);
    if(path==='/favicon.ico')return res.writeHead(204).end();
    const file=resolve(root,'.'+(path==='/'?'/index.html':path));assert(file.startsWith(root+'/'));
    const mime={'.html':'text/html','.js':'text/javascript','.css':'text/css','.wasm':'application/wasm','.woff2':'font/woff2'};
    res.writeHead(200,{'content-type':mime[extname(file)]||'application/octet-stream'}).end(await readFile(file));
  }catch(error){res.writeHead(500).end(JSON.stringify({error:error.message}));}
});

async function run(browser,mobile,base) {
  reset();
  const context=await browser.newContext({viewport:mobile?{width:390,height:844}:{width:1440,height:1000}});
  const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
  try {
    await page.goto(base);
    if(mobile)await page.getByRole('button',{name:'打开菜单',exact:true}).click();
    await page.locator('button[aria-label$="的账户菜单"]').last().click();
    await page.getByRole('menuitem',{name:'插件市场',exact:true}).click();
    const tree=page.getByRole('tree',{name:'插件',exact:true});await tree.getByRole('treeitem').first().waitFor();
    assert.equal(await tree.getByRole('treeitem').count(),3);
    if(mobile)await tree.getByRole('treeitem').first().click();
    await page.getByRole('heading',{name:'Counter workspace',exact:true}).waitFor();
    assert.equal(await page.locator('.dx-markdown table').count(),1);
    await page.waitForFunction(()=>{const img=document.querySelector('.dx-markdown img');return img?.complete&&img.naturalWidth===240;});
    assert.equal(await page.evaluate(()=>window.__readmeExecuted),undefined);
    assert.equal(await page.getByRole('link',{name:'unsafe',exact:true}).getAttribute('href'),'#');
    assert((await page.getByRole('link',{name:'Source',exact:true}).getAttribute('href')).includes('/blob/'+ 'c'.repeat(40)+'/shared/model.ts'));
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-details.png`)});
    if(mobile)await page.getByRole('button',{name:'插件列表',exact:true}).click();
    await page.getByRole('textbox',{name:'搜索插件',exact:true}).fill('KMP');
    await page.waitForFunction(()=>document.querySelectorAll('[role=treeitem]').length===1);
    await tree.getByRole('treeitem').click();
    if(mobile)await page.getByRole('button',{name:'插件列表',exact:true}).click();
    await page.getByRole('textbox',{name:'搜索插件',exact:true}).fill('');
    await page.waitForFunction(()=>document.querySelectorAll('[role=treeitem]').length===3);
    if(!mobile){await tree.getByRole('treeitem').nth(1).focus();await page.keyboard.press('ArrowUp');await page.waitForFunction(()=>document.querySelector('[role=treeitem]')===document.activeElement);}
    await tree.getByRole('treeitem').last().click();
    await page.getByRole('button',{name:'安装',exact:true}).click();
    await page.locator('.extension-browser__actions').getByText('已启用',{exact:true}).waitFor();
    await page.getByLabel('管理插件',{exact:true}).click();
    await page.getByRole('menuitem',{name:'停用',exact:true}).click();
    await page.locator('.extension-browser__actions').getByText('已停用',{exact:true}).waitFor();
    await page.getByRole('menuitem',{name:'启用',exact:true}).click();
    await page.locator('.extension-browser__actions').getByText('已启用',{exact:true}).waitFor();
    await page.getByRole('menuitem',{name:'卸载',exact:true}).click();
    await page.getByRole('dialog').waitFor();await page.getByRole('button',{name:'取消',exact:true}).click();
    revision='b'.repeat(64);entries=entries.map(e=>({...e,rev:revision}));
    await page.getByRole('heading',{name:'Updated documentation',exact:true}).waitFor({timeout:35000});
    assert.equal(await page.locator('.extension-browser__heading h1').innerText(),'TypeScript Counter');
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    assert.deepEqual(errors,[]);
    return {viewport:mobile?'mobile':'desktop',tree:true,markdown:true,relativeImage:true,sanitized:true,directInstall:true,management:true,automaticReadme:true};
  }catch(error){await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-failure.png`)});console.error(await page.locator('body').innerText());throw error;}
  finally{await context.close();}
}
(async()=>{
  await mkdir(output,{recursive:true});reset();
  await new Promise(r=>server.listen(Number(process.env.AIO_MARKETPLACE_PREVIEW_PORT||0),'127.0.0.1',r));
  const base=`http://127.0.0.1:${server.address().port}`;
  if(process.env.AIO_MARKETPLACE_PREVIEW_PORT){console.log(base);return;}
  const browser=await chromium.launch({channel:'chrome',headless:true});
  try{const report=[await run(browser,false,base),await run(browser,true,base)];await writeFile(resolve(output,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));}
  finally{await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;server.close();});
