import {describe,it,expect} from 'vitest';

import {agentSetupMode,agentVersionView,connectionView,controls,failure,isHttpOrigin,isValidAccessCode,isValidServerOrigin,normalizeAccessCode,type Status} from './client';
const status:Status={agent_version:'0.1.1',phase:'failed',session:{sid:'n',net_name:'net',node_ip:'10.0.0.2',node_id:101,network_id:11,access_code:'TL-7K9M-2Q4F'},configured:{sid:'n',net_name:'net',node_ip:'10.0.0.2',node_id:101,network_id:11,access_code:'TL-7K9M-2Q4F'},process_running:false,connection:'unknown',confirmation:'unavailable',stage:'',last_error:null};
describe('failure recovery controls',()=>{
 it('allows retry and starting retained config only for the logged-in profile',()=>expect(controls(status,false)).toEqual({configure:true,rotate:true,start:true,stop:false}));
 it('prevents duplicate operations while configuring',()=>expect(controls({...status,phase:'configuring'},false)).toEqual({configure:false,rotate:false,start:false,stop:false}));
 it('allows stopping a running process after session logout',()=>expect(controls({...status,session:null,process_running:true,phase:'running'},false)).toEqual({configure:false,rotate:false,start:false,stop:true}));
 it('does not invent state before service connection',()=>expect(controls(null,false)).toEqual({configure:false,rotate:false,start:false,stop:false}));
 it('rejects starting a retained profile under a different node session',()=>expect(controls({...status,session:{sid:'other',net_name:'net',node_ip:'10.0.0.3'}},false).start).toBe(false));
 it('requires reconfiguration when stable identity metadata appears after an upgrade',()=>expect(controls({...status,configured:{sid:'n',net_name:'net',node_ip:'10.0.0.2'}},false).start).toBe(false));
 it('rejects starting when the configured node address changed',()=>expect(controls({...status,configured:{...status.configured!,node_ip:'10.0.0.9'}},false).start).toBe(false));
 it('retains profile continuity when access code rotates for same node',()=>expect(controls({...status,session:{...status.session!,access_code:'TL-8H2P-9R3X'}},false)).toEqual({configure:true,rotate:true,start:true,stop:false}));
 it('rejects starting a revoked profile',()=>expect(controls({...status,last_error:{code:'node_revoked',message:'revoked',retryable:false}},false).start).toBe(false));
 it('blocks configuration and starting through an incompatible Agent',()=>expect(controls(status,false,false)).toEqual({configure:false,rotate:false,start:false,stop:false}));
 it('still allows an incompatible Agent to stop a running VPN',()=>expect(controls({...status,phase:'running',process_running:true},false,false)).toEqual({configure:false,rotate:false,start:false,stop:true}));
 it('does not surface untrusted exception contents',()=>expect(failure('secret token')).toEqual({code:'desktop',message:'桌面接口调用失败，请检查本地服务并重试',retryable:true}));
});
describe('access code specification controls',()=>{
 it('normalizes lowercase and surrounding whitespace to uppercase', ()=>{
  expect(normalizeAccessCode('  tl-7k9m-2q4f  ')).toBe('TL-7K9M-2Q4F');
 });
 it('validates correct access code structure and alphabet', ()=>{
  expect(isValidAccessCode('TL-7K9M-2Q4F')).toBe(true);
  expect(isValidAccessCode('tl-7k9m-2q4f')).toBe(true);
  expect(isValidAccessCode('TL-1111-2222')).toBe(false);
  expect(isValidAccessCode('TL-OOOO-XXXX')).toBe(false);
  expect(isValidAccessCode('TL-SHORT')).toBe(false);
  expect(isValidAccessCode('NODE_NAME_ONLY')).toBe(false);
  expect(isValidAccessCode('   ')).toBe(false);
 });
});
describe('server address controls',()=>{
 it('recognizes HTTP after whitespace and with uppercase scheme',()=>{
  expect(isHttpOrigin('  HTTP://127.0.0.1:8081')).toBe(true);
  expect(isHttpOrigin('https://vpn.example.com')).toBe(false);
 });
 it('accepts only an HTTP or HTTPS service root without credentials or URL suffixes',()=>{
  expect(isValidServerOrigin('https://vpn.example.com')).toBe(true);
  expect(isValidServerOrigin('http://8.148.13.80:83')).toBe(true);
  for (const invalid of ['https://','vpn.example.com','ftp://vpn.example.com','https://user:pass@vpn.example.com','https://vpn.example.com/api','https://vpn.example.com?token=secret','https://vpn.example.com/#fragment']) {
   expect(isValidServerOrigin(invalid)).toBe(false);
  }
 });
});
describe('connection status',()=>{
 it('only reports a verified probe as connected',()=>{
  expect(connectionView({...status,connection:'connected'}).label).toBe('已验证连通');
  expect(connectionView({...status,connection:'unreachable'}).label).toBe('暂不可达');
  expect(connectionView({...status,connection:'unknown'}).label).toBe('尚未验证');
  expect(connectionView({...status,process_running:true,connection:'unknown'}).label).toBe('正在连接');
 });
});
describe('agent version status',()=>{
 it('does not request an upgrade when UI and Agent versions match',()=>expect(agentVersionView(status,'0.1.1',true)).toEqual({label:'0.1.1',upgradeRequired:false}));
 it('marks a different Agent version as requiring an upgrade',()=>expect(agentVersionView({...status,agent_version:'0.1.0'},'0.1.1',true)).toEqual({label:'0.1.0',upgradeRequired:true}));
 it('recognizes a legacy response without a version',()=>expect(agentVersionView({...status,agent_version:''},'0.1.1',true)).toEqual({label:'旧版本',upgradeRequired:true}));
 it('does not report an upgrade while the Agent is unavailable',()=>expect(agentVersionView(null,'0.1.1',false)).toEqual({label:'未连接',upgradeRequired:false}));
});
describe('platform Agent setup',()=>{
 it('offers install only for a missing Linux Agent',()=>{
  const unavailable={code:'agent_unavailable',message:'missing',retryable:true};
  expect(agentSetupMode('linux',false,unavailable,false)).toBe('install');
  expect(agentSetupMode('windows',false,unavailable,false)).toBeNull();
  expect(agentSetupMode('linux',false,{...unavailable,code:'ipc_identity'},false)).toBeNull();
 });
 it('offers upgrade only for a connected outdated Linux Agent',()=>{
  expect(agentSetupMode('linux',true,null,true)).toBe('upgrade');
  expect(agentSetupMode('linux',true,null,false)).toBeNull();
  expect(agentSetupMode('windows',true,null,true)).toBeNull();
 });
});
