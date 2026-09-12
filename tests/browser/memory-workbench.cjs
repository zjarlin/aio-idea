const assert = require('node:assert/strict');
const path = require('node:path');
const {mkdirSync, writeFileSync} = require('node:fs');
const {PNG} = require('pngjs');
const {launchBrowser, contextFor, closeBrowser} = require('./live-session.cjs');

const base = process.env.AIO_URL || 'https://aio.addzero.site';
const directory = path.resolve('target/component-delivery/memory-workbench');
mkdirSync(directory, {recursive:true});
async function run() {
  const browser = await launchBrowser();
  try {
    const context = await contextFor(browser,base,false);
    const page = await context.newPage();
    page.setDefaultTimeout(120000);
    const errors = [];
    const responses = [];
    const redact = value=>value.replace(/\/components\/assets\/[^/]+/g,'/components/assets/[token]').replace(/\/components\/[^/]+\/(request|renew)/g,'/components/[token]/$1');
    page.on('pageerror',error=>errors.push(redact(error.message)));
    page.on('console',message=>{if(message.type()==='error')errors.push(redact(message.text()));});
    page.on('response',response=>{
      if(response.status()>=400) errors.push(`${redact(new URL(response.url()).pathname)}: HTTP ${response.status()}`);
      if(response.url().endsWith('/request')&&response.ok()) responses.push(response.json().then(value=>{
        if(value.data.status>=400) errors.push(`Memory service: HTTP ${value.data.status}`);
      }));
    });
    await page.goto(base);
    await page.getByRole('navigation',{name:'场景'}).getByRole('button',{name:'社区插件',exact:true}).click();
    const navigation = page.locator('.application-shell__sidebar');
    const memory = navigation.getByRole('button',{name:'记忆图谱',exact:true});
    if(!await memory.isVisible()) await navigation.getByRole('button',{name:'智能体',exact:true}).click();
    const graphReady = page.waitForResponse(response=>response.url().endsWith('/request')&&response.request().postDataJSON()?.path==='/graph');
    await memory.click();
    const frame = page.frameLocator('iframe[title="记忆图谱"]');
    await frame.locator('canvas').first().waitFor();
    await frame.getByRole('button',{name:'新建记忆',exact:true}).first().waitFor();
    const graph = await graphReady;
    assert(graph.request().postDataJSON().query.startsWith('spaceId='));
    assert.equal((await graph.json()).data.status,200);
    for(const [name,viewport] of [['desktop',{width:1440,height:1000}],['mobile',{width:390,height:844}]]) {
      await page.setViewportSize(viewport);
      await page.waitForTimeout(1500);
      const png = PNG.sync.read(await page.screenshot({path:path.join(directory,`${name}.png`)}));
      let colored = 0;
      for(let i=0;i<png.data.length;i+=4) if(png.data[i+1]>png.data[i]+15&&png.data[i+1]>png.data[i+2]) colored++;
      assert(colored>100,'Memory workspace was not painted');
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    }
    await Promise.all(responses);
    assert.deepEqual(errors,[]);
    writeFileSync(path.join(directory,'report.json'),JSON.stringify({base,desktop:true,mobile:true,errors},null,2));
    console.log('Memory workbench desktop/mobile passed');
  } finally {await closeBrowser(browser);}
}
run().catch(error=>{console.error(error.message.split('Call log:')[0]);process.exitCode=1;});
