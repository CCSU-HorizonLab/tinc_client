#Requires -RunAsAdministrator
param(
    [Parameter(Mandatory=$true)][string]$AgentBinary,
    [Parameter(Mandatory=$true)][string]$TincDirectory,
    [Parameter(Mandatory=$true)][string]$OwnerSid,
    [Parameter(Mandatory=$true)][string]$Interface,
    # The installer must receive the hash approved for the exact Tinc payload.
    # Do not silently trust an arbitrary executable supplied as an installer input.
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$TincSha256
)
$ErrorActionPreference = 'Stop'
if ($OwnerSid -notmatch '^S-1-\d+(?:-\d+)+$') { throw '需要有效的交互用户 SID。' }
if ($Interface -notmatch '^[a-zA-Z0-9_. -]{1,128}$') { throw '网卡名包含不支持的字符或长度不正确。' }
if (Get-Service -Name TincClientAgent -ErrorAction SilentlyContinue) { throw '服务已存在；先停止并卸载服务，设置和密钥将保留。' }
$sourceAgent = (Resolve-Path -LiteralPath $AgentBinary).Path
$sourceTinc = (Resolve-Path -LiteralPath $TincDirectory).Path
if (-not (Test-Path -LiteralPath $sourceAgent -PathType Leaf)) { throw 'AgentBinary 必须指向 vpn-agent.exe 文件。' }
$sourceTincBinary = Join-Path $sourceTinc 'tincd.exe'
if (-not (Test-Path -LiteralPath $sourceTincBinary -PathType Leaf)) { throw 'TincDirectory 中没有 tincd.exe。' }
$actualTincSha256 = (Get-FileHash -LiteralPath $sourceTincBinary -Algorithm SHA256).Hash
if ($actualTincSha256 -ne $TincSha256.ToUpperInvariant()) { throw 'tincd.exe SHA-256 与安装器批准值不一致。' }
$tincVersion = & $sourceTincBinary --version 2>&1
if ($LASTEXITCODE -ne 0 -or (($tincVersion | Out-String) -notmatch '(?im)^tinc version 1\.0\.36(?:\s|$)')) {
    throw '当前安装器仅接受已审核的 tinc version 1.0.36。'
}
$adapter = Get-NetAdapter -Name $Interface -IncludeHidden -ErrorAction SilentlyContinue
if (-not $adapter) { throw "未找到专用 TAP 网卡 '$Interface'。请先由受信任安装程序安装驱动并创建该网卡。" }
if ($adapter.InterfaceDescription -notmatch '(?i)TAP-Windows Adapter V9') {
    throw "网卡 '$Interface' 不是受支持的 TAP-Windows Adapter V9。"
}
$installRoot = Join-Path $env:ProgramFiles 'TincClientAgent'
$dataRoot = Join-Path $env:ProgramData 'TincClient'
if (Test-Path -LiteralPath $installRoot) { throw '安装目录已存在，拒绝直接覆盖。请审核已有安装后手工处理。' }
if (Test-Path -LiteralPath (Join-Path $dataRoot 'settings.json')) { throw '已有设置，拒绝覆盖。请根据文档执行升级。' }
New-Item -ItemType Directory -Path $installRoot,$dataRoot -Force | Out-Null
# All write access is restricted before copying executable code or storing keys.
foreach ($path in @($installRoot,$dataRoot)) {
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true,$false)
    foreach ($sidText in @('S-1-5-18','S-1-5-32-544')) {
        $sidObj = New-Object Security.Principal.SecurityIdentifier($sidText)
        $rule = New-Object Security.AccessControl.FileSystemAccessRule($sidObj,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $path -AclObject $acl
}
Copy-Item -LiteralPath $sourceAgent -Destination (Join-Path $installRoot 'vpn-agent.exe')
# The approved hash covers tincd.exe only. Do not copy unverified sidecar DLLs;
# a future multi-file Tinc payload needs a separately reviewed hash manifest.
$tincRoot = Join-Path $installRoot 'tinc'
New-Item -ItemType Directory -Path $tincRoot | Out-Null
Copy-Item -LiteralPath $sourceTincBinary -Destination (Join-Path $tincRoot 'tincd.exe')
$settings = @{ data_dir = (Join-Path $dataRoot 'data'); tinc_binary = (Join-Path $tincRoot 'tincd.exe'); interface = $Interface; interface_prefix = 32; owner = $OwnerSid }
[IO.File]::WriteAllText((Join-Path $dataRoot 'settings.json'), ($settings | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
$servicePath = '"' + (Join-Path $installRoot 'vpn-agent.exe') + '" --service'
New-Service -Name TincClientAgent -DisplayName 'Tinc Client Agent' -BinaryPathName $servicePath -StartupType Automatic -Description 'Restricted local Tinc VPN management service' | Out-Null
Start-Service -Name TincClientAgent
(Get-Service -Name TincClientAgent).WaitForStatus('Running',[TimeSpan]::FromSeconds(20))
Write-Output '服务已安装并运行。已验证审核的 Tinc 1.0.36 和指定 TAP 网卡；未更改网卡地址或路由。'
