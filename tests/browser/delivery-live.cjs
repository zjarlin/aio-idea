const assert=require('node:assert/strict');
const {mkdir,writeFile}=require('node:fs/promises');
const {resolve}=require('node:path');
const {PNG}=require('pngjs');
const {contextFor,select,marketplace,getJson,launchBrowser,closeBrowser}=require('./live-session.cjs');
const base=process.env.AIO_URL||'https://aio.addzero.site';
const output=resolve('target/delivery-test');
const mode=process.env.AIO_DELIVERY_E2E_MODE||'counter';
const git='https://github.com/zjarlin/aio-plugin-kmp-example.git';
const otherPlugin=process.env.AIO_DELIVERY_OTHER_PLUGIN||'计数器示例';

async function metadata(context){
  const list=await getJson(context,`${base}/api/runtime/marketplace`);
  const entry=list.data.find(e=>e.git===git);assert(entry);
  const details=await getJson(context,`${base}/api/runtime/marketplace/${entry.rev}/details`);
  return {entry,details:details.data};
}
async function counter(page,frame) {
  const button=frame.getByRole('button',{name:'+1',exact:true});
  await button.waitFor();await page.mouse.move(0,0);await page.waitForTimeout(500);
  const before=PNG.sync.read(await frame.locator('canvas').first().screenshot());
  const bounds=await button.boundingBox();assert(bounds);
  await page.mouse.click(bounds.x+bounds.width/2,bounds.y+bounds.height/2);
  try{await frame.getByText('1',{exact:true}).waitFor({timeout:10000});}
  catch(error){
    await page.screenshot({path:resolve(output,`${page.viewportSize().width}-counter-click-failure.png`)});
    console.error(JSON.stringify({bounds,canvas:await frame.locator('canvas').first().boundingBox(),text:await frame.locator('body').innerText()}));
    throw error;
  }
  const after=PNG.sync.read(await frame.locator('canvas').first().screenshot());
  let changed=0;for(let i=0;i<Math.min(before.data.length,after.data.length);i+=4)if(before.data.readUInt32BE(i)!==after.data.readUInt32BE(i))changed++;
  assert(changed>30);return changed;
}
async function prepare(browser,mobile) {
  console.log(`Opening ${mobile?'mobile':'desktop'} ${mode} page`);
  const context=await contextFor(browser,base,mobile);const page=await context.newPage();
  const events={mainNavigations:0,navigationUrls:[],mounts:[],errors:[]};
  page.deliveryEvents=events;
  page.on('framenavigated',frame=>{if(frame===page.mainFrame()){events.mainNavigations++;events.navigationUrls.push(frame.url());}});
  page.on('request',request=>{if(request.url().endsWith('/api/runtime/frontend/mount'))events.mounts.push({at:Date.now(),body:request.postDataJSON()});});
  page.on('pageerror',error=>events.errors.push(error.message));
  page.on('requestfailed',request=>{console.log(JSON.stringify({viewport:mobile?'mobile':'desktop',failed:new URL(request.url()).pathname,error:request.failure()}));});
  page.on('response',response=>{if(response.url().includes('/api/runtime/frontend/')&&response.status()>=400)console.log(JSON.stringify({status:response.status(),path:new URL(response.url()).pathname}));});
  await page.goto(base);await page.locator('.application-shell:visible').waitFor();
  const initial=await metadata(context);
  console.log(`${mobile?'mobile':'desktop'} baseline ${initial.entry.rev}`);
  const shellMarker=await page.evaluate(()=>window.__deliveryShellMarker=Math.random());
  if(mode==='readme') {
    await marketplace(page,mobile);
    await page.getByRole('treeitem').filter({hasText:'任务工作台示例'}).click();
    await page.locator('.dx-markdown').getByRole('heading',{name:'自动发布',exact:true}).waitFor();
    await page.waitForFunction(()=>{const image=document.querySelector('.dx-markdown img');return image?.complete&&image.naturalWidth>0;});
    await page.locator('.dx-markdown').getByRole('heading',{name:'自动发布',exact:true}).scrollIntoViewIfNeeded();
    const readingScroll=await page.locator('.extension-browser__detail').evaluate(element=>element.scrollTop);
    await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-readme-before.png`)});
    return {context,page,mobile,events,initial,shellMarker,readingScroll};
  }
  await page.getByRole('navigation',{name:'场景'}).getByRole('button',{name:'社区插件',exact:true}).click();
  await select(page,mobile,otherPlugin);
  const other=page.frameLocator(`iframe[title="${otherPlugin}"]`);
  await other.getByRole('button').first().waitFor({timeout:180000});
  const otherMarker=await other.locator('body').evaluate(()=>window.__deliveryOtherMarker=Math.random());
  await select(page,mobile,'任务工作台示例');
  const frame=page.frameLocator('iframe[title="任务工作台示例"]');
  await frame.getByRole('button',{name:'Counter',exact:true}).waitFor({timeout:180000});
  await page.waitForTimeout(700);await frame.getByRole('button',{name:'Counter',exact:true}).click({force:true});
  await frame.getByText('KMP Counter',{exact:true}).waitFor();
  await counter(page,frame);
  assert.equal(await frame.locator('body').evaluate(()=>location.hash),'#counter');
  const source=await page.locator('iframe[title="任务工作台示例"]').getAttribute('src');
  await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-counter-before.png`)});
  return {context,page,mobile,events,initial,shellMarker,otherMarker,source};
}
async function verify(item){
  const {context,page,mobile,events,initial}=item;
  let painted;
  if(mode==='readme') {
    await page.locator('.dx-markdown').getByRole('heading',{name:'自动发布与滚动更新',exact:true}).waitFor({timeout:3600000});
    assert.equal(await page.locator('.extension-browser__heading h1').innerText(),'任务工作台示例');
    await page.waitForFunction(()=>{const image=document.querySelector('.dx-markdown img');return image?.complete&&image.naturalWidth>0;});
    assert(Math.abs(await page.locator('.extension-browser__detail').evaluate(element=>element.scrollTop)-item.readingScroll)<=2,'README update must preserve the reading position');
  } else {
    const frame=page.frameLocator('iframe[title="任务工作台示例"]');
    await frame.getByText('KMP Counter1',{exact:true}).waitFor({timeout:3600000});
    assert.equal(await frame.locator('body').evaluate(()=>location.hash),'#counter');
    assert.notEqual(await page.locator('iframe[title="任务工作台示例"]').getAttribute('src'),item.source);
    painted=await counter(page,frame);
    assert.equal(await page.frameLocator(`iframe[title="${otherPlugin}"]`).locator('body').evaluate(()=>window.__deliveryOtherMarker),item.otherMarker);
  }
  const observedAt=Date.now();const latest=await metadata(context);
  assert.notEqual(latest.entry.rev,initial.entry.rev);
  assert.equal(latest.entry.rev,latest.entry.active_revision);
  assert.notEqual(latest.details.source_revision,initial.details.source_revision);
  assert.equal(await page.evaluate(()=>window.__deliveryShellMarker),item.shellMarker);
  assert.equal(events.mainNavigations,item.baselineNavigations);assert.deepEqual(events.errors,[]);
  assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
  await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-${mode}-after.png`)});
  return {viewport:mobile?'mobile':'desktop',mode,observedAt,changedCanvasPixels:painted,initialNavigations:item.baselineNavigations,navigationsDuringUpdate:events.mainNavigations-item.baselineNavigations,mounts:events.mounts,initial,latest};
}
(async()=>{
  await mkdir(output,{recursive:true});
  const browser=await launchBrowser();
  try {
    if(process.env.AIO_DELIVERY_BASE_SHA){
      const context=await contextFor(browser,base,false);const deadline=Date.now()+3600000;
      console.log('Waiting for the automatic baseline publication before opening the two persistent pages.');
      while((await metadata(context)).details.source_revision!==process.env.AIO_DELIVERY_BASE_SHA){
        assert(Date.now()<deadline,'baseline publication timed out');
        await new Promise(resolve=>setTimeout(resolve,10000));
      }
      await context.close();
    }
    const items=[await prepare(browser,false),await prepare(browser,true)];
    for(const item of items)item.baselineNavigations=item.events.mainNavigations;
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
  }catch(error){
    let index=0;
    for(const context of browser.contexts())for(const page of context.pages()){
      const frames=await Promise.all(page.frames().filter(frame=>frame!==page.mainFrame()).map(async frame=>({url:frame.url(),text:await frame.locator('body').innerText().catch(()=>''),resources:await frame.evaluate(()=>performance.getEntriesByType('resource').map(({name,duration,transferSize})=>({name,duration,transferSize}))).catch(()=>[])})));
      await page.screenshot({path:resolve(output,`${mode}-${index++}-failure.png`)});
      console.error(JSON.stringify({events:page.deliveryEvents,text:await page.locator('body').innerText(),frames}));
    }
    throw error;
  }finally{await closeBrowser(browser);}
})().catch(error=>{console.error(error);process.exitCode=1;});
