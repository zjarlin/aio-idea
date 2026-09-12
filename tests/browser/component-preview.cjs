const fs=require('node:fs');
const path=require('node:path');
const {spawn}=require('node:child_process');
const root=path.resolve('target/component-delivery');
const profile=JSON.parse(fs.readFileSync(path.join(root,'rehearsal.json'),'utf8'));
if(!/^aio_component_host_test_[0-9]+$/.test(profile.database))throw new Error('Only an isolated database copy is accepted');
const child=spawn(path.resolve('target/debug/aio-idea'),[],{
  stdio:'inherit',env:{...process.env,AIO_DATABASE_URL:profile.url,AIO_COMPONENT_DATABASE_URL:profile.url,
    AIO_COMPONENT_HOME:path.join(root,'runtime'),AIO_PLUGIN_CACHE:path.join(root,'cache'),AIO_FILE_STORAGE_DIR:path.join(root,'files'),
    AIO_WEB_HOST:'127.0.0.1',AIO_WEB_PORT:String(profile.port),AIO_PUBLIC_ORIGIN:`http://127.0.0.1:${profile.port}`,
    AIO_PLUGIN_PUBLISH_ACCOUNTS:'zjarlin',AIO_DELIVERY_TOKEN:'',AIO_SESSION_SECURE:'0'},
});
process.on('SIGTERM',()=>child.kill('SIGTERM'));
process.on('SIGINT',()=>child.kill('SIGINT'));
child.on('exit',code=>{process.exitCode=code||0;});
