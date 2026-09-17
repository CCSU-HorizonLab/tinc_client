// Static contract checks for installer preflight guards. They never touch services,
// drivers, adapters, or real system paths, so they are safe in CI.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {resolve} from 'node:path';

const root=resolve(import.meta.dirname,'..','..');
const windows=await readFile(resolve(root,'packaging/windows/install-agent.ps1'),'utf8');
const clientInstaller=await readFile(resolve(root,'packaging/windows/install-client.ps1'),'utf8');
const clientUninstaller=await readFile(resolve(root,'packaging/windows/uninstall-client.ps1'),'utf8');
const installerHooks=await readFile(resolve(root,'packaging/windows/installer-hooks.nsh'),'utf8');
const windowsBuild=await readFile(resolve(root,'scripts/build/windows.ps1'),'utf8');
const tauriConfig=JSON.parse(await readFile(resolve(root,'apps/desktop/src-tauri/tauri.conf.json'),'utf8'));
const linux=await readFile(resolve(root,'packaging/linux/install-agent.sh'),'utf8');
const desktopCargo=await readFile(resolve(root,'apps/desktop/src-tauri/Cargo.toml'),'utf8');

for(const expected of ['TincSha256','Get-FileHash','-Algorithm SHA256','tinc version 1\\.0\\.36','Get-NetAdapter','TAP-Windows Adapter V9']) assert.match(windows,new RegExp(expected));
assert.doesNotMatch(windows,/\$_\.Extension -eq '\.dll'/,'A single-file Tinc approval must not copy arbitrary DLLs');
assert.match(windows,/interface_prefix = 32/);
for(const expected of ['AGENT_BINARY must be an absolute path','tinc version 1\\.0\\.36','test -x /usr/sbin/ip','iproute2','/dev/net/tun','"interface_prefix":32']) assert.match(linux,new RegExp(expected));
assert(windows.indexOf('Get-NetAdapter')<windows.indexOf('New-Item -ItemType Directory'),'Windows preflight must run before creating protected install directories');
assert(linux.indexOf('/dev/net/tun')<linux.indexOf('install -d -m 755'),'Linux preflight must run before installation writes');
assert.match(desktopCargo,/\[features\][\s\S]*default\s*=\s*\["custom-protocol"\][\s\S]*custom-protocol\s*=\s*\["tauri\/custom-protocol"\]/,'Release desktop must embed frontend assets through Tauri custom protocol');
for(const expected of [
  '746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C',
  '1C44E77AB148DDFB174DE8041100EFC38C4F23500183FAFF88F9BADAC5782E3C',
  'Get-AuthenticodeSignature','VerifyOnly','Ensure-TapAdapter',
  'Get-AdapterGuidValue','New-Service','Wait-ServiceState','Wait-AgentPipe','installer.log','interface_prefix = 32'
]) assert.match(clientInstaller,new RegExp(expected));
assert.doesNotMatch(clientInstaller,/InterfaceGuid\.Guid/,'Get-NetAdapter exposes InterfaceGuid as a string on supported Windows builds');
assert.match(clientInstaller,/\$SettingsPath\s*=\s*Join-Path \$DataRoot "settings\.json"/,'Installer must write the exact settings file read by the Windows Agent');
assert.doesNotMatch(clientInstaller,/agent-settings\.json|owner_windows_sid/,'Installer must not use obsolete Agent settings names');
assert.match(clientInstaller,/\n\s*owner = \$ownerSid/,'Installer settings must use the Rust Settings.owner contract');
assert.match(clientInstaller,/expectedSettingNames = @\("data_dir", "interface", "interface_prefix", "owner", "tinc_binary"\)/,'Installer must verify the complete settings contract before starting the service');
assert(clientInstaller.includes("O=OpenVPN Inc\\."),'Installer must constrain the TAP Authenticode signer');
assert.match(clientUninstaller,/ProgramData\\TincClient[\s\S]*preserve/i,'Uninstaller must preserve user network data and key material');
assert.match(clientUninstaller,/PathName -ne \$expectedCommand/,'Uninstaller must not delete a same-named unexpected service');
assert.match(installerHooks,/NSIS_HOOK_POSTINSTALL[\s\S]*install-client\.ps1[\s\S]*NSIS_HOOK_PREUNINSTALL[\s\S]*uninstall-client\.ps1/);
assert.equal(tauriConfig.bundle.windows.nsis.installMode,'perMachine','Driver/service installer must request machine-level elevation');
assert.equal(tauriConfig.bundle.windows.nsis.installerHooks,'../../../packaging/windows/installer-hooks.nsh');
for(const expected of ['vpn-agent.exe','tinc\\\\tincd.exe','COPYING.txt','tap-windows-9.24.7-I601-Win10.exe','payload-manifest.json','-VerifyOnly']) assert.match(windowsBuild,new RegExp(expected));
assert.doesNotMatch(windowsBuild,/latest/i,'Build must use a pinned TAP-Windows artifact, not a moving latest URL');

if(process.platform==='win32'){
  const parser='$tokens=$null;$errors=$null;[System.Management.Automation.Language.Parser]::ParseFile($env:TINC_CLIENT_SCRIPT,[ref]$tokens,[ref]$errors)|Out-Null;if($errors.Count){$errors|ForEach-Object{$_.ToString()};exit 1}';
  for(const script of ['packaging/windows/install-agent.ps1','packaging/windows/install-client.ps1','packaging/windows/uninstall-client.ps1','scripts/build/windows.ps1']){
    const parsed=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',parser],{encoding:'utf8',env:{...process.env,TINC_CLIENT_SCRIPT:resolve(root,script)}});
    assert.equal(parsed.status,0,`${script}: ${parsed.stderr||parsed.stdout}`);
  }
}
if(process.platform!=='win32'){
  const parsed=spawnSync('sh',['-n',resolve(root,'packaging/linux/install-agent.sh')],{encoding:'utf8'});
  assert.equal(parsed.status,0,parsed.stderr||parsed.stdout);
}
console.log('PASS installer preflight guards and Windows PowerShell syntax');
