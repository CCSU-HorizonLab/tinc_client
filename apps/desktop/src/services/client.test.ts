import {describe,it,expect} from 'vitest';

import {connectionView,controls,failure,isHttpOrigin,type Status} from './client';
const status:Status={phase:'failed',session:{sid:'n',net_name:'net',node_ip:'10.0.0.2'},configured:{sid:'n',net_name:'net',node_ip:'10.0.0.2'},process_running:false,connection:'unknown',confirmation:'unavailable',stage:'',last_error:null};
describe('failure recovery controls',()=>{
 it('allows retry and starting retained config after failed update',()=>expect(controls(status,false)).toEqual({configure:true,start:true,stop:false}));
 it('prevents duplicate operations while configuring',()=>expect(controls({...status,phase:'configuring'},false)).toEqual({configure:false,start:false,stop:false}));
 it('allows stopping a running process after session logout',()=>expect(controls({...status,session:null,process_running:true,phase:'running'},false)).toEqual({configure:false,start:false,stop:true}));
 it('does not invent state before service connection',()=>expect(controls(null,false)).toEqual({configure:false,start:false,stop:false}));
 it('does not surface untrusted exception contents',()=>expect(failure('secret token')).toEqual({code:'desktop',message:'桌面接口调用失败，请检查本地服务并重试',retryable:true}));
});
describe('server address controls',()=>{
 it('recognizes HTTP after whitespace and with uppercase scheme',()=>{
  expect(isHttpOrigin('  HTTP://127.0.0.1:8081')).toBe(true);
  expect(isHttpOrigin('https://vpn.example.com')).toBe(false);
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
