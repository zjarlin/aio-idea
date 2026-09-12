const { readFile } = require('node:fs/promises');

async function contextFor(browser,base,mobile) {
  const context=await browser.newContext({viewport:mobile?{width:390,height:844}:{width:1440,height:1000}});
  const source=await readFile(process.env.AIO_COOKIE_FILE,'utf8');
  const cookies=source.split(/\r?\n/).filter(line=>line&&(!line.startsWith('#')||line.startsWith('#HttpOnly_'))).map(line=>{
    const [, , path,secure,expires,name,value]=line.replace(/^#HttpOnly_/,'').split('\t');
    return {name,value,url:new URL(path,base).href,httpOnly:line.startsWith('#HttpOnly_'),secure:secure==='TRUE',...(Number(expires)>0?{expires:Number(expires)}:{})};
  });
  await context.addCookies(cookies);
  return context;
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
module.exports={contextFor,select,marketplace};
