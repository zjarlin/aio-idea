const assert=require('node:assert/strict');
const {mkdir,writeFile}=require('node:fs/promises');
const {resolve}=require('node:path');
const {chromium}=require('playwright');
const {contextFor,select}=require('./live-session.cjs');
const base=process.env.AIO_URL||'https://aio.addzero.site';
const plugins=[
  {label:'TypeScript Delivery E2E',front:'TypeScript Counter',back:'请求后端 +1',result:'1'},
  {label:'Kotlin Delivery E2E',front:'KMP Workspace',back:'Tasks',result:'Tenant: default'},
  {label:'Rust Delivery E2E',front:'Rust Delivery E2E',back:'请求后端 +1',result:'服务端结果：1'},
];

(async()=>{
  const output=resolve('target/delivery-test');await mkdir(output,{recursive:true});
  const browser=await chromium.launch({channel:'chrome',headless:true});
  const context=await contextFor(browser,base,false);const page=await context.newPage();const errors=[];const responses=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('response',response=>{if(/\/api\/runtime\/frontend\/[^/]+\/request$/.test(response.url()))responses.push(response.json().then(body=>({status:response.status(),path:response.request().postDataJSON().path,body})));});
  try {
    await page.goto(base);await page.locator('.application-shell:visible').waitFor();
    await page.getByRole('navigation',{name:'场景'}).getByRole('button',{name:'社区插件',exact:true}).click();
    const reports=[];
    for(const plugin of plugins){
      const firstResponse=responses.length;
      await select(page,false,plugin.label);const frame=page.frameLocator(`iframe[title="${plugin.label}"]`);
      await frame.getByText(plugin.front,{exact:true}).first().waitFor({timeout:180000});
      if(plugin.label.startsWith('Kotlin')){
        await frame.getByText(plugin.back,{exact:true}).first().waitFor();
        await frame.getByText(plugin.result,{exact:false}).first().waitFor({timeout:30000});
      }else{
        await frame.getByRole('button',{name:plugin.back,exact:true}).click();
        if(plugin.label.startsWith('TypeScript'))await frame.locator('#backend').filter({hasText:'1'}).waitFor({timeout:30000});
        else await frame.getByText(plugin.result,{exact:false}).first().waitFor({timeout:30000});
      }
      const iframe=page.locator(`iframe[title="${plugin.label}"]`);
      const calls=await Promise.all(responses.slice(firstResponse));assert(calls.length>0);assert(calls.every(call=>call.status===200&&call.body.data.status===200));
      const market=await context.request.get(`${base}/api/runtime/marketplace`);assert(market.ok());
      const entry=(await market.json()).data.find(entry=>entry.title===plugin.label);assert(entry?.installed);assert.equal(entry.active_revision,entry.rev);
      const details=await context.request.get(`${base}/api/runtime/marketplace/${entry.rev}/details`);assert(details.ok());
      reports.push({label:plugin.label,source:await iframe.getAttribute('src'),entry,details:(await details.json()).data,calls});
      await page.screenshot({path:resolve(output,`${plugin.label.split(' ')[0].toLowerCase()}-cli-live.png`)});
    }
    assert.deepEqual(errors,[]);assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    await writeFile(resolve(output,'cli-live-report.json'),JSON.stringify(reports,null,2));
    console.log(JSON.stringify(reports));
  }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
