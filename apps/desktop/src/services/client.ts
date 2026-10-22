import { invoke, isTauri } from '@tauri-apps/api/core';

export interface Failure { code: string; message: string; retryable: boolean }
export interface Identity { sid: string; net_name: string; node_ip: string; node_id?: number|null; network_id?: number|null; access_code?: string|null }
export type Phase = 'unconfigured'|'configuring'|'waiting_approval'|'stopped'|'starting'|'running'|'failed';
export interface Status { agent_version: string; phase: Phase; session: Identity|null; configured: Identity|null; process_running: boolean; connection: string; confirmation: string; stage: string; last_error: Failure|null }
export interface LogEntry { time: number; message: string }
export interface ConnectionView { label: string; detail: string }
export interface AgentVersionView { label: string; upgradeRequired: boolean }
export type DesktopPlatform = 'windows'|'linux'|'unknown';
export type Command = 'login'|'configure'|'rotate_key'|'start_vpn'|'stop_vpn'|'get_status'|'get_recent_logs'|'logout'|'get_platform'|'setup_linux_agent';
export const labels: Record<Phase,string> = { unconfigured:'未配置', configuring:'配置中', waiting_approval:'等待后台确认', stopped:'已停止', starting:'启动中', running:'进程运行中', failed:'操作失败' };
export function connectionView(status: Status|null): ConnectionView {
  if (status?.connection === 'connected') return {label:'已验证连通',detail:'服务端 VPN 节点可经专用 TAP 路由到达'};
  if (status?.connection === 'unreachable') return {label:'暂不可达',detail:'Tinc 仍在运行，Agent 将继续自动探测'};
  if (status?.process_running) return {label:'正在连接',detail:'Tinc 已运行，正在等待隧道握手与连通性验证'};
  return {label:'尚未验证',detail:'进程运行不代表 VPN 已连通'};
}
export function agentVersionView(status: Status|null, appVersion:string, available:boolean):AgentVersionView {
  if (!available) return {label:'未连接',upgradeRequired:false};
  const version=status?.agent_version?.trim();
  if (!version) return {label:'旧版本',upgradeRequired:true};
  return {label:version,upgradeRequired:version!==appVersion};
}
export function canSetupAgent(platform:DesktopPlatform,available:boolean,error:Failure|null):boolean {
  return platform==='linux'&&!available&&error?.code==='agent_unavailable';
}
export function isHttpOrigin(value:string):boolean {
  return value.trim().toLowerCase().startsWith('http://');
}
export function isValidServerOrigin(value:string):boolean {
  try {
    const url=new URL(value.trim());
    return (url.protocol==='https:'||url.protocol==='http:')
      && !!url.hostname
      && !url.username
      && !url.password
      && !url.search
      && !url.hash
      && url.pathname==='/';
  } catch {
    return false;
  }
}
export function normalizeAccessCode(value:string):string {
  return value.trim().toUpperCase();
}
export function isValidAccessCode(value:string):boolean {
  return /^TL-[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{4}-[23456789ABCDEFGHJKMNPQRSTUVWXYZ]{4}$/i.test(value.trim());
}
export function failure(error: unknown): Failure {
  if (typeof error==='object' && error!==null && 'code' in error && 'message' in error) return error as Failure;
  return { code:'desktop',message:'桌面接口调用失败，请检查本地服务并重试',retryable:true };
}
export async function call<T>(command: Command,args?:Record<string,unknown>):Promise<T> {
  if (!isTauri()) throw {code:'desktop_required',message:'当前为浏览器预览。请通过 Tauri 桌面应用连接本地服务；此处不模拟 VPN 数据。',retryable:false};
  return invoke<T>(command,args);
}
export function controls(status:Status|null,busy:boolean,agentCompatible=true) {
  const running=!!status?.process_running;
  const working=busy || status?.phase==='configuring' || status?.phase==='starting';
  const session=status?.session;
  const configured=status?.configured;
  const sameProfile=!!session&&!!configured
    && session.sid===configured.sid
    && session.net_name===configured.net_name
    && session.node_ip===configured.node_ip
    && session.node_id===configured.node_id
    && session.network_id===configured.network_id;
  const revoked=status?.last_error?.code==='node_revoked';
  const changesAllowed=agentCompatible&&!working;
  return {configure:!!session&&!running&&changesAllowed,rotate:sameProfile&&!revoked&&!running&&changesAllowed,start:sameProfile&&!revoked&&!running&&changesAllowed,stop:running&&!working};
}
