const assert=require('node:assert/strict');
const {mkdir,writeFile}=require('node:fs/promises');
const {resolve}=require('node:path');
const {contextFor,select,getJson,launchBrowser,closeBrowser}=require('./live-session.cjs');
const base=process.env.AIO_URL||'https://aio.addzero.site';
const scenario=process.env.AIO_DELIVERY_SCENARIO||'e2e';
assert(['e2e','acceptance'].includes(scenario));
const plugins=[
  {label:'TypeScript Delivery E2E',front:'TypeScript Counter',back:'请求后端 +1',result:'1'},
  {label:'Kotlin Delivery E2E',front:'KMP Workspace',back:'Tasks',result:'Tenant: default'},
  {label:'Rust Delivery E2E',front:'Rust Delivery E2E',back:'请求后端 +1',result:'服务端结果：1'},
].map(plugin=>scenario==='acceptance'?{...plugin,label:plugin.label.replace('E2E','Acceptance'),front:plugin.front.replace('E2E','Acceptance')}:plugin);

(async()=>{
  const output=resolve('target/delivery-test');await mkdir(output,{recursive:true});
  const browser=await launchBrowser();
  const context=await contextFor(browser,base,false);const page=await context.newPage();const errors=[];const responses=[];const network=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('requestfailed',request=>network.push({url:request.url(),failure:request.failure()}));
  page.on('console',message=>{if(message.type()==='error')network.push({console:message.text()});});
  page.on('response',response=>{if(/\/api\/runtime\/frontend\/[^/]+\/request$/.test(response.url()))responses.push(response.json().then(body=>({status:response.status(),path:response.request().postDataJSON().path,body})));});
  try {
    await page.goto(base);await page.locator('.application-shell:visible').waitFor();
    await page.getByRole('navigation',{name:'场景'}).getByRole('button',{name:'社区插件',exact:true}).click();
    const reports=[];
    const selected=plugins.filter(plugin=>!process.env.AIO_DELIVERY_LANGUAGE||plugin.label.toLowerCase().startsWith(process.env.AIO_DELIVERY_LANGUAGE));
    assert(selected.length>0,'unknown delivery language');
    for(const plugin of selected){
      const firstResponse=responses.length;
      await select(page,false,plugin.label);const frame=page.frameLocator(`iframe[title="${plugin.label}"]`);
      await frame.getByText(plugin.front,{exact:true}).first().waitFor({timeout:180000});
      if(plugin.label.startsWith('Kotlin')){
        await frame.getByText(plugin.back,{exact:true}).first().waitFor();
        await frame.getByText(plugin.result,{exact:false}).first().waitFor({timeout:30000});
      }else{
        if(plugin.label.startsWith('TypeScript'))await frame.locator('#request').evaluate(element=>new Promise((resolve,reject)=>{
          const deadline=Date.now()+180000;
          const interval=setInterval(()=>{if(element.onclick){clearInterval(interval);resolve();}else if(Date.now()>deadline){clearInterval(interval);reject(new Error('frontend initialization timed out'));}},100);
        }));
        await frame.getByRole('button',{name:plugin.back,exact:true}).click();
        if(plugin.label.startsWith('TypeScript'))await frame.locator('#backend').filter({hasText:'1'}).waitFor({timeout:30000});
        else await frame.getByText(plugin.result,{exact:false}).first().waitFor({timeout:30000});
      }
      const iframe=page.locator(`iframe[title="${plugin.label}"]`);
      const calls=await Promise.all(responses.slice(firstResponse));assert(calls.length>0);assert(calls.every(call=>call.status===200&&call.body.data.status===200));
      const market=await getJson(context,`${base}/api/runtime/marketplace`);
      const entry=market.data.find(entry=>entry.title===plugin.label);assert(entry?.installed);assert.equal(entry.active_revision,entry.rev);
      const details=await getJson(context,`${base}/api/runtime/marketplace/${entry.rev}/details`);
      reports.push({label:plugin.label,observedAt:Date.now(),source:await iframe.getAttribute('src'),entry,details:details.data,calls});
      await page.screenshot({path:resolve(output,`${scenario}-${plugin.label.split(' ')[0].toLowerCase()}-cli-live.png`)});
    }
    assert.deepEqual(errors,[]);assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await writeFile(resolve(output,`${scenario}-${process.env.AIO_DELIVERY_LANGUAGE||'all'}-cli-live-report.json`),JSON.stringify(reports,null,2));
    console.log(JSON.stringify(reports));
  }catch(error){
    const frames=await Promise.all(page.frames().filter(frame=>frame!==page.mainFrame()).map(async frame=>({url:frame.url(),text:await frame.locator('body').innerText().catch(()=>''),buttons:await frame.getByRole('button').allTextContents()})));
    await page.screenshot({path:resolve(output,`${scenario}-${process.env.AIO_DELIVERY_LANGUAGE||'all'}-cli-failure.png`)});
    console.error(JSON.stringify({frames,calls:await Promise.all(responses),errors,network}));
    throw error;
  }finally{await closeBrowser(browser);}
})().catch(error=>{console.error(error);process.exitCode=1;});
