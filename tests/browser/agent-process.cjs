const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {randomUUID} = require('node:crypto');
const {PNG} = require('pngjs');
const {launchBrowser, select, closeBrowser} = require('./live-session.cjs');

const base = process.env.AIO_URL || 'http://127.0.0.1:4245';
const directory = path.resolve('target/component-delivery', new URL(base).hostname === '127.0.0.1' ? 'agent-rehearsal' : 'agent-public');
const agentGit = 'https://github.com/zjarlin/aio-plugin-agent.git';
const memoryGit = 'https://github.com/zjarlin/aio-plugin-agent-memory.git';
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
fs.mkdirSync(directory, {recursive: true, mode: 0o700});
function cookies() {
  return fs.readFileSync(process.env.AIO_COOKIE_FILE, 'utf8').split(/\r?\n/)
    .filter(line => line && (!line.startsWith('#') || line.startsWith('#HttpOnly_')))
    .map(line => {
      const [,,cookiePath,,expires,name,value] = line.replace(/^#HttpOnly_/, '').split('\t');
      return {name,value,url:new URL(cookiePath,base).href,secure:base.startsWith('https:'),httpOnly:line.startsWith('#HttpOnly_'),...(Number(expires)>0?{expires:Number(expires)}:{})};
    });
}
async function json(response, action) {
  if (!response.ok()) {
    const error = await response.json().catch(()=>({}));
    throw new Error(`${action}: HTTP ${response.status()} ${error.error || ''}`);
  }
  return (await response.json()).data;
}
async function openAgent(page, mobile) {
  await page.goto(base);
  await page.locator('.application-shell:visible').waitFor();
  await page.getByRole('navigation', {name:'场景'}).getByRole('button', {name:'工作空间',exact:true}).click();
  await select(page,mobile,'智能体');
  const frame = page.frameLocator('iframe[title="智能体"]');
  await frame.locator('canvas').first().waitFor();
  await frame.getByRole('button', {name:'删除会话',exact:true}).waitFor();
  return frame;
}
async function rpc(page, method, url, value) {
  const frame = page.frames().find(frame => frame.url().includes('/components/assets/'));
  assert(frame, 'Agent frontend is not mounted');
  return frame.evaluate(async ({method,url,value}) => {
    const response = await window.aioPlugin.request({method,path:url,body:value === undefined ? undefined : new TextEncoder().encode(JSON.stringify(value))});
    if (response.status >= 400) throw new Error(`Agent request ${url}: HTTP ${response.status}`);
    return response.body.length ? JSON.parse(new TextDecoder().decode(response.body)) : null;
  }, {method,url,value});
}
async function eventually(read, accepts) {
  for (let i=0;i<120;i++) { const value = await read(); if (accepts(value)) return value; await wait(500); }
  throw new Error('Agent task did not reach the expected state');
}
async function run() {
  const browser = await launchBrowser();
  const context = await browser.newContext({viewport:{width:1440,height:1000},permissions:['clipboard-read','clipboard-write']});
  await context.addCookies(cookies());
  const page = await context.newPage(); page.setDefaultTimeout(90000);
  const errors = []; page.on('pageerror',error=>errors.push(error.message));
  try {
    const marketplace = () => context.request.get(`${base}/api/runtime/marketplace`).then(response=>json(response,'marketplace'));
    if (process.argv[2] === 'publish') {
      const binary = fs.readFileSync('../aio-plugin-agent/dist/agent-0.1.0.aio-plugin');
      const publication = await json(await context.request.post(`${base}/api/runtime/components/publish`,{headers:{'content-type':'application/vnd.aio.component+gzip'},data:binary,timeout:240000}),'publish');
      console.log('Validated Agent process bundle');
      const download = await context.request.get(`${base}/api/runtime/packages/${publication.revision}`,{timeout:120000});
      assert(download.ok() && (await download.body()).equals(binary));
      console.log('Verified package download');
      await json(await context.request.post(`${base}/api/runtime/components/${publication.revision}/documentation`,{headers:{'content-type':'text/plain'},data:fs.readFileSync('../aio-plugin-agent/README.md','utf8')}),'documentation');
      for (const git of [agentGit,memoryGit]) {
        const entry = (await marketplace()).find(entry=>entry.git===git);
        assert(entry, `Missing marketplace source ${git}`);
        if (!entry.installed || entry.state !== 'active' || entry.active_revision !== entry.rev)
          await json(await context.request.post(`${base}/api/runtime/plugins/install`,{data:{git},timeout:240000}),'install');
        console.log(`Installed ${entry.title}`);
      }
      const installed = (await marketplace()).filter(entry=>[agentGit,memoryGit].includes(entry.git));
      assert(installed.every(entry=>entry.installed&&entry.state==='active'));
      fs.writeFileSync(path.join(directory,'publication.json'),JSON.stringify({revision:publication.revision,installed:installed.map(entry=>({git:entry.git,source:entry.source_id,revision:entry.active_revision}))},null,2));
      console.log('Published and installed Agent + Memory');
      return;
    }
    // Create a mount through the host API before opening Compose's first conversation.
    const installed = (await marketplace()).find(entry=>entry.git===agentGit);
    assert(installed?.installed, 'Agent is not installed');
    const mount = await json(await context.request.post(`${base}/api/runtime/frontend/mount`,{data:{page_id:`component:${installed.source_id}:chat`}}),'mount');
    const direct = async (method,url,value) => {
      const response = await json(await context.request.post(`${base}/api/runtime/components/${mount.token}/request`,{data:{method,path:url,query:null,headers:[{name:'content-type',value:'application/json'}],body:value===undefined?[]:Array.from(Buffer.from(JSON.stringify(value)))}}),'request');
      assert(response.status < 400, `${url}: HTTP ${response.status}`);
      return response.body.length ? JSON.parse(Buffer.from(response.body)) : null;
    };
    let fixture;
    if (process.argv[2] === 'resume') fixture = JSON.parse(fs.readFileSync(path.join(directory,'fixture.json'),'utf8'));
    else {
      const space = (await direct('POST','/memory',{method:'GET',path:'/spaces',body:null})).find(space=>space.personal);
      assert(space,'Personal memory space missing');
      const conversation = await direct('POST','/conversations',{title:'正式 AIO 接入验收',spaceId:space.id});
      fixture = {space:space.id,conversation:conversation.id,canary:`aio-test-${randomUUID()}`};
      fs.writeFileSync(path.join(directory,'fixture.json'),JSON.stringify(fixture),{mode:0o600});
    }
    const memory = (method,url,body=null) => direct('POST','/memory',{method,path:url,body});
    const threadPath = `/conversations/${fixture.conversation}`;
    if (process.argv[2] !== 'resume') {
      const prompt = {requestId:randomUUID(),content:JSON.stringify({project:'正式接入项目',website:'https://example.test',password:fixture.canary,note:'周三整理资料'})};
      const accepted = await direct('POST',`${threadPath}/messages`,prompt);
      assert(!JSON.stringify(accepted).includes(fixture.canary));
      await direct('POST',`${threadPath}/messages`,prompt);
    }
    const thread = await eventually(()=>direct('GET',threadPath),value=>value.messages[0]?.sourceId);
    assert(!JSON.stringify(thread).includes(fixture.canary));
    fixture.source = thread.messages[0].sourceId;
    const source = await eventually(()=>memory('GET',`/sources/${fixture.source}`),value=>value.secrets.length===1);
    assert(!JSON.stringify(source).includes(fixture.canary));
    fixture.secret = source.secrets[0].id;
    fs.writeFileSync(path.join(directory,'fixture.json'),JSON.stringify(fixture),{mode:0o600});
    assert.equal((await memory('POST',`/secrets/${fixture.secret}/reveal`)).value,fixture.canary);
    console.log('Verified encrypted intake and controlled reveal');
    const graph = await memory('GET',`/graph?spaceId=${fixture.space}`);
    assert(!JSON.stringify(graph).includes(fixture.canary));
    if (process.argv[2] !== 'resume') {
      assert.equal(thread.messages.length,2,'Idempotent intake created duplicate messages');
      await direct('POST',`${threadPath}/messages`,{requestId:randomUUID(),content:'查找 正式接入项目'});
    }
    const recalled = await eventually(()=>direct('GET',threadPath),value=>value.messages.at(-1)?.status==='complete'&&value.messages.at(-1)?.route==='recall');
    assert.equal(recalled.messages.at(-1).tokens,0);
    assert(recalled.messages.at(-1).activatedNodeIds.includes(fixture.source));
    assert(!JSON.stringify(recalled).includes(fixture.canary));
    console.log('Verified local recall and graph activation');
    for (const [name,viewport,mobile] of [['desktop',{width:1440,height:1000},false],['mobile',{width:390,height:844},true]]) {
      await page.setViewportSize(viewport);
      console.log(`Opening Compose ${name}`);
      const frame = await openAgent(page,mobile);
      await wait(2500);
      await frame.getByLabel(/知识图谱，.*个激活节点/).waitFor();
      const png = PNG.sync.read(await page.screenshot({path:path.join(directory,`${name}.png`)}));
      let highlighted=0;
      for (let i=0;i<png.data.length;i+=4) { const [r,g,b]=png.data.subarray(i,i+3); if(r>140&&r<210&&g>70&&g<140&&b<65) highlighted++; }
      assert(highlighted>15,'Activated graph node was not painted');
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      if (!mobile) {
        await frame.getByRole('button',{name:'正式接入项目',exact:true}).last().click({force:true});
        const toggle = frame.getByRole('button',{name:'查看秘密',exact:true});
        await toggle.waitFor(); await wait(300);
        const closeBounds = await frame.getByRole('button',{name:'关闭',exact:true}).last().boundingBox();
        const toggleBounds = await toggle.boundingBox();
        assert(closeBounds && toggleBounds);
        const revealed = page.waitForResponse(response=>{
          if(!response.url().endsWith('/request'))return false;
          const request=response.request().postDataJSON();
          if(request?.path!=='/memory')return false;
          return JSON.parse(Buffer.from(request.body)).path.endsWith('/reveal');
        });
        await toggle.click({force:true}); await page.mouse.move(0,0);
        const protectedResponse = await json(await revealed,'controlled reveal');
        assert.equal(JSON.parse(Buffer.from(protectedResponse.body)).value,fixture.canary);
        console.log('Verified password display through Compose');
        await wait(300);
        await page.screenshot({path:path.join(directory,'controlled-reveal.png')});
        // Compose may remove dynamic semantic nodes after revealing a field.
        await page.mouse.click(toggleBounds.x+toggleBounds.width/2,toggleBounds.y+toggleBounds.height/2);
        await wait(200);
        await page.mouse.click(closeBounds.x+closeBounds.width/2,closeBounds.y+closeBounds.height/2);
        await wait(250);
      }
    }
    assert.deepEqual(errors,[]);
    fs.writeFileSync(path.join(directory,'report.json'),JSON.stringify({base,restart:process.argv[2]==='resume',receipt:true,idempotency:true,controlledReveal:true,localRecall:true,tokens:0,graphActivation:true,desktop:true,mobile:true,errors},null,2));
    console.log('Agent intake, local recall, graph activation, controlled reveal and Compose desktop/mobile passed');
    if (process.env.AIO_AGENT_TEST_CLEANUP==='1') {
      for (const id of new Set(recalled.messages.map(message=>message.sourceId).filter(Boolean))) await memory('DELETE',`/nodes/${id}?spaceId=${fixture.space}`);
      const clean = await memory('GET',`/graph?spaceId=${fixture.space}`);
      assert(!clean.nodes.some(node=>node.id===fixture.source));
      await direct('DELETE',threadPath);
      fs.unlinkSync(path.join(directory,'fixture.json'));
    }
    await context.request.delete(`${base}/api/runtime/frontend/${mount.token}`);
  } catch (error) {
    await page.screenshot({path:path.join(directory,'failure.png')}).catch(()=>{});
    throw error;
  } finally { await closeBrowser(browser); }
}
run().catch(error=>{console.error(error.message.split('Call log:')[0]);process.exitCode=1;});
