const assert=require('node:assert/strict');
const {mkdir,writeFile}=require('node:fs/promises');
const {resolve}=require('node:path');
const {contextFor,marketplace,launchBrowser,closeContext,closeBrowser}=require('./live-session.cjs');
const base=process.env.AIO_URL||'https://aio.addzero.site';
const output=resolve('target/delivery-test');
const temporaryGit='https://github.com/zjarlin/aio-delivery-e2e-typescript.git';

async function entry(context){
  const response=await context.request.get(`${base}/api/runtime/marketplace`);assert(response.ok());
  const plugin=(await response.json()).data.find(item=>item.git===temporaryGit);assert(plugin);return plugin;
}
async function run(browser,mobile){
  const context=await contextFor(browser,base,mobile);const page=await context.newPage();const errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  try{
    await page.goto(base);await page.locator('.application-shell:visible').waitFor();await marketplace(page,mobile);
    const search=page.getByRole('textbox',{name:'搜索插件',exact:true});
    if(!mobile){
      const handle=page.getByRole('separator',{name:'调整插件列表宽度'});await handle.focus();
      const before=Number(await handle.getAttribute('aria-valuenow'));await page.keyboard.press('ArrowRight');
      assert.equal(Number(await handle.getAttribute('aria-valuenow')),Math.min(420,before+10));
    }
    await search.fill('任务工作台');await page.waitForFunction(()=>document.querySelectorAll('[role=treeitem]').length===1);
    await page.getByRole('treeitem').click();await page.locator('.dx-markdown h1').waitFor();
    assert.equal(await page.locator('.extension-browser__heading h1').innerText(),'任务工作台示例');
    await page.waitForFunction(()=>{const image=document.querySelector('.dx-markdown img');return image?.complete&&image.naturalWidth>0;});
    assert(await page.locator('.extension-browser__detail').evaluate(element=>element.scrollWidth<=element.clientWidth),'detail content must fit its pane');
    await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-marketplace-live.png`)});
    if(mobile){
      await page.getByRole('button',{name:'插件列表',exact:true}).click();assert.equal(await search.inputValue(),'任务工作台');
      await page.getByRole('treeitem').click();await page.getByRole('tab',{name:'版本记录',exact:true}).click();
      await page.locator('.extension-browser__versions li').first().waitFor();
    }else{
      await search.fill('TypeScript Delivery E2E');await page.waitForFunction(()=>document.querySelectorAll('[role=treeitem]').length===1);
      await page.getByRole('treeitem').click();assert.equal((await entry(context)).git,temporaryGit);
      await page.getByLabel('管理插件',{exact:true}).click();
      await page.getByRole('menuitem',{name:'停用',exact:true}).click();
      await page.locator('.extension-browser__actions').getByText('已停用',{exact:true}).waitFor();
      await page.getByRole('menuitem',{name:'启用',exact:true}).click();
      await page.locator('.extension-browser__actions').getByText('已启用',{exact:true}).waitFor();
      const downloadEvent=page.waitForEvent('download');await page.getByRole('menuitem',{name:'下载插件包',exact:true}).click();
      const download=await downloadEvent;assert.equal(await download.failure(),null);
      await page.getByRole('menuitem',{name:'卸载',exact:true}).click();await page.getByRole('dialog').waitFor();
      await page.getByRole('button',{name:'确认卸载',exact:true}).click();await page.getByRole('button',{name:'安装',exact:true}).waitFor();
      assert.equal((await entry(context)).installed,false);
      await page.getByRole('button',{name:'安装',exact:true}).click();
      await page.locator('.extension-browser__actions').getByText('已启用',{exact:true}).waitFor();
      const latest=await entry(context);assert.equal(latest.active_revision,latest.rev);
    }
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));assert.deepEqual(errors,[]);
    return {viewport:mobile?'mobile':'desktop',readmeImage:true,search:true,navigation:true,management:!mobile};
  }catch(error){await page.screenshot({path:resolve(output,`${mobile?'mobile':'desktop'}-marketplace-live-failure.png`)});throw error;}
  finally{await closeContext(context);}
}
(async()=>{
  await mkdir(output,{recursive:true});const browser=await launchBrowser();
  try{const report=[await run(browser,false),await run(browser,true)];await writeFile(resolve(output,'marketplace-live-report.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report));}
  finally{await closeBrowser(browser);}
})().catch(error=>{console.error(error);process.exitCode=1;});
