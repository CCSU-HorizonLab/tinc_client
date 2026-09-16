// Real Windows Named Pipe / Linux UDS + real agent / crypto / filesystem.
// The remote HTTP backend is a controlled fixture, never a production server.
import {createServer} from 'node:http';
import {connect} from 'node:net';
import {spawn} from 'node:child_process';
import {generateKeyPairSync,createHash,randomBytes} from 'node:crypto';
import {mkdtemp,mkdir,writeFile,readFile,readdir} from 'node:fs/promises';
import {resolve,join} from 'node:path';
import assert from 'node:assert/strict';
import {once} from 'node:events';

const root=resolve(import.meta.dirname,'..','..');
const desktopMode=process.argv.includes('--desktop');
const binary=resolve(process.argv.slice(2).find(a=>!a.startsWith('--'))??join(root,'target/debug',process.platform==='win32'?'vpn-agent.exe':'vpn-agent'));
const ipcInstance=`e2e-${process.pid}-${randomBytes(6).toString('hex')}`;
const endpoint=process.platform==='win32'?String.raw`\\.\pipe\tinc-client-dev-v1-${ipcInstance}`:`/tmp/tinc-client-${process.getuid()}-${ipcInstance}.sock`;
const delay=ms=>new Promise(r=>setTimeout(r,ms));
let desktopPage,desktopProcess,viteProcess,browser;
function rpc(request){
 if(desktopPage)return desktopPage.evaluate(async request=>{
  const {command,...args}=request;if('allow_http' in args){args.allowHttp=args.allow_http;delete args.allow_http;}
  try{return {kind:command==='get_recent_logs'?'logs':'status',data:await window.__TAURI_INTERNALS__.invoke(command,args)}}catch(error){return {kind:'error',data:error}}
 },request);
 return new Promise((resolve,reject)=>{
 const socket=connect(endpoint);let bytes=Buffer.alloc(0);let finished=false;
 const done=(err,value)=>{if(finished)return;finished=true;socket.destroy();err?reject(err):resolve(value);};
 socket.setTimeout(140000,()=>done(new Error('IPC timed out')));
 socket.on('error',err=>done(err));socket.on('close',()=>{if(!finished)done(new Error('IPC closed without reply'));});
 socket.on('connect',()=>{const payload=Buffer.from(JSON.stringify(request));const head=Buffer.alloc(4);head.writeUInt32BE(payload.length);socket.write(Buffer.concat([head,payload]));});
 socket.on('data',chunk=>{bytes=Buffer.concat([bytes,chunk]);if(bytes.length>=4){const size=bytes.readUInt32BE();if(size>65536)return done(new Error('oversized reply'));if(bytes.length>=size+4){try{done(null,JSON.parse(bytes.subarray(4,size+4)))}catch(e){done(e)}}}});
});}
async function openDesktop(){
 const {chromium}=await import('@playwright/test');
 viteProcess=spawn(process.execPath,[join(root,'node_modules/vite/bin/vite.js'),'--host','127.0.0.1','--port','1420'],{cwd:join(root,'apps/desktop'),windowsHide:true,stdio:'ignore'});
 for(let i=0;i<100;i++){try{if((await fetch('http://127.0.0.1:1420')).ok)break;}catch{}await delay(100);}
 const webviewData=join(work,'webview2');
 desktopProcess=spawn(join(root,'target/debug/tinc-desktop.exe'),[],{windowsHide:true,stdio:'ignore',env:{...process.env,TINC_CLIENT_DEV:'1',TINC_CLIENT_DEV_INSTANCE:ipcInstance,WEBVIEW2_USER_DATA_FOLDER:webviewData,WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=9223 --remote-debugging-address=127.0.0.1 --user-data-dir=${webviewData}`}});
 for(let i=0;i<100;i++){try{browser=await chromium.connectOverCDP('http://127.0.0.1:9223');break;}catch{}await delay(200);}
 assert(browser,'WebView2 debug endpoint did not become available');
 for(let i=0;i<100;i++){desktopPage=browser.contexts().flatMap(c=>c.pages()).find(p=>p.url().includes('127.0.0.1:1420'));if(desktopPage)break;await delay(100);}
 assert(desktopPage,'Tauri webview page missing');await desktopPage.getByRole('heading',{name:'连接，从这里开始'}).waitFor();
 await desktopPage.screenshot({path:join(work,'desktop-initial.png'),fullPage:true});
}
function zip(files){
 const locals=[],centrals=[];let offset=0;
 for(const [name,text] of Object.entries(files)){
  const filename=Buffer.from(name),data=Buffer.from(text);let crc=0xffffffff;
  for(const byte of data){crc^=byte;for(let j=0;j<8;j++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}crc=(crc^0xffffffff)>>>0;
  const header=Buffer.alloc(30);header.writeUInt32LE(0x04034b50);header.writeUInt16LE(20,4);header.writeUInt32LE(crc,14);header.writeUInt32LE(data.length,18);header.writeUInt32LE(data.length,22);header.writeUInt16LE(filename.length,26);
  const central=Buffer.alloc(46);central.writeUInt32LE(0x02014b50);central.writeUInt16LE(20,4);central.writeUInt16LE(20,6);central.writeUInt32LE(crc,16);central.writeUInt32LE(data.length,20);central.writeUInt32LE(data.length,24);central.writeUInt16LE(filename.length,28);central.writeUInt32LE(offset,42);
  locals.push(header,filename,data);centrals.push(central,filename);offset+=header.length+filename.length+data.length;
 }
 const directory=Buffer.concat(centrals),end=Buffer.alloc(22);end.writeUInt32LE(0x06054b50);end.writeUInt16LE(Object.keys(files).length,8);end.writeUInt16LE(Object.keys(files).length,10);end.writeUInt32LE(directory.length,12);end.writeUInt32LE(offset,16);
 return Buffer.concat([...locals,directory,end]);
}
await mkdir(join(root,'.runtime'),{recursive:true});const work=await mkdtemp(join(root,'.runtime/e2e-'));
const publicKey=generateKeyPairSync('rsa',{modulusLength:2048,publicKeyEncoding:{type:'pkcs1',format:'pem'},privateKeyEncoding:{type:'pkcs1',format:'pem'}}).publicKey;
let mode='ok',uploads=0;const uploaded=[];let releaseDownload;
const backend=createServer(async(req,res)=>{
 try{
  const chunks=[];for await(const c of req)chunks.push(c);const body=JSON.parse(Buffer.concat(chunks));
  if(req.url==='/api/tinc/client/login'){
   if(body.password!=='fixture-password'){res.setHeader('Content-Type','application/json');return res.end(JSON.stringify({status:0,msg:'rejected'}));}
   assert.deepEqual(Object.keys(body).sort(),['password','sid']);return res.end(JSON.stringify({status:1,sid:'client_a',token:'fixture-token',net_name:'office',node_ip:'10.42.0.2'}));
  }
  if(req.url==='/api/tinc/client/config/download'){
   assert.deepEqual(body,{sid:'client_a',token:'fixture-token'});
   if(mode==='expired'){res.statusCode=401;return res.end();}
   if(mode==='pause'){await new Promise(r=>{releaseDownload=r;});}
   res.setHeader('Content-Type','application/zip');return res.end(zip({'office/tinc.conf':'Name = client_a\nConnectTo = server_master\n',...(mode==='unsafe'?{'../victim':'escape'}:{})}));
  }
  if(req.url==='/api/tinc/client/key/upload'){
   assert.deepEqual(Object.keys(body).sort(),['action','content','sid','token']);assert.equal(body.sid,'client_a');assert.equal(body.token,'fixture-token');assert.equal(body.action,'exchangeFile');assert(body.content.startsWith('Subnet = 10.42.0.2/32\n\n-----BEGIN RSA PUBLIC KEY-----\n'));assert(!body.content.includes('PRIVATE KEY'));uploads++;uploaded.push(body.content);
   if(uploads===1){res.statusCode=503;return res.end('fixture upload failure');}
   res.setHeader('Content-Type','text/plain; charset=UTF-8');res.setHeader('Cache-Control','no-store');return res.end(`Address = 127.0.0.1\nSubnet = 10.42.0.1/32\n${publicKey}`);
  }
  res.statusCode=404;res.end('unexpected endpoint');
 }catch(e){res.statusCode=500;res.end('fixture assertion failed');console.error(e);}
});
backend.listen(0,'127.0.0.1');await once(backend,'listening');const url=`http://127.0.0.1:${backend.address().port}`;
await writeFile(join(work,'settings.json'),JSON.stringify({data_dir:join(work,'data'),tinc_binary:join(work,'missing-tincd.exe'),owner:'unused-in-dev',interface:'tinc_test',interface_prefix:32}));
let child;
async function startAgent(){child=spawn(binary,['--dev',join(work,'settings.json')],{windowsHide:true,stdio:['ignore','ignore','pipe'],env:{...process.env,TINC_CLIENT_DEV_INSTANCE:ipcInstance}});child.stderr.on('data',c=>process.stderr.write(c));for(let i=0;i<100;i++){if(child.exitCode!==null)throw new Error(`agent exited ${child.exitCode}`);try{const reply=await rpc({command:'get_status'});if(reply.kind==='status')return reply;}catch{}await delay(100);}throw new Error('agent did not start');}
async function stopAgent(){if(child&&child.exitCode===null){const stopped=once(child,'exit');child.kill();await stopped;}}
async function ok(command,extra={}){const r=await rpc({command,...extra});assert.equal(r.kind,'status',JSON.stringify(r));return r.data;}
async function login(password='fixture-password'){return rpc({command:'login',server:url,sid:'client_a',password,allow_http:true});}
const passed=[];function pass(label){passed.push(label);console.log(`PASS ${label}`);}
try{
 assert.equal((await startAgent()).data.phase,'unconfigured');pass('agent starts with actual unconfigured state');
 if(desktopMode){await openDesktop();assert.equal((await rpc({command:'get_status'})).kind,'status');pass('real Tauri WebView2 renders and calls authenticated native IPC');}
 assert.equal((await login('wrong')).data.code,'auth_failed');pass('authentication rejection surfaces');
 assert.equal((await login()).kind,'status');
 assert.equal((await rpc({command:'configure'})).data.code,'http');pass('upload failure is not configuration success');
 const profile=(await readdir(join(work,'data/keys')))[0],privatePath=join(work,'data/keys',profile,'rsa_key.priv');
 const hash=async()=>createHash('sha256').update(await readFile(privatePath)).digest('hex');const before=await hash();
 assert.equal((await ok('configure')).phase,'stopped');assert.equal(await hash(),before);assert.equal(uploaded[0],uploaded[1]);pass('retry reuses real RSA private key and identical public upload');
 const activePath=join(work,'data/active.json');const active=await readFile(activePath,'utf8');
 mode='unsafe';assert.equal((await rpc({command:'configure'})).kind,'error');assert.equal(await readFile(activePath,'utf8'),active);pass('unsafe archive rejected and previous configuration preserved');
 mode='pause';const pending=rpc({command:'configure'});while(!releaseDownload)await delay(20);
 assert.equal((await ok('get_status')).phase,'configuring');assert.equal((await rpc({command:'configure'})).data.code,'busy');releaseDownload();mode='ok';assert.equal((await pending).kind,'status');pass('duplicate configure rejected while status remains readable');
 assert.equal((await rpc({command:'start_vpn'})).data.code,'tinc_missing');pass('missing real Tinc binary is a visible failure');
 mode='expired';assert.equal((await rpc({command:'configure'})).data.code,'auth_expired');assert.equal((await ok('get_status')).session,null);pass('expired session cleared while retained configuration survives');
 const logs=(await rpc({command:'get_recent_logs'})).data;assert(!JSON.stringify(logs).includes('fixture-password'));assert(!JSON.stringify(logs).includes('fixture-token'));assert(!JSON.stringify(logs).includes('PRIVATE KEY'));pass('logs exclude credentials and private keys');
 await stopAgent();assert.equal((await startAgent()).data.phase,'stopped');assert.equal((await ok('get_status')).session,null);pass('agent restart recovers committed config without inventing a live session');
 await stopAgent();
 const settings=JSON.parse(await readFile(join(work,'settings.json'),'utf8'));settings.tinc_binary=join(root,'target/debug/examples',process.platform==='win32'?'process_fixture.exe':'process_fixture');await writeFile(join(work,'settings.json'),JSON.stringify(settings));
 await startAgent();assert.equal((await rpc({command:'start_vpn'})).data.code,'start_failed');assert.equal((await ok('get_status')).process_running,false);pass('early child-process exit is a start failure, never a running connection');
 if(desktopPage){await desktopPage.getByRole('button',{name:'刷新状态'}).click();await delay(500);await desktopPage.screenshot({path:join(work,'desktop-final.png'),fullPage:true});}
 console.log(`Completed ${passed.length} local integration checks. Backend was a fixture; no real VPN connection claimed.`);
 await writeFile(join(work,'result.json'),JSON.stringify({checks:passed,remoteBackend:'fixture',realVpn:false},null,2));
}finally{if(releaseDownload)releaseDownload();desktopPage=null;if(browser)await browser.close();if(desktopProcess)desktopProcess.kill();if(viteProcess)viteProcess.kill();await stopAgent();backend.closeAllConnections();backend.close();}
