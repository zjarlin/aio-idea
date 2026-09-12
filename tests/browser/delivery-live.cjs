const assert=require('node:assert/strict');
const {mkdir,writeFile}=require('node:fs/promises');
const {resolve}=require('node:path');
const {chromium}=require('playwright');
const {PNG}=require('pngjs');
const {contextFor,select,marketplace}=require('./live-session.cjs');
const base=process.env.AIO_URL||'https://aio.addzero.site';
const output=resolve('target/delivery-test');
const mode=process.env.AIO_DELIVERY_E2E_MODE||'counter';
const git='https://github.com/zjarlin/aio-plugin-kmp-example.git';

async function metadata(context){
  const list=await context.request.get(`${base}/api/runtime/marketplace`);assert(list.ok());
  const entry=(await list.json()).data.find(e=>e.git===git);assert(entry);
  const details=await context.request.get(`${base}/api/runtime/marketplace/${entry.rev}/details`);assert(details.ok());
  return {entry,details:(await details.json()).data};
}
async function counter(page,frame) {
  const button=frame.getByRole('button',{name:'+1',exact:true});
  await button.waitFor();await page.waitForTimeout(500);
  const before=PNG.sync.read(await frame.locator('canvas').first().screenshot());
  const bounds=await button.boundingBox();assert(bounds);
  await page.mouse.click(bounds.x+bounds.width/2,bounds.y+bounds.height/2);
  await frame.getByText('1',{exact:true}).waitFor({timeout:10000});
  const after=PNG.sync.read(await frame.locator('canvas').first().screenshot());
  let changed=0;for(let i=0;i<Math.min(before.data.length,after.data.length);i+=4)if(before.data.readUInt32BE(i)!==after.data.readUInt32BE(i))changed++;
  assert(changed>30);return changed;
}
async function prepare(browser,mobile) {
  const context=await contextFor(browser,base,mobile);const page=await context.newPage();
  const events={mainNavigations:0,mounts:[],errors:[]};
  page.on('framenavigated',frame=>{if(frame===page.mainFrame())events.mainNavigations++;});
  page.on('request',request=>{if(request.url().endsWith('/api/runtime/frontend/mount'))events.mounts.push({at:Date.now(),body:request.postDataJSON()});});
  page.on('pageerror',error=>events.errors.push(error.message));
  await page.goto(base);await page.locator('.application-shell:visible').waitFor();
  const initial=await metadata(context);
  const shellMarker=await page.evaluate(()=>window.__deliveryShellMarker=Math.random());
  if(mode==='readme') {
    await marketplace(page,mobile);
    await page.getByRole('treeitem').filter({hasText:'KMP 全栈示例'}).click();
    await page.locator('.dx-markdown').getByRole('heading',{name:'自动发布',exact:true}).waitFor();
    await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-readme-before.png`)});
    return {context,page,mobile,events,initial,shellMarker};
  }
  await page.getByRole('navigation',{name:'场景'}).getByRole('button',{name:'社区插件',exact:true}).click();
  await select(page,mobile,'Dioxus 全栈计数器');
  const other=page.frameLocator('iframe[title="Dioxus 全栈计数器"]');
  await other.getByRole('button',{name:'+1',exact:true}).waitFor({timeout:90000});
  const otherMarker=await other.locator('body').evaluate(()=>window.__deliveryOtherMarker=Math.random());
  await select(page,mobile,'KMP 全栈示例');
  const frame=page.frameLocator('iframe[title="KMP 全栈示例"]');
  await frame.getByRole('button',{name:'Counter',exact:true}).waitFor({timeout:90000});
  await page.waitForTimeout(700);await frame.getByRole('button',{name:'Counter',exact:true}).click({force:true});
  await frame.getByText('KMP Counter',{exact:true}).waitFor();
  await counter(page,frame);
  assert.equal(await frame.locator('body').evaluate(()=>location.hash),'#counter');
  const source=await page.locator('iframe[title="KMP 全栈示例"]').getAttribute('src');
  await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-counter-before.png`)});
  return {context,page,mobile,events,initial,shellMarker,otherMarker,source};
}
async function verify(item){
  const {context,page,mobile,events,initial}=item;
  let painted;
  if(mode==='readme') {
    await page.locator('.dx-markdown').getByRole('heading',{name:'自动发布与滚动更新',exact:true}).waitFor({timeout:3600000});
    assert.equal(await page.locator('.extension-browser__heading h1').innerText(),'KMP 全栈示例');
    await page.waitForFunction(()=>{const image=document.querySelector('.dx-markdown img');return image?.complete&&image.naturalWidth>0;});
  } else {
    const frame=page.frameLocator('iframe[title="KMP 全栈示例"]');
    await frame.getByText('KMP Counter1',{exact:true}).waitFor({timeout:3600000});
    assert.equal(await frame.locator('body').evaluate(()=>location.hash),'#counter');
    assert.notEqual(await page.locator('iframe[title="KMP 全栈示例"]').getAttribute('src'),item.source);
    painted=await counter(page,frame);
    assert.equal(await page.frameLocator('iframe[title="Dioxus 全栈计数器"]').locator('body').evaluate(()=>window.__deliveryOtherMarker),item.otherMarker);
  }
  const observedAt=Date.now();const latest=await metadata(context);
  assert.notEqual(latest.entry.rev,initial.entry.rev);
  assert.equal(latest.entry.rev,latest.entry.active_revision);
  assert.notEqual(latest.details.source_revision,initial.details.source_revision);
  assert.equal(await page.evaluate(()=>window.__deliveryShellMarker),item.shellMarker);
  assert.equal(events.mainNavigations,1);assert.deepEqual(events.errors,[]);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-${mode}-after.png`)});
  return {viewport:mobile?'mobile':'desktop',mode,observedAt,changedCanvasPixels:painted,mainNavigations:events.mainNavigations,mounts:events.mounts,initial,latest};
}
(async()=>{
  await mkdir(output,{recursive:true});
  const browser=await chromium.launch({channel:'chrome',headless:true});
  try {
    const items=[await prepare(browser,false),await prepare(browser,true)];
    const counts=items.map(item=>item.events.mounts.length);
    await Promise.all(items.map(item=>item.page.waitForTimeout(61000)));
    for(let i=0;i<items.length;i++){
      assert.equal(items[i].events.mounts.length,counts[i], 'unchanged polling must not remount plugins');
      assert.equal((await metadata(items[i].context)).entry.rev,items[i].initial.entry.rev);
    }
    await writeFile(resolve(output,`${mode}-ready.json`),JSON.stringify({readyAt:Date.now(),initial:items[0].initial},null,2));
    console.log(`${mode}: desktop and mobile ready; awaiting real default-branch push`);
    const report=await Promise.all(items.map(verify));
    await writeFile(resolve(output,`${mode}-report.json`),JSON.stringify(report,null,2));
    console.log(JSON.stringify(report.map(({viewport,mode,observedAt,changedCanvasPixels,latest})=>({viewport,mode,observedAt,changedCanvasPixels,revision:latest.entry.rev,source:latest.details.source_revision}))));
  }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
