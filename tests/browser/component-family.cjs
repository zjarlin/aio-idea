const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {launchBrowser,marketplace,closeBrowser}=require('./live-session.cjs');

const base=process.env.AIO_URL||'http://127.0.0.1:4215';
const repository=path.resolve('../aio-plugin-agent-memory');
const parent='https://github.com/zjarlin/aio-plugin-agent.git';
const git='https://github.com/zjarlin/aio-plugin-agent-memory.git';
const output=path.resolve('target/component-delivery',new URL(base).hostname==='127.0.0.1'?'rehearsal':'public');

async function run(){
  const browser=await launchBrowser();
  const context=await browser.newContext({viewport:{width:1440,height:1000}});
  const cookies=fs.readFileSync(process.env.AIO_COOKIE_FILE,'utf8').split(/\r?\n/).filter(l=>l&&(!l.startsWith('#')||l.startsWith('#HttpOnly_'))).map(l=>{
    const [,,cookiePath,,expires,name,value]=l.replace(/^#HttpOnly_/,'').split('\t');
    return {name,value,url:new URL(cookiePath,base).href,secure:new URL(base).protocol==='https:',httpOnly:l.startsWith('#HttpOnly_'),...(Number(expires)>0?{expires:Number(expires)}:{})};
  });
  await context.addCookies(cookies);
  const page=await context.newPage();page.setDefaultTimeout(60000);
  const errors=[];page.on('pageerror',e=>errors.push(e.message));
  try{
    const response=await context.request.post(`${base}/api/runtime/components/publish`,{
      headers:{'content-type':'application/vnd.aio.component+gzip'},
      data:fs.readFileSync(path.join(repository,'dist/agent-memory-0.1.0.aio-plugin')),timeout:240000,
    });
    assert(response.ok(),`Publish ${response.status()}: ${await response.text()}`);
    const published=(await response.json()).data;console.log('Published Agent Memory');
    assert((await context.request.post(`${base}/api/runtime/components/${published.revision}/documentation`,{data:fs.readFileSync(path.join(repository,'README.md'),'utf8'),headers:{'content-type':'text/plain'}})).ok());
    const catalog=await context.request.get(`${base}/api/runtime/marketplace`);
    const entries=(await catalog.json()).data;
    const child=entries.find(e=>e.git===git),root=entries.find(e=>e.git===parent);
    assert.equal(child.parent_git,parent);assert(!child.installed);
    let parentRequired=false;
    if(root?.state!=='active'){
      const denied=await context.request.post(`${base}/api/runtime/plugins/install`,{data:{git}});
      assert(!denied.ok(),'Child must require an enabled parent');parentRequired=true;
    }
    await page.goto(base);await page.locator('.application-shell:visible').waitFor();await marketplace(page,false);
    await page.getByRole('textbox',{name:'搜索插件',exact:true}).fill('Agent');
    const node=page.getByRole('treeitem').filter({hasText:'Agent Memory'});
    await node.click();assert.equal(await node.getAttribute('aria-level'),'2');
    await page.getByText('正在读取 README',{exact:true}).waitFor({state:'hidden'});
    const toggle=page.getByRole('button',{name:/收起 .*agent/i});await toggle.click();assert.equal(await node.count(),0);
    await page.getByRole('button',{name:/展开 .*agent/i}).click();await node.waitFor();
    if(!root)await page.getByText('父插件尚未发布',{exact:true}).first().waitFor();
    fs.mkdirSync(output,{recursive:true});await page.screenshot({path:path.join(output,'plugin-family.png')});
    assert.deepEqual(errors,[]);
    const report={base,revision:published.revision,parent,child:git,parentPublished:!!root,parentRequired,treeLevel:2,optional:true,errors};
    fs.writeFileSync(path.join(output,'family-report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
  }finally{await closeBrowser(browser);}
}
run().catch(e=>{console.error(e.message.split('Call log:')[0]);process.exitCode=1;});
