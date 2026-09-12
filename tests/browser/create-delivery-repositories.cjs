const assert=require('node:assert/strict');
const {execFileSync}=require('node:child_process');
const {mkdirSync,writeFileSync}=require('node:fs');
const {resolve}=require('node:path');
const output=resolve('target/delivery-test/acceptance-pushes.json');
const reports=[];
mkdirSync(resolve('target/delivery-test'),{recursive:true});
for(const language of ['rust','kotlin','typescript']) {
  const name=`aio-delivery-acceptance-${language}`;
  const cwd=`/tmp/${name}`;
  const run=(command,args)=>execFileSync(command,args,{cwd,encoding:'utf8'}).trim();
  assert.equal(run('git',['status','--porcelain']),'');
  assert.equal(run('git',['remote']),'');
  const source=run('git',['rev-parse','HEAD']);
  const startedAt=Date.now();
  run('gh',['repo','create',`zjarlin/${name}`,'--public','--source=.','--remote=origin','--push']);
  const pushedAt=Date.now();
  assert.equal(run('git',['ls-remote','origin','refs/heads/main']).split(/\s+/)[0],source);
  reports.push({name,language,source,startedAt,pushedAt,git:`https://github.com/zjarlin/${name}.git`});
  writeFileSync(output,JSON.stringify(reports,null,2));
  console.log(JSON.stringify(reports.at(-1)));
}
