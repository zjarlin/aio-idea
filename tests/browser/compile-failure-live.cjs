const assert=require('node:assert/strict');
const {readFile,writeFile}=require('node:fs/promises');
const {resolve}=require('node:path');
const {launchBrowser,contextFor,getJson,closeBrowser}=require('./live-session.cjs');
const base=process.env.AIO_URL||'https://aio.addzero.site';
const git='https://github.com/zjarlin/aio-delivery-e2e-typescript.git';
(async()=>{
  const output=resolve('target/delivery-test');
  const before=JSON.parse(await readFile(resolve(output,'before-compile-failure.json'),'utf8'));
  const previous=before.versions.find(version=>version.git===git);
  const browser=await launchBrowser();
  const context=await contextFor(browser,base,false);
  let token;
  try {
    const entry=(await getJson(context,`${base}/api/runtime/marketplace`)).data.find(entry=>entry.git===git);
    assert.equal(entry.rev,previous.revision);assert.equal(entry.active_revision,previous.revision);
    const details=(await getJson(context,`${base}/api/runtime/marketplace/${entry.rev}/details`)).data;
    const failed=details.builds.find(build=>build.source_revision!==previous.source_revision&&build.state==='failed'&&build.error.includes('TS2322'));
    assert(failed,'real compiler failure must be recorded');
    const mount=await context.request.post(`${base}/api/runtime/frontend/mount`,{data:{page_id:'aio-delivery-e2e-typescript'}});
    assert(mount.ok());token=(await mount.json()).data.token;
    const response=await context.request.post(`${base}/api/runtime/frontend/${token}/request`,{data:{method:'POST',path:'/counter',body:JSON.stringify({value:2})}});
    assert(response.ok());const call=(await response.json()).data;
    assert.equal(call.status,200);assert.deepEqual(JSON.parse(call.body),{value:3,tenant_id:'default'});
    const report={observedAt:Date.now(),previous,entry,failed,call};
    await writeFile(resolve(output,'compile-failure-report.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify({failedJob:failed.id,activeRevision:entry.rev,backendResult:JSON.parse(call.body)}));
  }finally {
    if(token)await context.request.delete(`${base}/api/runtime/frontend/${token}`);
    await closeBrowser(browser);
  }
})().catch(error=>{console.error(error.message.split('Call log:')[0]);process.exitCode=1;});
