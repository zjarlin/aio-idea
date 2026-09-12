const fs = require('node:fs');
const path = require('node:path');
const {execFileSync, spawn} = require('node:child_process');
const {randomBytes} = require('node:crypto');

process.umask(0o077);
const root = '/opt/aio-idea/process-test-20260913';
const profileFile = path.join(root, 'profile.json');
const image = 'sha256:8ae75514bb53a1386dd623789f02cdc7648bcc2d37fe6dcef2fb684c15d1eadc';
const endpoints = 'https://api.openai.com/v1';
const hostDatabase = 'aio_process_host_test_20260913';
const componentDatabase = 'aio_process_data_test_20260913';
const output = (...args) => execFileSync(...args).toString().trim();
const quote = value => '"' + value.replaceAll('"', '""') + '"';
let stage = 'initialize';

async function prepare() {
  if (fs.existsSync(profileFile)) throw new Error('Rehearsal already prepared');
  const dump = '/tmp/aio-process-rehearsal-20260913.dump';
  stage = 'snapshot';
  if (!fs.existsSync(path.join(root, 'snapshot.dump'))) throw new Error('Verified database backup is required');
  fs.mkdirSync(root, {recursive: true, mode: 0o700});
  const password = randomBytes(32).toString('hex');
  const target = 'aio-process-postgres-20260913';
  const envFile = path.join(root, 'postgres.env');
  stage = 'isolated PostgreSQL';
  fs.writeFileSync(envFile, `POSTGRES_PASSWORD=${password}\n`, {mode: 0o600});
  execFileSync('docker', ['network', 'create', '--internal', 'aio-process-test-db'], {stdio: ['ignore', 'ignore', 'pipe']});
  execFileSync('docker', ['run', '-d', '--name', target, '--network', 'aio-process-test-db', '--publish', '127.0.0.1:55433:5432', '--env-file', envFile, 'postgres:17-bookworm'], {stdio: ['ignore', 'ignore', 'pipe']});
  let ready = false;
  for (let i = 0; i < 60; i++) {
    try { execFileSync('docker', ['exec', target, 'pg_isready', '-U', 'postgres'], {stdio: 'ignore'}); ready = true; break; } catch {}
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  if (!ready) throw new Error('Rehearsal PostgreSQL did not start');
  const sql = (name, text) => output('docker', ['exec', '-i', target, 'psql', '-X', '-U', 'postgres', '-d', name, '-v', 'ON_ERROR_STOP=1', '-At'], {input: text});
  for (const name of [hostDatabase, componentDatabase]) {
    stage = 'create rehearsal databases';
    if (sql('postgres', `SELECT count(*) FROM pg_database WHERE datname='${name}'`) !== '0') throw new Error('Rehearsal database already exists');
    sql('postgres', `CREATE DATABASE ${quote(name)}`);
    sql(name, 'REVOKE ALL ON SCHEMA public FROM PUBLIC');
  }
  stage = 'restore';
  execFileSync('docker', ['cp', path.join(root, 'snapshot.dump'), `${target}:${dump}`], {stdio: ['ignore', 'ignore', 'pipe']});
  execFileSync('docker', ['exec', target, 'pg_restore', '-U', 'postgres', '--no-owner', '--no-acl', '--exit-on-error', '-d', hostDatabase, dump], {stdio: ['ignore', 'ignore', 'pipe']});
  sql(hostDatabase, "REVOKE ALL ON SCHEMA public FROM PUBLIC; UPDATE tenant_plugin_bindings SET enabled=false; UPDATE component_installations SET enabled=false; UPDATE plugin_runtime_instances SET state='stopped'; UPDATE plugin_publish_jobs SET state='failed' WHERE state IN ('queued','running'); UPDATE delivery_sources SET enabled=false;");
  stage = 'configuration';
  const address = output('docker', ['inspect', '--format', '{{(index .NetworkSettings.Networks "aio-process-test-db").IPAddress}}', target]);
  const hostUrl = new URL(`postgresql://postgres:${password}@${address}:5432/${hostDatabase}`);
  const componentUrl = new URL(hostUrl); componentUrl.pathname = '/' + componentDatabase;
  fs.mkdirSync(root, {recursive: true, mode: 0o700});
  fs.writeFileSync(profileFile, JSON.stringify({hostDatabase, componentDatabase, hostUrl: hostUrl.href, componentUrl: componentUrl.href, root, image, endpoints}), {mode: 0o600});
  for (const name of ['components', 'processes', 'cache', 'files']) fs.mkdirSync(path.join(root, name), {recursive: true, mode: 0o700});
  fs.writeFileSync(path.join(root, 'aio.toml'), 'plugins = []\n[application]\nname = "aio-idea"\ntitle = "AIO 演练"\n');
  execFileSync('chown', ['-R', 'aio-shell:aio-shell', root]);
  console.log(JSON.stringify({hostDatabase, componentDatabase, prepared: true}));
}

function stop() {
  for (const name of ['host', 'supervisor']) {
    const file = path.join(root, name + '.pid');
    if (fs.existsSync(file)) {
      const pid = Number(fs.readFileSync(file, 'utf8'));
      let executable;
      try { executable = fs.realpathSync(`/proc/${pid}/exe`); } catch {}
      if (executable === path.join(root, 'aio-idea')) process.kill(pid, 'SIGTERM');
      fs.unlinkSync(file);
    }
  }
}

async function start() {
  const profile = JSON.parse(fs.readFileSync(profileFile, 'utf8'));
  if (profile.hostDatabase !== hostDatabase || profile.componentDatabase !== componentDatabase) throw new Error('Not the isolated rehearsal');
  const address = output('docker', ['inspect', '--format', '{{(index .NetworkSettings.Networks "aio-process-test-db").IPAddress}}', 'aio-process-postgres-20260913']);
  for (const key of ['hostUrl','componentUrl']) { const url=new URL(profile[key]); url.hostname=address; url.port='5432'; profile[key]=url.href; }
  const uid = Number(output('id', ['-u', 'aio-shell']));
  const gid = Number(output('id', ['-g', 'aio-shell']));
  const env = {...process.env, AIO_DATABASE_URL: profile.hostUrl, AIO_COMPONENT_DATABASE_URL: profile.componentUrl, AIO_COMPONENT_HOME: path.join(root, 'components'), AIO_PROCESS_ROOT: path.join(root, 'processes'), AIO_PROCESS_IMAGES: profile.image, AIO_PROCESS_ENDPOINTS: profile.endpoints, AIO_PLUGIN_SUPERVISOR_SOCKET: path.join(root, 'supervisor.sock'), AIO_PLUGIN_CACHE: path.join(root, 'cache'), AIO_FILE_STORAGE_DIR: path.join(root, 'files'), AIO_CONFIG: path.join(root, 'aio.toml'), AIO_WEB_HOST: '127.0.0.1', AIO_WEB_PORT: '4245', AIO_PUBLIC_ORIGIN: 'http://127.0.0.1:4245', AIO_WEB_DIST: '/opt/aio-idea/current/web', AIO_PLUGIN_PUBLISH_ACCOUNTS: 'zjarlin', AIO_DELIVERY_TOKEN: '', AIO_SESSION_SECURE: '0'};
  for (const name of ['supervisor', 'host']) {
    const log = fs.openSync(path.join(root, name + '.log'), 'a', 0o600);
    const previous = process.umask(0o007);
    const child = spawn(path.join(root, 'aio-idea'), name === 'supervisor' ? ['supervisor'] : [], {cwd: root, env, uid: name === 'host' ? uid : 0, gid, detached: true, stdio: ['ignore', log, log]});
    process.umask(previous);
    child.unref();
    fs.writeFileSync(path.join(root, name + '.pid'), String(child.pid));
    if (name === 'supervisor') await new Promise(resolve => setTimeout(resolve, 1500));
  }
  for (let i = 0; i < 120; i++) {
    try { if ((await fetch('http://127.0.0.1:4245/health')).ok) { console.log('Rehearsal ready on 4245'); return; } } catch {}
    await new Promise(resolve => setTimeout(resolve, 1000));
  }
  throw new Error('Rehearsal health check failed');
}

(async () => {
  if (process.argv[2] === 'prepare') await prepare();
  else if (process.argv[2] === 'start') await start();
  else if (process.argv[2] === 'stop') stop();
  else throw new Error('Use prepare, start or stop');
})().catch(error => { fs.mkdirSync(root, {recursive:true,mode:0o700}); fs.writeFileSync(path.join(root, 'rehearsal-error.log'), String(error.stack) + '\n' + (error.stderr?.toString() || ''), {mode:0o600}); console.error(`Process rehearsal failed at ${stage}; details are in the private server log`); process.exitCode = 1; });
