const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const {marketplace,launchBrowser}=require('./live-session.cjs');
const base=process.env.AIO_URL||'http://127.0.0.1:4215';
const output=path.resolve('target/component-delivery',new URL(base).hostname==='127.0.0.1'?'rehearsal':'public');
const plugin=path.resolve('../aio-plugin-screen');
const git='https://github.com/zjarlin/aio-plugin-screen.git';
const name=`市场验收 ${Date.now()}`;
fs.mkdirSync(output,{recursive:true});
fs.writeFileSync(path.join(output,'current-run.json'),JSON.stringify({base,name}));
function cookies(){return fs.readFileSync(process.env.AIO_COOKIE_FILE,'utf8').split(/\r?\n/).filter(l=>l&&(!l.startsWith('#')||l.startsWith('#HttpOnly_'))).map(l=>{
  const [,,cookiePath,,expires,name,value]=l.replace(/^#HttpOnly_/,'').split('\t');
  return {name,value,url:new URL(cookiePath,base).href,secure:new URL(base).protocol==='https:',httpOnly:l.startsWith('#HttpOnly_'),...(Number(expires)>0?{expires:Number(expires)}:{})};
});}
async function openScreen(page,mobile){
  await page.goto(base);await page.locator('.application-shell:visible').waitFor();
  await page.getByRole('navigation',{name:'场景'}).getByRole('button',{name:'社区插件',exact:true}).click();
  if(mobile)await page.getByRole('button',{name:'打开菜单',exact:true}).click();
  const nav=mobile?page.getByRole('dialog'):page.locator('.application-shell__sidebar');
  await nav.getByRole('button',{name:'数据大屏',exact:true}).click();
  const frame=page.frameLocator('iframe[title="数据大屏"]');
  await frame.getByRole('heading',{name:'我的大屏',exact:true}).waitFor();
  return frame;
}
async function rpc(page,method,url,value){
  const frame=page.frames().find(f=>f.url().includes('/components/assets/'));
  assert(frame,'Component not mounted');
  return frame.evaluate(async({method,url,value})=>{
    const response=await window.aioPlugin.request({method,path:url,body:value===undefined?undefined:new TextEncoder().encode(JSON.stringify(value))});
    const data=JSON.parse(new TextDecoder().decode(response.body));
    if(response.status>=400)throw new Error(data.message);return data;
  },{method,url,value});
}
const pixels=canvas=>canvas.evaluate(c=>{
  const a=c.getContext('2d').getImageData(0,0,c.width,c.height).data;let painted=0;const colors=new Set();
  for(let i=0;i<a.length;i+=16)if(a[i+3]){painted++;colors.add(`${a[i]},${a[i+1]},${a[i+2]}`);}
  return {painted,colors:colors.size,width:c.width,height:c.height};
});
async function run(){
  const browser=await launchBrowser();
  const context=await browser.newContext({viewport:{width:1440,height:1000}});await context.addCookies(cookies());
  const page=await context.newPage();page.setDefaultTimeout(60000);const errors=[];
  page.on('pageerror',e=>errors.push(e.message));page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  page.on('requestfailed',request=>{if(request.failure()?.errorText!=='net::ERR_ABORTED')errors.push(`${new URL(request.url()).pathname.replace(/\/components\/assets\/[^/]+/,'/components/assets/[token]')}: ${request.failure()?.errorText}`);});
  let screenId,datasetId,installedByTest=false;
  const entry=async()=>{const response=await context.request.get(`${base}/api/runtime/marketplace`);assert(response.ok());return (await response.json()).data.find(e=>e.git===git);};
  try{
    if(process.env.AIO_COMPONENT_TEST_RESET==='1'){
      assert.equal(new URL(base).hostname,'127.0.0.1','Reset is limited to local rehearsal');
      const previous=await entry();
      if(previous?.installed)assert((await context.request.post(`${base}/api/runtime/plugins/${previous.source_id}/uninstall`)).ok());
    }
    const binary=fs.readFileSync(path.join(plugin,'dist/screen-0.1.0.aio-plugin'));
    const publish=await context.request.post(`${base}/api/runtime/components/publish`,{headers:{'content-type':'application/vnd.aio.component+gzip'},data:binary,timeout:240000});
    assert(publish.ok(),`Publish ${publish.status()}: ${(await publish.text()).slice(0,300)}`);
    const published=(await publish.json()).data;console.log('Published Component');
    const download=await context.request.get(`${base}/api/runtime/packages/${published.revision}`,{timeout:120000});
    assert(download.ok());assert((await download.body()).equals(binary),'Downloaded bundle must match the published file');
    const docs=await context.request.post(`${base}/api/runtime/components/${published.revision}/documentation`,{headers:{'content-type':'text/plain'},data:fs.readFileSync(path.join(plugin,'README.md'),'utf8')});assert(docs.ok());
    assert(!(await entry()).installed,'Publication must not install the plugin');
    await page.goto(base);await page.locator('.application-shell:visible').waitFor();await marketplace(page,false);
    await page.getByRole('textbox',{name:'搜索插件',exact:true}).fill('数据大屏');
    await page.getByRole('treeitem').filter({hasText:'数据大屏'}).click();
    await page.getByRole('button',{name:'安装',exact:true}).waitFor();
    await page.screenshot({path:path.join(output,'marketplace-available.png')});
    installedByTest=true;await page.getByRole('button',{name:'安装',exact:true}).click();
    await page.locator('.extension-browser__actions').getByText('已启用',{exact:true}).waitFor();
    const installed=await entry();assert.equal(installed.active_revision,published.revision);console.log('Installed through marketplace');
    let frame=await openScreen(page,false);console.log('Mounted native frontend');
    assert.equal(await page.locator('iframe[title="数据大屏"]').getAttribute('sandbox'),'allow-scripts allow-forms');
    await frame.getByRole('button',{name:'数据集',exact:true}).click();
    await frame.getByRole('button',{name:'导入数据',exact:true}).first().click();
    await frame.getByLabel('导入数据文件').setInputFiles({name:`${name}.csv`,mimeType:'text/csv',buffer:Buffer.from('区域,金额\n华东,10\n华南,20\n')});
    await frame.getByRole('button',{name:'保存数据集',exact:true}).click();
    await frame.getByRole('dialog').waitFor({state:'hidden'});
    datasetId=(await rpc(page,'GET','/datasets')).find(d=>d.name===name).id;console.log('Imported CSV');
    await frame.getByRole('button',{name:'大屏',exact:true}).click();
    await frame.getByRole('button',{name:'新建大屏',exact:true}).first().click();
    await frame.getByLabel('大屏名称',{exact:true}).fill(name);
    await frame.getByRole('button',{name:'创建大屏',exact:true}).click();
    await frame.getByRole('button',{name:'添加柱状图',exact:true}).dragTo(frame.locator('.canvas-stage'),{targetPosition:{x:160,y:240}});
    screenId=(await rpc(page,'GET','/screens')).find(s=>s.title===name).id;
    await frame.getByRole('button',{name:'数据',exact:true}).click();
    await frame.getByLabel('绑定数据集',{exact:true}).selectOption(datasetId);
    const canvas=frame.locator('[data-kind="bar"] canvas');await canvas.waitFor();
    await page.waitForTimeout(1000);const desktopPixels=await pixels(canvas);assert(desktopPixels.painted>100&&desktopPixels.colors>8);
    await frame.getByRole('button',{name:'保存',exact:true}).click();await frame.getByText('草稿已保存',{exact:true}).waitFor();
    await frame.getByRole('button',{name:'发布',exact:true}).click();await frame.getByText('已发布',{exact:true}).waitFor();
    await page.screenshot({path:path.join(output,'screen-desktop.png')});console.log('Saved and published screen');
    const before=await rpc(page,'GET',`/screens/${screenId}`);
    frame=await openScreen(page,false);assert.deepEqual((await rpc(page,'GET',`/screens/${screenId}`)).document,before.document);
    const invalid=Buffer.from(binary);invalid[Math.floor(invalid.length/2)]^=255;
    const rejected=await context.request.post(`${base}/api/runtime/components/publish`,{headers:{'content-type':'application/vnd.aio.component+gzip'},data:invalid,timeout:120000});assert(!rejected.ok());assert.equal((await entry()).active_revision,published.revision);
    const mobileContext=await browser.newContext({viewport:{width:390,height:844},isMobile:true});await mobileContext.addCookies(cookies());
    const mobile=await mobileContext.newPage();mobile.setDefaultTimeout(120000);mobile.on('pageerror',e=>errors.push(e.message));
    const mf=await openScreen(mobile,true);await mf.getByRole('button',{name:`播放${name}`,exact:true}).click();await mf.locator('.player').waitFor();
    const mobileCanvas=mf.locator('[data-kind="bar"] canvas');await mobileCanvas.waitFor();await mobile.waitForTimeout(700);
    const mobilePixels=await pixels(mobileCanvas);assert(mobilePixels.painted>50);
    assert(await mobile.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await mobile.screenshot({path:path.join(output,'screen-mobile.png')});
    await mobileContext.close();
    await rpc(page,'DELETE',`/screens/${screenId}`);screenId=null;await rpc(page,'DELETE',`/datasets/${datasetId}`);datasetId=null;
    await marketplace(page,false);await page.getByRole('textbox',{name:'搜索插件',exact:true}).fill('数据大屏');await page.getByRole('treeitem').filter({hasText:'数据大屏'}).click();
    await page.getByLabel('管理插件',{exact:true}).click();await page.getByRole('menuitem',{name:'卸载',exact:true}).click();
    await page.getByRole('button',{name:'确认卸载',exact:true}).click();await page.getByRole('button',{name:'安装',exact:true}).waitFor();
    assert(!(await entry()).installed);installedByTest=false;assert.deepEqual(errors,[]);
    await page.screenshot({path:path.join(output,'marketplace-final.png')});
    const report={base,revision:published.revision,source:installed.source_id,available:true,downloadVerified:true,installedThroughUI:true,csvImport:true,dragAndBind:true,publish:true,reload:true,badPackageRejected:true,uninstalledThroughUI:true,desktopPixels,mobilePixels,errors};
    fs.writeFileSync(path.join(output,'report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));
  }catch(error){
    await page.screenshot({path:path.join(output,'failure.png')}).catch(()=>{});
    if(installedByTest){
      try{
        const screen=(await rpc(page,'GET','/screens')).find(s=>s.title===name);
        if(screen)await rpc(page,'DELETE',`/screens/${screen.id}`);
        const dataset=(await rpc(page,'GET','/datasets')).find(d=>d.name===name);
        if(dataset)await rpc(page,'DELETE',`/datasets/${dataset.id}`);
      }catch(cleanup){console.error(`Fixture cleanup incomplete: ${cleanup.message.split('Call log:')[0]}`);}
      try{const current=await entry();if(current?.installed)assert((await context.request.post(`${base}/api/runtime/plugins/${current.source_id}/uninstall`)).ok());}
      catch(cleanup){console.error(`Installation cleanup incomplete: ${cleanup.message.split('Call log:')[0]}`);}
    }
    console.error(JSON.stringify({errors}));throw error;
  }
  finally{await browser.close();}
}
run().catch(e=>{console.error(e.message.split('Call log:')[0]);process.exitCode=1;});
