const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {createHash} = require('node:crypto');

process.umask(0o077);
const root=path.resolve('target/component-delivery');
fs.mkdirSync(root,{recursive:true,mode:0o700});
const host=process.env.AIO_DEPLOY_HOST||'root@192.168.31.252';
const quote=value=>"'"+value.replaceAll("'","'\\''")+"'";
function remote(code){return execFileSync('ssh',['-o','BatchMode=yes','-o','ConnectTimeout=10',host,`/usr/local/bin/node -e ${quote(code)}`],{encoding:'utf8',maxBuffer:1024*1024});}
function connection(){
  const result=remote(String.raw`const fs=require('fs');const lines=fs.readFileSync('/opt/aio-idea/runtime.env','utf8').split('\n');const line=lines.find(l=>l.startsWith('AIO_DATABASE_URL='));if(!line)process.exit(1);process.stdout.write(line.slice(line.indexOf('=')+1).replace(/^["']|["']$/g,''));`);
  return new URL(result.trim());
}
function pgEnvironment(url){return {...process.env,PGHOST:url.hostname,PGPORT:url.port||'5432',PGUSER:decodeURIComponent(url.username),PGPASSWORD:decodeURIComponent(url.password),PGDATABASE:url.pathname.slice(1),PGCONNECT_TIMEOUT:'10'};}
function sql(env,statement){return execFileSync('psql',['-X','-v','ON_ERROR_STOP=1','-Atc',statement],{env,encoding:'utf8'}).trim();}

function backup(){
  const url=connection();
  const file=path.join(root,`database-${Date.now()}.dump`);
  execFileSync('pg_dump',['--format=custom','--file',file],{env:pgEnvironment(url),stdio:['ignore','ignore','pipe']});
  const report={file,sha256:createHash('sha256').update(fs.readFileSync(file)).digest('hex'),bytes:fs.statSync(file).size};
  fs.writeFileSync(path.join(root,'backup.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
}
function backupComponents(){
  const url=connection();url.pathname='/aio_plugin_components';
  const file=path.join(root,`components-${Date.now()}.dump`);
  execFileSync('pg_dump',['--format=custom','--file',file],{env:pgEnvironment(url),stdio:['ignore','ignore','pipe']});
  const report={file,sha256:createHash('sha256').update(fs.readFileSync(file)).digest('hex'),bytes:fs.statSync(file).size};
  remote(String.raw`const fs=require('fs');const cp=require('child_process');fs.mkdirSync('/opt/aio-idea/backups',{recursive:true,mode:0o700});cp.execFileSync('tar',['-C','/opt/aio-idea','-czf','/opt/aio-idea/backups/component-storage-'+Date.now()+'.tgz','component-storage']);console.log('Key storage backed up');`);
  fs.writeFileSync(path.join(root,'components-backup.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify(report));
}
function processes(){
  const image=process.env.AIO_PROCESS_IMAGE;
  if(!/^sha256:[a-f0-9]{64}$/.test(image||''))throw new Error('AIO_PROCESS_IMAGE must be the verified immutable image ID');
  console.log(remote(String.raw`const fs=require('fs');const cp=require('child_process');const image='${image}';cp.execFileSync('docker',['image','inspect',image],{stdio:'ignore'});const p='/opt/aio-idea/process.env';fs.mkdirSync('/opt/aio-idea/backups',{recursive:true,mode:0o700});let text='';if(fs.existsSync(p)){text=fs.readFileSync(p,'utf8');fs.copyFileSync(p,'/opt/aio-idea/backups/process.env.'+Date.now());}const lines=text.split('\n').filter(Boolean);const values={AIO_PROCESS_ROOT:'/opt/aio-idea/process-storage',AIO_PROCESS_IMAGES:image,AIO_PROCESS_ENDPOINTS:'https://api.openai.com/v1'};for(const [key,value]of Object.entries(values)){const at=lines.findIndex(line=>line.startsWith(key+'='));if(at<0)lines.push(key+'='+value);else if(key==='AIO_PROCESS_ROOT'){if(lines[at]!==key+'='+value)throw Error('Existing process root differs');}else{const entries=new Set(lines[at].slice(key.length+1).split(','));entries.add(value);lines[at]=key+'='+[...entries].join(',');}}fs.writeFileSync(p,lines.join('\n')+'\n',{mode:0o600});cp.execFileSync('install',['-d','-o','aio-shell','-g','aio-shell','-m','0700',values.AIO_PROCESS_ROOT]);console.log('Process storage, image and model endpoint configured');`).trim());
}
function rehearse(){
  const saved=JSON.parse(fs.readFileSync(path.join(root,'backup.json'),'utf8'));
  if(createHash('sha256').update(fs.readFileSync(saved.file)).digest('hex')!==saved.sha256)throw new Error('Backup digest changed');
  const database=`aio_component_host_test_${Date.now()}`;
  const env={...process.env,PGHOST:'/tmp',PGUSER:'postgres',PGDATABASE:database};
  execFileSync('createdb',[database],{env});
  execFileSync('pg_restore',['--no-owner','--no-acl','--exit-on-error','--dbname',database,saved.file],{env,stdio:['ignore','ignore','pipe']});
  sql(env,"REVOKE ALL ON SCHEMA public FROM PUBLIC; UPDATE tenant_plugin_bindings SET enabled=false; UPDATE plugin_runtime_instances SET state='stopped'; UPDATE plugin_publish_jobs SET state='failed' WHERE state IN ('queued','running'); UPDATE delivery_sources SET enabled=false;");
  const profile={database,url:`postgresql://postgres@localhost/${database}?host=/tmp`,port:4215,backupSha256:saved.sha256};
  fs.writeFileSync(path.join(root,'rehearsal.json'),JSON.stringify(profile,null,2));
  console.log(JSON.stringify({database,backupSha256:saved.sha256}));
}
function provision(){
  const url=connection();
  const env=pgEnvironment(url);
  const database='aio_plugin_components';
  const exists=sql(env,`SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname='${database}')`)==='t';
  if(!exists){
    sql(env,`CREATE DATABASE ${database}`);
    sql(env,`COMMENT ON DATABASE ${database} IS 'aio:plugin@2 production storage'`);
  }else if(sql(env,`SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname='${database}'`)!=='aio:plugin@2 production storage')throw new Error('Database belongs to another installation');
  sql({...env,PGDATABASE:database},'REVOKE ALL ON SCHEMA public FROM PUBLIC');
  const result=remote(String.raw`const fs=require('fs');const cp=require('child_process');const p='/opt/aio-idea/runtime.env';const text=fs.readFileSync(p,'utf8');const line=text.split('\n').find(l=>l.startsWith('AIO_DATABASE_URL='));const url=new URL(line.slice(line.indexOf('=')+1).replace(/^["']|["']$/g,''));url.pathname='/${database}';fs.mkdirSync('/opt/aio-idea/backups',{recursive:true,mode:0o700});fs.copyFileSync(p,'/opt/aio-idea/backups/runtime.env.component-'+Date.now());const lines=text.split('\n').filter(l=>!l.startsWith('AIO_COMPONENT_DATABASE_URL=')&&!l.startsWith('AIO_COMPONENT_HOME='));lines.push('AIO_COMPONENT_DATABASE_URL='+url.href,'AIO_COMPONENT_HOME=/opt/aio-idea/component-storage');fs.writeFileSync(p,lines.join('\n')+'\n',{mode:0o600});cp.execFileSync('install',['-d','-o','aio-shell','-g','aio-shell','-m','0700','/opt/aio-idea/component-storage']);console.log('Component storage configured');`);
  console.log(result.trim());
}
try {
  const action=process.argv[2];
  if(action==='backup')backup();else if(action==='backup-components')backupComponents();else if(action==='processes')processes();else if(action==='rehearse')rehearse();else if(action==='provision')provision();else throw new Error('Use backup, backup-components, processes, rehearse or provision');
}catch(error){console.error(error.stderr?.toString()||error.message);process.exitCode=1;}
