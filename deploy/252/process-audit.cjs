const fs = require('node:fs');
const http = require('node:http');
const assert = require('node:assert/strict');
const {execFileSync} = require('node:child_process');
const root = '/opt/aio-idea/process-test-20260913';
const fixture = JSON.parse(fs.readFileSync(0,'utf8'));
const docker = args => execFileSync('docker',args,{encoding:'utf8',stdio:['ignore','pipe','pipe']}).trim();
const containers = docker(['ps','--filter','label=io.addzero.aio.v2=true','--format','{{.Names}}']).split('\n').filter(Boolean);
const container = containers.find(name=>{
  const mounts=JSON.parse(docker(['inspect','--format','{{json .Mounts}}',name]));
  return mounts.some(m=>m.Destination==='/grant'&&m.Source.startsWith(root+'/processes/'));
});
assert(container,'Rehearsal process not running');
const inspect = JSON.parse(docker(['inspect',container]))[0];
const grant = inspect.Mounts.find(m=>m.Destination==='/grant').Source;
const configuration = JSON.parse(fs.readFileSync(grant+'/config.json','utf8'));
const broker = inspect.Mounts.find(m=>m.Destination==='/broker').Source+'/gateway.sock';
assert.equal(inspect.HostConfig.NetworkMode,'none');
assert.equal(inspect.HostConfig.ReadonlyRootfs,true);
assert.equal(inspect.Config.User,'65532:65532');
assert(!inspect.Mounts.some(m=>m.Destination.includes('docker.sock')));
const sql = statement => execFileSync('docker',['exec','-i','aio-process-postgres-20260913','psql','-X','-U','postgres','-d','aio_process_host_test_20260913','-At','-v','ON_ERROR_STOP=1'],{input:statement,encoding:'utf8'}).trim();
const literal = value => "'"+value.replaceAll("'","''")+"'";
const user = sql(`SELECT user_id FROM component_process_actors WHERE tenant_id=${literal(configuration.tenantId||configuration.tenant_id)} LIMIT 1`);
assert(user,'Interactive actor missing');
function request(path,body,headers={}) {
  return new Promise((resolve,reject)=>{
    const req=http.request({socketPath:broker,path,method:'POST',headers:{'content-type':'application/json','x-aio-token':configuration.ingressToken||configuration.ingress_token,...headers},timeout:30000},res=>{
      const chunks=[];res.on('data',c=>chunks.push(c));res.on('end',()=>resolve({status:res.statusCode,text:Buffer.concat(chunks).toString()}));
    });req.on('error',reject);req.on('timeout',()=>req.destroy(new Error('Broker timeout')));req.end(JSON.stringify(body));
  });
}
(async()=>{
  const tenant = configuration.tenantId||configuration.tenant_id;
  const input={target:'https://github.com/zjarlin/aio-plugin-agent-memory.git',method:'POST',path:`/secrets/${fixture.secret}/reveal`,body:null,tenantId:tenant,userId:user,interactive:true,contextId:'forged-request'};
  assert.equal((await request('/invoke',input)).status,403);
  const background=await request('/invoke',{...input,interactive:false,contextId:null});
  assert.equal(background.status,200);assert.equal(JSON.parse(background.text).status,403);
  assert.equal((await request('/invoke',{...input,tenantId:'other-tenant'})).status,403);
  assert.equal((await request('/invoke',{...input,userId:'other-user'})).status,403);
  assert.equal((await request('/invoke',{...input,target:'https://github.com/zjarlin/aio-plugin-screen.git'})).status,403);
  const body={model:'gpt-4o-mini',stream:true,messages:[{role:'user',content:'AIO synthetic transport probe'}]};
  assert.equal((await request('/egress',body,{'x-aio-endpoint':'https://example.test/v1'})).status,502);
  const egress=await request('/egress',body,{'x-aio-endpoint':'https://api.openai.com/v1',authorization:'Bearer aio-invalid-transport-test'});
  assert([401,403,429,502].includes(egress.status),`Approved transport: HTTP ${egress.status}`);
  const network = docker(['exec',container,'node','-e','fetch("https://example.com",{signal:AbortSignal.timeout(3000)}).then(()=>process.exit(1)).catch(()=>console.log("isolated"))']);
  assert.equal(network,'isolated');
  assert(!docker(['logs',container]).includes(fixture.canary));
  console.log(JSON.stringify({forgedContextDenied:true,backgroundRevealDenied:true,crossTenantDenied:true,otherUserDenied:true,undeclaredServiceDenied:true,undeclaredEndpointDenied:true,approvedEgressStatus:egress.status,approvedEgressReachable:egress.status!==502,networkIsolated:true,logsSanitized:true}));
})().catch(error=>{console.error(error.message);process.exitCode=1;});
