// Static contract checks for installer preflight guards. They never touch services,
// drivers, adapters, or real system paths, so they are safe in CI.
import assert from 'node:assert/strict';
import {existsSync} from 'node:fs';
import {readFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {resolve} from 'node:path';

const root=resolve(import.meta.dirname,'..','..');
const windows=await readFile(resolve(root,'packaging/windows/install-agent.ps1'),'utf8');
const clientInstaller=await readFile(resolve(root,'packaging/windows/install-client.ps1'),'utf8');
const clientUninstaller=await readFile(resolve(root,'packaging/windows/uninstall-client.ps1'),'utf8');
const installerHooks=await readFile(resolve(root,'packaging/windows/installer-hooks.nsh'),'utf8');
const windowsBuild=await readFile(resolve(root,'scripts/build/windows.ps1'),'utf8');
const phase2Preflight=await readFile(resolve(root,'scripts/test/phase2-preflight.ps1'),'utf8');
const tauriConfig=JSON.parse(await readFile(resolve(root,'apps/desktop/src-tauri/tauri.conf.json'),'utf8'));
const rootPackage=JSON.parse(await readFile(resolve(root,'package.json'),'utf8'));
const desktopPackage=JSON.parse(await readFile(resolve(root,'apps/desktop/package.json'),'utf8'));
const desktopApp=await readFile(resolve(root,'apps/desktop/src/App.vue'),'utf8');
const workspaceCargo=await readFile(resolve(root,'Cargo.toml'),'utf8');
const linux=await readFile(resolve(root,'packaging/linux/install-agent.sh'),'utf8');
const linuxSetup=await readFile(resolve(root,'packaging/linux/setup-agent.sh'),'utf8');
const linuxUpgrade=await readFile(resolve(root,'packaging/linux/upgrade-agent.sh'),'utf8');
const linuxInstallerInputs=await readFile(resolve(root,'scripts/test/linux-installer-inputs.sh'),'utf8');
const linuxInstallRollback=await readFile(resolve(root,'scripts/test/linux-install-rollback.sh'),'utf8');
const linuxUpgradeRollback=await readFile(resolve(root,'scripts/test/linux-upgrade-rollback.sh'),'utf8');
const linuxUninstall=await readFile(resolve(root,'packaging/linux/uninstall-agent.sh'),'utf8');
const linuxUninstallOwnership=await readFile(resolve(root,'scripts/test/linux-uninstall-ownership.sh'),'utf8');
const desktopCargo=await readFile(resolve(root,'apps/desktop/src-tauri/Cargo.toml'),'utf8');
const linuxBundle=JSON.parse(await readFile(resolve(root,'apps/desktop/src-tauri/tauri.linux.conf.json'),'utf8'));
const linuxBuild=await readFile(resolve(root,'scripts/build/linux.mjs'),'utf8');
const desktopRust=await readFile(resolve(root,'apps/desktop/src-tauri/src/main.rs'),'utf8');
const ciWorkflow=await readFile(resolve(root,'.github/workflows/check.yml'),'utf8');

const cargoVersion=workspaceCargo.match(/\[workspace\.package\][\s\S]*?\nversion\s*=\s*"([^"]+)"/)?.[1];
assert(cargoVersion,'Cargo workspace version is missing');
assert.equal(rootPackage.version,desktopPackage.version,'Root and desktop npm versions must match');
assert.equal(rootPackage.version,tauriConfig.version,'npm and Tauri bundle versions must match');
assert.equal(rootPackage.version,cargoVersion,'UI, Agent, and Rust core versions must match');
assert.notEqual(rootPackage.version,'0.1.0','Current development installer must not be confused with the old 0.1.0 package');
assert.match(desktopApp,/import desktopPackage from '\.\.\/package\.json'/,'Desktop UI must read its displayed version from package metadata');
assert.match(desktopApp,/客户端 \{\{ appVersion \}\}/,'Desktop footer must render the package version');
assert.match(desktopApp,/Agent \{\{ agentVersion\.label \}\}/,'Desktop footer must render the running Agent version');
assert.doesNotMatch(desktopApp,/<footer>[^\r\n]*\b\d+\.\d+\.\d+\b/,'Desktop footer must not hard-code a release version');

for(const expected of ['TincSha256','Get-FileHash','-Algorithm SHA256','tinc version 1\\.0\\.36','Get-NetAdapter','TAP-Windows Adapter V9']) assert.match(windows,new RegExp(expected));
assert.doesNotMatch(windows,/\$_\.Extension -eq '\.dll'/,'A single-file Tinc approval must not copy arbitrary DLLs');
assert.match(windows,/interface_prefix = 32/);
for(const expected of [
  'AGENT_BINARY must be an absolute path','OWNER_UID must identify a non-root desktop user',
  'getent passwd','Linux interface names must not exceed 15 characters','tinc version 1\\.0\\.36',
  'test -x /usr/sbin/ip','iproute2','test -x /usr/bin/ping','command -v systemctl','/dev/net/tun',
  'Refusing symlinked installation directories','Existing systemd unit: refusing overwrite','"interface_prefix":32'
]) assert.match(linux,new RegExp(expected));
assert(windows.indexOf('Get-NetAdapter')<windows.indexOf('New-Item -ItemType Directory'),'Windows preflight must run before creating protected install directories');
assert(linux.indexOf('/dev/net/tun')<linux.indexOf('install -d -m 755'),'Linux preflight must run before installation writes');
assert.match(linuxInstallerInputs,/Expected installer rejection/,'Linux installer input regressions must remain covered in CI');
assert.match(linuxInstallRollback,/Install rollback left a managed file/,'Linux partial-install rollback must remain covered in CI');
assert.match(linuxUpgradeRollback,/Rollback did not restore the old Agent/,'Linux upgrade rollback must remain covered in CI');
assert.match(linuxUninstallOwnership,/Untrusted Agent service reached systemctl/,'Linux uninstall ownership checks must remain covered in CI');
const debFiles=linuxBundle.bundle?.linux?.deb?.files??{};
for(const expected of [
  '/usr/share/tinc-connect/payload/vpn-agent',
  '/usr/share/tinc-connect/payload/setup-agent.sh',
  '/usr/share/tinc-connect/payload/install-agent.sh',
  '/usr/share/tinc-connect/payload/upgrade-agent.sh',
  '/usr/share/tinc-connect/payload/uninstall-agent.sh',
  '/usr/share/tinc-connect/payload/tinc-client-agent.service'
]) assert.equal(typeof debFiles[expected],'string',`Linux DEB must include ${expected}`);
for(const expected of ['PKEXEC_UID','SUDO_UID','Unable to identify the non-root desktop user',`exec /bin/sh "$installer" "$agent" "$owner_uid" tinc`])
  assert.match(linuxSetup,new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
for(const expected of ['restore_previous','upgrade_started=1','upgrade_succeeded=1','systemctl stop tinc-client-agent.service','systemctl is-active --quiet tinc-client-agent.service','Existing settings, profiles and keys were retained'])
  assert.match(linuxUpgrade,new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
assert(linuxUpgrade.indexOf('install -m 755 "$agent_target" "$backup_dir/vpn-agent"')<linuxUpgrade.indexOf('systemctl stop tinc-client-agent.service'),'Linux upgrade must back up the Agent before stopping or replacing it');
for(const expected of ['stat -c %u','-perm /022','grep -Fxc "ExecStart=$agent_target"','systemctl disable --now tinc-client-agent.service'])
  assert.match(linuxUninstall,new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
for(const expected of ["'--release'", "'-p', 'vpn-agent'", "'--bundles', 'deb'", "chmodSync(stagedAgent, 0o755)", "spawnSync('dpkg-deb'", 'Debian package is missing required payload'])
  assert.match(linuxBuild,new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
for(const expected of ['/usr/bin/pkexec','/usr/share/tinc-connect/payload/setup-agent.sh','metadata.uid() == 0','mode() & 0o022 == 0','unsupported_platform_action'])
  assert.match(desktopRust,new RegExp(expected.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));
assert.match(desktopCargo,/\[features\][\s\S]*default\s*=\s*\["custom-protocol"\][\s\S]*custom-protocol\s*=\s*\["tauri\/custom-protocol"\]/,'Release desktop must embed frontend assets through Tauri custom protocol');
assert.match(ciWorkflow,/apt-get install[^\r\n]*\blibxdo-dev\b/,'Ubuntu CI must install the Linux desktop xdo development dependency');
assert.doesNotMatch(ciWorkflow,/npm run desktop:build:windows/,'Ordinary CI cannot build the offline installer without the approved external Tinc payload');
assert.match(ciWorkflow,/runner\.os == 'Windows'[\s\S]*cargo build --locked --release -p vpn-agent -p tinc-desktop/,'Windows CI must still compile the release Agent and desktop binaries');
assert.match(ciWorkflow,/runner\.os == 'Linux'[\s\S]*npm run desktop:build:linux/,'Linux CI must build the DEB with its Agent payload');
assert.match(ciWorkflow,/sudo sh scripts\/test\/linux-install-rollback\.sh/,'Linux CI must execute the isolated Agent install rollback test');
assert.match(ciWorkflow,/sudo sh scripts\/test\/linux-upgrade-rollback\.sh/,'Linux CI must execute the isolated Agent rollback test');
assert.match(ciWorkflow,/sudo sh scripts\/test\/linux-uninstall-ownership\.sh/,'Linux CI must execute the isolated Agent uninstall ownership test');
for(const expected of [
  '746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C',
  '1C44E77AB148DDFB174DE8041100EFC38C4F23500183FAFF88F9BADAC5782E3C',
  'Get-AuthenticodeSignature','VerifyOnly','Ensure-TapAdapter',
  'Get-AdapterGuidValue','New-Service','Wait-ServiceState','Wait-AgentPipe','installer.log','interface_prefix = 32',
  'ProgramW6432','Get-NativeProgramFiles'
]) assert.match(clientInstaller,new RegExp(expected));
assert.doesNotMatch(clientInstaller,/InterfaceGuid\.Guid/,'Get-NetAdapter exposes InterfaceGuid as a string on supported Windows builds');
assert.match(clientInstaller,/\$SettingsPath\s*=\s*Join-Path \$DataRoot "settings\.json"/,'Installer must write the exact settings file read by the Windows Agent');
assert.doesNotMatch(clientInstaller,/agent-settings\.json|owner_windows_sid/,'Installer must not use obsolete Agent settings names');
assert.match(clientInstaller,/\n\s*owner = \$ownerSid/,'Installer settings must use the Rust Settings.owner contract');
assert.match(clientInstaller,/expectedSettingNames = @\("data_dir", "interface", "interface_prefix", "owner", "tinc_binary"\)/,'Installer must verify the complete settings contract before starting the service');
for(const variable of ['named','renamed','installedAdapter']){
  assert.match(clientInstaller,new RegExp(`\\$${variable} = @\\(Get-TincNamedAdapter\\)`),`${variable} must remain an array for zero, one, or multiple adapters under StrictMode`);
}
assert.match(clientInstaller,/@\(Compare-Object -ReferenceObject \$expectedSettingNames -DifferenceObject \$settingNames\)\.Count/,'Settings differences must remain an array under StrictMode');
assert.match(clientInstaller,/\[IO\.File\]::WriteAllText\(\$temporaryPath, \$json, \(New-Object Text\.UTF8Encoding\(\$false\)\)\)/,'Installer JSON must be UTF-8 without BOM for Rust serde_json');
assert.doesNotMatch(clientInstaller,/ConvertTo-Json[^\r\n|]*\|\s*Set-Content[^\r\n]*-Encoding UTF8/,'Windows PowerShell 5.1 Set-Content UTF8 adds a BOM that Rust rejects');
assert(clientInstaller.indexOf('Copy-Item -LiteralPath $agentPath -Destination (Join-Path $rollbackRoot "vpn-agent.exe")')<clientInstaller.indexOf('Installing the verified Agent and Tinc runtime.'),'Upgrade backup must be created before replacing the Agent');
assert.match(clientInstaller,/Set-ProtectedDirectoryAcl -Path \$rollbackRoot/,'Upgrade backup must inherit neither ordinary-user ACLs nor parent permissions');
assert.match(clientInstaller,/Restore-UpgradeFile[\s\S]*-DestinationPath \$agentPath[\s\S]*-DestinationPath \$installedTincPath[\s\S]*-DestinationPath \$SettingsPath[\s\S]*-DestinationPath \$InstallStatePath/,'Failed upgrades must restore runtime and settings together');
assert.match(clientInstaller,/Stop-Service -Name \$ServiceName -Force[\s\S]*Restore-UpgradeFile/,'Rollback must stop a partially started replacement Agent before restoring its executable');
assert.doesNotMatch(clientInstaller,/Start-Service -Name \$ServiceName -ErrorAction SilentlyContinue/,'Rollback must not report success when the previous service cannot restart');
assert(clientInstaller.includes("O=OpenVPN Inc\\."),'Installer must constrain the TAP Authenticode signer');
assert.match(clientUninstaller,/ProgramData\\TincClient[\s\S]*preserve/i,'Uninstaller must preserve user network data and key material');
assert.match(clientUninstaller,/PathName -ne \$expectedCommand/,'Uninstaller must not delete a same-named unexpected service');
assert.match(clientUninstaller,/ProgramW6432/,'Uninstaller must resolve the native Program Files directory when launched by 32-bit NSIS');
assert.match(clientUninstaller,/\$AgentRoot\s*=\s*Join-Path \(Get-NativeProgramFiles\) "TincClientAgent"/,'Uninstaller must validate the same native Agent location used by the installer');
assert.match(installerHooks,/NSIS_HOOK_POSTINSTALL[\s\S]*install-client\.ps1[\s\S]*NSIS_HOOK_PREUNINSTALL[\s\S]*uninstall-client\.ps1/);
assert.equal(tauriConfig.bundle.windows.nsis.installMode,'perMachine','Driver/service installer must request machine-level elevation');
assert.equal(tauriConfig.bundle.windows.nsis.installerHooks,'../../../packaging/windows/installer-hooks.nsh');
for(const expected of ['vpn-agent.exe','tinc\\\\tincd.exe','COPYING.txt','tap-windows-9.24.7-I601-Win10.exe','payload-manifest.json','-VerifyOnly']) assert.match(windowsBuild,new RegExp(expected));
assert.doesNotMatch(windowsBuild,/latest/i,'Build must use a pinned TAP-Windows artifact, not a moving latest URL');
for(const forbidden of ['Stop-Service','Start-Service','New-NetRoute','Remove-NetRoute','Remove-NetAdapter','Disable-NetAdapter','Enable-NetAdapter','Copy-Item','Move-Item','Remove-Item']){
  assert.doesNotMatch(phase2Preflight,new RegExp(forbidden),`Phase 2 preflight must remain read-only: ${forbidden}`);
}
assert.doesNotMatch(phase2Preflight,/Get-Content[^\r\n]*(settings|install-state|adapter-state|installer\.log)/i,'Preflight must not read configuration, credentials, keys, or log contents');
for(const expected of ['upgrade_required','ready_for_upgrade','ready_for_vpn_test','Get-NetAdapter','Get-NetRoute','Get-Sha256']) assert.match(phase2Preflight,new RegExp(expected));

if(process.platform==='win32'){
  const parser='$tokens=$null;$errors=$null;[System.Management.Automation.Language.Parser]::ParseFile($env:TINC_CLIENT_SCRIPT,[ref]$tokens,[ref]$errors)|Out-Null;if($errors.Count){$errors|ForEach-Object{$_.ToString()};exit 1}';
  for(const script of ['packaging/windows/install-agent.ps1','packaging/windows/install-client.ps1','packaging/windows/uninstall-client.ps1','scripts/build/windows.ps1','scripts/test/phase2-preflight.ps1']){
    const parsed=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',parser],{encoding:'utf8',env:{...process.env,TINC_CLIENT_SCRIPT:resolve(root,script)}});
    assert.equal(parsed.status,0,`${script}: ${parsed.stderr||parsed.stdout}`);
  }
  const cardinality=`
Set-StrictMode -Version Latest
function Get-MockedAdapters([int]$CaseCount) {
  for ($index = 0; $index -lt $CaseCount; $index++) {
    [pscustomobject]@{ Name = "adapter-$index" }
  }
}
foreach ($expected in @(0, 1, 2)) {
  $named = @(Get-MockedAdapters -CaseCount $expected)
  if ($named.Count -ne $expected) { throw "Expected $expected adapters, got $($named.Count)." }
  if ($expected -gt 0 -and $named[0].Name -ne 'adapter-0') { throw 'Array indexing changed adapter semantics.' }
}`;
  const cardinalityCheck=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',cardinality],{encoding:'utf8'});
  assert.equal(cardinalityCheck.status,0,cardinalityCheck.stderr||cardinalityCheck.stdout);
  const utf8NoBom=`
$path = [IO.Path]::GetTempFileName()
try {
  $json = [ordered]@{ owner = 'S-1-5-21-1001' } | ConvertTo-Json
  [IO.File]::WriteAllText($path, $json, (New-Object Text.UTF8Encoding($false)))
  $bytes = [IO.File]::ReadAllBytes($path)
  if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) { throw 'Unexpected UTF-8 BOM.' }
  $parsed = Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
  if ($parsed.owner -ne 'S-1-5-21-1001') { throw 'JSON round trip failed.' }
} finally {
  [IO.File]::Delete($path)
}`;
  const utf8NoBomCheck=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',utf8NoBom],{encoding:'utf8'});
  assert.equal(utf8NoBomCheck.status,0,utf8NoBomCheck.stderr||utf8NoBomCheck.stdout);
  const rollbackRestore=`
Set-StrictMode -Version Latest
$tokens=$null
$errors=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile($env:TINC_CLIENT_SCRIPT,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw $errors[0] }
$functionAst=$ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Restore-UpgradeFile' },$true)
if ($null -eq $functionAst) { throw 'Restore-UpgradeFile was not found.' }
. ([scriptblock]::Create($functionAst.Extent.Text))
$root=Join-Path ([IO.Path]::GetTempPath()) ('tinc-rollback-test-'+[guid]::NewGuid().ToString('N'))
try {
  New-Item -ItemType Directory -Path $root | Out-Null
  $backup=Join-Path $root 'agent.backup'
  $destination=Join-Path $root 'agent.exe'
  [IO.File]::WriteAllText($backup,'old')
  [IO.File]::WriteAllText($destination,'new')
  Restore-UpgradeFile -BackupPath $backup -DestinationPath $destination -ExistedBeforeUpgrade $true
  if ((Get-Content -LiteralPath $destination -Raw) -ne 'old') { throw 'Existing file was not restored.' }
  $created=Join-Path $root 'created-during-upgrade.json'
  [IO.File]::WriteAllText($created,'new')
  Restore-UpgradeFile -BackupPath (Join-Path $root 'unused') -DestinationPath $created -ExistedBeforeUpgrade $false
  if (Test-Path -LiteralPath $created) { throw 'New upgrade-only file was not removed.' }
} finally {
  Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}`;
  const rollbackRestoreCheck=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',rollbackRestore],{encoding:'utf8',env:{...process.env,TINC_CLIENT_SCRIPT:resolve(root,'packaging/windows/install-client.ps1')}});
  assert.equal(rollbackRestoreCheck.status,0,rollbackRestoreCheck.stderr||rollbackRestoreCheck.stdout);
  const nativeProgramFiles=`
Set-StrictMode -Version Latest
$tokens=$null
$errors=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile($env:TINC_CLIENT_SCRIPT,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw $errors[0] }
$functionAst=$ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-NativeProgramFiles' },$true)
if ($null -eq $functionAst) { throw 'Get-NativeProgramFiles was not found.' }
. ([scriptblock]::Create($functionAst.Extent.Text))
$expected=$env:ProgramW6432
if ([string]::IsNullOrWhiteSpace($expected)) { $expected=$env:ProgramFiles }
$actual=Get-NativeProgramFiles
if (-not $actual.Equals([IO.Path]::GetFullPath($expected).TrimEnd('\\'),[StringComparison]::OrdinalIgnoreCase)) {
  throw "Expected native Program Files '$expected', got '$actual'."
}`;
  const nativeShell=resolve(process.env.SystemRoot,'SysWOW64','WindowsPowerShell','v1.0','powershell.exe');
  const powershellForNativePath=existsSync(nativeShell)?nativeShell:'powershell.exe';
  for(const script of ['packaging/windows/install-client.ps1','packaging/windows/uninstall-client.ps1']){
    const nativePathCheck=spawnSync(powershellForNativePath,['-NoProfile','-NonInteractive','-Command',nativeProgramFiles],{encoding:'utf8',env:{...process.env,TINC_CLIENT_SCRIPT:resolve(root,script)}});
    assert.equal(nativePathCheck.status,0,`${script}: ${nativePathCheck.stderr||nativePathCheck.stdout}`);
  }
  const upgradeDecision=`
Set-StrictMode -Version Latest
$tokens=$null
$errors=$null
$ast=[System.Management.Automation.Language.Parser]::ParseFile($env:TINC_CLIENT_SCRIPT,[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw $errors[0] }
$functionAst=$ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Get-UpgradeRequired' },$true)
if ($null -eq $functionAst) { throw 'Get-UpgradeRequired was not found.' }
. ([scriptblock]::Create($functionAst.Extent.Text))
$expected=[ordered]@{ exists=$true; accessible=$true; sha256='EXPECTED' }
$missing=[ordered]@{ exists=$false; accessible=$true; sha256=$null }
$inaccessible=[ordered]@{ exists=$null; accessible=$false; sha256=$null }
$matching=[ordered]@{ exists=$true; accessible=$true; sha256='EXPECTED' }
$different=[ordered]@{ exists=$true; accessible=$true; sha256='DIFFERENT' }
if ((Get-UpgradeRequired -ExpectedAgent $expected -InstalledAgent $missing) -ne $true) { throw 'Missing Agent must require an upgrade.' }
if ($null -ne (Get-UpgradeRequired -ExpectedAgent $expected -InstalledAgent $inaccessible)) { throw 'Inaccessible Agent must produce an unknown upgrade decision.' }
if ((Get-UpgradeRequired -ExpectedAgent $expected -InstalledAgent $matching) -ne $false) { throw 'Matching Agent must not require an upgrade.' }
if ((Get-UpgradeRequired -ExpectedAgent $expected -InstalledAgent $different) -ne $true) { throw 'Different Agent must require an upgrade.' }`;
  const upgradeDecisionCheck=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',upgradeDecision],{encoding:'utf8',env:{...process.env,TINC_CLIENT_SCRIPT:resolve(root,'scripts/test/phase2-preflight.ps1')}});
  assert.equal(upgradeDecisionCheck.status,0,upgradeDecisionCheck.stderr||upgradeDecisionCheck.stdout);
  const phase2Check=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',resolve(root,'scripts/test/phase2-preflight.ps1')],{encoding:'utf8'});
  assert.equal(phase2Check.status,0,phase2Check.stderr||phase2Check.stdout);
  const phase2Report=JSON.parse(phase2Check.stdout);
  assert.equal(phase2Report.schema_version,2);
  assert.equal(phase2Report.client_version,rootPackage.version);
  assert(phase2Report.upgrade_required===null||typeof phase2Report.upgrade_required==='boolean');
  assert.equal(typeof phase2Report.ready_for_upgrade,'boolean');
  assert.equal(typeof phase2Report.ready_for_vpn_test,'boolean');
}
if(process.platform!=='win32'){
  for(const script of ['install-agent.sh','setup-agent.sh','upgrade-agent.sh','uninstall-agent.sh']){
    const parsed=spawnSync('sh',['-n',resolve(root,'packaging/linux',script)],{encoding:'utf8'});
    assert.equal(parsed.status,0,parsed.stderr||parsed.stdout);
  }
}
console.log('PASS installer preflight guards and Windows PowerShell syntax');
