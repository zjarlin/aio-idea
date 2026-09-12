const { readFile, writeFile } = require('node:fs/promises');
const {chromium}=require('playwright');

const launchBrowser=()=>chromium.launch({channel:'chrome',headless:true,args:['--disable-quic',...(process.env.AIO_BROWSER_HTTP1==='1'?['--disable-http2']:[])]});
const grants=new WeakMap();

async function contextFor(browser,base,mobile) {
  const context=await browser.newContext({viewport:mobile?{width:390,height:844}:{width:1440,height:1000}});
  const source=await readFile(process.env.AIO_COOKIE_FILE,'utf8');
  const cookies=source.split(/\r?\n/).filter(line=>line&&(!line.startsWith('#')||line.startsWith('#HttpOnly_'))).map(line=>{
    const [, , path,secure,expires,name,value]=line.replace(/^#HttpOnly_/,'').split('\t');
    return {name,value,url:new URL(path,base).href,httpOnly:line.startsWith('#HttpOnly_'),secure:secure==='TRUE',...(Number(expires)>0?{expires:Number(expires)}:{})};
  });
  await context.addCookies(cookies);
  const tickets=new Set();grants.set(context,{base,tickets});
  context.on('response',response=>{
    if(response.url()===`${base}/api/runtime/frontend/mount`&&response.ok())void response.json().then(body=>tickets.add(body.data.token)).catch(()=>{});
  });
  return context;
}
async function closeContext(context) {
  const issued=grants.get(context);
  if(issued)await Promise.allSettled([...issued.tickets].map(token=>context.request.delete(`${issued.base}/api/runtime/frontend/${token}`,{timeout:15000})));
  await context.close();
}
async function closeBrowser(browser) {
  await Promise.allSettled(browser.contexts().map(closeContext));
  await browser.close();
}
async function select(page,mobile,label) {
  if(mobile)await page.getByRole('button',{name:'打开菜单',exact:true}).click();
  const nav=mobile?page.getByRole('dialog'):page.locator('.application-shell__sidebar');
  await nav.getByRole('button',{name:label,exact:true}).click();
}
async function marketplace(page,mobile) {
  if(mobile)await page.getByRole('button',{name:'打开菜单',exact:true}).click();
  const nav=mobile?page.getByRole('dialog'):page.locator('.application-shell__sidebar');
  await nav.locator('button[aria-label$="的账户菜单"]').click();
  await page.getByRole('menuitem',{name:'插件市场',exact:true}).click();
  await page.getByRole('treeitem').first().waitFor();
}
async function getJson(context,url) {
  for(let attempt=0;attempt<3;attempt++) {
    try {
      const response=await context.request.get(url,{timeout:20000});
      if(response.ok())return await response.json();
      if(response.status()<500)throw new Error(`GET ${new URL(url).pathname}: HTTP ${response.status()}`);
    } catch(error) {
      if(attempt===2||/HTTP 4\d\d/.test(error.message))throw new Error(`Read failed: ${new URL(url).pathname}`);
    }
    await new Promise(resolve=>setTimeout(resolve,2000*(attempt+1)));
  }
  throw new Error(`Read failed: ${new URL(url).pathname}`);
}
async function viewportScreenshot(page, path) {
  const session = await page.context().newCDPSession(page);
  try {
    const {data} = await session.send('Page.captureScreenshot', {format: 'png', captureBeyondViewport: false});
    await writeFile(path, Buffer.from(data, 'base64'));
  } finally {await session.detach();}
}
module.exports={contextFor,select,marketplace,getJson,launchBrowser,closeContext,closeBrowser,viewportScreenshot};
