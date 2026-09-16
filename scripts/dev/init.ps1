param(
    [Parameter(Mandatory=$true)][string]$TincBinary,
    [Parameter(Mandatory=$true)][string]$Interface
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$runtimeRoot = Join-Path $projectRoot '.runtime'
$settingsPath = Join-Path $runtimeRoot 'settings.json'
if (Test-Path -LiteralPath $settingsPath) { throw '已有开发设置，拒绝覆盖；请直接编辑 .runtime/settings.json。' }
if ($Interface -notmatch '^[a-zA-Z0-9_. -]{1,128}$') { throw '网卡名包含不支持的字符或长度不正确。' }
$resolvedTinc = (Resolve-Path -LiteralPath $TincBinary -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $resolvedTinc -PathType Leaf)) { throw 'TincBinary 必须指向 tincd.exe 文件。' }
New-Item -ItemType Directory -Path $runtimeRoot -Force | Out-Null
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$settings = @{ data_dir = (Join-Path $runtimeRoot 'data'); tinc_binary = $resolvedTinc; interface = $Interface; interface_prefix = 32; owner = $sid }
[IO.File]::WriteAllText($settingsPath, ($settings | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
Write-Output '开发设置已创建。普通权限开发服务用于协议验证；网卡设置仍需要系统权限。'
